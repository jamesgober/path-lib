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
//! user temp is private without extra work.

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
        let home = absolute_var(env, "HOME").ok_or("home")?;
        let temp = absolute_var(env, "TMPDIR").unwrap_or_else(|| PathBuf::from("/tmp"));
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
    fn test_user_missing_tmpdir_falls_back_to_tmp() {
        let env = env_from(&[("HOME", "/Users/u")]);
        let l = UserBases::resolve(&env).map(|b| b.layout("a")).ok();
        assert_eq!(l.map(|l| l.temp), Some(PathBuf::from("/tmp/a")));
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
