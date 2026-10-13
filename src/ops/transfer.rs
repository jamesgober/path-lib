//! Copying and moving, with staged copies and cross-volume fallback.

use std::fs;
use std::io;
use std::path::Path;

use super::{
    MAX_WALK_DEPTH, RenameFn, absolute, create_dir, exists, guard_destructive, is_within,
    not_found_or_io, os_rename, remove_entry, resolve_lenient, special_file, staging_path,
    too_deep,
};
use crate::error::{Error, Result};

/// Copies a file or a directory tree. Returns the number of bytes copied.
///
/// The destination must not exist; missing parents are created. A
/// directory is copied into a temporary sibling first and renamed into
/// place when complete, so `to` either appears whole or not at all.
///
/// # Errors
///
/// - [`Error::NotFound`] if `from` does not exist.
/// - [`Error::AlreadyExists`] if `to` exists.
/// - [`Error::Refused`] if `to` is inside `from`, if `from` is or contains
///   a symbolic link or a special file (socket, FIFO, device), or if the
///   tree is deeper than [`MAX_WALK_DEPTH`].
/// - [`Error::Io`] if reading or writing fails. The partial copy is
///   deleted before the error is returned.
///
/// # Examples
///
/// ```
/// let base = std::env::temp_dir().join("path-lib-doc-copy");
/// # let _ = path_lib::remove(&base);
/// path_lib::make_file(base.join("src/a.txt"))?;
/// path_lib::copy(base.join("src"), base.join("dst"))?;
/// assert!(base.join("dst/a.txt").is_file());
/// # path_lib::remove(&base)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64> {
    let (from, to) = (from.as_ref(), to.as_ref());
    let meta = source_metadata(from)?;
    check_destination(from, to, &meta)?;
    copy_staged(from, to, &meta, &os_rename, MAX_WALK_DEPTH)
}

/// Moves (renames) a file or directory.
///
/// Uses a single atomic rename when `from` and `to` are on the same
/// volume. Across volumes it falls back to [`copy`] followed by
/// [`remove`](crate::remove) of the source.
///
/// # Errors
///
/// - [`Error::NotFound`] if `from` does not exist.
/// - [`Error::AlreadyExists`] if `to` exists.
/// - [`Error::Refused`] if `from` is protected (see [`remove`](crate::remove)) or `to` is
///   inside `from`.
/// - [`Error::Io`] if the rename or the fallback fails. If the fallback
///   copy succeeded but deleting the source failed, the error's `op` is
///   `"remove moved source"`: the destination is a complete copy, and the
///   source may be partly deleted (a directory's contents can be removed
///   before the directory itself fails to be), so treat the destination
///   as the surviving version.
pub fn move_to(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    move_with(from.as_ref(), to.as_ref(), &os_rename)
}

pub(super) fn move_with(from: &Path, to: &Path, rename: RenameFn<'_>) -> Result<()> {
    guard_destructive(from)?;
    let meta = source_metadata(from)?;
    check_destination(from, to, &meta)?;
    match rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            if meta.file_type().is_symlink() {
                return Err(Error::Refused {
                    path: from.to_path_buf(),
                    reason: "symbolic links cannot be moved across volumes",
                });
            }
            let _bytes = copy_staged(from, to, &meta, &os_rename, MAX_WALK_DEPTH)?;
            remove_entry(from, &meta).map_err(|err| match err {
                Error::Io { path, source, .. } => Error::Io {
                    op: "remove moved source",
                    path,
                    source,
                },
                other => other,
            })
        }
        Err(e) => Err(Error::io("move", from, e)),
    }
}

pub(super) fn source_metadata(from: &Path) -> Result<fs::Metadata> {
    fs::symlink_metadata(from).map_err(|e| not_found_or_io("inspect", from, e))
}

