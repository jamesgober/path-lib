# Changelog

All notable changes to `path-lib` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

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

[Unreleased]: https://github.com/jamesgober/path-lib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamesgober/path-lib/releases/tag/v0.1.0
