# Changelog

All notable changes to `path-lib` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [1.1.0] - 2026-10-23

### Added

- `App::ensure_private(kind)` and `App::create_private(kind, sub)`: like
  `ensure` and `create`, but every directory they create is private to the
  current user on every OS and in every scope. On Unix the mode is `0700`,
  also for `Scope::Global`. On Windows the directory is created with a
  protected access control list that grants full control to the current
  user, `SYSTEM`, and `Administrators` and nothing to anyone else, with no
  inheritance from the parent; files and directories created inside inherit
  it. Missing parents are created the same way, existing directories are
  left alone, and a symbolic link in place of the directory is refused.

`ensure` and `create` are unchanged.

---

## [1.0.0] - 2026-10-13

Stable release. The public API is frozen for 1.x under
[`docs/STABILITY-1.0.md`](docs/STABILITY-1.0.md).

### Added

- `App::with_dir(kind, dir)` replaces the directory of one kind. `dir`
  must be absolute and is cleaned; it is not created until `ensure` or
  `create`. Fails with `InvalidPath` and `Reason::Empty`,
  `Reason::NotAbsolute`, or `Reason::SharedDir` when the new directory
  would equal another kind's.
- `App::with_env(prefix)` applies overrides from `<PREFIX>_<KIND>_DIR`
  environment variables, such as `HIVEDB_DATA_DIR`. Unset and empty
  variables are ignored; other values go through the same checks as
  `with_dir`, and errors show the offending `NAME=value`. The prefix must
  be ASCII letters, digits, and `_`, not starting with a digit.
- `App::auto(name)` builds a portable layout next to the executable when
  a `.portable` marker file sits beside it, and per-user directories
  otherwise. The marker name is exported as `PORTABLE_MARKER`.
- `App::dirs()` iterates `(Kind, &Path)` pairs in `Kind::ALL` order.
- `Kind::env_name()` returns the upper-case variable suffix (`DATA`,
  `CONFIG`, ...) used by `with_env`.
- `write_atomic(path, contents)` replaces a file's contents through a
  flushed temporary sibling and a rename, so readers see the old or the
  new file and never a partial one. Preserves an existing file's Unix
  permission bits, writes through symbolic links while keeping the link,
  creates missing parents, and flushes the parent directory on Unix.
  Refuses directories, broken links, and paths without a file name. It is
  the one operation in the crate meant to overwrite.
- `expand_home(input)` expands a leading `~` or `~/` to the home
  directory and returns other input unchanged. `~name` is rejected with
  `Reason::OtherUser`; `$VAR` and `%VAR%` are deliberately not expanded.
- `Reason::NotAbsolute`, `Reason::SharedDir`, and `Reason::OtherUser`.

### Changed

- `Kind` and `Scope` are `#[non_exhaustive]`, so kinds and scopes can be
  added in minor releases. Exhaustive `match` expressions written against
  0.1.0 need a wildcard arm.
- `move_to` documentation corrected: when the cross-volume fallback has
  copied successfully but deleting the source fails (`op` is
  `"remove moved source"`), the destination is complete and the source may
  be partly deleted. Treat the destination as the surviving version.
  0.1.0 stated that both copies remain.
- Internal layout: `src/ops.rs` is split into `src/ops/` (`basic`,
  `guard`, `remove`, `transfer`, `find`, `write`), and `Kind` and `Scope`
  live in `src/kind.rs`. No public API change.

### Fixed

- The property test `prop_join_slash_style_is_irrelevant` could generate
  Windows device names such as `con` and then fail. It now asserts that
  both slash styles produce the same outcome, whether success or the same
  rejection. Test-only.

---

## [0.1.0] - 2026-10-13

Initial release.

### Added

- `App` with `new` (user scope), `global` (machine scope), and `portable`
  (single root) constructors, resolving data, config, cache, logs, state,
  runtime, temp, and documents directories per platform conventions: XDG
  and FHS on Linux and the BSDs, the `Library` domains on macOS, and Known
  Folders on Windows.
- Hardened sub-path accessors (`data_at`, `config_at`, ..., `at`) and the
  free function `join`, accepting either slash style and rejecting
  traversal, drive letters, control and reserved characters, Unicode
  look-alike separators, bidirectional overrides, Windows device names,
  trailing dots and spaces, and oversized input.
- `sanitize` for lossy repair of user-supplied names, and `clean` for
  lexical normalisation.
- Cached OS folders: `home`, `documents`, `desktop`, `downloads`,
  `pictures`, `music`, `videos`, `root`, `temp`, `exe`, `exe_dir`, plus the
  compile-time `os()`. Linux user folders honour `user-dirs.dirs`.
- Guarded filesystem operations: `make_dir`, `make_file`, `exists`,
  `list`, `remove`, `copy`, `move_to`, `find`, and the scoped `App`
  helpers `ensure`, `ensure_all`, `create`, `list`, `remove`, `find`.
- Unit, integration, and property tests; criterion benchmarks; CI for
  Linux, macOS, and Windows on stable and MSRV 1.85.

[Unreleased]: https://github.com/jamesgober/path-lib/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/jamesgober/path-lib/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/jamesgober/path-lib/compare/v0.1.0...v1.0.0
[0.1.0]: https://github.com/jamesgober/path-lib/releases/tag/v0.1.0
