//! Guarded filesystem operations on paths.
//!
//! These are deliberately conservative, because a path library's
//! mistakes delete people's files:
//!
//! - Nothing is ever overwritten. Copy, move, and file creation fail with
//!   [`Error::AlreadyExists`] if the destination is taken.
//! - Symbolic links are never followed while walking a tree. [`remove`]
//!   deletes a link, not what it points to; [`find`] reports links but
//!   does not descend into them; [`copy`] refuses trees containing them.
//! - [`remove`] and [`move_to`] refuse filesystem roots, the home
//!   directory and everything above it, the user's well-known folders,
//!   and any path with fewer than two named components (`/usr`, `C:\data`).
//! - [`copy`] builds the destination under a temporary sibling name and
//!   renames it into place only once complete, so a failed copy never
//!   leaves a half-written destination behind.
//!
//! Checks and actions are separate system calls, so another process can
//! still change the filesystem in between (a time-of-check to
//! time-of-use race). The guards prevent mistakes, not a hostile process
//! racing this one in the same directory.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Reason, Result};
use crate::join::clean;
use crate::os;
use crate::rules;

/// Deepest directory nesting [`find`] and [`copy`] will walk.
pub const MAX_WALK_DEPTH: usize = 256;

/// Creates a directory and any missing parents.
///
/// Succeeds if the directory already exists.
///
/// # Errors
///
/// - [`Error::AlreadyExists`] if `path` exists but is not a directory.
/// - [`Error::Io`] if the directory cannot be created.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-make-dir").join("a/b");
/// path_lib::make_dir(&dir)?;
/// assert!(dir.is_dir());
/// # path_lib::remove(std::env::temp_dir().join("path-lib-doc-make-dir"))?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn make_dir(path: impl AsRef<Path>) -> Result<()> {
    create_dir(path.as_ref(), None)
}

/// Creates a directory tree with Unix permission bits `mode` on every
/// directory it creates (subject to the process umask). `None` uses the
/// platform default. On Windows `mode` is ignored and directories inherit
/// the parent's access control list.
pub(crate) fn create_dir(path: &Path, mode: Option<u32>) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(Error::Refused {
                path: path.to_path_buf(),
                reason: "it is a symbolic link, not a directory",
            });
        }
        Ok(_) => {
            return Err(Error::AlreadyExists {
                path: path.to_path_buf(),
            });
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io("inspect", path, e)),
    }
    let mut builder = fs::DirBuilder::new();
    let _builder = builder.recursive(true);
    apply_mode(&mut builder, mode);
    builder
        .create(path)
        .map_err(|e| Error::io("create directory", path, e))
}

#[cfg(unix)]
fn apply_mode(builder: &mut fs::DirBuilder, mode: Option<u32>) {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(mode) = mode {
        let _builder = builder.mode(mode);
    }
}

#[cfg(windows)]
fn apply_mode(_builder: &mut fs::DirBuilder, _mode: Option<u32>) {}

/// Creates an empty file, plus any missing parent directories.
///
/// # Errors
///
/// - [`Error::AlreadyExists`] if anything already exists at `path`.
/// - [`Error::Io`] if the file or a parent cannot be created.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-make-file");
/// let file = dir.join("notes/today.txt");
/// path_lib::make_file(&file)?;
/// assert!(file.is_file());
/// assert!(path_lib::make_file(&file).is_err()); // never overwrites
/// # path_lib::remove(&dir)?;
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn make_file(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_dir(parent, None)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(_file) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(Error::AlreadyExists {
            path: path.to_path_buf(),
        }),
        Err(e) => Err(Error::io("create file", path, e)),
    }
}

/// Whether anything exists at `path`: a file, directory, or symbolic link
/// (even a broken one).
///
/// Errors other than "not found" (such as permission denied on a parent)
/// are reported as `true`, because something may well be there; use
/// [`std::fs::symlink_metadata`] when the distinction matters.
#[must_use]
pub fn exists(path: impl AsRef<Path>) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}

/// Lists the entries of a directory, sorted by path.
///
/// # Errors
///
/// - [`Error::NotFound`] if `dir` does not exist.
/// - [`Error::Io`] if it is not a directory or cannot be read.
pub fn list(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    let entries = fs::read_dir(dir).map_err(|e| not_found_or_io("list", dir, e))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io("list", dir, e))?;
        out.push(entry.path());
    }
    out.sort_unstable();
    Ok(out)
}

