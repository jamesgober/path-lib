//! macOS: the `Library` domains from Apple's File System Programming Guide.
//!
//! | Kind      | User (`~/Library/...`)                 | Global (`/Library/...`)                |
//! |-----------|----------------------------------------|----------------------------------------|
//! | data      | `Application Support/<app>`            | `Application Support/<app>`            |
//! | config    | `Preferences/<app>`                    | `Preferences/<app>`                    |
//! | cache     | `Caches/<app>`                         | `Caches/<app>`                         |
//! | logs      | `Logs/<app>`                           | `Logs/<app>`                           |
//! | state     | `Application Support/<app>/state`      | `Application Support/<app>/state`      |
//! | runtime   | `Application Support/<app>/run`        | `/var/run/<app>`                       |
//! | temp      | `$TMPDIR/<app>`                        | `/var/tmp/<app>`                       |
//! | documents | `~/Documents/<app>`                    | `/Users/Shared/<app>`                  |
//!
//! `$TMPDIR` on macOS is a per-user directory under `/var/folders`, so
//! user temp is private without extra work. Processes started without
//! `TMPDIR` (launchd jobs, cron, `sudo`, some CI shells) get the same
//! directory from `confstr(_CS_DARWIN_USER_TEMP_DIR)`, which is where
//! `TMPDIR` normally comes from, never the shared `/tmp`. If that fails
//! too, user temp falls back to `~/Library/Caches/TemporaryItems/<app>`.

use std::path::{Path, PathBuf};

use super::{Env, Folders, Layout, absolute_var};

/// Per-user bases on macOS.
#[derive(Debug, Clone)]
pub(crate) struct UserBases {
    pub folders: Folders,
    library: PathBuf,
    temp: PathBuf,
}

impl UserBases {
    pub(crate) fn resolve(env: Env<'_>) -> Result<Self, &'static str> {
        Self::resolve_with(env, &darwin_user_temp)
    }

    /// Resolution with an injectable per-user temp lookup, for tests.
    pub(crate) fn resolve_with(
        env: Env<'_>,
        user_temp: &dyn Fn() -> Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        let home = absolute_var(env, "HOME").ok_or("home")?;
        let temp = absolute_var(env, "TMPDIR")
            .or_else(user_temp)
            .unwrap_or_else(|| home.join("Library").join("Caches").join("TemporaryItems"));
        let folders = Folders {
            documents: home.join("Documents"),
            desktop: home.join("Desktop"),
            downloads: home.join("Downloads"),
            pictures: home.join("Pictures"),
            music: home.join("Music"),
            videos: home.join("Movies"),
            home,
        };
        Ok(Self {
            library: folders.home.join("Library"),
            folders,
            temp,
        })
    }

    pub(crate) fn layout(&self, name: &str) -> Layout {
        let data = self.library.join("Application Support").join(name);
        Layout {
            config: self.library.join("Preferences").join(name),
            cache: self.library.join("Caches").join(name),
            logs: self.library.join("Logs").join(name),
            state: data.join("state"),
            runtime: data.join("run"),
            temp: self.temp.join(name),
            documents: self.folders.documents.join(name),
            data,
        }
    }
}

/// The per-user temporary directory the system assigns
/// (`/var/folders/.../T/`), the value `TMPDIR` normally holds.
#[allow(unsafe_code)]
fn darwin_user_temp() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    unsafe extern "C" {
        /// `confstr(3)`: copies the value of `name`, NUL-terminated, into
        /// `buf` (truncated to `len` bytes) and returns the full length
        /// including the NUL, or 0 on error.
        fn confstr(name: i32, buf: *mut std::ffi::c_char, len: usize) -> usize;
    }
    /// From `<unistd.h>`.
    const CS_DARWIN_USER_TEMP_DIR: i32 = 65537;

    let mut buf = vec![0u8; 1024];
    // SAFETY: `buf` is a live, writable buffer of `buf.len()` bytes, and
    // `confstr` writes at most that many.
    let needed = unsafe { confstr(CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr().cast(), buf.len()) };
    if needed == 0 || needed > buf.len() {
        return None;
    }
    buf.truncate(needed - 1);
    let path = PathBuf::from(OsString::from_vec(buf));
    path.is_absolute().then(|| crate::join::clean(&path))
}

