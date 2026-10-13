//! Per-application directories.
//!
//! [`App`] is the main entry point of the crate. It resolves, once, the
//! eight directories an application keeps files in ([`Kind`]), for one
//! [`Scope`], and hands out hardened sub-paths inside them.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Reason, Result};
use crate::join::{clean, join_impl};
use crate::kind::{Kind, Scope};
use crate::platform::Layout;
use crate::{ops, os, rules};

/// The directories of one application, for one [`Scope`].
///
/// Construction validates the name and resolves every directory up
/// front; after that, every accessor is infallible and allocation-free,
/// and the `*_at` helpers allocate exactly one [`PathBuf`]. Nothing is
/// created on disk until [`ensure`](App::ensure) or [`create`](App::create)
/// is called.
///
/// `App` is cheap to construct (the OS lookups behind it are cached for
/// the life of the process), `Clone`, `Send`, and `Sync`.
///
/// # Examples
///
/// ```
/// use path_lib::{App, Kind};
///
/// let app = App::new("HiveDB")?;
///
/// // Base directories, e.g. ~/Library/Application Support/HiveDB on macOS.
/// let data = app.data();
///
/// // Hardened sub-paths: any slash style, always inside the base.
/// let db = app.data_at("/big/sloppy\\floppy/")?;
/// assert!(db.starts_with(data));
/// assert!(app.data_at("../../etc/passwd").is_err());
///
/// // The same thing, chosen at runtime.
/// assert_eq!(app.at(Kind::Data, "big/sloppy/floppy")?, db);
/// # Ok::<(), path_lib::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    name: Box<str>,
    scope: Scope,
    layout: Layout,
}

macro_rules! kind_accessors {
    ($($kind:ident => $get:ident, $at:ident, $what:literal;)*) => {
        $(
            #[doc = concat!("The ", $what, " directory. See [`Kind::", stringify!($kind), "`].")]
            #[must_use]
            #[inline]
            pub fn $get(&self) -> &Path {
                &self.layout.$get
            }

            #[doc = concat!("A hardened sub-path inside the ", $what, " directory.")]
            ///
            /// Shorthand for [`at`](App::at); see it for the accepted syntax.
            ///
            /// # Errors
            ///
            /// [`Error::InvalidPath`] if `sub` is unsafe or non-portable.
            #[inline]
            pub fn $at(&self, sub: impl AsRef<str>) -> Result<PathBuf> {
                join_impl(&self.layout.$get, sub.as_ref())
            }
        )*
    };
}

impl App {
    /// Directories for the current user.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidName`] if `name` is not a safe directory name.
    /// - [`Error::Unresolved`] if the home directory cannot be determined.
    pub fn new(name: impl AsRef<str>) -> Result<Self> {
        let name = validate_name(name.as_ref())?;
        let layout = os::user_bases()?.layout(name);
        Ok(Self::build(name, Scope::User, layout))
    }

    /// Machine-wide directories, for services and daemons.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidName`] if `name` is not a safe directory name.
    /// - [`Error::Unresolved`] on Windows if `%ProgramData%` cannot be
    ///   determined. Never fails on Unix.
    ///
    /// # Examples
    ///
    /// ```
    /// let app = path_lib::App::global("HiveDB")?;
    /// if cfg!(target_os = "linux") {
    ///     assert_eq!(app.config(), std::path::Path::new("/etc/HiveDB"));
    /// }
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    pub fn global(name: impl AsRef<str>) -> Result<Self> {
        let name = validate_name(name.as_ref())?;
        let layout = os::system_bases()?.layout(name);
        Ok(Self::build(name, Scope::Global, layout))
    }

