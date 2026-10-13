//! Atomic file replacement.

use std::fs;
use std::io;
use std::path::Path;

use super::{RenameFn, create_dir, os_rename, remove_entry, staging_path};
use crate::error::{Error, Result};

/// Replaces the contents of a file atomically, creating it (and missing
/// parents) if needed.
///
/// The new contents are written to a hidden temporary sibling, flushed to
/// disk, and renamed over `path`, so readers see either the old file or
/// the complete new one, never a partial write, even if the process or
/// machine fails midway. Use it for configuration, state, and `.env`
/// files.
///
/// - **Permissions.** On Unix, an existing file's permission bits are
///   copied to the new file, so a `0600` secrets file stays `0600`; a new
///   file gets the process default. On Windows the new file inherits the
///   directory's access control list.
/// - **Symbolic links.** If `path` is a link, the file it points to is
///   replaced and the link is kept, so managed dotfiles keep working.
/// - **Durability.** On Unix the parent directory is also flushed, so the
///   rename itself survives a power loss. Windows journals the rename as
///   part of the filesystem metadata.
///
/// Unlike the rest of this module, this function exists to overwrite:
/// replacing a file's contents is its purpose.
///
/// # Errors
///
/// - [`Error::Refused`] if `path` is a directory, a broken symbolic link,
///   or has no file name.
/// - [`Error::Io`] if writing, flushing, or renaming fails (including a
///   read-only target on Windows). The temporary file is removed and the
///   original file is left untouched.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-write-atomic");
/// let file = dir.join("app.conf");
/// path_lib::write_atomic(&file, "port = 7000\n")?;
/// path_lib::write_atomic(&file, "port = 7001\n")?;
/// assert_eq!(std::fs::read_to_string(&file).unwrap(), "port = 7001\n");
/// # path_lib::remove(&dir)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn write_atomic(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    write_atomic_with(path.as_ref(), contents.as_ref(), &os_rename)
}

pub(super) fn write_atomic_with(path: &Path, contents: &[u8], rename: RenameFn<'_>) -> Result<()> {
    let refuse = |reason| {
        Err(Error::Refused {
            path: path.to_path_buf(),
            reason,
        })
    };
    if path.file_name().is_none() {
        return refuse("the destination has no file name");
    }
    let existing = match fs::symlink_metadata(path) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::io("inspect", path, e)),
    };
    // Follow a symbolic link to the file it names, so the link survives.
    let target = match &existing {
        Some(meta) if meta.file_type().is_symlink() => match fs::canonicalize(path) {
            Ok(real) => return write_atomic_with(&real, contents, rename),
            Err(_) => return refuse("it is a broken symbolic link"),
        },
        Some(meta) if meta.is_dir() => return refuse("it is a directory"),
        _ => path,
    };
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_dir(parent, None)?;
    }
    let staging = staging_path(target);
    let written = write_staged(&staging, contents, existing.as_ref())
        .and_then(|()| rename(&staging, target).map_err(|e| Error::io("replace", target, e)));
    if let Err(err) = written {
        // Best-effort cleanup: the write error is what the caller needs,
        // and a leftover is a hidden, uniquely named file, never `path`.
        if let Ok(m) = fs::symlink_metadata(&staging) {
            let _cleanup = remove_entry(&staging, &m);
        }
        return Err(err);
    }
    sync_parent(target)
}

/// Writes and flushes the staging file, copying `existing`'s permissions.
pub(super) fn write_staged(
    staging: &Path,
    contents: &[u8],
    existing: Option<&fs::Metadata>,
) -> Result<()> {
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging)
        .map_err(|e| Error::io("create file", staging, e))?;
    file.write_all(contents)
        .map_err(|e| Error::io("write", staging, e))?;
    copy_permissions(staging, existing)?;
    file.sync_all().map_err(|e| Error::io("flush", staging, e))
}

#[cfg(unix)]
pub(super) fn copy_permissions(staging: &Path, existing: Option<&fs::Metadata>) -> Result<()> {
    match existing {
        Some(meta) => fs::set_permissions(staging, meta.permissions())
            .map_err(|e| Error::io("set permissions", staging, e)),
        None => Ok(()),
    }
}

/// Windows: the new file inherits the directory's ACL. Copying the
/// read-only attribute would only make the following rename fail.
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)] // Same signature as the Unix version.
pub(super) fn copy_permissions(_staging: &Path, _existing: Option<&fs::Metadata>) -> Result<()> {
    Ok(())
}

/// Flushes the directory entry for `path` so the rename is durable.
#[cfg(unix)]
pub(super) fn sync_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| Error::io("flush directory", parent, e))
}

/// Windows journals renames as filesystem metadata, and the standard
/// library cannot open a directory handle to flush it.
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)] // Same signature as the Unix version.
pub(super) fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_support::*;

    #[test]
    fn test_write_atomic_failed_rename_keeps_original() {
        let dir = scratch("wa-fail");
        let file = dir.join("conf");
        fs::write(&file, b"old").unwrap_or_default();
        let r = write_atomic_with(&file, b"new", &denied);
        assert!(matches!(r, Err(Error::Io { op: "replace", .. })), "{r:?}");
        assert_eq!(fs::read(&file).ok(), Some(b"old".to_vec()));
        let count = fs::read_dir(&dir).map_or(0, Iterator::count);
        assert_eq!(count, 1, "staging removed");
        let _ = fs::remove_dir_all(&dir);
    }
}
