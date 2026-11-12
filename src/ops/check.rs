//! Owner and permission checks on existing files and directories.
//!
//! Every check opens the entry without following a link at its final
//! component and reads the owner and permissions from the open handle, so
//! the entry checked is the entry opened. On Unix the handle is also
//! compared with the path's own metadata (device and inode), so an entry
//! swapped in between the two is refused rather than checked in its place.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

/// What accounts other than the owner may do to a checked file or
/// directory, for [`check_owner`] and [`open_checked`].
///
/// The trusted owners are the current user and the administrator
/// accounts: root on Unix; `SYSTEM` and `Administrators` on Windows.
///
/// Marked `#[non_exhaustive]` so a level can be added in a minor release;
/// `match` on it with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Others {
    /// Other accounts may read (and list, for a directory), but not
    /// change, create, delete, or rename anything. On Unix: no group or
    /// other write bit. For configuration that is not secret.
    MayRead,
    /// Other accounts may not read, list, or change anything. On Unix: no
    /// group or other bits at all. For secrets and private directories.
    NoAccess,
}

/// Checks that an existing file or directory is owned by a trusted
/// account and that other accounts can do no more than `others` allows.
///
/// The trusted owners are the current user (the effective user on Unix),
/// root on Unix, and `SYSTEM` or `Administrators` on Windows. The checks:
///
/// - **Unix:** the owner, and the group and other permission bits: no
///   write bit for [`Others::MayRead`], no bit at all for
///   [`Others::NoAccess`]. With POSIX ACLs the group bits hold the ACL
///   mask, so a named entry that grants write is caught too.
/// - **Windows:** the owner, and the DACL: no entry for any other account
///   (including `Users`, `Authenticated Users`, and `Everyone`) may grant
///   a right that changes the entry, and for [`Others::NoAccess`] none may
///   grant read. For a directory, entries that only pass rights on to new
///   files and sub-directories count as well, except `CREATOR OWNER`,
///   which becomes whoever creates the child. A volume that keeps no
///   access control lists (FAT, exFAT) fails the check, since any account
///   can change anything on it.
///
/// A symbolic link (or, on Windows, a junction) at `path` is refused, not
/// followed; links in the parent directories are followed as usual. Use
/// [`open_checked`] for a file you are about to read, so the file checked
/// is the file read.
///
/// # Errors
///
/// - [`Error::NotFound`] if nothing exists at `path`.
/// - [`Error::Refused`] if the entry is a link or a special file, was
///   replaced during the check, has an untrusted owner, or grants other
///   accounts more than `others` allows; `reason` says which.
/// - [`Error::Io`] if the entry cannot be opened or its security read.
///
/// # Examples
///
/// ```
/// use path_lib::{App, Kind, Others};
///
/// let root = std::env::temp_dir().join("path-lib-doc-check-owner");
/// let app = App::portable("demo", &root)?;
/// let keys = app.ensure_private(Kind::State)?;
/// path_lib::check_owner(keys, Others::NoAccess)?;
/// # path_lib::remove(&root)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn check_owner(path: impl AsRef<Path>, others: Others) -> Result<()> {
    let path = path.as_ref();
    let meta = inspect(path)?;
    if meta.is_dir() {
        check_existing_dir(path, others, false)
    } else if meta.is_file() {
        let _file = open_file(path, &meta, others, false)?;
        Ok(())
    } else {
        Err(super::special_file(path))
    }
}

/// [`check_owner`] with [`Others::NoAccess`]: the entry must be owned by a
/// trusted account and other accounts must not be able to read or change
/// it.
///
/// # Errors
///
/// As for [`check_owner`].
pub fn check_owner_private(path: impl AsRef<Path>) -> Result<()> {
    check_owner(path, Others::NoAccess)
}

/// Opens an existing regular file for reading after the checks of
/// [`check_owner`], made on the returned handle.
///
/// The final component is not followed if it is a link, and on Unix the
/// file is opened without blocking, so a FIFO planted where the file
/// should be is refused instead of hanging the caller. Read from the
/// returned handle: it is the file that was checked.
///
/// # Errors
///
/// - [`Error::NotFound`] if nothing exists at `path`.
/// - [`Error::Refused`] if `path` is a directory, a link, or a special
///   file, or fails a check of [`check_owner`].
/// - [`Error::Io`] if the file cannot be opened or its security read.
///
/// # Examples
///
/// ```
/// use std::io::Read as _;
/// use path_lib::{App, Kind, Others};
///
/// let root = std::env::temp_dir().join("path-lib-doc-open-checked");
/// let app = App::portable("demo", &root)?;
/// let env = app.ensure_private(Kind::Config)?.join(".env");
/// path_lib::write_atomic_private(&env, "TOKEN=abc\n")?;
///
/// let mut text = String::new();
/// path_lib::open_checked(&env, Others::NoAccess)?.read_to_string(&mut text)?;
/// assert_eq!(text, "TOKEN=abc\n");
/// # path_lib::remove(&root)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn open_checked(path: impl AsRef<Path>, others: Others) -> Result<fs::File> {
    let path = path.as_ref();
    let meta = inspect(path)?;
    if meta.is_dir() {
        return Err(Error::Refused {
            path: path.to_path_buf(),
            reason: "it is a directory, not a file",
        });
    }
    if !meta.is_file() {
        return Err(super::special_file(path));
    }
    open_file(path, &meta, others, true)
}