    /// Every directory under `root`, one sub-directory per kind
    /// (`root/data`, `root/config`, ...).
    ///
    /// A relative `root` is resolved against the current directory once,
    /// here, so later changes of directory do not move the paths.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidName`] if `name` is not a safe directory name.
    /// - [`Error::InvalidPath`] if `root` is empty.
    /// - [`Error::Io`] if `root` is relative and the current directory
    ///   cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// // Keep everything next to the executable.
    /// let app = path_lib::App::portable("HiveDB", path_lib::exe_dir()?)?;
    /// assert!(app.data().ends_with("data"));
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    pub fn portable(name: impl AsRef<str>, root: impl AsRef<Path>) -> Result<Self> {
        let name = validate_name(name.as_ref())?;
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(Error::invalid_path("", Reason::Empty));
        }
        let root = if root.is_absolute() {
            clean(root)
        } else {
            let cwd = std::env::current_dir()
                .map_err(|e| Error::io("read current directory", root, e))?;
            clean(cwd.join(root))
        };
        Ok(Self::build(name, Scope::Portable, Layout::under(&root)))
    }

    /// Portable directories next to the executable when a
    /// [`PORTABLE_MARKER`] file (`.portable`) sits beside it, otherwise
    /// per-user directories as with [`App::new`].
    ///
    /// This is the usual "portable build" switch: ship the same binary in
    /// an installer and in a zip, and put an empty `.portable` file in the
    /// zip. The marker is checked once, here.
    ///
    /// # Errors
    ///
    /// As for [`App::new`] or [`App::portable`], whichever applies.
    ///
    /// # Examples
    ///
    /// ```
    /// use path_lib::{App, Scope};
    ///
    /// let app = App::auto("HiveDB")?;
    /// assert!(matches!(app.scope(), Scope::User | Scope::Portable));
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    pub fn auto(name: impl AsRef<str>) -> Result<Self> {
        let name = name.as_ref();
        match os::exe_dir() {
            Ok(dir) => Self::auto_in(name, dir),
            Err(_) => Self::new(name),
        }
    }

    /// [`App::auto`] with an explicit directory to look for the marker in.
    fn auto_in(name: &str, exe_dir: &Path) -> Result<Self> {
        if std::fs::symlink_metadata(exe_dir.join(PORTABLE_MARKER)).is_ok() {
            Self::portable(name, exe_dir)
        } else {
            Self::new(name)
        }
    }

    fn build(name: &str, scope: Scope, layout: Layout) -> Self {
        Self {
            name: name.into(),
            scope,
            layout,
        }
    }

    /// Replaces the directory for one kind.
    ///
    /// Use it to put data on a mounted volume, logs under `/var/log/...`
    /// for a user-scoped tool, and so on. The path must be absolute and is
    /// cleaned; it is not created until [`ensure`](App::ensure) or
    /// [`create`](App::create).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] when `dir` is empty ([`Reason::Empty`]),
    /// relative ([`Reason::NotAbsolute`]), or already the directory of
    /// another kind ([`Reason::SharedDir`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use path_lib::{App, Kind};
    ///
    /// let data = std::env::temp_dir().join("hivedb-data");
    /// let app = App::new("HiveDB")?.with_dir(Kind::Data, &data)?;
    /// assert_eq!(app.data(), path_lib::clean(&data));
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    pub fn with_dir(mut self, kind: Kind, dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let checked = self
            .check_override(kind, dir)
            .map_err(|reason| Error::invalid_path(&dir.to_string_lossy(), reason))?;
        *self.dir_mut(kind) = checked;
        Ok(self)
    }

    /// Validates a directory override and returns it cleaned.
    fn check_override(&self, kind: Kind, dir: &Path) -> std::result::Result<PathBuf, Reason> {
        if dir.as_os_str().is_empty() {
            return Err(Reason::Empty);
        }
        if !dir.is_absolute() {
            return Err(Reason::NotAbsolute);
        }
        let dir = clean(dir);
        if Kind::ALL
            .iter()
            .any(|other| *other != kind && self.dir(*other) == dir)
        {
            return Err(Reason::SharedDir);
        }
        Ok(dir)
    }

    /// Applies directory overrides from environment variables named
    /// `<PREFIX>_<KIND>_DIR`, such as `HIVEDB_DATA_DIR` or
    /// `HIVEDB_LOGS_DIR` (see [`Kind::env_name`]).
    ///
    /// Unset and empty variables leave the directory unchanged; every
    /// other value goes through [`with_dir`](App::with_dir). This is the
    /// conventional way to relocate a service's directories in containers
    /// and service managers without code changes.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidName`] when `prefix` is empty, starts with a
    ///   digit, or has characters other than ASCII letters, digits, and
    ///   `_`.
    /// - [`Error::InvalidPath`] when a variable holds a relative path or a
    ///   directory already used by another kind. The error's `path` shows
    ///   `NAME=value`, so the offending variable is easy to find.
    ///
    /// # Examples
    ///
    /// ```
    /// // With HIVEDB_DATA_DIR=/srv/hivedb set, data moves there.
    /// let app = path_lib::App::global("HiveDB")?.with_env("HIVEDB")?;
    /// # let _ = app;
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    pub fn with_env(self, prefix: impl AsRef<str>) -> Result<Self> {
        self.with_env_from(prefix.as_ref(), &|key| std::env::var_os(key))
    }

    /// [`App::with_env`] with an injectable environment, for tests.
    fn with_env_from(
        mut self,
        prefix: &str,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Result<Self> {
        validate_env_prefix(prefix)?;
        for kind in Kind::ALL {
            let key = format!("{prefix}_{}_DIR", kind.env_name());
            let Some(value) = env(&key).filter(|v| !v.is_empty()) else {
                continue;
            };
            let checked = self
                .check_override(kind, Path::new(&value))
                .map_err(|reason| {
                    Error::invalid_path(&format!("{key}={}", Path::new(&value).display()), reason)
                })?;
            *self.dir_mut(kind) = checked;
        }
        Ok(self)
    }

    /// Every kind with its directory, in [`Kind::ALL`] order.
    ///
    /// # Examples
    ///
    /// ```
    /// let app = path_lib::App::new("HiveDB")?;
    /// for (kind, dir) in app.dirs() {
    ///     assert_eq!(app.dir(kind), dir);
    /// }
    /// # Ok::<(), path_lib::Error>(())
    /// ```
    pub fn dirs(&self) -> impl Iterator<Item = (Kind, &Path)> + '_ {
        Kind::ALL.iter().map(move |kind| (*kind, self.dir(*kind)))
    }

    fn dir_mut(&mut self, kind: Kind) -> &mut PathBuf {
        match kind {
            Kind::Data => &mut self.layout.data,
            Kind::Config => &mut self.layout.config,
            Kind::Cache => &mut self.layout.cache,
            Kind::Logs => &mut self.layout.logs,
            Kind::State => &mut self.layout.state,
            Kind::Runtime => &mut self.layout.runtime,
            Kind::Temp => &mut self.layout.temp,
            Kind::Documents => &mut self.layout.documents,
        }
    }

    /// The application name, exactly as given.
    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The scope the directories were resolved for.
    #[must_use]
    #[inline]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The directory for `kind`.
    #[must_use]
    #[inline]
    pub fn dir(&self, kind: Kind) -> &Path {
        match kind {
            Kind::Data => &self.layout.data,
            Kind::Config => &self.layout.config,
            Kind::Cache => &self.layout.cache,
            Kind::Logs => &self.layout.logs,
            Kind::State => &self.layout.state,
            Kind::Runtime => &self.layout.runtime,
            Kind::Temp => &self.layout.temp,
            Kind::Documents => &self.layout.documents,
        }
    }

    /// A hardened sub-path inside the directory for `kind`.
    ///
    /// `sub` may use `/`, `\`, or both, with or without leading and
    /// trailing separators; the result uses the platform separator and is
    /// always inside the base directory. An empty `sub` returns the base
    /// directory itself. See [`join`](crate::join) for every rule.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] if `sub` is unsafe or non-portable, such as
    /// `../x`, `C:\x`, `a:b`, or `CON`.
    #[inline]
    pub fn at(&self, kind: Kind, sub: impl AsRef<str>) -> Result<PathBuf> {
        join_impl(self.dir(kind), sub.as_ref())
    }

    kind_accessors! {
        Data => data, data_at, "persistent data";
        Config => config, config_at, "configuration";
        Cache => cache, cache_at, "cache";
        Logs => logs, logs_at, "log";
        State => state, state_at, "state";
        Runtime => runtime, runtime_at, "runtime";
        Temp => temp, temp_at, "private temporary";
        Documents => documents, documents_at, "user-visible documents";
    }

    /// Creates the directory for `kind` if it is missing, and returns it.
    ///
    /// On Unix, directories created for [`Scope::User`] and
    /// [`Scope::Portable`] are private to the owner (mode `0700`);
    /// [`Scope::Global`] directories are world-readable (`0755`), as
    /// system services expect. A symbolic link where the directory should
    /// be is refused rather than followed, so a link planted in a shared
    /// location cannot redirect the application's files.
    ///
    /// # Errors
    ///
    /// - [`Error::Refused`] if a symbolic link occupies the path.
    /// - [`Error::AlreadyExists`] if a file occupies the path.
    /// - [`Error::Io`] if the directory cannot be created, for example
    ///   when a global directory is created without privileges.
    pub fn ensure(&self, kind: Kind) -> Result<&Path> {
        let dir = self.dir(kind);
        ops::create_dir(dir, Some(self.dir_mode()))?;
        Ok(dir)
    }

    /// Runs [`ensure`](App::ensure) for every [`Kind`].
    ///
    /// # Errors
    ///
    /// Stops at the first failure and returns it; directories created
    /// before the failure are kept.
    pub fn ensure_all(&self) -> Result<()> {
        for kind in Kind::ALL {
            let _dir = self.ensure(kind)?;
        }
        Ok(())
    }

    /// Creates a sub-directory (and any missing parents) inside the
    /// directory for `kind`, and returns its path.
    ///
    /// Uses the same permissions and symbolic-link rules as
    /// [`ensure`](App::ensure).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for an unsafe `sub`, otherwise as for
    /// [`ensure`](App::ensure).
    pub fn create(&self, kind: Kind, sub: impl AsRef<str>) -> Result<PathBuf> {
        let path = self.at(kind, sub)?;
        ops::create_dir(&path, Some(self.dir_mode()))?;
        Ok(path)
    }

    /// Lists the entries of a sub-directory inside the directory for
    /// `kind`, sorted. An empty `sub` lists the base directory.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for an unsafe `sub`, otherwise as for
    /// [`list`](crate::list).
    pub fn list(&self, kind: Kind, sub: impl AsRef<str>) -> Result<Vec<PathBuf>> {
        ops::list(self.at(kind, sub)?)
    }

    /// Deletes a file or directory tree inside the directory for `kind`.
    /// An empty `sub` deletes the base directory itself (useful to clear
    /// a cache). Returns `Ok(false)` if nothing was there.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for an unsafe `sub`, otherwise as for
    /// [`remove`](crate::remove).
    pub fn remove(&self, kind: Kind, sub: impl AsRef<str>) -> Result<bool> {
        ops::remove(self.at(kind, sub)?)
    }

    /// Finds entries whose name matches `pattern` anywhere under the
    /// directory for `kind`. See [`find`](crate::find).
    ///
    /// # Errors
    ///
    /// As for [`find`](crate::find).
    pub fn find(&self, kind: Kind, pattern: impl AsRef<str>) -> Result<Vec<PathBuf>> {
        ops::find(self.dir(kind), pattern)
    }

    fn dir_mode(&self) -> u32 {
        match self.scope {
            Scope::Global => 0o755,
            Scope::User | Scope::Portable => 0o700,
        }
    }
}

