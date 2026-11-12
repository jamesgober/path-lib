//! Atomic file replacement.

use std::fs;
use std::io;
use std::path::Path;

use super::basic::{NewDir, create_tree};
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
///   copied to the new file, so a `0600` secrets file stays `0600`. The
///   temporary file is created with mode `0600` and receives the final
///   bits only once the contents are written and flushed, so the contents
///   are never readable by more accounts than the final file allows. When
///   the process may (as root, or for a group the user belongs to), the
///   existing file's owner and group are kept too. A new file gets the
///   process default. On Windows the new file inherits the directory's
///   access control list. For a file that holds secrets, use
///   [`write_atomic_private`].
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
    write_atomic_with(path.as_ref(), contents.as_ref(), &os_rename, false)
}

/// Like [`write_atomic`], but the file is private to the current user,
/// whether it is new or already exists: for `.env` files, keys, and
/// tokens.
///
/// - **Unix:** the temporary file is created with mode `0600` and keeps
///   it, so the result is `0600` (an existing `0644` file is narrowed).
///   The existing owner and group are kept when the process may set them.
/// - **Windows:** the temporary file is opened with no sharing, so no
///   other process can open it while it is written, and its access
///   control list is replaced with a protected one for the current user,
///   `SYSTEM`, and `Administrators` before anything is written to it.
///   The rename keeps that list.
///
/// Missing parent directories are created private as well, as by
/// [`App::ensure_private`](crate::App::ensure_private); existing ones are
/// left as they are. Check them with
/// [`check_owner`](crate::check_owner) when they may be shared.
///
/// # Errors
///
/// As for [`write_atomic`].
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-write-atomic-private");
/// let file = dir.join(".env");
/// path_lib::write_atomic_private(&file, "TOKEN=abc\n")?;
/// path_lib::check_owner_private(&file)?;
/// # path_lib::remove(&dir)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn write_atomic_private(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    write_atomic_with(path.as_ref(), contents.as_ref(), &os_rename, true)
}

pub(super) fn write_atomic_with(
    path: &Path,
    contents: &[u8],
    rename: RenameFn<'_>,
    private: bool,
) -> Result<()> {
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
            Ok(real) => return write_atomic_with(&real, contents, rename, private),
            Err(_) => return refuse("it is a broken symbolic link"),
        },
        Some(meta) if meta.is_dir() => return refuse("it is a directory"),
        _ => path,
    };
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        if private {
            create_tree(parent, NewDir::Private)?;
        } else {
            create_dir(parent, None)?;
        }
    }
    let staging = staging_path(target);
    let written = write_staged(&staging, contents, existing.as_ref(), private)
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

/// Writes and flushes the staging file, then gives it its final
/// permissions: `existing`'s, or `0600` (a private DACL on Windows) when
/// `private` is set.
pub(super) fn write_staged(
    staging: &Path,
    contents: &[u8],
    existing: Option<&fs::Metadata>,
    private: bool,
) -> Result<()> {
    use std::io::Write as _;
    let mut file = open_staging(staging, existing, private)
        .map_err(|e| Error::io("create file", staging, e))?;
    file.write_all(contents)
        .map_err(|e| Error::io("write", staging, e))?;
    finish_permissions(&file, staging, existing, private)?;
    file.sync_all().map_err(|e| Error::io("flush", staging, e))
}

/// Unix: created exclusively with mode `0600` when there is a mode to
/// protect (an existing file, or a private one), so the contents are
/// never readable by more accounts than the final file allows. A new
/// ordinary file gets the process default from the start, which is also
/// its final mode.
#[cfg(unix)]
fn open_staging(
    staging: &Path,
    existing: Option<&fs::Metadata>,
    private: bool,
) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mode = if private || existing.is_some() {
        0o600
    } else {
        0o666
    };
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(staging)
}

/// Windows: a private file is opened with no sharing, so nobody else can
/// open it while it is written, and its DACL is replaced before anything
/// is written to it.
#[cfg(windows)]
fn open_staging(
    staging: &Path,
    _existing: Option<&fs::Metadata>,
    private: bool,
) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const READ_CONTROL: u32 = 0x0002_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    let mut options = fs::OpenOptions::new();
    let _options = options.write(true).create_new(true);
    if private {
        let _options = options
            .access_mode(GENERIC_WRITE | READ_CONTROL | WRITE_DAC)
            .share_mode(0);
    }
    let file = options.open(staging)?;
    if private {
        crate::platform::set_private_file_dacl(&file)?;
    }
    Ok(file)
}

