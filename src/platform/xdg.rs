//! Linux and the BSDs: the XDG Base Directory specification for user
//! scope, the Filesystem Hierarchy Standard for global scope.
//!
//! | Kind      | User                                     | Global              |
//! |-----------|------------------------------------------|---------------------|
//! | data      | `$XDG_DATA_HOME/<app>` (`~/.local/share`)| `/var/lib/<app>`    |
//! | config    | `$XDG_CONFIG_HOME/<app>` (`~/.config`)   | `/etc/<app>`        |
//! | cache     | `$XDG_CACHE_HOME/<app>` (`~/.cache`)     | `/var/cache/<app>`  |
//! | logs      | `$XDG_STATE_HOME/<app>/logs`             | `/var/log/<app>`    |
//! | state     | `$XDG_STATE_HOME/<app>` (`~/.local/state`)| `/var/lib/<app>/state` |
//! | runtime   | `$XDG_RUNTIME_DIR/<app>`, else `<state>/run` | `/run/<app>` (`/var/run` on BSD) |
//! | temp      | `$XDG_CACHE_HOME/<app>/tmp`              | `/var/tmp/<app>`    |
//! | documents | `XDG_DOCUMENTS_DIR/<app>` (`~/Documents`)| `/srv/<app>`        |
//!
//! User temp lives under the user's cache rather than `/tmp`: on Linux
//! `/tmp` is usually shared by every user, and a predictable name there
//! (`/tmp/<app>`) can be pre-created by another user to hijack writes.
//! The cache directory is private to the user.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::user_dirs::{self, MAX_FILE_BYTES};
use super::{Env, Folders, Layout, absolute_var};

/// Per-user bases on XDG systems.
#[derive(Debug, Clone)]
pub(crate) struct UserBases {
    pub folders: Folders,
    data_home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    state_home: PathBuf,
    runtime_dir: Option<PathBuf>,
}

impl UserBases {
    pub(crate) fn resolve(env: Env<'_>) -> Result<Self, &'static str> {
        Self::resolve_with(env, &read_small_file)
    }

    /// Resolution with an injectable file reader, for tests.
    pub(crate) fn resolve_with(
        env: Env<'_>,
        read: &dyn Fn(&Path) -> Option<String>,
    ) -> Result<Self, &'static str> {
        let home = absolute_var(env, "HOME").ok_or("home")?;
        let xdg = |key: &str, fallback: &str| {
            absolute_var(env, key).unwrap_or_else(|| home.join(fallback))
        };
        let data_home = xdg("XDG_DATA_HOME", ".local/share");
        let config_home = xdg("XDG_CONFIG_HOME", ".config");
        let cache_home = xdg("XDG_CACHE_HOME", ".cache");
        let state_home = xdg("XDG_STATE_HOME", ".local/state");
        let runtime_dir = absolute_var(env, "XDG_RUNTIME_DIR");

        let named = read(&config_home.join("user-dirs.dirs"))
            .map(|text| user_dirs::parse(&text, &home))
            .unwrap_or_default();
        let pick =
            |found: Option<PathBuf>, fallback: &str| found.unwrap_or_else(|| home.join(fallback));
        let folders = Folders {
            documents: pick(named.documents, "Documents"),
            desktop: pick(named.desktop, "Desktop"),
            downloads: pick(named.download, "Downloads"),
            pictures: pick(named.pictures, "Pictures"),
            music: pick(named.music, "Music"),
            videos: pick(named.videos, "Videos"),
            home,
        };
        Ok(Self {
            folders,
            data_home,
            config_home,
            cache_home,
            state_home,
            runtime_dir,
        })
    }

    pub(crate) fn layout(&self, name: &str) -> Layout {
        let state = self.state_home.join(name);
        let cache = self.cache_home.join(name);
        Layout {
            data: self.data_home.join(name),
            config: self.config_home.join(name),
            logs: state.join("logs"),
            runtime: match &self.runtime_dir {
                Some(dir) => dir.join(name),
                None => state.join("run"),
            },
            temp: cache.join("tmp"),
            documents: self.folders.documents.join(name),
            cache,
            state,
        }
    }
}

/// Machine-wide bases on XDG systems. These are fixed by the FHS, so
/// resolution cannot fail.
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
        let lib = Path::new("/var/lib").join(name);
        Layout {
            data: lib.clone(),
            config: Path::new("/etc").join(name),
            cache: Path::new("/var/cache").join(name),
            logs: Path::new("/var/log").join(name),
            state: lib.join("state"),
            runtime: Path::new(RUN_DIR).join(name),
            temp: Path::new("/var/tmp").join(name),
            documents: Path::new("/srv").join(name),
        }
    }
}

#[cfg(target_os = "linux")]
const RUN_DIR: &str = "/run";
#[cfg(not(target_os = "linux"))]
const RUN_DIR: &str = "/var/run";

