//! The operating system, and its well-known folders.
//!
//! The OS is fixed at compile time, so [`os()`] is a constant: there is
//! nothing to detect at runtime and nothing to cache.
//!
//! Folders are resolved once per process, on first use, and cached in
//! `OnceLock` statics. A static lives for the life of the whole binary,
//! not just this crate, so every caller in the process shares one
//! resolution. These statics are the crate's only global state; they are
//! written once and read-only afterwards, which keeps them thread-safe
//! without locks on the read path.
//!
//! User folders and machine folders are cached separately. A process
//! started without a home directory (some service managers do this) can
//! still use machine-wide paths; only the user-scoped calls fail.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::error::{Error, Result};
use crate::join::clean;
use crate::platform::{self, SystemBases, UserBases};

/// An operating system family supported by this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Os {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    MacOs,
    /// Linux.
    Linux,
    /// FreeBSD, OpenBSD, NetBSD, and other Unix-like systems that follow
    /// the XDG conventions.
    OtherUnix,
}

impl Os {
    /// A short lowercase name: `"windows"`, `"macos"`, `"linux"`, or
    /// `"unix"`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
            Self::OtherUnix => "unix",
        }
    }
}

impl std::fmt::Display for Os {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The operating system this binary was compiled for.
///
/// Evaluated at compile time; calling it costs nothing.
///
/// # Examples
///
/// ```
/// use path_lib::Os;
///
/// let os = path_lib::os();
/// if cfg!(windows) {
///     assert_eq!(os, Os::Windows);
/// }
/// ```
#[must_use]
#[inline]
pub const fn os() -> Os {
    #[cfg(windows)]
    {
        Os::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Os::MacOs
    }
    #[cfg(target_os = "linux")]
    {
        Os::Linux
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
    {
        Os::OtherUnix
    }
}

static USER: OnceLock<std::result::Result<UserBases, &'static str>> = OnceLock::new();
static SYSTEM: OnceLock<std::result::Result<SystemBases, &'static str>> = OnceLock::new();
static TEMP: OnceLock<PathBuf> = OnceLock::new();
static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();

pub(crate) fn user_bases() -> Result<&'static UserBases> {
    USER.get_or_init(|| UserBases::resolve(&platform::process_env))
        .as_ref()
        .map_err(|dir| Error::Unresolved { dir })
}

pub(crate) fn system_bases() -> Result<&'static SystemBases> {
    SYSTEM
        .get_or_init(|| SystemBases::resolve(&platform::process_env))
        .as_ref()
        .map_err(|dir| Error::Unresolved { dir })
}

macro_rules! user_folder {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// # Errors
        ///
        /// [`Error::Unresolved`] when the current user's home directory
        /// cannot be determined.
        #[inline]
        pub fn $name() -> Result<&'static Path> {
            user_bases().map(|b| b.folders.$name.as_path())
        }
    };
}

user_folder!(
    /// The current user's home directory.
    ///
    /// `$HOME` on Unix; the profile folder (`C:\Users\<name>`) on Windows.
    ///
    /// # Examples
    ///
    /// ```
    /// let home = path_lib::home()?;
    /// assert!(home.is_absolute());
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    home
);
user_folder!(
    /// The current user's Documents folder.
    ///
    /// Follows redirection: OneDrive-backed folders on Windows, and
    /// `user-dirs.dirs` (localised names such as `~/Dokumente`) on Linux.
    documents
);
user_folder!(
    /// The current user's Desktop folder. Follows redirection like
    /// [`documents`].
    desktop
);
user_folder!(
    /// The current user's Downloads folder. Follows redirection like
    /// [`documents`].
    downloads
);
user_folder!(
    /// The current user's Pictures folder. Follows redirection like
    /// [`documents`].
    pictures
);
user_folder!(
    /// The current user's Music folder. Follows redirection like
    /// [`documents`].
    music
);
user_folder!(
    /// The current user's Videos folder (`~/Movies` on macOS). Follows
    /// redirection like [`documents`].
    videos
);

/// The root of the system volume: `/` on Unix, the system drive (usually
/// `C:\`) on Windows.
///
/// # Errors
///
/// [`Error::Unresolved`] on Windows when neither `%SystemDrive%` nor the
/// `ProgramData` Known Folder is available. Never fails on Unix.
///
/// # Examples
///
/// ```
/// let root = path_lib::root()?;
/// assert!(root.parent().is_none());
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn root() -> Result<&'static Path> {
    system_bases().map(|b| b.root.as_path())
}

/// The temporary directory for this process, as reported by
/// [`std::env::temp_dir`], cleaned.
///
/// This is the OS-level temp directory, which on Linux is usually the
/// shared `/tmp`. For an application's private scratch space use
/// [`App::temp`](crate::App::temp) instead.
#[must_use]
pub fn temp() -> &'static Path {
    TEMP.get_or_init(|| clean(std::env::temp_dir()))
}

/// The full path of the running executable.
///
/// # Errors
///
/// [`Error::Unresolved`] when the OS cannot report it (for example, a
/// Linux process without `/proc` mounted).
///
/// # Examples
///
/// ```
/// let exe = path_lib::exe()?;
/// assert!(exe.is_absolute());
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn exe() -> Result<&'static Path> {
    EXE.get_or_init(|| std::env::current_exe().ok().map(clean))
        .as_deref()
        .ok_or(Error::Unresolved { dir: "executable" })
}

/// The directory containing the running executable.
///
/// Useful for portable installs that keep their files next to the
/// binary. Pair it with [`App::portable`](crate::App::portable).
///
/// # Errors
///
/// Same as [`exe`].
pub fn exe_dir() -> Result<&'static Path> {
    exe()?
        .parent()
        .ok_or(Error::Unresolved { dir: "executable" })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_os_matches_target() {
        let os = os();
        assert_eq!(cfg!(windows), os == Os::Windows);
        assert_eq!(cfg!(target_os = "macos"), os == Os::MacOs);
        assert_eq!(cfg!(target_os = "linux"), os == Os::Linux);
        assert!(!os.name().is_empty());
        assert_eq!(os.to_string(), os.name());
    }

    #[test]
    fn test_user_folders_are_absolute_and_cached() {
        let first = home().unwrap();
        let second = home().unwrap();
        assert!(first.is_absolute());
        assert!(
            std::ptr::eq(first, second),
            "home() must return the cached value"
        );
        for f in [documents, desktop, downloads, pictures, music, videos] {
            assert!(f().unwrap().is_absolute());
        }
    }

    #[test]
    fn test_root_has_no_parent() {
        let r = root().unwrap();
        assert!(r.is_absolute());
        assert!(r.parent().is_none());
    }

    #[test]
    fn test_temp_and_exe_are_absolute() {
        assert!(temp().is_absolute());
        assert!(exe().unwrap().is_absolute());
        assert!(exe_dir().unwrap().is_dir());
    }

    #[test]
    fn test_os_queries_are_thread_safe() {
        let handles: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| home().map(Path::to_path_buf).ok()))
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(results.windows(2).all(|w| w[0] == w[1]));
    }
}