/// Unix: the existing file's owner and group (best effort: only root may
/// give a file away, and a user only to their own groups), then its
/// permission bits, set on the handle. `chown` clears set-id bits, so it
/// comes first.
#[cfg(unix)]
fn finish_permissions(
    file: &fs::File,
    staging: &Path,
    existing: Option<&fs::Metadata>,
    private: bool,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(meta) = existing else {
        return Ok(());
    };
    let staged = file
        .metadata()
        .map_err(|e| Error::io("inspect", staging, e))?;
    if staged.uid() != meta.uid() || staged.gid() != meta.gid() {
        // A failure leaves the file owned by this process, which is what
        // a plain rewrite does anyway; the mode below still applies.
        let _kept = std::os::unix::fs::fchown(file, Some(meta.uid()), Some(meta.gid()))
            .or_else(|_| std::os::unix::fs::fchown(file, None, Some(meta.gid())));
    }
    let mode = if private {
        0o600
    } else {
        meta.permissions().mode() & 0o7777
    };
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|e| Error::io("set permissions", staging, e))
}

/// Windows: the new file inherits the directory's ACL, or got a private
/// one when it was opened. Copying the read-only attribute would only
/// make the following rename fail.
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)] // Same signature as the Unix version.
fn finish_permissions(
    _file: &fs::File,
    _staging: &Path,
    _existing: Option<&fs::Metadata>,
    _private: bool,
) -> Result<()> {
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
        let r = write_atomic_with(&file, b"new", &denied, false);
        assert!(matches!(r, Err(Error::Io { op: "replace", .. })), "{r:?}");
        assert_eq!(fs::read(&file).ok(), Some(b"old".to_vec()));
        let count = fs::read_dir(&dir).map_or(0, Iterator::count);
        assert_eq!(count, 1, "staging removed");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Records the mode of the staging file at the moment of the rename,
    /// that is, after the contents were written.
    #[cfg(unix)]
    fn staged_mode(
        seen: &std::sync::Mutex<Vec<u32>>,
    ) -> impl Fn(&Path, &Path) -> io::Result<()> + '_ {
        use std::os::unix::fs::PermissionsExt;
        move |from, to| {
            let mode = fs::metadata(from)?.permissions().mode() & 0o7777;
            seen.lock().map(|mut v| v.push(mode)).unwrap_or_default();
            fs::rename(from, to)
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_write_atomic_staging_never_wider_than_the_target() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::sync::Mutex;
        let dir = scratch("wa-mode");
        let file = dir.join("secrets.env");
        let _ = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .unwrap_or_else(|e| panic!("{e}"));
        let seen = Mutex::new(Vec::new());
        // The staging file is created 0600 and keeps the target's 0600.
        write_atomic_with(&file, b"TOKEN=1", &staged_mode(&seen), false)
            .unwrap_or_else(|e| panic!("{e}"));
        let mode = fs::metadata(&file).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode.ok(), Some(0o600));
        // The private variant narrows a 0644 file to 0600.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap_or_default();
        write_atomic_with(&file, b"TOKEN=2", &staged_mode(&seen), true)
            .unwrap_or_else(|e| panic!("{e}"));
        let mode = fs::metadata(&file).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode.ok(), Some(0o600));
        // A new private file is 0600 from the start.
        let fresh = dir.join("new.env");
        write_atomic_with(&fresh, b"TOKEN=3", &staged_mode(&seen), true)
            .unwrap_or_else(|e| panic!("{e}"));
        let mode = fs::metadata(&fresh).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode.ok(), Some(0o600));
        let seen = seen.lock().map(|v| v.clone()).unwrap_or_default();
        assert_eq!(seen, vec![0o600, 0o600, 0o600], "{seen:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_write_atomic_keeps_a_wider_mode_after_writing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("wa-mode-wide");
        let file = dir.join("app.conf");
        fs::write(&file, b"old").unwrap_or_default();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap_or_default();
        write_atomic(&file, b"new").unwrap_or_else(|e| panic!("{e}"));
        let mode = fs::metadata(&file).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode.ok(), Some(0o640));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn test_write_atomic_private_sets_a_private_dacl() {
        let dir = scratch("wa-private-win");
        let file = dir.join(".env");
        // Open the directory to Users, as %ProgramData% is.
        crate::platform::create_protected_dir(&dir.join("shared"), true)
            .unwrap_or_else(|e| panic!("{e}"));
        let shared = dir.join("shared").join(".env");
        write_atomic(&shared, b"A=1").unwrap_or_else(|e| panic!("{e}"));
        let open = crate::file_access(&fs::File::open(&shared).unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            open.others_can_read,
            "plain write inherits Users read: {open:?}"
        );
        write_atomic_private(&shared, b"A=2").unwrap_or_else(|e| panic!("{e}"));
        let closed = crate::file_access(&fs::File::open(&shared).unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            closed.owner_trusted && !closed.others_can_read && !closed.others_can_write,
            "{closed:?}"
        );
        assert_eq!(fs::read(&shared).ok(), Some(b"A=2".to_vec()));
        write_atomic_private(&file, b"B=1").unwrap_or_else(|e| panic!("{e}"));
        crate::check_owner_private(&file).unwrap_or_else(|e| panic!("{e}"));
        let _ = fs::remove_dir_all(&dir);
    }
}
