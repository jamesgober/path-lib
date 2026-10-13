//! Per-platform directory tables.
//!
//! Every supported platform has one module that answers the same four
//! questions, so the full layout for an OS can be read (and audited) in a
//! single file:
//!
//! - `UserBases::resolve(env)` — the current user's folders. Needs a home
//!   directory and fails without one.
//! - `UserBases::layout(name)` — where application `name` keeps each
//!   [`Kind`](crate::Kind) of file for this user.
//! - `SystemBases::resolve(env)` — machine-wide folders. Never needs a
//!   home directory.
//! - `SystemBases::layout(name)` — where application `name` keeps each
//!   kind of file for the whole machine.
//!
//! Resolution reads the environment through an [`Env`] closure rather
//! than `std::env`, so every rule can be unit-tested against any
//! environment, including hostile ones, without mutating the process
//! environment (which is `unsafe` under edition 2024).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::join::clean;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{SystemBases, UserBases};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::{SystemBases, UserBases};

#[cfg(all(unix, not(target_os = "macos")))]
mod xdg;
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) use xdg::{SystemBases, UserBases};

// The user-dirs.dirs parser is pure string handling, so it is compiled
// and tested on every platform, not only where it is used.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
pub(crate) mod user_dirs;

#[cfg(not(any(unix, windows)))]
compile_error!(
    "path-lib supports Unix-like targets (Linux, macOS, BSD) and Windows; \
     this target has no home directory model or filesystem layout to resolve"
);

/// An environment-variable lookup.
pub(crate) type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The real process environment.
pub(crate) fn process_env(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

/// The current user's well-known folders. Shared by every platform.
#[derive(Debug, Clone)]
pub(crate) struct Folders {
    pub home: PathBuf,
    pub documents: PathBuf,
    pub desktop: PathBuf,
    pub downloads: PathBuf,
    pub pictures: PathBuf,
    pub music: PathBuf,
    pub videos: PathBuf,
}

/// Where one application keeps each kind of file, for one scope.
///
/// Invariant (tested per platform): the eight directories are pairwise
/// distinct, so clearing one kind can never clear another wholesale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub state: PathBuf,
    pub runtime: PathBuf,
    pub temp: PathBuf,
    pub documents: PathBuf,
}

impl Layout {
    /// The portable layout: one sub-directory per kind under `root`.
    pub(crate) fn under(root: &Path) -> Self {
        Self {
            data: root.join("data"),
            config: root.join("config"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            state: root.join("state"),
            runtime: root.join("run"),
            temp: root.join("temp"),
            documents: root.join("documents"),
        }
    }

    #[cfg(test)]
    pub(crate) fn all(&self) -> [&Path; 8] {
        [
            &self.data,
            &self.config,
            &self.cache,
            &self.logs,
            &self.state,
            &self.runtime,
            &self.temp,
            &self.documents,
        ]
    }
}

/// Reads `key` as an absolute directory, cleaned.
///
/// Relative or empty values are ignored rather than trusted: a relative
/// `XDG_*` or `LOCALAPPDATA` would make every resolved path depend on the
/// current working directory. The XDG Base Directory specification
/// requires the same treatment.
pub(crate) fn absolute_var(env: Env<'_>, key: &str) -> Option<PathBuf> {
    let value = env(key)?;
    if value.is_empty() {
        return None;
    }
    let path = Path::new(&value);
    if !path.is_absolute() {
        return None;
    }
    Some(clean(path))
}

#[cfg(test)]
pub(crate) mod test_env {
    use std::collections::HashMap;
    use std::ffi::OsString;

    /// Builds an environment lookup from key/value pairs.
    pub(crate) fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(*v)))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    /// Asserts the layout invariant: no two kinds share a directory.
    pub(crate) fn assert_distinct(layout: &super::Layout) {
        let all = layout.all();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two kinds share a directory: {a:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_env::{assert_distinct, env_from};
    use super::*;

    #[test]
    fn test_absolute_var_rejects_relative_and_empty() {
        let env = env_from(&[("REL", "relative/dir"), ("EMPTY", ""), ("DOT", ".")]);
        assert_eq!(absolute_var(&env, "REL"), None);
        assert_eq!(absolute_var(&env, "EMPTY"), None);
        assert_eq!(absolute_var(&env, "DOT"), None);
        assert_eq!(absolute_var(&env, "MISSING"), None);
    }

    #[test]
    fn test_absolute_var_cleans_value() {
        let raw = if cfg!(windows) {
            "C:/Users//x/./y/"
        } else {
            "/home//x/./y/"
        };
        let want = if cfg!(windows) {
            r"C:\Users\x\y"
        } else {
            "/home/x/y"
        };
        let env = env_from(&[("V", raw)]);
        assert_eq!(absolute_var(&env, "V"), Some(PathBuf::from(want)));
    }

    #[test]
    fn test_portable_layout_is_distinct_and_under_root() {
        let root = PathBuf::from("portable-root");
        let layout = Layout::under(&root);
        assert_distinct(&layout);
        for p in layout.all() {
            assert!(p.starts_with(&root));
        }
    }
}
