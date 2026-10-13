//! Recursive name search with linear-time wildcard matching.

use std::fs;
use std::path::{Path, PathBuf};

use super::{MAX_WALK_DEPTH, not_found_or_io, too_deep};
use crate::error::{Error, Result};
use crate::rules;

/// Finds files and directories under `root` whose name matches `pattern`.
///
/// `pattern` matches a single name, not a path: `*` matches any run of
/// characters and `?` matches exactly one. Matching ignores case on
/// Windows and macOS (whose default filesystems do too) and respects it
/// elsewhere. Results are sorted. Symbolic links are matched by name but
/// never followed.
///
/// # Errors
///
/// - [`Error::InvalidPath`] if `pattern` is empty, contains a separator,
///   or breaks another name rule (wildcards are allowed).
/// - [`Error::NotFound`] if `root` does not exist.
/// - [`Error::Refused`] if the tree is deeper than [`MAX_WALK_DEPTH`].
/// - [`Error::Io`] if a directory in the tree cannot be read.
///
/// # Examples
///
/// ```
/// let base = std::env::temp_dir().join("path-lib-doc-find");
/// # let _ = path_lib::remove(&base);
/// path_lib::make_file(base.join("logs/2026-10-01.log"))?;
/// path_lib::make_file(base.join("logs/old/2026-09-30.log"))?;
/// path_lib::make_file(base.join("notes.txt"))?;
/// let logs = path_lib::find(&base, "*.log")?;
/// assert_eq!(logs.len(), 2);
/// # path_lib::remove(&base)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn find(root: impl AsRef<Path>, pattern: impl AsRef<str>) -> Result<Vec<PathBuf>> {
    find_limited(root.as_ref(), pattern.as_ref(), MAX_WALK_DEPTH)
}

pub(super) fn find_limited(root: &Path, pattern: &str, max_depth: usize) -> Result<Vec<PathBuf>> {
    rules::check_segment(pattern, true).map_err(|r| Error::invalid_path(pattern, r))?;
    let pattern: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| {
            if depth == 0 {
                not_found_or_io("search", &dir, e)
            } else {
                Error::io("search", &dir, e)
            }
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io("search", &dir, e))?;
            let kind = entry
                .file_type()
                .map_err(|e| Error::io("inspect", entry.path(), e))?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| matches(&pattern, n))
            {
                out.push(entry.path());
            }
            if kind.is_dir() {
                if depth + 1 > max_depth {
                    return Err(too_deep(entry.path()));
                }
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// Wildcard match of one name: `*` any run, `?` one character.
///
/// Linear-time two-pointer algorithm with single-star backtracking, so a
/// hostile pattern such as `*a*a*a*a*b` cannot cause exponential work.
pub(crate) fn matches(pattern: &[char], name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || same_char(pattern[p], name[n])) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[inline]
pub(super) fn same_char(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    if cfg!(any(windows, target_os = "macos")) {
        a.to_lowercase().eq(b.to_lowercase())
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn test_matches_literals_and_wildcards() {
        assert!(matches(&pat("a.log"), "a.log"));
        assert!(matches(&pat("*.log"), "x.log"));
        assert!(matches(&pat("*.log"), ".log"));
        assert!(matches(&pat("a?c"), "abc"));
        assert!(matches(&pat("*"), ""));
        assert!(matches(&pat("**"), "abc"));
        assert!(matches(&pat("a*b*c"), "aXXbYYc"));
        assert!(!matches(&pat("a?c"), "ac"));
        assert!(!matches(&pat("*.log"), "x.log.bak"));
        assert!(!matches(&pat("abc"), "ab"));
        assert!(!matches(&pat(""), "a"));
    }

    #[test]
    fn test_matches_unicode_by_character() {
        assert!(matches(&pat("?.txt"), "é.txt"));
        assert!(matches(&pat("名*"), "名前"));
    }

    #[test]
    fn test_matches_case_rule_follows_platform() {
        let insensitive = cfg!(any(windows, target_os = "macos"));
        assert_eq!(matches(&pat("*.LOG"), "a.log"), insensitive);
    }

    #[test]
    fn test_matches_pathological_pattern_is_fast() {
        let name = "a".repeat(10_000);
        let pattern = pat(&format!("{}b", "*a".repeat(50)));
        let start = std::time::Instant::now();
        assert!(!matches(&pattern, &name));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
