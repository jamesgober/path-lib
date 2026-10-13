//! Error type for every fallible operation in the crate.
//!
//! One enum covers the crate because every operation shares the same
//! three failure families: rejected input ([`Error::InvalidName`],
//! [`Error::InvalidPath`]), a directory the OS could not tell us about
//! ([`Error::Unresolved`]), and filesystem failures ([`Error::Io`] plus
//! the guard variants). Splitting these per module would force callers
//! to convert between near-identical types at every boundary.
//!
//! Caller input echoed back in an error is truncated to
//! [`PREVIEW_CHARS`] characters and printed with escapes, so a hostile
//! string cannot flood a log line or smuggle control characters into it.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Shorthand for `Result<T, path_lib::Error>`.
pub type Result<T> = std::result::Result<T, Error>;

/// Maximum number of characters of caller input kept in an error.
pub const PREVIEW_CHARS: usize = 96;

/// Everything that can go wrong in `path-lib`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An application name passed to [`App`](crate::App) was rejected.
    ///
    /// The name becomes a directory name on every platform, so it is held
    /// to the same rules as a single path segment. Pick a different name;
    /// retrying with the same input fails the same way.
    InvalidName {
        /// The rejected name, truncated to [`PREVIEW_CHARS`] characters.
        name: String,
        /// Which rule the name broke.
        reason: Reason,
    },

    /// A relative sub-path, search pattern, or segment was rejected.
    ///
    /// Raised before anything touches the filesystem. The input is
    /// unsafe or non-portable as written; fix the input (or run it through
    /// [`sanitize`](crate::sanitize) first when it comes from an end user).
    InvalidPath {
        /// The rejected input, truncated to [`PREVIEW_CHARS`] characters.
        path: String,
        /// Which rule the input broke.
        reason: Reason,
    },

    /// The operating system did not report a directory this call needs.
    ///
    /// Typically the home directory on a Unix process started without
    /// `HOME` (some service managers do this). Set the variable, use a
    /// [`Scope::Global`](crate::Scope::Global) or
    /// [`App::portable`](crate::App::portable) layout instead, or treat the
    /// feature that needed the directory as unavailable.
    Unresolved {
        /// Name of the directory that could not be resolved, such as `"home"`.
        dir: &'static str,
    },

    /// A create, copy, or move target already exists.
    ///
    /// `path-lib` never overwrites. Remove the target first, or choose a
    /// different destination.
    AlreadyExists {
        /// The existing path.
        path: PathBuf,
    },

    /// The source of a copy, move, list, or find does not exist.
    NotFound {
        /// The missing path.
        path: PathBuf,
    },

    /// A guard refused an operation that would be destructive or unsafe.
    ///
    /// Examples: removing a filesystem root or the home directory, copying
    /// a directory into itself, or copying a tree that contains a symbolic
    /// link. The operation was not started; nothing was changed.
    Refused {
        /// The path the guard refused to act on.
        path: PathBuf,
        /// Why the guard refused.
        reason: &'static str,
    },

    /// The operating system returned an I/O error.
    ///
    /// `op` names the step that failed and `path` the path it failed on;
    /// the underlying [`io::Error`] is available through
    /// [`std::error::Error::source`]. Permission and disk-space errors are
    /// the usual causes and are not retried by this crate.
    Io {
        /// The step that failed, such as `"create directory"`.
        op: &'static str,
        /// The path the step was operating on.
        path: PathBuf,
        /// The error reported by the operating system.
        source: io::Error,
    },
}

