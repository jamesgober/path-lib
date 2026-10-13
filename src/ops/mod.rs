//! Guarded filesystem operations on paths.
//!
//! These are deliberately conservative, because a path library's
//! mistakes delete people's files:
//!
//! - Nothing is ever overwritten. Copy, move, and file creation fail with
//!   [`Error::AlreadyExists`] if the destination is taken.
//! - Symbolic links are never followed while walking a tree. [`remove`]
//!   deletes a link, not what it points to; [`find`] reports links but
//!   does not descend into them; [`copy`] refuses trees containing them.
//! - [`remove`] and [`move_to`] refuse filesystem roots, the home
//!   directory and everything above it, the user's well-known folders,
//!   and any path with fewer than two named components (`/usr`, `C:\data`).
//! - [`copy`] builds the destination under a temporary sibling name and
//!   renames it into place only once complete, so a failed copy never
//!   leaves a half-written destination behind.
//!
//! Checks and actions are separate system calls, so another process can
//! still change the filesystem in between (a time-of-check to
//! time-of-use race). The guards prevent mistakes, not a hostile process
//! racing this one in the same directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Error;

mod basic;
mod find;
mod guard;
mod remove;
mod transfer;
mod write;

pub(crate) use basic::create_dir;
pub use basic::{exists, list, make_dir, make_file};
pub use find::find;
pub use remove::remove;
pub use transfer::{copy, move_to};
pub use write::write_atomic;

// Shared between the submodules, which reach them through `super::`.
use guard::{absolute, guard_destructive, is_within, resolve_lenient};
use remove::remove_entry;

/// Deepest directory nesting [`find`] and [`copy`] will walk.
pub const MAX_WALK_DEPTH: usize = 256;

/// A rename step, injectable so tests can force the failures that real
/// filesystems only produce rarely (cross-volume moves, a rename that
/// fails after a copy completed).
pub(crate) type RenameFn<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

/// The real rename.
pub(crate) fn os_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// A hidden, unique sibling of `to` used while a copy is in progress.
pub(crate) fn staging_path(to: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = to
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staged = format!(".{name}.path-lib-{}-{n}.partial", std::process::id());
    to.with_file_name(staged)
}

pub(crate) fn not_found_or_io(op: &'static str, path: &Path, e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::NotFound {
        Error::NotFound {
            path: path.to_path_buf(),
        }
    } else {
        Error::io(op, path, e)
    }
}

pub(crate) fn special_file(path: &Path) -> Error {
    Error::Refused {
        path: path.to_path_buf(),
        reason: "it is a special file (socket, FIFO, or device)",
    }
}

pub(crate) fn too_deep(path: PathBuf) -> Error {
    Error::Refused {
        path,
        reason: "the directory tree is nested deeper than the walk limit (MAX_WALK_DEPTH)",
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::fs;

    pub(crate) fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("path-lib-unit-ops")
            .join(format!("{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap_or_default();
        dir
    }

    pub(crate) fn cross_device(_: &Path, _: &Path) -> io::Result<()> {
        #[cfg(unix)]
        const CODE: i32 = 18;
        #[cfg(windows)]
        const CODE: i32 = 17;
        Err(io::Error::from_raw_os_error(CODE))
    }

    pub(crate) fn denied(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "denied by test",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_staging_path_is_unique_hidden_sibling() {
        let to = Path::new("dir").join("target");
        let a = staging_path(&to);
        let b = staging_path(&to);
        assert_ne!(a, b);
        assert_eq!(a.parent(), to.parent());
        let name = a.file_name().map(|n| n.to_string_lossy().into_owned());
        assert!(name.is_some_and(|n| n.starts_with(".target.path-lib-")));
    }
}