/// Deletes a file, a symbolic link, or a directory and everything in it.
///
/// Returns `Ok(false)` if nothing existed at `path`, so removal is
/// idempotent. Symbolic links are removed themselves; their targets are
/// never touched, including links found inside a directory being removed.
///
/// # Errors
///
/// - [`Error::Refused`] for protected paths: a filesystem root, the home
///   directory or any of its ancestors, the user's well-known folders
///   (Documents, Desktop, ...), the OS temp directory, or any path with
///   fewer than two named components.
/// - [`Error::Io`] if deletion fails part-way. Entries already deleted
///   stay deleted.
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join("path-lib-doc-remove");
/// path_lib::make_dir(dir.join("x/y"))?;
/// assert!(path_lib::remove(&dir)?);
/// assert!(!path_lib::remove(&dir)?); // already gone
/// assert!(path_lib::remove("/").is_err()); // refused
/// # Ok::<(), path_lib::Error>(())
/// ```
pub fn remove(path: impl AsRef<Path>) -> Result<bool> {
    let path = path.as_ref();
    guard_destructive(path)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(Error::io("inspect", path, e)),
    };
    remove_entry(path, &meta)?;
    Ok(true)
}

fn remove_entry(path: &Path, meta: &fs::Metadata) -> Result<()> {
    let result = if meta.file_type().is_symlink() {
        remove_link(path, meta)
    } else if meta.is_dir() {
        // `remove_dir_all` does not follow symbolic links (fixed for
        // CVE-2022-21658 in Rust 1.58.1, below this crate's MSRV).
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| Error::io("remove", path, e))
}

#[cfg(windows)]
fn remove_link(path: &Path, meta: &fs::Metadata) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if meta.file_type().is_symlink_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(unix)]
fn remove_link(path: &Path, _meta: &fs::Metadata) -> io::Result<()> {
    fs::remove_file(path)
}

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
    copy_staged(from, to, &meta)
}

