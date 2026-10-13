//! Hardened sub-path joining, lossy sanitising, and lexical cleaning.
//!
//! [`join`] is the core primitive behind every `*_at` accessor on
//! [`App`](crate::App). It accepts a relative sub-path written with
//! either slash style, validates every segment against the shared rules
//! in [`crate::rules`], and builds the result with the platform's own
//! separator. Anything that could escape the base directory or mean
//! different things on different platforms is rejected before a
//! [`PathBuf`] is built.

use std::ffi::{OsStr, OsString};
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

use crate::error::{Error, Reason, Result};
use crate::rules::{self, MAX_DEPTH, MAX_INPUT_BYTES, MAX_SEGMENT_BYTES};

/// Joins a relative sub-path onto `base`, rejecting anything unsafe.
///
/// `sub` may use `/`, `\`, or a mix. Leading, trailing, and repeated
/// separators are ignored, as are `.` segments, so `"/a//b/"`, `"a\\b"`,
/// and `"./a/./b"` all mean `a/b`. The result always uses the current
/// platform's separator and always stays inside `base`.
///
/// An empty `sub` (or one made only of separators and `.`) returns `base`
/// itself.
///
/// # Errors
///
/// Returns [`Error::InvalidPath`] when `sub`:
///
/// - contains a `..` segment ([`Reason::Traversal`]);
/// - starts with a drive letter such as `C:` ([`Reason::Absolute`]);
/// - starts with `~` ([`Reason::HomeShortcut`]);
/// - contains a control character, a Windows-reserved character
///   (`< > : " | ? *`), or a look-alike of one;
/// - contains a bidirectional override or byte-order mark;
/// - contains a Windows device name (`CON`, `NUL`, `COM1`, ...) or a
///   segment ending in `.` or a space;
/// - exceeds [`MAX_SEGMENT_BYTES`] per segment, [`MAX_INPUT_BYTES`] in
///   total, or [`MAX_DEPTH`] segments.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// let base = Path::new("/srv/app");
/// let p = path_lib::join(base, "/big\\sloppy//floppy/")?;
/// assert_eq!(p, base.join("big").join("sloppy").join("floppy"));
///
/// assert!(path_lib::join(base, "../etc/passwd").is_err());
/// assert!(path_lib::join(base, "report?.txt").is_err());
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn join(base: impl AsRef<Path>, sub: impl AsRef<str>) -> Result<PathBuf> {
    join_impl(base.as_ref(), sub.as_ref())
}

pub(crate) fn join_impl(base: &Path, sub: &str) -> Result<PathBuf> {
    if sub.len() > MAX_INPUT_BYTES {
        return Err(Error::invalid_path(sub, Reason::TooLong));
    }
    let mut out = PathBuf::with_capacity(base.as_os_str().len() + sub.len() + 1);
    out.push(base);
    let mut depth = 0usize;
    for segment in sub.split(['/', '\\']) {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(Error::invalid_path(sub, Reason::Traversal));
        }
        if depth == 0 {
            check_first_segment(sub, segment)?;
        }
        depth += 1;
        if depth > MAX_DEPTH {
            return Err(Error::invalid_path(sub, Reason::TooDeep));
        }
        rules::check_segment(segment, false).map_err(|r| Error::invalid_path(sub, r))?;
        out.push(segment);
    }
    Ok(out)
}

/// Rules that only apply to the first segment of a relative sub-path.
fn check_first_segment(sub: &str, segment: &str) -> Result<()> {
    let b = segment.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Err(Error::invalid_path(sub, Reason::Absolute));
    }
    if b.first() == Some(&b'~') {
        return Err(Error::invalid_path(sub, Reason::HomeShortcut));
    }
    Ok(())
}