/// Checks an existing directory. `allow_unenforced` accepts a Windows
/// volume without access control lists (a portable layout on a USB
/// stick); it has no effect on Unix.
pub(crate) fn check_existing_dir(
    path: &Path,
    others: Others,
    allow_unenforced: bool,
) -> Result<()> {
    let meta = inspect(path)?;
    if !meta.is_dir() {
        return Err(Error::AlreadyExists {
            path: path.to_path_buf(),
        });
    }
    let handle = sys::open(path, true, false).map_err(|e| Error::io("open", path, e))?;
    let opened = handle
        .metadata()
        .map_err(|e| Error::io("inspect", path, e))?;
    if !opened.is_dir() || !sys::same_entry(&meta, &opened) {
        return Err(replaced(path));
    }
    verdict(path, &handle, &opened, others, allow_unenforced)
}

/// The path's own metadata, not following a final link; a link is refused.
fn inspect(path: &Path) -> Result<fs::Metadata> {
    let meta =
        fs::symlink_metadata(path).map_err(|e| super::not_found_or_io("inspect", path, e))?;
    if meta.file_type().is_symlink() {
        return Err(Error::Refused {
            path: path.to_path_buf(),
            reason: "it is a symbolic link, not a directory or file",
        });
    }
    Ok(meta)
}

fn open_file(path: &Path, meta: &fs::Metadata, others: Others, read: bool) -> Result<fs::File> {
    let handle = sys::open(path, false, read).map_err(|e| Error::io("open", path, e))?;
    let opened = handle
        .metadata()
        .map_err(|e| Error::io("inspect", path, e))?;
    if !opened.is_file() || !sys::same_entry(meta, &opened) {
        return Err(replaced(path));
    }
    verdict(path, &handle, &opened, others, false)?;
    Ok(handle)
}

fn replaced(path: &Path) -> Error {
    Error::Refused {
        path: path.to_path_buf(),
        reason: "it was replaced while it was being checked",
    }
}

fn refused(path: &Path, reason: &'static str) -> Result<()> {
    Err(Error::Refused {
        path: path.to_path_buf(),
        reason,
    })
}

#[cfg(unix)]
fn verdict(
    path: &Path,
    _handle: &fs::File,
    opened: &fs::Metadata,
    others: Others,
    _allow_unenforced: bool,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let owner = opened.uid();
    if owner != sys::effective_uid() && owner != 0 {
        return refused(path, OWNER);
    }
    let mode = opened.mode();
    if mode & 0o022 != 0 {
        return refused(path, WRITE);
    }
    if others == Others::NoAccess && mode & 0o077 != 0 {
        return refused(path, READ);
    }
    Ok(())
}

#[cfg(windows)]
fn verdict(
    path: &Path,
    handle: &fs::File,
    opened: &fs::Metadata,
    others: Others,
    allow_unenforced: bool,
) -> Result<()> {
    let access = if opened.is_dir() {
        crate::platform::dir_access(handle)
    } else {
        crate::platform::file_access(handle)
    }
    .map_err(|e| Error::io("read security", path, e))?;
    if !access.enforced {
        return if allow_unenforced {
            Ok(())
        } else {
            refused(path, UNENFORCED)
        };
    }
    if !access.owner_trusted {
        return refused(path, OWNER);
    }
    if access.others_can_write {
        return refused(path, WRITE);
    }
    if others == Others::NoAccess && access.others_can_read {
        return refused(path, READ);
    }
    Ok(())
}

#[cfg(unix)]
const OWNER: &str = "it is owned by an account other than the current user or root";
#[cfg(windows)]
const OWNER: &str =
    "it is owned by an account other than the current user, SYSTEM, or Administrators";
#[cfg(unix)]
const WRITE: &str = "its group or other users can write to it; remove their write permission, or delete it so it is created again";
#[cfg(windows)]
const WRITE: &str =
    "other accounts can change it; remove their write access, or delete it so it is created again";
#[cfg(unix)]
const READ: &str = "its group or other users can read it; remove their permissions, or delete it so it is created again";
#[cfg(windows)]
const READ: &str =
    "other accounts can read it; remove their access, or delete it so it is created again";
#[cfg(windows)]
const UNENFORCED: &str =
    "its volume keeps no access control lists (FAT or exFAT), so any account can change it";

#[cfg(unix)]
#[allow(unsafe_code)]
mod sys {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    unsafe extern "C" {
        /// `geteuid(2)`: always succeeds. `uid_t` is a 32-bit unsigned
        /// integer on every Unix target Rust supports.
        fn geteuid() -> u32;
    }