/// Shared destination checks for copy and move. Creates the
/// destination's parent only after every check has passed.
pub(super) fn check_destination(from: &Path, to: &Path, from_meta: &fs::Metadata) -> Result<()> {
    if to.file_name().is_none() {
        return Err(Error::Refused {
            path: to.to_path_buf(),
            reason: "the destination has no file name",
        });
    }
    if exists(to) {
        return Err(Error::AlreadyExists {
            path: to.to_path_buf(),
        });
    }
    if from_meta.is_dir() && is_inside(from, to)? {
        return Err(Error::Refused {
            path: to.to_path_buf(),
            reason: "the destination is inside the source",
        });
    }
    if let Some(parent) = to.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_dir(parent, None)?;
    }
    Ok(())
}

/// Whether `to` is `from` or lies inside it, comparing both the lexical
/// and the symlink-resolved forms.
pub(super) fn is_inside(from: &Path, to: &Path) -> Result<bool> {
    if is_within(&absolute(to)?, &absolute(from)?) {
        return Ok(true);
    }
    Ok(is_within(&resolve_lenient(to)?, &resolve_lenient(from)?))
}

/// Copies through a temporary sibling, then renames into place.
pub(super) fn copy_staged(
    from: &Path,
    to: &Path,
    meta: &fs::Metadata,
    rename: RenameFn<'_>,
    max_depth: usize,
) -> Result<u64> {
    if meta.file_type().is_symlink() {
        return Err(Error::Refused {
            path: from.to_path_buf(),
            reason: "copying symbolic links is not supported",
        });
    }
    let staging = staging_path(to);
    let result = if meta.is_dir() {
        copy_tree(from, &staging, max_depth)
    } else if meta.is_file() {
        fs::copy(from, &staging).map_err(|e| Error::io("copy", from, e))
    } else {
        Err(special_file(from))
    };
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(err) => {
            // Cleanup is best-effort: the copy error is the one the caller
            // needs, and a failed cleanup leaves only a hidden, uniquely
            // named staging entry, never a partial `to`.
            if let Ok(m) = fs::symlink_metadata(&staging) {
                let _cleanup = remove_entry(&staging, &m);
            }
            return Err(err);
        }
    };
    if let Err(e) = rename(&staging, to) {
        if let Ok(m) = fs::symlink_metadata(&staging) {
            // Same rationale as above.
            let _cleanup = remove_entry(&staging, &m);
        }
        return Err(Error::io("finish copy", to, e));
    }
    Ok(bytes)
}

/// Iterative recursive copy; refuses links and special files.
pub(super) fn copy_tree(from: &Path, to: &Path, max_depth: usize) -> Result<u64> {
    let mut bytes = 0u64;
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf(), 0usize)];
    while let Some((src, dst, depth)) = stack.pop() {
        fs::create_dir(&dst).map_err(|e| Error::io("create directory", &dst, e))?;
        let entries = fs::read_dir(&src).map_err(|e| Error::io("read directory", &src, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io("read directory", &src, e))?;
            let kind = entry
                .file_type()
                .map_err(|e| Error::io("inspect", entry.path(), e))?;
            let target = dst.join(entry.file_name());
            if kind.is_symlink() {
                return Err(Error::Refused {
                    path: entry.path(),
                    reason: "the tree contains a symbolic link",
                });
            } else if kind.is_dir() {
                if depth + 1 > max_depth {
                    return Err(too_deep(entry.path()));
                }
                stack.push((entry.path(), target, depth + 1));
            } else if kind.is_file() {
                let n = fs::copy(entry.path(), &target)
                    .map_err(|e| Error::io("copy", entry.path(), e))?;
                bytes = bytes.saturating_add(n);
            } else {
                return Err(special_file(&entry.path()));
            }
        }
    }
    Ok(bytes)
}

