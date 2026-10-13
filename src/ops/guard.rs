//! Protection against destructive operations on critical paths.
//!
//! Every check runs on the lexical absolute form of a path and again on
//! the form with its parent resolved through symbolic links, comparing
//! case-insensitively where the platform does.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Reason, Result};
use crate::join::clean;
use crate::os;

/// Refuses destructive operations on paths whose loss would be
/// catastrophic.
///
/// The path is checked twice: in its lexical absolute form, and with its
/// parent directory resolved through any symbolic links (the final
/// component is left alone, so removing a link that points at the home
/// directory is still allowed). Comparisons ignore case on Windows and
/// macOS, whose default filesystems do.
pub(super) fn guard_destructive(path: &Path) -> Result<()> {
    let lexical = absolute(path)?;
    let resolved = resolve_parent(&lexical);
    for candidate in std::iter::once(&lexical).chain(resolved.as_ref()) {
        if let Some(reason) = protected_reason(candidate) {
            return Err(Error::Refused {
                path: path.to_path_buf(),
                reason,
            });
        }
    }
    Ok(())
}

pub(super) fn protected_reason(absolute: &Path) -> Option<&'static str> {
    let named = absolute
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if named == 0 {
        return Some("it is a filesystem root");
    }
    if named < 2 {
        return Some("it is a top-level directory");
    }
    if with_resolved(os::temp()).any(|t| same_path(&t, absolute)) {
        return Some("it is the system temporary directory");
    }
    // The user folders are best-effort: a process without a home
    // directory has none to protect, so an unresolved home is not an
    // error here.
    if let Ok(home) = os::home() {
        if with_resolved(home).any(|h| is_within(&h, absolute)) {
            return Some("it is the home directory or one of its parents");
        }
        let standard = [
            os::documents(),
            os::desktop(),
            os::downloads(),
            os::pictures(),
            os::music(),
            os::videos(),
        ];
        let hit = standard
            .into_iter()
            .flatten()
            .flat_map(with_resolved)
            .any(|p| same_path(&p, absolute));
        if hit {
            return Some("it is one of the user's standard folders");
        }
    }
    None
}

/// `path` itself plus its fully resolved form, when that differs.
pub(super) fn with_resolved(path: &Path) -> impl Iterator<Item = PathBuf> {
    let resolved = fs::canonicalize(path).ok().map(strip_verbatim);
    std::iter::once(path.to_path_buf()).chain(resolved)
}

/// Resolves symbolic links in the parent of `path`, keeping the final
/// component as written. `None` when the parent does not exist.
pub(super) fn resolve_parent(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    let real = fs::canonicalize(parent).ok()?;
    Some(strip_verbatim(real).join(name))
}

/// Resolves symbolic links in the longest existing prefix of `path` and
/// appends the rest unchanged. Works for paths that do not exist yet.
pub(super) fn resolve_lenient(path: &Path) -> Result<PathBuf> {
    let path = absolute(path)?;
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(existing) {
            let mut out = strip_verbatim(real);
            for name in rest.iter().rev() {
                out.push(name);
            }
            return Ok(out);
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return Ok(path.clone()),
        }
    }
}

/// Removes the `\\?\` prefix `canonicalize` adds on Windows, so resolved
/// paths compare equal to ordinary ones.
#[cfg(windows)]
pub(super) fn strip_verbatim(path: PathBuf) -> PathBuf {
    use std::path::Prefix;
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };
    let rest = components.as_path();
    match prefix.kind() {
        Prefix::VerbatimDisk(drive) => {
            let mut out = PathBuf::from(format!("{}:\\", char::from(drive)));
            out.push(rest);
            out
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut out = PathBuf::from(r"\\");
            out.push(server);
            out.push(share);
            out.push(rest);
            out
        }
        _ => path,
    }
}

#[cfg(unix)]
pub(super) fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
}

/// Whether `inner` is `outer` or lies inside it, component by component,
/// ignoring case where the platform does.
pub(super) fn is_within(inner: &Path, outer: &Path) -> bool {
    let mut inner = inner.components();
    for o in outer.components() {
        match inner.next() {
            Some(i) if same_component(i, o) => {}
            _ => return false,
        }
    }
    true
}

pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    a.components().count() == b.components().count() && is_within(a, b)
}

pub(super) fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// Lexically absolute form of `path`, relative to the current directory.
pub(super) fn absolute(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::invalid_path("", Reason::Empty));
    }
    if path.is_absolute() {
        return Ok(clean(path));
    }
    let cwd = std::env::current_dir().map_err(|e| Error::io("read current directory", path, e))?;
    Ok(clean(cwd.join(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_refuses_roots_and_top_level() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let top = if cfg!(windows) { "C:\\Users" } else { "/usr" };
        assert!(matches!(
            guard_destructive(Path::new(root)),
            Err(Error::Refused { .. })
        ));
        assert!(matches!(
            guard_destructive(Path::new(top)),
            Err(Error::Refused { .. })
        ));
    }

    #[test]
    fn test_guard_refuses_home_and_ancestors_and_standard_folders() {
        if let Ok(home) = os::home() {
            assert!(guard_destructive(home).is_err());
            if let Some(parent) = home.parent() {
                assert!(guard_destructive(parent).is_err());
            }
        }
        if let Ok(docs) = os::documents() {
            assert!(guard_destructive(docs).is_err());
        }
        assert!(guard_destructive(os::temp()).is_err());
    }

    #[test]
    fn test_guard_refuses_empty_and_cwd_escapes() {
        assert!(guard_destructive(Path::new("")).is_err());
        // `..` repeated past the root cleans to the root.
        let deep = "../".repeat(64);
        assert!(guard_destructive(Path::new(&deep)).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_strip_verbatim_handles_disk_unc_and_plain() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\a\b")),
            PathBuf::from(r"C:\a\b")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"C:\a")),
            PathBuf::from(r"C:\a")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"rel\a")),
            PathBuf::from(r"rel\a")
        );
    }

    #[test]
    fn test_guard_allows_ordinary_nested_paths() {
        let p = os::temp().join("path-lib-guard").join("x");
        assert!(guard_destructive(&p).is_ok());
    }
}
