<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>path-lib</b><br>
    <sub><sup>API REFERENCE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <span>API</span>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./PERFORMANCE.md" title="Performance"><b>PERFORMANCE</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./GUIDELINES.md" title="Developer Guidelines"><b>GUIDELINES</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./STABILITY-1.0.md" title="Stability Contract"><b>STABILITY</b></a>
    </sup>
</div>
<br>

> **Status:** This reference tracks the public surface of `path-lib` **1.1.1**. Everything documented here is frozen for 1.x under the [stability contract](./STABILITY-1.0.md).

## Table of Contents

- [App](#app)
  - [Constructors](#constructors)
  - [Portable detection](#portable-detection)
  - [Overrides](#overrides)
  - [Directories](#directories)
  - [Sub-paths](#sub-paths)
  - [Creating and managing](#creating-and-managing)
- [Kind and Scope](#kind-and-scope)
- [Directory Tables](#directory-tables)
- [OS Folders](#os-folders)
  - [`expand_home`](#expand_home)
- [Path Helpers](#path-helpers)
- [Validation Rules](#validation-rules)
- [Filesystem Operations](#filesystem-operations)
  - [`write_atomic`](#write_atomic)
- [Errors](#errors)
  - [Reasons](#reasons)
- [Design Notes](#design-notes)

<br>

## App

`App` holds the eight directories of one application for one scope. Construction validates the name and resolves every directory; after that, accessors are infallible and allocation-free. Nothing is created on disk until `ensure` or `create` is called. `App` is `Clone + Send + Sync`.

### Constructors

| Function | Scope | Fails when |
|---|---|---|
| `App::new(name)` | Current user | Name invalid; home directory unknown |
| `App::global(name)` | Whole machine | Name invalid; on Windows, `%ProgramData%` unknown |
| `App::portable(name, root)` | Everything under `root` | Name invalid; `root` empty |
| `App::auto(name)` | Portable beside the executable, or current user | As for `portable` or `new` |

The name becomes a directory name, so it follows the single-segment [validation rules](#validation-rules) and may not start with `~`. It is used exactly as given; `App::new("HiveDB")` produces `HiveDB` directories on every platform.

```rust
use path_lib::App;

let user = App::new("HiveDB")?;
let svc = App::global("HiveDB")?;
let port = App::portable("HiveDB", path_lib::exe_dir()?)?;
# Ok::<(), path_lib::Error>(())
```

`App::portable` lays out `root/data`, `root/config`, `root/cache`, `root/logs`, `root/state`, `root/run`, `root/temp`, and `root/documents`. A relative `root` is pinned to the current directory at construction.

### Portable detection

```rust
use path_lib::{App, Scope};

let app = App::auto("HiveDB")?;
assert!(matches!(app.scope(), Scope::User | Scope::Portable));
# Ok::<(), path_lib::Error>(())
```

`App::auto` returns `App::portable(name, exe_dir())` when an entry named `PORTABLE_MARKER` (`.portable`) exists in the executable's directory, and `App::new(name)` otherwise, including when the executable's directory cannot be determined. Ship the same binary in an installer and in a zip, and put an empty `.portable` file in the zip. The marker is checked once, at construction; creating or deleting it later does not affect an existing `App`.

### Overrides

```rust
use path_lib::{App, Kind};

let data = std::env::temp_dir().join("hivedb-data");
let app = App::new("HiveDB")?
    .with_dir(Kind::Data, &data)?    // explicit
    .with_env("HIVEDB")?;            // HIVEDB_DATA_DIR, HIVEDB_LOGS_DIR, ...
# let _ = app;
# Ok::<(), path_lib::Error>(())
```

| Method | Does | Fails with `InvalidPath` and |
|---|---|---|
| `with_dir(kind, dir)` | Replaces one kind's directory | `Reason::Empty`, `Reason::NotAbsolute`, or `Reason::SharedDir` |
| `with_env(prefix)` | Applies `<PREFIX>_<KIND>_DIR` for every kind | `Reason::NotAbsolute` or `Reason::SharedDir` |

Both take and return the `App` by value, so they chain after any constructor. The new directory must be absolute; it is cleaned, and it is not created until `ensure` or `create`. An override that would equal the directory of another kind is rejected with `Reason::SharedDir`, which keeps every kind distinct. Setting a kind to its current directory is allowed.

`with_env` reads one variable per kind, named with [`Kind::env_name`](#kind-and-scope). With the prefix `HIVEDB` these are `HIVEDB_DATA_DIR`, `HIVEDB_CONFIG_DIR`, `HIVEDB_CACHE_DIR`, `HIVEDB_LOGS_DIR`, `HIVEDB_STATE_DIR`, `HIVEDB_RUNTIME_DIR`, `HIVEDB_TEMP_DIR`, and `HIVEDB_DOCUMENTS_DIR`. Unset and empty variables leave the directory unchanged. Other values go through the same checks as `with_dir`, in `Kind::ALL` order, and an error's `path` reads `NAME=value` so the offending variable is easy to find. The prefix must be non-empty ASCII letters, digits, and `_`, not starting with a digit; otherwise `with_env` fails with `InvalidName`.

### Directories

```rust
# let app = path_lib::App::new("HiveDB")?;
let data: &std::path::Path = app.data();
let same = app.dir(path_lib::Kind::Data);
# assert_eq!(data, same);
# Ok::<(), path_lib::Error>(())
```

`data()`, `config()`, `cache()`, `logs()`, `state()`, `runtime()`, `temp()`, `documents()`, and `dir(kind)` return `&Path`. `dirs()` iterates `(Kind, &Path)` pairs in `Kind::ALL` order. `name()` and `scope()` return what the `App` was built with. `Display` prints a table of every directory.

```rust
# let app = path_lib::App::new("HiveDB")?;
for (kind, dir) in app.dirs() {
    println!("{kind:<9} {}", dir.display());
}
# Ok::<(), path_lib::Error>(())
```

### Sub-paths

```rust
# let app = path_lib::App::new("HiveDB")?;
let p = app.data_at("big/sloppy/floppy")?;            // PathBuf
let q = app.at(path_lib::Kind::Data, "\\big\\sloppy\\floppy\\")?;
assert_eq!(p, q);
# Ok::<(), path_lib::Error>(())
```

`data_at`, `config_at`, `cache_at`, `logs_at`, `state_at`, `runtime_at`, `temp_at`, `documents_at`, and `at(kind, sub)` join a relative sub-path onto the base after validating it. See [`join`](#join) for the accepted syntax. An empty sub-path returns the base directory.

### Creating and managing

| Method | Does |
|---|---|
| `ensure(kind)` | Creates the kind's directory if missing; returns it |
| `ensure_all()` | `ensure` for every kind |
| `create(kind, sub)` | Validates `sub`, creates it with parents, returns the path |
| `ensure_private(kind)` | Like `ensure`, but private to the current user on every OS (since 1.1) |
| `create_private(kind, sub)` | Like `create`, with the permissions of `ensure_private` (since 1.1) |
| `list(kind, sub)` | Sorted entries of a sub-directory |
| `remove(kind, sub)` | Deletes a sub-path; empty `sub` clears the whole kind |
| `find(kind, pattern)` | Name search under the kind's directory |

On Unix, `ensure` and `create` create user and portable directories with mode `0700` and global ones with `0755`. Both refuse a symbolic link where the directory should be, so a link planted in a shared location cannot redirect the application's files.

`ensure_private` and `create_private` (since 1.1) are for directories other users must not read, such as logs, audit records, crash reports, and keys, in every scope:

- **Unix:** mode `0700` (subject to the umask), also for `Scope::Global`.
- **Windows:** a protected access control list, set when the directory is created: full control for the current user, `SYSTEM`, and `Administrators`, nothing for anyone else, no inheritance from the parent, and inherited by everything created inside.

Missing parents are created the same way; a directory that already exists is left as it is.

<br>

## Kind and Scope

| `Kind` | Holds |
|---|---|
| `Data` | Persistent data: databases, user content, indexes. Back this up. |
| `Config` | Configuration a user or administrator may edit |
| `Cache` | Regenerable data. Safe to delete at any time. |
| `Logs` | Log files |
| `State` | Survives restarts but not worth backing up: history, window positions |
| `Runtime` | Sockets, PID files, locks |
| `Temp` | Private scratch space |
| `Documents` | Files the user is meant to open, inside their documents folder |

`Kind::ALL` lists every kind; `kind.name()` gives `"data"`, `"config"`, and so on, and `kind.env_name()` gives the upper-case suffix used by [`with_env`](#overrides): `DATA`, `CONFIG`, `CACHE`, `LOGS`, `STATE`, `RUNTIME`, `TEMP`, or `DOCUMENTS`.

`Scope` is `User`, `Global`, or `Portable`.

Both enums are `#[non_exhaustive]`, so a kind or scope can be added in a minor release. Match on them with a wildcard arm, or iterate `Kind::ALL`:

```rust
use path_lib::Kind;

fn backed_up(kind: Kind) -> bool {
    match kind {
        Kind::Data | Kind::Config | Kind::Documents => true,
        _ => false,
    }
}
```

<br>

## Directory Tables

### Linux and BSD

| Kind | User | Global |
|---|---|---|
| data | `$XDG_DATA_HOME/<app>` (`~/.local/share/<app>`) | `/var/lib/<app>` |
| config | `$XDG_CONFIG_HOME/<app>` (`~/.config/<app>`) | `/etc/<app>` |
| cache | `$XDG_CACHE_HOME/<app>` (`~/.cache/<app>`) | `/var/cache/<app>` |
| logs | `$XDG_STATE_HOME/<app>/logs` | `/var/log/<app>` |
| state | `$XDG_STATE_HOME/<app>` (`~/.local/state/<app>`) | `/var/lib/<app>/state` |
| runtime | `$XDG_RUNTIME_DIR/<app>`, else `<state>/run` | `/run/<app>` (`/var/run/<app>` on BSD) |
| temp | `$XDG_CACHE_HOME/<app>/tmp` | `/var/tmp/<app>` |
| documents | `XDG_DOCUMENTS_DIR/<app>` (`~/Documents/<app>`) | `/srv/<app>` |

Relative `XDG_*` values are ignored, as the specification requires. User temp is placed under the private cache directory rather than the shared `/tmp`, where another user could pre-create `/tmp/<app>`.

### macOS

| Kind | User | Global |
|---|---|---|
| data | `~/Library/Application Support/<app>` | `/Library/Application Support/<app>` |
| config | `~/Library/Preferences/<app>` | `/Library/Preferences/<app>` |
| cache | `~/Library/Caches/<app>` | `/Library/Caches/<app>` |
| logs | `~/Library/Logs/<app>` | `/Library/Logs/<app>` |
| state | `<data>/state` | `<data>/state` |
| runtime | `<data>/run` | `/var/run/<app>` |
| temp | `$TMPDIR/<app>` (per-user) | `/var/tmp/<app>` |
| documents | `~/Documents/<app>` | `/Users/Shared/<app>` |

### Windows

| Kind | User | Global |
|---|---|---|
| data | `%LOCALAPPDATA%\<app>` | `%ProgramData%\<app>` |
| config | `%APPDATA%\<app>` (roaming) | `<data>\config` |
| cache | `<data>\cache` | `<data>\cache` |
| logs | `<data>\logs` | `<data>\logs` |
| state | `<data>\state` | `<data>\state` |
| runtime | `<data>\run` | `<data>\run` |
| temp | `%TEMP%\<app>` | `<data>\temp` |
| documents | `<Documents>\<app>` | `%PUBLIC%\Documents\<app>` |

Folders come from `SHGetKnownFolderPath`, so redirected folders (OneDrive backup on Windows 11) resolve correctly. Environment variables are the fallback.

<br>

## OS Folders

| Function | Returns | Fails |
|---|---|---|
| `home()` | `Result<&'static Path>` | No home directory |
| `documents()`, `desktop()`, `downloads()`, `pictures()`, `music()`, `videos()` | `Result<&'static Path>` | No home directory |
| `root()` | `Result<&'static Path>` (`/` or `C:\`) | Windows only, if the system drive is unknown |
| `temp()` | `&'static Path` | Never |
| `exe()`, `exe_dir()` | `Result<&'static Path>` | The OS cannot report the executable |
| `os()` | `Os` (compile-time constant) | Never |

Values are resolved on first use and cached for the life of the process. User folders and machine folders are cached separately, so a process without a home directory can still use global paths. On Linux, the user folders follow `user-dirs.dirs` (including localised names such as `~/Dokumente`).

### `expand_home`

```rust
let home = path_lib::home()?;
assert_eq!(path_lib::expand_home("~/data")?, home.join("data"));
assert_eq!(path_lib::expand_home("/var/lib/x")?, std::path::Path::new("/var/lib/x"));
assert!(path_lib::expand_home("~root/x").is_err());
# Ok::<(), path_lib::Error>(())
```

| Input | Result |
|---|---|
| `~` | `home()` |
| `~/rest` (also `~\rest` on Windows) | `home()` joined with `rest`, cleaned |
| `~name` | `InvalidPath` with `Reason::OtherUser` |
| Anything not starting with `~` | Returned unchanged |

Use it on paths read from configuration files and environment variables, such as values passed to [`with_dir`](#overrides). It fails with `Unresolved` only when the input starts with `~` and there is no home directory. `$VAR` and `%VAR%` are deliberately not expanded: expanding them would let a configuration value pull arbitrary environment state into a path.

<br>

## Path Helpers

### `join`

```rust
let p = path_lib::join("/srv/files", "uploads/2026")?;
# Ok::<(), path_lib::Error>(())
```

Joins a relative sub-path onto any base, with the same rules as the `*_at` accessors. `/` and `\` are both separators; leading, trailing, and repeated separators and `.` segments are ignored. The result uses the platform separator and is always inside the base.

### `sanitize`

```rust
assert_eq!(path_lib::sanitize("Q3: final?.pdf"), "Q3_ final_.pdf");
assert_eq!(path_lib::sanitize("CON"), "_CON");
```

Turns any text into one valid name: rejected characters become `_`, trailing dots and spaces are removed, a leading `~` becomes `_`, device names gain a `_` prefix, long input is cut on a character boundary, and empty results become `_`. The output is always accepted by `join`, and `sanitize` is idempotent.

### `clean`

```rust
use std::path::Path;
assert_eq!(path_lib::clean("a/./b//../c/"), Path::new("a").join("c"));
```

Lexical normalisation without touching the filesystem: removes `.` and redundant separators, resolves `..` against the previous segment, and never climbs above a root. On Windows `/` is accepted as a separator; on Unix `\` is an ordinary character.

<br>

## Validation Rules

Applied by `join`, every `*_at` accessor, `App` names (as a single segment), and `find` patterns (with `*` and `?` allowed).

| Rule | `Reason` |
|---|---|
| `..` anywhere | `Traversal` |
| Starts with a drive letter (`C:`) | `Absolute` |
| First segment starts with `~` | `HomeShortcut` |
| ASCII or C1 control character, including NUL | `ControlChar` |
| `< > : " \| ? *`, or a full-width / look-alike form of a reserved character or separator | `ReservedChar(c)` |
| Bidirectional override or isolate (U+202A&ndash;202E, U+2066&ndash;2069) or BOM | `InvisibleChar(c)` |
| `CON`, `PRN`, `AUX`, `NUL`, `COM0`&ndash;`COM9`, `LPT0`&ndash;`LPT9`, `COM¹²³`, `LPT¹²³`, `CONIN$`, `CONOUT$`, with any extension | `ReservedName` |
| Segment ends in `.` or space | `TrailingDotOrSpace` |
| Segment over 255 bytes | `SegmentTooLong` |
| Input over 4096 bytes | `TooLong` |
| More than 128 segments | `TooDeep` |
| Separator inside a single name | `Separator` |

Every rule applies on every platform, so a path accepted on Linux is also valid on Windows. Joiners and directional marks (U+200C, U+200D, U+200E, U+200F) are allowed because real scripts depend on them.

<br>

## Filesystem Operations

| Function | Behaviour |
|---|---|
| `make_dir(path)` | Creates a directory and parents; succeeds if it exists |
| `make_file(path)` | Creates an empty file and parents; never overwrites |
| `exists(path)` | `true` for anything at the path, including broken links |
| `list(dir)` | Sorted entries |
| `remove(path)` | Deletes a file, link, or tree; `Ok(false)` if absent |
| `copy(from, to)` | File or tree; returns bytes copied; never overwrites |
| `move_to(from, to)` | Atomic rename, or copy then delete across volumes |
| `find(root, pattern)` | Recursive name search with `*` and `?`; sorted |
| `write_atomic(path, contents)` | Replaces a file's contents atomically; creates the file and its parents if missing |

Guarantees:

- **No overwrites.** Copy, move, and file creation fail with `AlreadyExists`. [`write_atomic`](#write_atomic) is the deliberate exception: replacing a file is its purpose.
- **No link following.** `remove` deletes a link, never its target. `find` lists links but does not descend into them. `copy` refuses trees containing links or special files.
- **Protected paths.** `remove` and `move_to` refuse filesystem roots, top-level directories (`/usr`, `C:\Users`), the home directory and its parents, the user's standard folders, and the OS temp directory. Checks compare case-insensitively on Windows and macOS and are repeated on the symlink-resolved form.
- **Atomic copies.** A copy is built under a hidden sibling name and renamed into place only when complete; on failure the partial copy is deleted.
- **Bounded walks.** Trees deeper than `MAX_WALK_DEPTH` (256) are refused; walks are iterative, so depth never threatens the stack. Pattern matching is linear-time.

Checks and actions are separate system calls, so another process can change the filesystem between them. The guards prevent mistakes, not a hostile process racing this one in the same directory.

`move_to` renames when source and destination are on the same volume. Across volumes it copies, then removes the source. If the copy succeeded but removing the source failed, the error is `Io` with `op` set to `"remove moved source"`: the destination is complete, and the source may be partly deleted, because a directory's contents can be removed before the directory itself fails. Treat the destination as the surviving version.

### `write_atomic`

```rust
let dir = std::env::temp_dir().join("path-lib-api-write-atomic");
let file = dir.join("app.conf");
path_lib::write_atomic(&file, "port = 7000\n")?;
path_lib::write_atomic(&file, "port = 7001\n")?;
assert_eq!(std::fs::read_to_string(&file).unwrap(), "port = 7001\n");
# path_lib::remove(&dir)?;
# Ok::<(), path_lib::Error>(())
```

The contents are written to a hidden temporary sibling, flushed to disk with `sync_all`, and renamed over `path`. A reader sees either the old file or the complete new one, never a partial write, even if the process or machine fails midway. On Unix the parent directory is flushed after the rename, so the rename itself survives a power loss. Missing parent directories are created.

- **Permissions.** On Unix an existing file's permission bits are copied to the new file, so a `0600` file stays `0600`. A new file gets the process default. On Windows the file inherits the directory's access control list.
- **Symbolic links.** If `path` is a link, the file it points to is replaced and the link is kept.
- **Refused.** A directory, a broken symbolic link, or a path without a file name fails with `Refused`.
- **Failure.** On any error the temporary file is removed and the original file is untouched.

It is the one operation in the crate whose purpose is to overwrite. Use it for configuration, state, and other files that must never be seen half-written.

<br>

## Errors

All fallible functions return `path_lib::Result<T>`.

| Variant | Meaning | What to do |
|---|---|---|
| `InvalidName { name, reason }` | App name rejected | Choose another name |
| `InvalidPath { path, reason }` | Sub-path or pattern rejected | Fix the input, or `sanitize` user input first |
| `Unresolved { dir }` | The OS did not report a directory | Set `HOME`, or use `App::global` / `App::portable` |
| `AlreadyExists { path }` | Destination is taken | Remove it first or pick another |
| `NotFound { path }` | Source is missing | Check the path |
| `Refused { path, reason }` | A guard blocked a dangerous operation | Nothing was changed; reconsider the operation |
| `Io { op, path, source }` | The OS reported an error | Usually permissions or disk space |

`error.reason()` returns the `Reason` for input errors. Caller input echoed in an error is truncated to 96 characters (`PREVIEW_CHARS`) and printed with escapes. `Error` is `#[non_exhaustive]`.

### Reasons

The [validation rules](#validation-rules) list the reasons `join` and the `*_at` accessors produce. The others are:

| `Reason` | Raised by |
|---|---|
| `Empty` | An empty app name, `App::portable` root, `with_dir` directory, or `with_env` prefix |
| `NotAbsolute` | `with_dir`, or a `with_env` variable, given a relative path |
| `SharedDir` | `with_dir` or `with_env` would give two kinds the same directory |
| `OtherUser` | `expand_home` given `~name` |

`Reason` is `#[non_exhaustive]`; new reasons may be added in minor releases.

<br>

## Design Notes

**No runtime OS detection.** The operating system is fixed when the binary is compiled, so `os()` is a constant. Storing it in an environment variable at startup would cost more than reading the constant, and writing environment variables is `unsafe` in edition 2024.

**Caching.** Folder lookups are cached in `OnceLock` statics. A static belongs to the whole binary, so every crate in the process shares one resolution. These statics are written once and read-only afterwards; they are the crate's only global state.

**Testability.** Every platform table is resolved through an injected environment lookup, so the rules are unit-tested against arbitrary (including hostile) environments without touching the real one.

**One `unsafe` block family.** Windows Known Folders need `SHGetKnownFolderPath`. The crate declares that one function and `CoTaskMemFree` directly instead of depending on `windows-sys`; every `unsafe` block carries a `SAFETY` comment and the crate root denies `unsafe_code` everywhere else.
