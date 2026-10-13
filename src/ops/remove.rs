//! Guarded deletion.

use std::fs;
use std::io;
use std::path::Path;

use super::guard_destructive;
use crate::error::{Error, Result};

/// Deletes a file, a symbolic link, or a directory and everything in it.
///
/// Returns `Ok(false)` if nothing existed at `path`, so removal is
/// idempotent. Symbolic links are removed themselves; their targets are
/// never touched, including links found inside a directory being removed.
///
/// # Errors
///
/// - [`Error::Refused`] for protected paths: a filesystem root, the home
///   directory or any of its ancestors, the user's well-known folders
///   (Documents, Desktop, ...), the OS temp directory, or any path with
///   fewer than two named components.
/// - [`Error::Io`] if deletion fails part-way. Entries already deleted
///   stay deleted.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-remove");
/// path_lib::make_dir(dir.join("x/y"))?;
/// assert!(path_lib::remove(&dir)?);
/// assert!(!path_lib::remove(&dir)?); // already gone
/// assert!(path_lib::remove("/").is_err()); // refused
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn remove(path: impl AsRef<Path>) -> Result<bool> {
    let path = path.as_ref();
    guard_destructive(path)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(Error::io("inspect", path, e)),
    };
    remove_entry(path, &meta)?;
    Ok(true)
}

pub(super) fn remove_entry(path: &Path, meta: &fs::Metadata) -> Result<()> {
    let result = if meta.file_type().is_symlink() {
        remove_link(path, meta)
    } else if meta.is_dir() {
        // `remove_dir_all` does not follow symbolic links (fixed for
        // CVE-2022-21658 in Rust 1.58.1, below this crate's MSRV).
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| Error::io("remove", path, e))
}

#[cfg(windows)]
pub(super) fn remove_link(path: &Path, meta: &fs::Metadata) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if meta.file_type().is_symlink_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(unix)]
pub(super) fn remove_link(path: &Path, _meta: &fs::Metadata) -> io::Result<()> {
    fs::remove_file(path)
}
