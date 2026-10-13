<h1 align="center">
    <b>path-lib</b><br>
    <sub><sup>STABILITY CONTRACT 1.0</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./API.md" title="API Reference"><b>API</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./PERFORMANCE.md" title="Performance"><b>PERFORMANCE</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./GUIDELINES.md" title="Developer Guidelines"><b>GUIDELINES</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <span>STABILITY</span>
    </sup>
</div>
<br>

This document is the contract `path-lib 1.0.0` ships under. It lists what is frozen for every 1.x release, what may still change in a minor or patch release, and how the contract itself changes.

> **Status:** In effect as of **1.0.0** and covers every 1.x release. Anything here that later needs to break requires a major version (2.0.0).

The signatures and behaviour of each item are documented in the [API reference](./API.md) and on [docs.rs](https://docs.rs/path-lib). Measured performance is in [PERFORMANCE.md](./PERFORMANCE.md).

---

## What 1.x Guarantees

### Public API surface

The items re-exported from the crate root are the complete public API. Their signatures and documented semantics do not change incompatibly within 1.x. All of them are `Send + Sync` where they are types, and none of them panic.

#### Application directories

| Item | Kind | Notes |
|---|---|---|
| `App` | struct | `Debug + Clone + PartialEq + Eq + Display + Send + Sync` |
| `Kind` | enum | `#[non_exhaustive]`; `Debug + Clone + Copy + PartialEq + Eq + Hash + Display` |
| `Kind::ALL` | const | every kind, in declaration order |
| `Kind::name` | fn | `"data"`, `"config"`, ... |
| `Kind::env_name` | fn | `"DATA"`, `"CONFIG"`, ... |
| `Scope` | enum | `#[non_exhaustive]`; `User`, `Global`, `Portable`; same traits as `Kind` |
| `PORTABLE_MARKER` | const | `".portable"` |

#### `App` methods

| Method | Notes |
|---|---|
| `App::new(name)` | current user |
| `App::global(name)` | whole machine |
| `App::portable(name, root)` | everything under `root` |
| `App::auto(name)` | portable beside the executable when `PORTABLE_MARKER` is present, else `new` |
| `App::with_dir(self, kind, dir)` | absolute override for one kind |
| `App::with_env(self, prefix)` | overrides from `<PREFIX>_<KIND>_DIR` |
| `App::name`, `App::scope` | what the `App` was built with |
| `App::dir(kind)`, `App::dirs()` | `&Path`; `dirs` iterates in `Kind::ALL` order |
| `data`, `config`, `cache`, `logs`, `state`, `runtime`, `temp`, `documents` | `&Path`, infallible |
| `App::at(kind, sub)` | hardened sub-path |
| `data_at`, `config_at`, `cache_at`, `logs_at`, `state_at`, `runtime_at`, `temp_at`, `documents_at` | shorthand for `at` |
| `App::ensure(kind)`, `App::ensure_all()` | create the base directories |
| `App::create(kind, sub)` | create a sub-directory |
| `App::list(kind, sub)`, `App::remove(kind, sub)`, `App::find(kind, pattern)` | scoped filesystem operations |

#### Path helpers

| Item | Kind | Notes |
|---|---|---|
| `join` | fn | validated join onto any base |
| `sanitize` | fn | lossy repair; output always accepted by `join` |
| `clean` | fn | lexical normalisation |
| `MAX_DEPTH` | const | `128` segments |
| `MAX_INPUT_BYTES` | const | `4096` bytes |
| `MAX_SEGMENT_BYTES` | const | `255` bytes |

#### OS folders

| Item | Kind | Notes |
|---|---|---|
| `home`, `documents`, `desktop`, `downloads`, `pictures`, `music`, `videos` | fn | `Result<&'static Path>`, cached |
| `root`, `exe`, `exe_dir` | fn | `Result<&'static Path>`, cached |
| `temp` | fn | `&'static Path`, cached |
| `expand_home` | fn | `~` and `~/rest` only |
| `os` | const fn | compile-time constant |
| `Os` | enum | `#[non_exhaustive]`; `Os::name` |

#### Filesystem operations

| Item | Kind | Notes |
|---|---|---|
| `make_dir`, `make_file`, `exists`, `list` | fn | |
| `remove`, `copy`, `move_to`, `find` | fn | guarded |
| `write_atomic` | fn | the one operation that overwrites |
| `MAX_WALK_DEPTH` | const | `256` levels |

#### Errors

| Item | Kind | Notes |
|---|---|---|
| `Error` | enum | `#[non_exhaustive]`; `InvalidName`, `InvalidPath`, `Unresolved`, `AlreadyExists`, `NotFound`, `Refused`, `Io` |
| `Error::reason` | fn | `Some` for input errors only |
| `Reason` | enum | `#[non_exhaustive]`; 16 variants as of 1.0.0 |
| `Result<T>` | type | `std::result::Result<T, Error>` |
| `PREVIEW_CHARS` | const | `96` |

The values of the constants are part of the contract.

### Directory tables

The default directory of every kind, for every platform and scope, is part of the contract. The tables in the [API reference](./API.md#directory-tables) are normative, including:

- which environment variables are consulted (`XDG_*` and `user-dirs.dirs` on Linux and the BSDs, `TMPDIR` on macOS, Known Folders with environment fallbacks on Windows) and that relative `XDG_*` values are ignored;
- the portable layout: `root/data`, `root/config`, `root/cache`, `root/logs`, `root/state`, `root/run`, `root/temp`, `root/documents`;
- the application name used exactly as given, with no case or spacing changes.

Moving a default directory changes where existing installations look for their files, so it requires a major version. A newly added kind gets a new row; existing rows do not change.

### Behavioural guarantees

- **Kinds are pairwise distinct.** No two kinds of one `App` resolve to the same directory, in any scope on any platform. `with_dir` and `with_env` keep this true by rejecting an override that would equal another kind's directory (`Reason::SharedDir`).
- **Construction resolves everything.** After an `App` is built, the directory accessors are infallible and allocation-free. Nothing is created on disk until `ensure`, `ensure_all`, or `create`. `App::auto` checks for the marker once, at construction.
- **Directory modes.** On Unix, `ensure` and `create` use mode `0700` for user and portable scope and `0755` for global scope, and refuse a symbolic link where the directory should be.
- **Validation rules.** `join` and every `*_at` accessor reject, before building any path, the inputs listed in the [validation rules](./API.md#validation-rules): `..`, absolute paths and drive letters, a leading `~`, control characters, Windows-reserved characters and their Unicode look-alikes, bidirectional overrides and isolates and the BOM, Windows device names with any extension, segments ending in `.` or a space, and input over the length and depth limits. The same rules apply on every platform, and an accepted result is always inside the base. Changing what is accepted or rejected is a major-version change.
- **`sanitize` output is valid.** Whatever the input, the result is accepted by `join`, and `sanitize` is idempotent.
- **No overwriting.** `make_file`, `copy`, and `move_to` fail with `AlreadyExists` when the destination exists. `write_atomic` is the only function that replaces an existing file.
- **No link following.** `remove` deletes a link, never its target. `find` reports links but does not descend into them. `copy` refuses trees that contain links or special files.
- **Protected paths.** `remove` and `move_to` refuse filesystem roots, paths with fewer than two named components, the home directory and its ancestors, the user's standard folders, and the OS temp directory, checked on both the lexical and the symlink-resolved form. Paths protected in 1.0.0 stay protected in every 1.x release.
- **Whole copies.** `copy` builds the destination under a temporary sibling and renames it into place only when complete; a failed copy leaves no destination behind.
- **Atomic writes.** `write_atomic` leaves either the old file or the complete new one at `path`, never a partial write. It flushes the new file before the rename and, on Unix, the parent directory after it. It keeps an existing file's Unix permission bits and writes through a symbolic link without replacing the link. On failure the temporary file is removed and the original is untouched.
- **Bounded walks.** `find` and `copy` refuse trees deeper than `MAX_WALK_DEPTH` and never recurse on the call stack.
- **Bounded error text.** Caller input echoed in an error is truncated to `PREVIEW_CHARS` characters and printed with escapes.

### Dependencies and toolchain

| Property | Value |
|---|---|
| **Runtime dependencies** | None, for all of 1.x |
| **Edition** | 2024 |
| **MSRV** | Rust 1.85, frozen for 1.x |
| **License** | Apache-2.0 OR MIT |
| **Platforms** | Linux, macOS, Windows; FreeBSD and other BSDs follow the Linux rules |

Raising the MSRV or adding a runtime dependency is a major-version change. The crate has no feature flags. Dev-dependencies may change freely.

---

## What Can Change in 1.x

### Additions in minor releases

- New `Kind`, `Scope`, `Os`, `Reason`, and `Error` variants. Every one of these enums is `#[non_exhaustive]`; match on them with a wildcard arm.
- New functions, constants, and methods, provided existing ones keep their signatures and semantics.
- New trait implementations on existing types.

### Not covered by the contract

- **Error message text.** The `Display` output of `Error` and `Reason` may be reworded. The variant and its fields are the stable part; match on those, not on strings.
- **The `reason` text of `Error::Refused`.** It is a human-readable explanation, not an identifier.
- **The `op` text of `Error::Io`,** except `"remove moved source"` from `move_to`, which callers use to recognise a completed copy whose source was not fully deleted.
- **Temporary file names** used by `copy` and `write_atomic`. They are hidden siblings of the destination; their exact form may change.
- **Performance.** The numbers in [PERFORMANCE.md](./PERFORMANCE.md) are measurements, not guarantees, and may change in any release.
- **Internal structure:** module layout, private types, and the Windows FFI declarations.

---

## Versioning Policy

| Bump | Triggered by |
|---|---|
| Major | Any incompatible change to an item listed above; a change to a directory table or the validation rules; weakening a behavioural guarantee; raising the MSRV; adding a runtime dependency |
| Minor | New items or enum variants on `#[non_exhaustive]` enums; performance improvements |
| Patch | Bug fixes, documentation, internal changes with no behavioural effect |

### Deprecation

An item is deprecated with `#[deprecated(since = "1.Y.0", note = "...")]` in a minor release and removed no earlier than 2.0.0. The release notes describe the replacement.

---

## How the Contract Is Enforced

- **CI** runs `cargo fmt --check`, `cargo clippy -D warnings`, the full test suite, and `cargo doc` with `-D warnings` on Linux, macOS, and Windows, on stable and Rust 1.85.0, and type-checks FreeBSD.
- **Unit tests** check every platform table through an injected environment lookup, so the directory tables are pinned without touching the real environment.
- **Property tests** check confinement of `join`, slash-style independence, validity and idempotence of `sanitize`, and idempotence of `clean`.
- **Integration tests** cover the no-overwrite, no-link-following, protected-path, and atomic-write guarantees, with injected rename failures for the paths real filesystems rarely produce.
- **`cargo audit`** and **`cargo deny`** run on every push.

---

## Changing This Document

When an item on the frozen list needs to change:

1. The change is a major version. It does not land on a 1.x branch.
2. Open an issue that states what would change and why.
3. Update this document on the 2.0 branch.
4. Where possible, deprecate the old item in a 1.x minor release first.

<br>

<div align="center">
  <sup>path-lib 1.0.0 &mdash; COPYRIGHT <small>&copy;</small> 2026 <strong>JAMES GOBER.</strong> Apache-2.0 OR MIT.</sup>
</div>