pub(super) fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    #[cfg(unix)]
    const CODE: i32 = 18;
    #[cfg(windows)]
    const CODE: i32 = 17;
    e.raw_os_error() == Some(CODE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::find::find_limited;
    use crate::ops::test_support::*;

    #[test]
    fn test_move_across_volumes_falls_back_to_copy_then_remove() {
        let dir = scratch("xdev");
        fs::create_dir_all(dir.join("src/inner")).unwrap_or_default();
        fs::write(dir.join("src/inner/a.txt"), b"abc").unwrap_or_default();
        let moved = move_with(&dir.join("src"), &dir.join("dst"), &cross_device);
        assert!(moved.is_ok(), "{moved:?}");
        assert!(!dir.join("src").exists());
        assert_eq!(
            fs::read(dir.join("dst/inner/a.txt")).ok(),
            Some(b"abc".to_vec())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_move_across_volumes_refuses_symlink_source() {
        let dir = scratch("xdev-link");
        let target = dir.join("target");
        fs::write(&target, b"x").unwrap_or_default();
        let made = std::os::unix::fs::symlink(&target, dir.join("link")).is_ok();
        assert!(made);
        let r = move_with(&dir.join("link"), &dir.join("moved"), &cross_device);
        assert!(matches!(r, Err(Error::Refused { .. })), "{r:?}");
        assert!(dir.join("link").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_move_across_volumes_reports_failed_source_removal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("xdev-keep");
        let parent = dir.join("ro");
        fs::create_dir_all(parent.join("src")).unwrap_or_default();
        fs::write(parent.join("src/a"), b"x").unwrap_or_default();
        // A read-only parent lets the copy read `src` but not delete it.
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap_or_default();
        let r = move_with(&parent.join("src"), &dir.join("dst"), &cross_device);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap_or_default();
        let as_root = fs::read_dir(&parent).is_ok() && r.is_ok();
        if !as_root {
            assert!(
                matches!(
                    r,
                    Err(Error::Io {
                        op: "remove moved source",
                        ..
                    })
                ),
                "{r:?}"
            );
            assert!(dir.join("dst/a").exists(), "destination is complete");
            assert!(
                parent.join("src").exists(),
                "source directory could not be removed"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_move_other_rename_error_is_reported() {
        let dir = scratch("move-err");
        fs::write(dir.join("a"), b"x").unwrap_or_default();
        let r = move_with(&dir.join("a"), &dir.join("b"), &denied);
        assert!(matches!(r, Err(Error::Io { op: "move", .. })), "{r:?}");
        assert!(dir.join("a").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_copy_failed_final_rename_leaves_no_trace() {
        let dir = scratch("copy-finish");
        fs::create_dir_all(dir.join("src")).unwrap_or_default();
        fs::write(dir.join("src/a"), b"x").unwrap_or_default();
        let meta = fs::symlink_metadata(dir.join("src")).ok();
        let r = meta.map(|m| copy_staged(&dir.join("src"), &dir.join("dst"), &m, &denied, 8));
        assert!(
            matches!(
                r,
                Some(Err(Error::Io {
                    op: "finish copy",
                    ..
                }))
            ),
            "{r:?}"
        );
        let names: Vec<_> = fs::read_dir(&dir)
            .map(|d| {
                d.filter_map(std::result::Result::ok)
                    .map(|e| e.file_name())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("src")],
            "staging removed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_walk_depth_limit_is_enforced_by_find_and_copy() {
        let dir = scratch("depth");
        fs::create_dir_all(dir.join("t/a/b/c")).unwrap_or_default();
        assert!(find_limited(&dir.join("t"), "*", 3).is_ok());
        assert!(matches!(
            find_limited(&dir.join("t"), "*", 2),
            Err(Error::Refused { .. })
        ));
        let meta = fs::symlink_metadata(dir.join("t")).ok();
        let r = meta.map(|m| copy_staged(&dir.join("t"), &dir.join("u"), &m, &os_rename, 2));
        assert!(matches!(r, Some(Err(Error::Refused { .. }))), "{r:?}");
        assert!(!dir.join("u").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