/// Machine-wide bases on macOS. Fixed locations, so resolution cannot fail.
#[derive(Debug, Clone)]
pub(crate) struct SystemBases {
    pub root: PathBuf,
}

impl SystemBases {
    // Same signature as the Windows resolver, which can fail and needs
    // the environment; here every location is fixed.
    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn resolve(_env: Env<'_>) -> Result<Self, &'static str> {
        Ok(Self {
            root: PathBuf::from("/"),
        })
    }

    // Same signature as the Windows layout, which reads resolved bases.
    #[allow(clippy::unused_self)]
    pub(crate) fn layout(&self, name: &str) -> Layout {
        let library = Path::new("/Library");
        let data = library.join("Application Support").join(name);
        Layout {
            config: library.join("Preferences").join(name),
            cache: library.join("Caches").join(name),
            logs: library.join("Logs").join(name),
            state: data.join("state"),
            runtime: Path::new("/var/run").join(name),
            temp: Path::new("/var/tmp").join(name),
            documents: Path::new("/Users/Shared").join(name),
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_env::{assert_distinct, env_from};
    use super::*;

    #[test]
    fn test_user_requires_absolute_home() {
        assert_eq!(UserBases::resolve(&env_from(&[])).err(), Some("home"));
        assert_eq!(
            UserBases::resolve(&env_from(&[("HOME", "x")])).err(),
            Some("home")
        );
    }

    #[test]
    fn test_user_layout_matches_apple_guidelines() {
        let env = env_from(&[("HOME", "/Users/u"), ("TMPDIR", "/var/folders/ab/T/")]);
        let l = UserBases::resolve(&env).map(|b| b.layout("HiveDB")).ok();
        let l = l.as_ref();
        assert_eq!(
            l.map(|l| l.data.as_path()),
            Some(Path::new("/Users/u/Library/Application Support/HiveDB"))
        );
        assert_eq!(
            l.map(|l| l.config.as_path()),
            Some(Path::new("/Users/u/Library/Preferences/HiveDB"))
        );
        assert_eq!(
            l.map(|l| l.temp.as_path()),
            Some(Path::new("/var/folders/ab/T/HiveDB"))
        );
        if let Some(l) = l {
            assert_distinct(l);
        }
    }

    #[test]
    fn test_user_missing_tmpdir_uses_the_system_user_temp() {
        let env = env_from(&[("HOME", "/Users/u")]);
        let assigned = || Some(PathBuf::from("/var/folders/xy/T"));
        let l = UserBases::resolve_with(&env, &assigned)
            .map(|b| b.layout("a"))
            .ok();
        assert_eq!(
            l.map(|l| l.temp),
            Some(PathBuf::from("/var/folders/xy/T/a"))
        );
        // Never the shared /tmp, even when the system lookup fails.
        let l = UserBases::resolve_with(&env, &|| None)
            .map(|b| b.layout("a"))
            .ok();
        assert_eq!(
            l.map(|l| l.temp),
            Some(PathBuf::from("/Users/u/Library/Caches/TemporaryItems/a"))
        );
    }

    #[test]
    fn test_darwin_user_temp_is_private_to_this_user() {
        use std::os::unix::fs::MetadataExt;
        let dir = darwin_user_temp();
        assert!(
            dir.as_ref().is_some_and(
                |d| d.starts_with("/var/folders") || d.starts_with("/private/var/folders")
            ),
            "{dir:?}"
        );
        let meta = dir.and_then(|d| std::fs::metadata(d).ok());
        assert!(meta.is_some_and(|m| m.mode() & 0o077 == 0), "not private");
    }

    #[test]
    fn test_system_layout_uses_library_domain() {
        let l = SystemBases::resolve(&|_: &str| None)
            .map(|s| s.layout("HiveDB"))
            .ok();
        let l = l.as_ref();
        assert_eq!(
            l.map(|l| l.data.as_path()),
            Some(Path::new("/Library/Application Support/HiveDB"))
        );
        if let Some(l) = l {
            assert_distinct(l);
        }
    }
}
