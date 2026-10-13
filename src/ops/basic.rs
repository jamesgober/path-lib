//! Creating, inspecting, and listing paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::not_found_or_io;
use crate::error::{Error, Result};

/// Creates a directory and any missing parents.
///
/// Succeeds if the directory already exists.
///
/// # Errors
///
/// - [`Error::AlreadyExists`] if `path` exists but is not a directory.
/// - [`Error::Io`] if the directory cannot be created.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-make-dir").join("a/b");
/// path_lib::make_dir(&dir)?;
/// assert!(dir.is_dir());
/// # path_lib::remove(std::env::temp_dir().join("path-lib-doc-make-dir"))?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn make_dir(path: impl AsRef<Path>) -> Result<()> {
    create_dir(path.as_ref(), None)
}

/// Creates a directory tree with Unix permission bits `mode` on every
/// directory it creates (subject to the process umask). `None` uses the
/// platform default. On Windows `mode` is ignored and directories inherit
/// the parent's access control list.
pub(crate) fn create_dir(path: &Path, mode: Option<u32>) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(Error::Refused {
                path: path.to_path_buf(),
                reason: "it is a symbolic link, not a directory",
            });
        }
        Ok(_) => {
            return Err(Error::AlreadyExists {
                path: path.to_path_buf(),
            });
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io("inspect", path, e)),
    }
    let mut builder = fs::DirBuilder::new();
    let _builder = builder.recursive(true);
    apply_mode(&mut builder, mode);
    builder
        .create(path)
        .map_err(|e| Error::io("create directory", path, e))
}

#[cfg(unix)]
pub(super) fn apply_mode(builder: &mut fs::DirBuilder, mode: Option<u32>) {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(mode) = mode {
        let _builder = builder.mode(mode);
    }
}

#[cfg(windows)]
pub(super) fn apply_mode(_builder: &mut fs::DirBuilder, _mode: Option<u32>) {}

/// Creates an empty file, plus any missing parent directories.
///
/// # Errors
///
/// - [`Error::AlreadyExists`] if anything already exists at `path`.
/// - [`Error::Io`] if the file or a parent cannot be created.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-make-file");
/// let file = dir.join("notes/today.txt");
/// path_lib::make_file(&file)?;
/// assert!(file.is_file());
/// assert!(path_lib::make_file(&file).is_err()); // never overwrites
/// # path_lib::remove(&dir)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn make_file(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_dir(parent, None)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(_file) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(Error::AlreadyExists {
            path: path.to_path_buf(),
        }),
        Err(e) => Err(Error::io("create file", path, e)),
    }
}

/// Whether anything exists at `path`: a file, directory, or symbolic link
/// (even a broken one).
///
/// Errors other than "not found" (such as permission denied on a parent)
/// are reported as `true`, because something may well be there; use
/// [`std::fs::symlink_metadata`] when the distinction matters.
#[must_use]
pub fn exists(path: impl AsRef<Path>) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}

/// Lists the entries of a directory, sorted by path.
///
/// # Errors
///
/// - [`Error::NotFound`] if `dir` does not exist.
/// - [`Error::Io`] if it is not a directory or cannot be read.
pub fn list(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    let entries = fs::read_dir(dir).map_err(|e| not_found_or_io("list", dir, e))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io("list", dir, e))?;
        out.push(entry.path());
    }
    out.sort_unstable();
    Ok(out)
}
