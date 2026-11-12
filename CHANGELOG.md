# Changelog

All notable changes to `path-lib` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [1.3.0] - 2026-11-12

Security release. Directories and files the crate creates, or is asked to
use, are now checked for who owns them and who else can change them.

### Security

- **Existing directories are checked before use.** `App::ensure`,
  `ensure_all`, `create`, `ensure_private`, and `create_private` used to
  accept any directory that already existed, so a user who created
  `%ProgramData%\<app>`, `/var/tmp/<app>`, or `/tmp/<app>` (macOS) first
  controlled the files the application later wrote there. Every existing
  directory of the application is now checked: the kind's directory, the
  kind it is nested in (for example `%ProgramData%\<app>` for the global
  config), and every component of `sub`. Each must be owned by the current
  user or an administrator account (root; `SYSTEM` or `Administrators`)
  and must not be writable by other accounts. `ensure_private` and
  `create_private` also require the final directory to be unreadable by
  other accounts. A directory that fails is refused with `Error::Refused`
  and a reason that says what to fix; it is never changed. On Windows the
  DACL is read from a directory handle, and entries that are only
  inherited by new children count too, since every file created inside
  would grant them. A portable layout on a Windows volume without access
  control lists (FAT, exFAT) is accepted.
- **Windows global directories are no longer writable by every user.**
  `ensure` and `create` in `Scope::Global` created directories that
  inherited the `%ProgramData%` list, which lets `Users` create files and
  folders in them. They now get a protected list: full control for the
  current user, `SYSTEM`, and `Administrators`, and read and list access
  for `Users`, the counterpart of mode `0755`. User and portable
  directories get the private list `ensure_private` uses, the counterpart
  of `0700`.
- **Links in the middle of a sub-path are refused.** `create(kind, "a/b")`
  with a symbolic link or junction at `a` created `b` wherever the link
  pointed. Directories are now created one component at a time, and a
  link at any component below the application's directory is refused.
- **`write_atomic` no longer exposes the new contents on Unix.** The
  temporary file was created with the default mode (usually `0644`) and
  narrowed only after the contents were written, so another user could
  open it in that window and keep reading. It is now created exclusively
  with mode `0600` and receives the final bits on the handle once the
  contents are written and flushed. The existing owner and group are kept
  when the process may set them.
- **macOS user temp never falls back to the shared `/tmp`.** Without
  `TMPDIR` (launchd jobs, cron, `sudo`), user temp was `/tmp/<app>`. It is
  now the per-user directory `confstr(_CS_DARWIN_USER_TEMP_DIR)` returns,
  which is where `TMPDIR` normally comes from, and
  `~/Library/Caches/TemporaryItems/<app>` if that fails.

### Added

- `check_owner(path, others)`, `check_owner_private(path)`, and the
  `Others` enum (`MayRead`, `NoAccess`): refuse a file or directory with
  an untrusted owner or that other accounts can change (or, with
  `NoAccess`, read). Unix checks the owner and the group and other bits;
  Windows checks the owner and the DACL. Everything is read from an open
  handle, the final component is never followed, and on Unix the handle
  is compared with the path's device and inode.
- `open_checked(path, others)`: the same checks on a regular file,
  returning the handle opened for reading, so the file checked is the file
  read. On Unix it opens without blocking, so a FIFO is refused instead of
  hanging the caller.
- `write_atomic_private(path, contents)`: `write_atomic` for secrets. The
  result is `0600` on Unix; on Windows the temporary file is opened with
  no sharing and given a protected list (current user, `SYSTEM`,
  `Administrators`) before anything is written. Missing parents are
  created private.
- `App::clear(kind)`: deletes everything inside a kind's directory and
  keeps the directories of other kinds nested in it.

### Changed

- `App::remove(kind, sub)` refuses a path that is, or contains, the
  directory of another kind. Kinds nest (on Windows cache, logs, state,
  and runtime are inside data; on Linux logs are inside state), so
  `remove(Kind::Data, "")` deleted all of them. Use `App::clear` to empty
  a directory while keeping the nested kinds.
- `copy` gives each copied directory the permission bits of its source on
  Unix, so a `0700` tree is no longer copied as `0755`.

### Documentation

- `make_dir`, `make_file`, and the parents `write_atomic` creates do not
  check existing directories; the docs now say so and point to the `App`
  methods and `check_owner`.
- `expand_home` is not a confinement check: `~/../x` can leave the home
  directory. The docs now say so.
- `find`'s matcher was described as linear-time; it is bounded by pattern
  length times name length.
- The README no longer says that no kind can wipe another; it describes
  the nesting and the new `remove` rule.

### Upgrading

A directory that an earlier version created and that another account can
write to is now refused. The usual case is a Windows global directory
made by 1.2 or earlier, which inherits the `%ProgramData%` list. Delete
the directory and let `ensure` create it again, or remove the access other
accounts have to it; the reason in the error says which check failed.

---

## [1.2.0] - 2026-11-12

### Added

- `file_access(&file)` and `FileAccess` (Windows only): who besides the
  current user, `SYSTEM`, and `Administrators` can change or read an open
  file, read from the handle with `GetSecurityInfo` rather than from the
  path, so the file checked is the file read. `FileAccess` reports whether
  the volume keeps access control lists at all (`enforced`), whether the
  owner is trusted (`owner_trusted`), and whether the DACL grants another
  account a write right (`others_can_write`) or a read right
  (`others_can_read`). Only allow entries that apply to the file are
  counted and deny entries are ignored, so the answer can overstate what
  others can do, never understate it. A NULL DACL reports both rights; on
  a volume without access control lists every field holds the unsafe
  answer. The security descriptor is parsed as a bounds-checked byte
  slice; a malformed one fails with `InvalidData`.
- docs.rs builds the Windows target as well, so `file_access` is
  documented there.

Nothing else changes; the crate still has no dependencies.

---

## [1.1.1] - 2026-10-23

### Fixed

- The documentation builds on docs.rs again. The crate root asked nightly
  rustdoc for the `doc_auto_cfg` feature, which nightly has since folded
  into `doc_cfg` and removed, so the docs.rs builds of 1.0.0 and 1.1.0
  failed. No code change.

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
  environment variables, such as `MYAPP_DATA_DIR`. Unset and empty
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

[Unreleased]: https://github.com/jamesgober/path-lib/compare/v1.3.0...HEAD
[1.3.0]: https://github.com/jamesgober/path-lib/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/jamesgober/path-lib/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/jamesgober/path-lib/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/jamesgober/path-lib/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/jamesgober/path-lib/compare/v0.1.0...v1.0.0
[0.1.0]: https://github.com/jamesgober/path-lib/releases/tag/v0.1.0