    /// The effective user id of the process.
    pub(super) fn effective_uid() -> u32 {
        // SAFETY: `geteuid` takes no arguments, cannot fail, and touches
        // no memory of ours.
        unsafe { geteuid() }
    }

    /// `O_NONBLOCK`, so opening a FIFO swapped in after the type check
    /// returns at once (and is then refused) instead of waiting for a
    /// writer. The value differs between kernels; on targets not listed
    /// the flag is left out, and only the earlier type check protects.
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        not(any(
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "mips32r6",
            target_arch = "mips64r6",
            target_arch = "sparc",
            target_arch = "sparc64"
        ))
    ))]
    pub(super) const O_NONBLOCK: i32 = 0o4000;
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "mips32r6",
            target_arch = "mips64r6"
        )
    ))]
    pub(super) const O_NONBLOCK: i32 = 0x80;
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(target_arch = "sparc", target_arch = "sparc64")
    ))]
    pub(super) const O_NONBLOCK: i32 = 0x4000;
    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    pub(super) const O_NONBLOCK: i32 = 0x4;
    #[cfg(any(target_os = "solaris", target_os = "illumos"))]
    pub(super) const O_NONBLOCK: i32 = 0x80;
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos"
    )))]
    pub(super) const O_NONBLOCK: i32 = 0;

    /// Opens `path` read-only without blocking. A final link is followed
    /// by `open(2)`, which is why the caller compares the result with the
    /// path's own metadata.
    pub(super) fn open(path: &Path, _directory: bool, _read: bool) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)
    }

    /// Whether two metadata records describe the same filesystem entry.
    pub(super) fn same_entry(a: &fs::Metadata, b: &fs::Metadata) -> bool {
        a.dev() == b.dev() && a.ino() == b.ino()
    }
}

#[cfg(windows)]
mod sys {
    use std::fs;
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    const READ_CONTROL: u32 = 0x0002_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const GENERIC_READ: u32 = 0x8000_0000;
    /// Needed to open a directory at all.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    /// Opens a link or junction itself rather than what it points to.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    /// Opens `path` without following a final link or junction: with
    /// `READ_CONTROL` for the security check, plus read access to the
    /// contents when `read` is set.
    pub(super) fn open(path: &Path, directory: bool, read: bool) -> io::Result<fs::File> {
        let access = if read {
            GENERIC_READ
        } else {
            READ_CONTROL | FILE_READ_ATTRIBUTES
        };
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if directory {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        fs::OpenOptions::new()
            .access_mode(access)
            .custom_flags(flags)
            .open(path)
    }

    /// The handle was opened without following links, so it refers to
    /// the entry at the path, and its own metadata says whether that entry
    /// is a link (which the caller refuses). Nothing else to compare.
    pub(super) fn same_entry(_a: &fs::Metadata, b: &fs::Metadata) -> bool {
        !b.file_type().is_symlink()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ops::test_support::scratch;

    #[test]
    fn test_check_owner_accepts_own_private_dir_and_file() {
        let dir = scratch("check-own");
        let private = dir.join("p");
        super::super::basic::create_tree(&private, super::super::basic::NewDir::Private).unwrap();
        check_owner(&private, Others::NoAccess).unwrap();
        check_owner_private(&private).unwrap();
        let file = private.join("secret");
        crate::write_atomic_private(&file, b"k").unwrap();
        check_owner_private(&file).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_check_owner_missing_is_not_found() {
        let dir = scratch("check-missing");
        let r = check_owner(dir.join("nope"), Others::MayRead);
        assert!(matches!(r, Err(Error::NotFound { .. })), "{r:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_open_checked_refuses_a_directory() {
        let dir = scratch("check-open-dir");
        let r = open_checked(&dir, Others::MayRead);
        assert!(matches!(r, Err(Error::Refused { .. })), "{r:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_check_owner_unix_mode_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("check-modes");
        let target = dir.join("d");
        fs::create_dir(&target).unwrap();
        for (mode, may_read, no_access) in [
            (0o700, true, true),
            (0o750, true, false),
            (0o755, true, false),
            (0o770, false, false),
            (0o702, false, false),
            (0o1777, false, false),
        ] {
            fs::set_permissions(&target, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                check_owner(&target, Others::MayRead).is_ok(),
                may_read,
                "{mode:o} may read"
            );
            assert_eq!(
                check_owner(&target, Others::NoAccess).is_ok(),
                no_access,
                "{mode:o} no access"
            );
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_check_owner_refuses_symlink_and_fifo() {
        let dir = scratch("check-links");
        let real = dir.join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let r = check_owner(&link, Others::MayRead);
        assert!(matches!(r, Err(Error::Refused { .. })), "{r:?}");
        let fifo = dir.join("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) {
            let r = open_checked(&fifo, Others::MayRead);
            assert!(matches!(r, Err(Error::Refused { .. })), "{r:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