impl fmt::Display for App {
    /// A table of every directory, one per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} ({} scope)", self.name, self.scope)?;
        for (i, kind) in Kind::ALL.iter().enumerate() {
            write!(f, "  {:<9} {}", kind.name(), self.dir(*kind).display())?;
            if i + 1 < Kind::ALL.len() {
                f.write_str("\n")?;
            }
        }
        Ok(())
    }
}

/// Name of the marker file that switches [`App::auto`] to portable mode.
pub const PORTABLE_MARKER: &str = ".portable";

/// Environment-variable prefixes: ASCII letters, digits, and `_`, not
/// starting with a digit, so `<PREFIX>_DATA_DIR` is a portable variable
/// name in every shell and service manager.
fn validate_env_prefix(prefix: &str) -> Result<()> {
    let mut chars = prefix.chars();
    match chars.next() {
        None => return Err(Error::invalid_name(prefix, Reason::Empty)),
        Some(c) if c.is_ascii_digit() => {
            return Err(Error::invalid_name(prefix, Reason::ReservedChar(c)));
        }
        Some(_) => {}
    }
    if let Some(bad) = prefix
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
    {
        return Err(Error::invalid_name(prefix, Reason::ReservedChar(bad)));
    }
    Ok(())
}

/// Application names become a directory name on every platform, so they
/// follow the single-segment rules, plus the first-segment rules of
/// [`crate::join`] (no `~`, no drive letter).
fn validate_name(name: &str) -> Result<&str> {
    rules::check_segment(name, false).map_err(|r| Error::invalid_name(name, r))?;
    if name.starts_with('~') {
        return Err(Error::invalid_name(name, Reason::HomeShortcut));
    }
    Ok(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_name_accepts_ordinary_names() {
        for n in ["HiveDB", "hive-db", "Hive DB", "app_1", "名前", ".hidden"] {
            assert!(validate_name(n).is_ok(), "{n:?}");
        }
    }

    #[test]
    fn test_validate_name_rejects_unsafe_names() {
        let cases = [
            ("", Reason::Empty),
            ("a/b", Reason::Separator),
            ("a\\b", Reason::Separator),
            ("..", Reason::TrailingDotOrSpace),
            ("C:", Reason::ReservedChar(':')),
            ("CON", Reason::ReservedName),
            ("~app", Reason::HomeShortcut),
            ("a\0", Reason::ControlChar),
        ];
        for (name, want) in cases {
            let err = validate_name(name).unwrap_err();
            assert!(matches!(err, Error::InvalidName { .. }), "{name:?}");
            assert_eq!(err.reason(), Some(want), "{name:?}");
        }
    }

    #[test]
    fn test_portable_layout_and_accessors_agree() {
        let root = std::env::temp_dir().join("path-lib-unit-portable");
        let app = App::portable("Unit", &root).unwrap();
        assert_eq!(app.scope(), Scope::Portable);
        assert_eq!(app.name(), "Unit");
        for kind in Kind::ALL {
            assert!(app.dir(kind).starts_with(clean(&root)));
            assert_eq!(app.at(kind, "").unwrap(), app.dir(kind));
        }
        assert_eq!(app.data(), app.dir(Kind::Data));
        assert_eq!(app.logs_at("x").unwrap(), app.logs().join("x"));
    }

    fn sandbox_app() -> App {
        App::portable("Unit", std::env::temp_dir().join("path-lib-unit-overrides")).unwrap()
    }

    fn abs(name: &str) -> PathBuf {
        std::env::temp_dir().join("path-lib-unit-abs").join(name)
    }

    #[test]
    fn test_with_dir_replaces_one_kind_cleaned() {
        let target = abs("data");
        let raw = format!("{}{}.", target.display(), std::path::MAIN_SEPARATOR);
        let app = sandbox_app().with_dir(Kind::Data, &raw).unwrap();
        assert_eq!(app.data(), target);
        assert_eq!(app.data_at("x").unwrap(), target.join("x"));
        assert_ne!(app.config(), target);
    }

    #[test]
    fn test_with_dir_rejects_empty_relative_and_shared() {
        let err = |r: Result<App>| r.err().and_then(|e| e.reason());
        assert_eq!(
            err(sandbox_app().with_dir(Kind::Data, "")),
            Some(Reason::Empty)
        );
        assert_eq!(
            err(sandbox_app().with_dir(Kind::Data, "relative/dir")),
            Some(Reason::NotAbsolute)
        );
        let app = sandbox_app();
        let cache = app.cache().to_path_buf();
        assert_eq!(
            err(app.with_dir(Kind::Data, cache)),
            Some(Reason::SharedDir)
        );
    }

    #[test]
    fn test_with_dir_same_kind_same_path_is_allowed() {
        let app = sandbox_app();
        let data = app.data().to_path_buf();
        assert_eq!(app.with_dir(Kind::Data, &data).unwrap().data(), data);
    }

    fn env_of(pairs: Vec<(String, OsString)>) -> impl Fn(&str) -> Option<OsString> {
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn test_with_env_applies_set_variables_only() {
        let env = env_of(vec![
            ("APP_DATA_DIR".into(), abs("d").into_os_string()),
            ("APP_LOGS_DIR".into(), abs("l").into_os_string()),
            ("APP_CACHE_DIR".into(), OsString::new()),
        ]);
        let base = sandbox_app();
        let cache = base.cache().to_path_buf();
        let app = base.with_env_from("APP", &env).unwrap();
        assert_eq!(app.data(), abs("d"));
        assert_eq!(app.logs(), abs("l"));
        assert_eq!(app.cache(), cache, "empty value leaves the default");
    }

    #[test]
    fn test_with_env_can_override_every_kind() {
        let pairs: Vec<(String, OsString)> = Kind::ALL
            .iter()
            .map(|k| {
                (
                    format!("ALL_{}_DIR", k.env_name()),
                    abs(k.name()).into_os_string(),
                )
            })
            .collect();
        let app = sandbox_app().with_env_from("ALL", &env_of(pairs)).unwrap();
        for kind in Kind::ALL {
            assert_eq!(app.dir(kind), abs(kind.name()), "{kind}");
        }
    }

    #[test]
    fn test_dir_mode_by_scope() {
        let mut app = sandbox_app();
        assert_eq!(app.dir_mode(), 0o700);
        app.scope = Scope::Global;
        assert_eq!(app.dir_mode(), 0o755);
        app.scope = Scope::User;
        assert_eq!(app.dir_mode(), 0o700);
    }

    #[test]
    fn test_with_env_reports_variable_name_on_bad_value() {
        let env = env_of(vec![("APP_STATE_DIR".into(), "rel".into())]);
        let err = sandbox_app().with_env_from("APP", &env).unwrap_err();
        assert_eq!(err.reason(), Some(Reason::NotAbsolute));
        assert!(err.to_string().contains("APP_STATE_DIR=rel"), "{err}");
    }

    #[test]
    fn test_with_env_validates_prefix() {
        let none = |_: &str| None;
        for bad in ["", "1APP", "APP-X", "APP X", "ÄPP"] {
            assert!(
                matches!(
                    sandbox_app().with_env_from(bad, &none),
                    Err(Error::InvalidName { .. })
                ),
                "{bad:?}"
            );
        }
        assert!(sandbox_app().with_env_from("_APP_2", &none).is_ok());
    }

    #[test]
    fn test_auto_in_switches_on_marker() {
        let dir = std::env::temp_dir().join(format!("path-lib-unit-auto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let without = App::auto_in("Unit", &dir).unwrap();
        assert_eq!(without.scope(), Scope::User);
        std::fs::write(dir.join(PORTABLE_MARKER), b"").unwrap();
        let with = App::auto_in("Unit", &dir).unwrap();
        assert_eq!(with.scope(), Scope::Portable);
        assert!(with.data().starts_with(clean(&dir)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_dirs_iterates_every_kind_in_order() {
        let app = sandbox_app();
        let kinds: Vec<Kind> = app.dirs().map(|(k, _)| k).collect();
        assert_eq!(kinds, Kind::ALL);
        assert!(app.dirs().all(|(k, d)| app.dir(k) == d));
    }

    #[test]
    fn test_portable_rejects_empty_root() {
        assert!(App::portable("a", "").is_err());
    }

    #[test]
    fn test_display_lists_every_kind() {
        let app = App::portable("Shown", std::env::temp_dir()).unwrap();
        let text = app.to_string();
        assert!(text.starts_with("Shown (portable scope)"));
        for kind in Kind::ALL {
            assert!(text.contains(kind.name()));
        }
        assert!(!text.ends_with('\n'));
    }
}
