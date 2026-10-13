//! Per-application directories.
//!
//! [`App`] is the main entry point of the crate. It resolves, once, the
//! eight directories an application keeps files in ([`Kind`]), for one
//! [`Scope`], and hands out hardened sub-paths inside them.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Reason, Result};
use crate::join::{clean, join_impl};
use crate::platform::Layout;
use crate::{ops, os, rules};

/// Who an application's directories belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The current user only, under their home directory. The right
    /// choice for desktop apps and CLI tools. Needs no special privileges.
    User,
    /// The whole machine (`/var/lib`, `/etc`, `/Library`, `%ProgramData%`).
    /// The right choice for services and daemons. Creating these
    /// directories usually needs administrator or root privileges.
    Global,
    /// Everything under one caller-chosen folder, such as the directory
    /// next to the executable. For portable installs, development
    /// sandboxes, and tests.
    Portable,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::User => "user",
            Self::Global => "global",
            Self::Portable => "portable",
        })
    }
}

/// A kind of application directory.
///
/// Every kind resolves to a different directory, so clearing one (say,
/// the cache) can never wipe another wholesale. Some kinds nest inside
/// [`Kind::Data`] on platforms that have no separate location for them;
/// the per-platform tables are in the crate documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Persistent application data: databases, user content, indexes.
    /// Back this up.
    Data,
    /// Configuration files the user or an administrator may edit.
    Config,
    /// Data that can be regenerated. Safe to delete at any time.
    Cache,
    /// Log files.
    Logs,
    /// State that should survive restarts but is not worth backing up:
    /// history, recently-used lists, window positions.
    State,
    /// Sockets, PID files, and locks that only matter while running.
    Runtime,
    /// Private scratch space for the application.
    Temp,
    /// Files the user is meant to see and open, inside their documents.
    Documents,
}

impl Kind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Data,
        Self::Config,
        Self::Cache,
        Self::Logs,
        Self::State,
        Self::Runtime,
        Self::Temp,
        Self::Documents,
    ];

    /// A short lowercase name, such as `"data"` or `"logs"`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Config => "config",
            Self::Cache => "cache",
            Self::Logs => "logs",
            Self::State => "state",
            Self::Runtime => "runtime",
            Self::Temp => "temp",
            Self::Documents => "documents",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

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

    fn build(name: &str, scope: Scope, layout: Layout) -> Self {
        Self {
            name: name.into(),
            scope,
            layout,
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
    fn test_kind_all_is_complete_and_named() {
        assert_eq!(Kind::ALL.len(), 8);
        for k in Kind::ALL {
            assert_eq!(k.to_string(), k.name());
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
