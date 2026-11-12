//! Creating, inspecting, and listing paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::check::{Others, check_existing_dir};
use super::not_found_or_io;
use crate::error::{Error, Result};

/// Creates a directory and any missing parents.
///
/// Succeeds if the directory already exists. Neither the existing
/// directory nor any parent is checked for its owner or permissions, and
/// links in the parents are followed: `make_dir` takes any path and has
/// no way to know which parents are meant to be shared. For an
/// application's own directories use [`App::ensure`](crate::App::ensure)
/// and [`App::create`](crate::App::create), which check, or call
/// [`check_owner`](crate::check_owner) on the result.
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
/// the parent's access control list. Existing directories are accepted
/// as they are: this is the unchecked path behind [`make_dir`]; the
/// [`App`](crate::App) methods use [`create_app_dir`].
pub(crate) fn create_dir(path: &Path, mode: Option<u32>) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(meta) if meta.file_type().is_symlink() => return Err(link_refused(path)),
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

/// How a directory the crate creates is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewDir {
    /// Unix `0700`; on Windows a protected access control list granting
    /// full control to the current user, `SYSTEM`, and `Administrators`.
    Private,
    /// Unix `0755`; on Windows the same list plus read and list access
    /// for `Users`, and no write access for anyone else.
    SharedRead,
}

/// Creates the missing directories of `path`, top down, each protected
/// as `new` says. Existing directories, and links in the parents, are
/// accepted as they are (the parents above an application's directories,
/// such as `/var` on macOS, are not the crate's to judge).
pub(crate) fn create_tree(path: &Path, new: NewDir) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(meta) if meta.file_type().is_symlink() => return Err(link_refused(path)),
        Ok(_) => {
            return Err(Error::AlreadyExists {
                path: path.to_path_buf(),
            });
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io("inspect", path, e)),
    }
    // The missing directories, deepest first; stop at the first that exists.
    let mut missing = Vec::new();
    for dir in path.ancestors() {
        if dir.as_os_str().is_empty() {
            break;
        }
        match fs::metadata(dir) {
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => {
                return Err(Error::AlreadyExists {
                    path: dir.to_path_buf(),
                });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => missing.push(dir),
            Err(e) => return Err(Error::io("inspect", dir, e)),
        }
    }
    for dir in missing.iter().rev() {
        match create_one(dir, new) {
            Ok(()) => {}
            // Another process created it first: fine, if it is a directory.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => {}
            Err(e) => return Err(Error::io("create directory", dir, e)),
        }
    }
    Ok(())
}

/// Creates `path` for an application and checks every existing directory
/// the application owns on the way.
///
/// `anchor` is the application's outermost directory for this kind (the
/// kind's own directory, or the one it is nested in); `path` is the
/// anchor or below it. Parents above the anchor that are missing are
/// created with [`create_tree`] and not checked. From the anchor down,
/// one component at a time:
///
/// - a missing directory is created, protected as `new` says;
/// - a symbolic link or junction is refused, never followed;
/// - an existing directory must pass [`check_existing_dir`]: `leaf` for
///   `path` itself, [`Others::MayRead`] for the directories above it, so
///   no other account can rename or replace what is created inside.
///
/// `allow_unenforced` accepts a Windows volume without access control
/// lists, for portable layouts.
pub(crate) fn create_app_dir(
    path: &Path,
    anchor: &Path,
    new: NewDir,
    leaf: Others,
    allow_unenforced: bool,
) -> Result<()> {
    let below = match path.strip_prefix(anchor) {
        Ok(below) => below,
        // Not under the anchor (App never asks for this): the path alone.
        Err(_) => return create_app_dir(path, path, new, leaf, allow_unenforced),
    };
    if let Some(parent) = anchor.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_tree(parent, new)?;
    }
    let mut current = anchor.to_path_buf();
    let mut steps = below.components();
    loop {
        let want = if current == path {
            leaf
        } else {
            Others::MayRead
        };
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(link_refused(&current)),
            Ok(meta) if meta.is_dir() => check_existing_dir(&current, want, allow_unenforced)?,
            Ok(_) => return Err(Error::AlreadyExists { path: current }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => match create_one(&current, new) {
                Ok(()) => {}
                // Another process created it first: check what it made.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    check_existing_dir(&current, want, allow_unenforced)?;
                }
                Err(e) => return Err(Error::io("create directory", &current, e)),
            },
            Err(e) => return Err(Error::io("inspect", &current, e)),
        }
        match steps.next() {
            Some(step) => current.push(step),
            None => return Ok(()),
        }
    }
}

fn link_refused(path: &Path) -> Error {
    Error::Refused {
        path: path.to_path_buf(),
        reason: "it is a symbolic link, not a directory",
    }
}

#[cfg(unix)]
fn create_one(dir: &Path, new: NewDir) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    let _builder = builder.mode(match new {
        NewDir::Private => 0o700,
        NewDir::SharedRead => 0o755,
    });
    builder.create(dir)
}

#[cfg(windows)]
fn create_one(dir: &Path, new: NewDir) -> io::Result<()> {
    crate::platform::create_protected_dir(dir, new == NewDir::SharedRead)
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
