//! Who besides the trusted accounts can change or read an open file
//! (Windows).

use std::fs::File;
use std::io;

/// What accounts other than the trusted ones can do to an open file,
/// returned by [`file_access`].
///
/// The trusted accounts are the current user (from the process token),
/// `SYSTEM` (`S-1-5-18`), and `BUILTIN\Administrators` (`S-1-5-32-544`).
/// Every other account counts as another account, including groups the
/// current user belongs to (`Users`, `Authenticated Users`, `Everyone`),
/// because other users belong to them too.
///
/// To accept a file only when nobody else can change it, require
/// `enforced && owner_trusted && !others_can_write`; for a secret, also
/// `!others_can_read`.
#[cfg_attr(docsrs, doc(cfg(windows)))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileAccess {
    /// The volume keeps access control lists (`FILE_PERSISTENT_ACLS`).
    ///
    /// When `false` (FAT, exFAT, some network shares) anyone can change
    /// any file there, and the other fields carry no information. They are
    /// then set to the unsafe answer (owner not trusted, others can write
    /// and read), so a check that forgets this field still refuses.
    pub enforced: bool,
    /// The owner is the current user, `SYSTEM`, or `Administrators`. The
    /// owner can always rewrite the access control list, so an untrusted
    /// owner can take the file over whatever the list says now.
    pub owner_trusted: bool,
    /// The DACL grants some right that changes the file, its attributes,
    /// or its security to an account other than the trusted ones: write,
    /// append, write extended attributes or attributes, delete, change
    /// permissions, take ownership, or a generic write or all right.
    /// `OWNER RIGHTS` (`S-1-3-4`) counts as the owner, so it is trusted
    /// exactly when the owner is. A NULL DACL, which lets everyone do
    /// anything, makes this `true`.
    pub others_can_write: bool,
    /// The same for rights that read the contents: read data, or a
    /// generic read or all right. A NULL DACL makes this `true`.
    pub others_can_read: bool,
}

/// Reports who besides the trusted accounts can change or read an open
/// file.
///
/// Use it on a configuration, `.env`, or secret file after opening it and
/// before reading it, so that the file checked is the file read: the
/// owner and the access control list come from the handle, never from the
/// path, and a file swapped in after the check is not the file being
/// read. It is the Windows counterpart of checking the mode and owner of
/// an open descriptor on Unix. A machine-wide directory such as
/// `%ProgramData%\<app>\config` lets every local user create files by
/// default, so a service that reads configuration from there needs this
/// check.
///
/// Only entries that grant access are counted, and only those that apply
/// to the file itself (inherit-only entries are skipped). Deny entries are
/// ignored, so the answer can overstate what other accounts can do but
/// never understate it. Holders of the backup and restore privileges can
/// read and write any file whatever the list says; they are
/// administrators by default.
///
/// The handle needs `READ_CONTROL` access, which [`File::open`] and
/// [`File::create`] both request. The cost is three system calls and one
/// pass over the access control list, with no path lookups.
///
/// # Errors
///
/// - [`io::ErrorKind::PermissionDenied`] if the handle was opened without
///   `READ_CONTROL`.
/// - [`io::ErrorKind::InvalidData`] if the security descriptor the system
///   returns is malformed.
/// - Any other error from `GetVolumeInformationByHandleW`,
///   `GetSecurityInfo`, or reading the process token.
///
/// # Examples
///
/// ```
/// use std::fs::File;
/// use path_lib::{App, Kind};
///
/// let root = std::env::temp_dir().join("path-lib-doc-file-access");
/// let app = App::portable("demo", &root)?;
/// let config = app.ensure_private(Kind::Config)?.join("app.conf");
/// path_lib::write_atomic(&config, "port = 7000\n")?;
///
/// let file = File::open(&config)?;
/// let access = path_lib::file_access(&file)?;
/// if !access.enforced || !access.owner_trusted || access.others_can_write {
///     return Err("another account can change the configuration".into());
/// }
/// // Read from `file`, the handle that was checked.
/// # drop(file);
/// # path_lib::remove(&root)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg_attr(docsrs, doc(cfg(windows)))]
pub fn file_access(file: &File) -> io::Result<FileAccess> {
    crate::platform::file_access(file)
}