/// Turns arbitrary text into one safe, portable file or directory name.
///
/// Use this on names that come from end users (uploaded file names,
/// titles, user IDs) before passing them to [`join`] or an `*_at`
/// accessor. Unlike [`join`], it never fails: every rejected character
/// becomes `_`, so the output is always accepted by [`join`] as a single
/// segment.
///
/// - Separators, control characters, Windows-reserved characters and
///   their look-alikes, and invisible formatting characters become `_`.
/// - Trailing dots and spaces are removed.
/// - A leading `~` becomes `_`.
/// - Output longer than [`MAX_SEGMENT_BYTES`] is cut on a character
///   boundary.
/// - Windows device names gain a leading `_` (`CON` becomes `_CON`).
/// - Input that sanitises to nothing becomes `_`.
///
/// # Examples
///
/// ```
/// assert_eq!(path_lib::sanitize("Q3 report: final?.pdf"), "Q3 report_ final_.pdf");
/// assert_eq!(path_lib::sanitize("../../etc/passwd"), ".._.._etc_passwd");
/// assert_eq!(path_lib::sanitize("CON"), "_CON");
/// assert_eq!(path_lib::sanitize("..."), "_");
/// ```
#[must_use]
pub fn sanitize(input: impl AsRef<str>) -> String {
    let input = input.as_ref();
    let mut out = String::with_capacity(input.len().min(MAX_SEGMENT_BYTES));
    for c in input.chars() {
        let mapped = if replace_in_sanitize(c) { '_' } else { c };
        if out.len() + mapped.len_utf8() > MAX_SEGMENT_BYTES {
            break;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_end_matches(['.', ' ']).len();
    out.truncate(trimmed);
    if out.starts_with('~') {
        out.replace_range(..1, "_");
    }
    if out.is_empty() {
        out.push('_');
    } else if rules::is_reserved_name(&out) {
        if out.len() >= MAX_SEGMENT_BYTES {
            // Make room for the prefix; the char that falls off is
            // discarded by design, then any newly exposed trailing dot or
            // space is trimmed again.
            let _dropped = out.pop();
            let trimmed = out.trim_end_matches(['.', ' ']).len();
            out.truncate(trimmed);
        }
        out.insert(0, '_');
    }
    out
}

#[inline]
fn replace_in_sanitize(c: char) -> bool {
    if c.is_ascii() {
        // `c` is ASCII, so the cast is lossless.
        let b = c as u8;
        b < 0x20 || b == 0x7F || b == b'/' || b == b'\\' || rules::is_reserved_ascii(b)
    } else {
        c.is_control() || rules::is_lookalike(c) || rules::is_invisible(c)
    }
}

/// Cleans a path lexically, without touching the filesystem.
///
/// Removes `.` segments and redundant separators, resolves `..` against
/// the preceding segment, and drops `..` that would climb above a root.
/// On Windows, `/` is accepted as a separator and the output uses `\`.
/// On Unix, `\` is an ordinary file-name character and is left alone.
///
/// An empty or fully-cancelling relative path becomes `.`.
///
/// Because symbolic links are not consulted, `a/link/..` cleans to `a`
/// even if `link` points elsewhere. Use [`std::fs::canonicalize`] when
/// the on-disk answer matters.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// assert_eq!(path_lib::clean("a/./b//../c/"), Path::new("a").join("c"));
/// assert_eq!(path_lib::clean("../x"), Path::new("..").join("x"));
/// assert_eq!(path_lib::clean(""), Path::new("."));
/// ```
#[must_use]
pub fn clean(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut prefix: Option<&OsStr> = None;
    let mut rooted = false;
    let mut climbs = 0usize;
    let mut names: Vec<&OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => prefix = Some(p.as_os_str()),
            Component::RootDir => rooted = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if names.pop().is_none() && !rooted {
                    climbs += 1;
                }
            }
            Component::Normal(name) => names.push(name),
        }
    }
    // Assembled by hand rather than with `PathBuf::push`, which re-parses
    // each piece: on Windows a name such as `C:x` would be re-read as a
    // drive prefix and replace everything before it.
    let mut out = OsString::with_capacity(path.as_os_str().len());
    if let Some(prefix) = prefix {
        out.push(prefix);
    }
    if rooted {
        out.push(MAIN_SEPARATOR_STR);
    }
    let parents = std::iter::repeat_n(OsStr::new(".."), climbs);
    for (i, name) in parents.chain(names).enumerate() {
        if i > 0 {
            out.push(MAIN_SEPARATOR_STR);
        }
        out.push(name);
    }
    if out.is_empty() {
        out.push(".");
    }
    PathBuf::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        PathBuf::from("base")
    }

    fn expect(parts: &[&str]) -> PathBuf {
        let mut p = base();
        for s in parts {
            p.push(s);
        }
        p
    }

    #[test]
    fn test_join_mixed_separators_normalised() {
        assert_eq!(join(base(), "a/b\\c").ok(), Some(expect(&["a", "b", "c"])));
    }

    #[test]
    fn test_join_leading_trailing_repeated_separators_ignored() {
        let want = Some(expect(&["big", "sloppy", "floppy"]));
        assert_eq!(join(base(), "/big/sloppy/floppy/").ok(), want);
        assert_eq!(join(base(), "\\\\big//sloppy\\\\floppy\\").ok(), want);
    }

    #[test]
    fn test_join_dot_segments_ignored() {
        assert_eq!(join(base(), "./a/./b/.").ok(), Some(expect(&["a", "b"])));
    }

    #[test]
    fn test_join_empty_returns_base() {
        for s in ["", "/", "\\", "./", "/./\\."] {
            assert_eq!(join(base(), s).ok(), Some(base()), "{s:?}");
        }
    }

    #[test]
    fn test_join_traversal_rejected_anywhere() {
        for s in ["..", "../x", "a/../b", "a/..", "a\\..\\..\\b", "/.."] {
            assert_eq!(join(base(), s).ok().map(|_| ()), None, "{s:?}");
            assert_eq!(
                join(base(), s).err().and_then(|e| e.reason()),
                Some(Reason::Traversal),
                "{s:?}"
            );
        }
    }

    #[test]
    fn test_join_drive_letter_rejected_as_absolute() {
        for s in ["C:", "C:\\Windows", "c:/x", "/C:/x"] {
            assert_eq!(
                join(base(), s).err().and_then(|e| e.reason()),
                Some(Reason::Absolute),
                "{s:?}"
            );
        }
    }

    #[test]
    fn test_join_colon_later_rejected_as_reserved() {
        assert_eq!(
            join(base(), "a/file:stream").err().and_then(|e| e.reason()),
            Some(Reason::ReservedChar(':'))
        );
    }

    #[test]
    fn test_join_home_shortcut_rejected_only_first_segment() {
        assert_eq!(
            join(base(), "~/x").err().and_then(|e| e.reason()),
            Some(Reason::HomeShortcut)
        );
        assert_eq!(join(base(), "a/~b").ok(), Some(expect(&["a", "~b"])));
    }

    #[test]
    fn test_join_unc_like_input_stays_confined() {
        // Leading separators are stripped, so this is just "server/share".
        assert_eq!(
            join(base(), "\\\\server\\share").ok(),
            Some(expect(&["server", "share"]))
        );
        assert!(join(base(), "\\\\?\\C:\\x").is_err());
    }

    #[test]
    fn test_join_input_length_limits() {
        let at = "a/".repeat(MAX_INPUT_BYTES / 2);
        assert_eq!(at.len(), MAX_INPUT_BYTES);
        assert_eq!(
            join(base(), &at).err().and_then(|e| e.reason()),
            Some(Reason::TooDeep)
        );
        let over = "a".repeat(MAX_INPUT_BYTES + 1);
        assert_eq!(
            join(base(), &over).err().and_then(|e| e.reason()),
            Some(Reason::TooLong)
        );
    }

    #[test]
    fn test_join_depth_limit_boundary() {
        let at = vec!["d"; MAX_DEPTH].join("/");
        let over = vec!["d"; MAX_DEPTH + 1].join("/");
        assert!(join(base(), &at).is_ok());
        assert_eq!(
            join(base(), &over).err().and_then(|e| e.reason()),
            Some(Reason::TooDeep)
        );
    }

    #[test]
    fn test_join_reports_first_failing_rule() {
        assert_eq!(
            join(base(), "ok/CON/x").err().and_then(|e| e.reason()),
            Some(Reason::ReservedName)
        );
        assert_eq!(
            join(base(), "ok/bad. /x").err().and_then(|e| e.reason()),
            Some(Reason::TrailingDotOrSpace)
        );
    }

    #[test]
    fn test_sanitize_examples() {
        assert_eq!(sanitize("hello"), "hello");
        assert_eq!(sanitize("a/b\\c"), "a_b_c");
        assert_eq!(sanitize("x\0y\nz"), "x_y_z");
        assert_eq!(sanitize("  trail.  "), "  trail");
        assert_eq!(sanitize("~root"), "_root");
        assert_eq!(sanitize("nul.txt"), "_nul.txt");
        assert_eq!(sanitize(""), "_");
        assert_eq!(sanitize(". ."), "_");
        assert_eq!(sanitize("a\u{FF0F}b"), "a_b");
        assert_eq!(sanitize("a\u{202E}b"), "a_b");
    }

    #[test]
    fn test_sanitize_truncates_on_char_boundary() {
        let s = sanitize("é".repeat(400));
        assert!(s.len() <= MAX_SEGMENT_BYTES);
        assert_eq!(rules::check_segment(&s, false), Ok(()));
    }

    #[test]
    fn test_sanitize_reserved_name_at_max_length_stays_within_limit() {
        let long = format!("CON.{}", "x".repeat(MAX_SEGMENT_BYTES));
        let s = sanitize(&long);
        assert!(s.len() <= MAX_SEGMENT_BYTES);
        assert!(s.starts_with("_CON."));
        assert_eq!(rules::check_segment(&s, false), Ok(()));
    }

    #[test]
    fn test_clean_examples() {
        assert_eq!(clean("a/./b//../c/"), PathBuf::from("a").join("c"));
        assert_eq!(clean("../../x"), PathBuf::from("..").join("..").join("x"));
        assert_eq!(clean("a/.."), PathBuf::from("."));
        assert_eq!(clean("."), PathBuf::from("."));
    }

    #[test]
    fn test_clean_never_climbs_above_root() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let p = format!("{root}..{sep}..{sep}x", sep = std::path::MAIN_SEPARATOR);
        assert_eq!(clean(p), PathBuf::from(root).join("x"));
    }

    #[cfg(windows)]
    #[test]
    fn test_clean_windows_converts_forward_slashes() {
        assert_eq!(clean("C:/a//b/"), PathBuf::from(r"C:\a\b"));
        assert_eq!(
            clean(r"\\server\share\a\..\b"),
            PathBuf::from(r"\\server\share\b")
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_clean_windows_colon_names_are_not_reparsed_as_drives() {
        // Regression found by proptest: `/C:C:` cleaned to `C:C:`, and a
        // second pass turned that into `C:`.
        let once = clean("/C:C:");
        assert_eq!(once, PathBuf::from(r"\C:C:"));
        assert_eq!(clean(&once), once);
        assert_eq!(clean("C:foo"), PathBuf::from("C:foo"));
        assert_eq!(clean("C:"), PathBuf::from("C:"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_clean_unix_keeps_backslash_as_name_char() {
        assert_eq!(clean("/a\\b/c"), PathBuf::from("/a\\b/c"));
    }
}
