//! The kinds of application directory and the scopes they resolve for.

use std::fmt;

/// Who an application's directories belong to.
///
/// Marked `#[non_exhaustive]` so a scope can be added in a minor release;
/// `match` on it with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
///
/// Marked `#[non_exhaustive]` so a kind can be added in a minor release;
/// `match` on it with a wildcard arm, or iterate [`Kind::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

    /// The suffix used for this kind's environment variable in
    /// [`App::with_env`](crate::App::with_env): `DATA`, `CONFIG`, `CACHE`, `LOGS`, `STATE`,
    /// `RUNTIME`, `TEMP`, or `DOCUMENTS`.
    #[must_use]
    pub const fn env_name(self) -> &'static str {
        match self {
            Self::Data => "DATA",
            Self::Config => "CONFIG",
            Self::Cache => "CACHE",
            Self::Logs => "LOGS",
            Self::State => "STATE",
            Self::Runtime => "RUNTIME",
            Self::Temp => "TEMP",
            Self::Documents => "DOCUMENTS",
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_all_is_complete_and_named() {
        assert_eq!(Kind::ALL.len(), 8);
        for k in Kind::ALL {
            assert_eq!(k.to_string(), k.name());
        }
    }

    #[test]
    fn test_env_names_are_unique_uppercase() {
        let names: Vec<_> = Kind::ALL.iter().map(|k| k.env_name()).collect();
        for (i, n) in names.iter().enumerate() {
            assert_eq!(n.to_ascii_uppercase(), *n);
            assert!(!names[i + 1..].contains(n));
        }
    }

    #[test]
    fn test_scope_display_names() {
        assert_eq!(Scope::User.to_string(), "user");
        assert_eq!(Scope::Global.to_string(), "global");
        assert_eq!(Scope::Portable.to_string(), "portable");
    }
}