/// Moves (renames) a file or directory.
///
/// Uses a single atomic rename when `from` and `to` are on the same
/// volume. Across volumes it falls back to [`copy`] followed by
/// [`remove`] of the source.
///
/// # Errors
///
/// - [`Error::NotFound`] if `from` does not exist.
/// - [`Error::AlreadyExists`] if `to` exists.
/// - [`Error::Refused`] if `from` is protected (see [`remove`]) or `to` is
///   inside `from`.
/// - [`Error::Io`] if the rename or the fallback fails. If the fallback
///   copy succeeded but deleting the source failed, both copies exist and
///   the error's `op` is `"remove moved source"`.
pub fn move_to(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    guard_destructive(from)?;
    let meta = source_metadata(from)?;
    check_destination(from, to, &meta)?;
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            if meta.file_type().is_symlink() {
                return Err(Error::Refused {
                    path: from.to_path_buf(),
                    reason: "symbolic links cannot be moved across volumes",
                });
            }
            let _bytes = copy_staged(from, to, &meta)?;
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
    let (root, pattern) = (root.as_ref(), pattern.as_ref());
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
            let name = entry.file_name();
            let kind = entry
                .file_type()
                .map_err(|e| Error::io("inspect", entry.path(), e))?;
            if name.to_str().is_some_and(|n| matches(&pattern, n)) {
                out.push(entry.path());
            }
            if kind.is_dir() {
                if depth + 1 > MAX_WALK_DEPTH {
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
fn same_char(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    if cfg!(any(windows, target_os = "macos")) {
        a.to_lowercase().eq(b.to_lowercase())
    } else {
        false
    }
}

// ---------------------------------------------------------------------
// Guards and helpers
// ---------------------------------------------------------------------

/// Refuses destructive operations on paths whose loss would be
/// catastrophic.
///
/// The path is checked twice: in its lexical absolute form, and with its
/// parent directory resolved through any symbolic links (the final
/// component is left alone, so removing a link that points at the home
/// directory is still allowed). Comparisons ignore case on Windows and
/// macOS, whose default filesystems do.
fn guard_destructive(path: &Path) -> Result<()> {
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

fn protected_reason(absolute: &Path) -> Option<&'static str> {
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
fn with_resolved(path: &Path) -> impl Iterator<Item = PathBuf> {
    let resolved = fs::canonicalize(path).ok().map(strip_verbatim);
    std::iter::once(path.to_path_buf()).chain(resolved)
}

/// Resolves symbolic links in the parent of `path`, keeping the final
/// component as written. `None` when the parent does not exist.
fn resolve_parent(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    let real = fs::canonicalize(parent).ok()?;
    Some(strip_verbatim(real).join(name))
}

/// Resolves symbolic links in the longest existing prefix of `path` and
/// appends the rest unchanged. Works for paths that do not exist yet.
fn resolve_lenient(path: &Path) -> Result<PathBuf> {
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
fn strip_verbatim(path: PathBuf) -> PathBuf {
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
fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
}

/// Whether `inner` is `outer` or lies inside it, component by component,
/// ignoring case where the platform does.
fn is_within(inner: &Path, outer: &Path) -> bool {
    let mut inner = inner.components();
    for o in outer.components() {
        match inner.next() {
            Some(i) if same_component(i, o) => {}
            _ => return false,
        }
    }
    true
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.components().count() == b.components().count() && is_within(a, b)
}

fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// Lexically absolute form of `path`, relative to the current directory.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::invalid_path("", Reason::Empty));
    }
    if path.is_absolute() {
        return Ok(clean(path));
    }
    let cwd = std::env::current_dir().map_err(|e| Error::io("read current directory", path, e))?;
    Ok(clean(cwd.join(path)))
}

fn source_metadata(from: &Path) -> Result<fs::Metadata> {
    fs::symlink_metadata(from).map_err(|e| not_found_or_io("inspect", from, e))
}

/// Shared destination checks for copy and move. Creates the
/// destination's parent only after every check has passed.
fn check_destination(from: &Path, to: &Path, from_meta: &fs::Metadata) -> Result<()> {
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
fn is_inside(from: &Path, to: &Path) -> Result<bool> {
    if is_within(&absolute(to)?, &absolute(from)?) {
        return Ok(true);
    }
    Ok(is_within(&resolve_lenient(to)?, &resolve_lenient(from)?))
}

/// Copies through a temporary sibling, then renames into place.
fn copy_staged(from: &Path, to: &Path, meta: &fs::Metadata) -> Result<u64> {
    if meta.file_type().is_symlink() {
        return Err(Error::Refused {
            path: from.to_path_buf(),
            reason: "copying symbolic links is not supported",
        });
    }
    let staging = staging_path(to);
    let result = if meta.is_dir() {
        copy_tree(from, &staging)
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
    if let Err(e) = fs::rename(&staging, to) {
        if let Ok(m) = fs::symlink_metadata(&staging) {
            // Same rationale as above.
            let _cleanup = remove_entry(&staging, &m);
        }
        return Err(Error::io("finish copy", to, e));
    }
    Ok(bytes)
}

/// Iterative recursive copy; refuses links and special files.
fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
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
                if depth + 1 > MAX_WALK_DEPTH {
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

/// A hidden, unique sibling of `to` used while a copy is in progress.
fn staging_path(to: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = to
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staged = format!(".{name}.path-lib-{}-{n}.partial", std::process::id());
    to.with_file_name(staged)
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    #[cfg(unix)]
    const CODE: i32 = 18;
    #[cfg(windows)]
    const CODE: i32 = 17;
    e.raw_os_error() == Some(CODE)
}

fn not_found_or_io(op: &'static str, path: &Path, e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::NotFound {
        Error::NotFound {
            path: path.to_path_buf(),
        }
    } else {
        Error::io(op, path, e)
    }
}

fn special_file(path: &Path) -> Error {
    Error::Refused {
        path: path.to_path_buf(),
        reason: "it is a special file (socket, FIFO, or device)",
    }
}

fn too_deep(path: PathBuf) -> Error {
    Error::Refused {
        path,
        reason: "the directory tree is nested deeper than 256 levels",
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

    #[test]
    fn test_staging_path_is_unique_hidden_sibling() {
        let to = Path::new("dir").join("target");
        let a = staging_path(&to);
        let b = staging_path(&to);
        assert_ne!(a, b);
        assert_eq!(a.parent(), to.parent());
        let name = a.file_name().map(|n| n.to_string_lossy().into_owned());
        assert!(name.is_some_and(|n| n.starts_with(".target.path-lib-")));
    }

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

    #[test]
    fn test_guard_allows_ordinary_nested_paths() {
        let p = os::temp().join("path-lib-guard").join("x");
        assert!(guard_destructive(&p).is_ok());
    }
}