/// The specific rule a name, sub-path, or pattern broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Reason {
    /// The input was empty where a value is required.
    Empty,
    /// A `..` segment tried to climb out of the base directory.
    Traversal,
    /// The input was absolute, or carried a drive letter or UNC prefix,
    /// where a relative path is required.
    Absolute,
    /// The first segment starts with `~`, which shells expand to a home
    /// directory but the filesystem does not.
    HomeShortcut,
    /// A character that is reserved on at least one supported platform,
    /// including full-width and look-alike forms of `/ \ : * ? " < > |`.
    ReservedChar(char),
    /// An ASCII or C1 control character, including NUL.
    ControlChar,
    /// A Unicode bidirectional override or isolate, or a byte-order mark.
    /// These can make a name display differently from what it is.
    InvisibleChar(char),
    /// A Windows device name such as `CON`, `NUL`, or `COM1`, with or
    /// without an extension. Rejected everywhere so paths stay portable.
    ReservedName,
    /// A segment ending in `.` or a space. Windows strips these silently,
    /// so two different inputs would map to the same file.
    TrailingDotOrSpace,
    /// A single segment longer than [`MAX_SEGMENT_BYTES`](crate::MAX_SEGMENT_BYTES).
    SegmentTooLong,
    /// The whole input is longer than [`MAX_INPUT_BYTES`](crate::MAX_INPUT_BYTES).
    TooLong,
    /// The input has more than [`MAX_DEPTH`](crate::MAX_DEPTH) segments.
    TooDeep,
    /// A separator appeared where a single name is required.
    Separator,
    /// An absolute path was required (for a directory override or an
    /// environment variable) and a relative one was given.
    NotAbsolute,
    /// A directory override would make two kinds share one directory,
    /// which would let clearing one kind wipe another.
    SharedDir,
    /// `~name` (another user's home directory) was used; only `~` for the
    /// current user is supported.
    OtherUser,
}

impl Error {
    pub(crate) fn invalid_path(input: &str, reason: Reason) -> Self {
        Self::InvalidPath {
            path: preview(input),
            reason,
        }
    }

    pub(crate) fn invalid_name(input: &str, reason: Reason) -> Self {
        Self::InvalidName {
            name: preview(input),
            reason,
        }
    }