/// Reads a small UTF-8 text file, or returns `None` if it is missing,
/// unreadable, too large, or not UTF-8. The `user-dirs.dirs` file is
/// optional by specification, so every failure means "not configured".
fn read_small_file(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut text = String::new();
    let _bytes = file.take(MAX_FILE_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::super::test_env::{assert_distinct, env_from};
    use super::*;

    fn no_file(_: &Path) -> Option<String> {
        None
    }

    #[test]
    fn test_user_requires_absolute_home() {
        for env in [
            env_from(&[]),
            env_from(&[("HOME", "relative")]),
            env_from(&[("HOME", "")]),
        ] {
            assert_eq!(UserBases::resolve_with(&env, &no_file).err(), Some("home"));
        }
    }

    #[test]
    fn test_user_defaults_follow_xdg_spec() {
        let env = env_from(&[("HOME", "/home/u")]);
        let b = UserBases::resolve_with(&env, &no_file).ok();
        let l = b.as_ref().map(|b| b.layout("app"));
        let l = l.as_ref();
        assert_eq!(
            l.map(|l| l.data.as_path()),
            Some(Path::new("/home/u/.local/share/app"))
        );
        assert_eq!(
            l.map(|l| l.config.as_path()),
            Some(Path::new("/home/u/.config/app"))
        );
        assert_eq!(
            l.map(|l| l.cache.as_path()),
            Some(Path::new("/home/u/.cache/app"))
        );
        assert_eq!(
            l.map(|l| l.state.as_path()),
            Some(Path::new("/home/u/.local/state/app"))
        );
        assert_eq!(
            l.map(|l| l.logs.as_path()),
            Some(Path::new("/home/u/.local/state/app/logs"))
        );
        assert_eq!(
            l.map(|l| l.runtime.as_path()),
            Some(Path::new("/home/u/.local/state/app/run"))
        );
        assert_eq!(
            l.map(|l| l.temp.as_path()),
            Some(Path::new("/home/u/.cache/app/tmp"))
        );
        assert_eq!(
            l.map(|l| l.documents.as_path()),
            Some(Path::new("/home/u/Documents/app"))
        );
    }

    #[test]
    fn test_user_xdg_overrides_and_runtime_dir() {
        let env = env_from(&[
            ("HOME", "/home/u"),
            ("XDG_DATA_HOME", "/d/"),
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_CACHE_HOME", "/k"),
            ("XDG_STATE_HOME", "/s"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        let l = UserBases::resolve_with(&env, &no_file)
            .map(|b| b.layout("app"))
            .ok();
        let l = l.as_ref();
        assert_eq!(l.map(|l| l.data.as_path()), Some(Path::new("/d/app")));
        assert_eq!(l.map(|l| l.config.as_path()), Some(Path::new("/c/app")));
        assert_eq!(
            l.map(|l| l.runtime.as_path()),
            Some(Path::new("/run/user/1000/app"))
        );
    }

    #[test]
    fn test_user_relative_xdg_values_ignored() {
        let env = env_from(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "rel/cfg")]);
        let l = UserBases::resolve_with(&env, &no_file)
            .map(|b| b.layout("app"))
            .ok();
        assert_eq!(
            l.map(|l| l.config),
            Some(PathBuf::from("/home/u/.config/app"))
        );
    }

    #[test]
    fn test_user_dirs_file_is_honoured() {
        let env = env_from(&[("HOME", "/home/u")]);
        let read = |p: &Path| {
            (p == Path::new("/home/u/.config/user-dirs.dirs"))
                .then(|| "XDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"".to_owned())
        };
        let b = UserBases::resolve_with(&env, &read).ok();
        assert_eq!(
            b.map(|b| b.folders.documents),
            Some(PathBuf::from("/home/u/Dokumente"))
        );
    }

    #[test]
    fn test_user_layout_kinds_are_distinct() {
        let env = env_from(&[("HOME", "/home/u")]);
        if let Ok(b) = UserBases::resolve_with(&env, &no_file) {
            assert_distinct(&b.layout("app"));
        }
        let env = env_from(&[("HOME", "/home/u"), ("XDG_RUNTIME_DIR", "/run/user/1")]);
        if let Ok(b) = UserBases::resolve_with(&env, &no_file) {
            assert_distinct(&b.layout("app"));
        }
    }

    #[test]
    fn test_system_layout_is_fhs() {
        let s = SystemBases::resolve(&|_: &str| None);
        let l = s.map(|s| s.layout("app")).ok();
        let l = l.as_ref();
        assert_eq!(l.map(|l| l.data.as_path()), Some(Path::new("/var/lib/app")));
        assert_eq!(l.map(|l| l.config.as_path()), Some(Path::new("/etc/app")));
        assert_eq!(l.map(|l| l.logs.as_path()), Some(Path::new("/var/log/app")));
        if let Some(l) = l {
            assert_distinct(l);
        }
    }
}