    pub(crate) fn io(op: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            op,
            path: path.into(),
            source,
        }
    }

    /// The [`Reason`] for an [`InvalidName`](Error::InvalidName) or
    /// [`InvalidPath`](Error::InvalidPath) error, or `None` for any other
    /// variant.
    ///
    /// # Examples
    ///
    /// ```
    /// use path_lib::Reason;
    ///
    /// let err = path_lib::join("/srv", "../etc").unwrap_err();
    /// assert_eq!(err.reason(), Some(Reason::Traversal));
    /// ```
    #[must_use]
    pub fn reason(&self) -> Option<Reason> {
        match self {
            Self::InvalidName { reason, .. } | Self::InvalidPath { reason, .. } => Some(*reason),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { name, reason } => {
                write!(f, "invalid application name {name:?}: {reason}")
            }
            Self::InvalidPath { path, reason } => write!(f, "invalid path {path:?}: {reason}"),
            Self::Unresolved { dir } => {
                write!(f, "the operating system did not report the {dir} directory")
            }
            Self::AlreadyExists { path } => write!(f, "{} already exists", path.display()),
            Self::NotFound { path } => write!(f, "{} does not exist", path.display()),
            Self::Refused { path, reason } => {
                write!(f, "refused to act on {}: {reason}", path.display())
            }
            Self::Io { op, path, source } => {
                write!(f, "cannot {op} {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("must not be empty"),
            Self::Traversal => f.write_str("`..` would leave the base directory"),
            Self::Absolute => f.write_str("must be relative (no root, drive, or UNC prefix)"),
            Self::HomeShortcut => f.write_str("`~` is a shell shortcut, not a directory"),
            Self::ReservedChar(c) => write!(f, "character {c:?} is reserved on some platforms"),
            Self::ControlChar => f.write_str("contains a control character"),
            Self::InvisibleChar(c) => {
                write!(
                    f,
                    "contains invisible formatting character U+{:04X}",
                    u32::from(*c)
                )
            }
            Self::ReservedName => f.write_str("is a reserved Windows device name"),
            Self::TrailingDotOrSpace => f.write_str("a segment ends with `.` or a space"),
            Self::SegmentTooLong => f.write_str("a segment is longer than 255 bytes"),
            Self::TooLong => f.write_str("is longer than 4096 bytes"),
            Self::TooDeep => f.write_str("has more than 128 segments"),
            Self::Separator => f.write_str("must be a single name without separators"),
            Self::NotAbsolute => f.write_str("must be an absolute path"),
            Self::SharedDir => f.write_str("is already the directory of another kind"),
            Self::OtherUser => f.write_str("only `~` for the current user is supported"),
        }
    }
}

/// Truncates caller input for inclusion in an error.
fn preview(input: &str) -> String {
    match input.char_indices().nth(PREVIEW_CHARS) {
        Some((cut, _)) => {
            let mut s = String::with_capacity(cut + 3);
            s.push_str(&input[..cut]);
            s.push_str("...");
            s
        }
        None => input.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preview_short_input_is_unchanged() {
        assert_eq!(preview("abc"), "abc");
        assert_eq!(preview(""), "");
    }

    #[test]
    fn test_preview_long_input_is_truncated_on_char_boundary() {
        let long = "é".repeat(PREVIEW_CHARS + 10);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 3);
        assert!(p.ends_with("..."));
    }

    #[test]
    fn test_display_escapes_control_characters() {
        let err = Error::invalid_path("a\nb\u{1b}[31m", Reason::ControlChar);
        let shown = err.to_string();
        assert!(!shown.contains('\n'));
        assert!(!shown.contains('\u{1b}'));
        assert!(shown.contains("\\n"));
    }

    #[test]
    fn test_reason_is_exposed_only_for_input_errors() {
        assert_eq!(
            Error::invalid_name("x", Reason::Empty).reason(),
            Some(Reason::Empty)
        );
        assert_eq!(Error::Unresolved { dir: "home" }.reason(), None);
    }

    #[test]
    fn test_display_every_variant_names_its_subject() {
        use std::path::Path;
        let cases: [(Error, &str); 7] = [
            (
                Error::invalid_name("x/y", Reason::Separator),
                "invalid application name",
            ),
            (
                Error::invalid_path("../a", Reason::Traversal),
                "invalid path",
            ),
            (Error::Unresolved { dir: "home" }, "home directory"),
            (
                Error::AlreadyExists {
                    path: Path::new("p").into(),
                },
                "already exists",
            ),
            (
                Error::NotFound {
                    path: Path::new("p").into(),
                },
                "does not exist",
            ),
            (
                Error::Refused {
                    path: Path::new("p").into(),
                    reason: "because",
                },
                "refused to act on p: because",
            ),
            (
                Error::io("create file", "p", io::Error::other("boom")),
                "cannot create file p: boom",
            ),
        ];
        for (err, needle) in cases {
            let shown = err.to_string();
            assert!(
                shown.contains(needle),
                "{shown:?} should contain {needle:?}"
            );
        }
    }

    #[test]
    fn test_source_present_only_for_io() {
        use std::error::Error as _;
        let io = Error::io("read", "/x", io::Error::other("boom"));
        assert!(io.source().is_some());
        assert!(Error::Unresolved { dir: "home" }.source().is_none());
    }

    #[test]
    fn test_every_reason_has_a_message() {
        let all = [
            Reason::Empty,
            Reason::Traversal,
            Reason::Absolute,
            Reason::HomeShortcut,
            Reason::ReservedChar(':'),
            Reason::ControlChar,
            Reason::InvisibleChar('\u{202e}'),
            Reason::ReservedName,
            Reason::TrailingDotOrSpace,
            Reason::SegmentTooLong,
            Reason::TooLong,
            Reason::TooDeep,
            Reason::Separator,
            Reason::NotAbsolute,
            Reason::SharedDir,
            Reason::OtherUser,
        ];
        for r in all {
            assert!(!r.to_string().is_empty());
        }
    }
}
