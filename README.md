<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>path-lib</b>
    <br><sub><sup>CROSS-PLATFORM PATHS FOR RUST</sup></sub>
</h1>

<p align="center">
    <a href="https://crates.io/crates/path-lib"><img src="https://img.shields.io/crates/v/path-lib.svg" alt="Crates.io"></a>
    <a href="https://crates.io/crates/path-lib" alt="Download"><img alt="Crates.io Downloads" src="https://img.shields.io/crates/d/path-lib?color=%230099ff"></a>
    <a href="https://docs.rs/path-lib"><img src="https://docs.rs/path-lib/badge.svg" alt="Documentation"></a>
    <a href="https://github.com/jamesgober/path-lib/actions"><img alt="GitHub CI" src="https://github.com/jamesgober/path-lib/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md" title="MSRV"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</p>

<p align="center">
    <b>Application and OS Directories, Resolved Correctly</b>
    <br>
    <i>User and global scopes, hardened sub-paths, and guarded file operations. Zero dependencies.</i>
</p>

<p>
    <strong>path-lib</strong> answers the question every application asks on startup: where do my data, config, cache, and logs go? The answer is different on Linux, macOS, and Windows, different again for a per-user tool and a machine-wide service, and quietly wrong the moment a Documents folder is redirected to OneDrive or a service starts without <code>HOME</code>. <strong>path-lib</strong> resolves all of it once per process and hands back clean, absolute paths.
</p>

<p>
    Building on top of those directories is where most path bugs live. Every <code>*_at</code> accessor takes a relative path in either slash style and <b>rejects</b> anything that could escape the base directory or mean different things on different systems: <code>..</code>, drive letters, control characters, Windows-reserved characters and their Unicode look-alikes, bidirectional overrides, and device names such as <code>CON</code>. For names typed by end users, <code>sanitize</code> repairs instead of rejecting.
</p>

<p>
    The file operations are deliberately conservative: nothing is ever overwritten, symbolic links are never followed while walking a tree, copies appear whole or not at all, and removing a filesystem root, the home directory, or a standard user folder is refused outright. The one exception is <code>write_atomic</code>, whose job is to replace a file, and which does it so that readers only ever see the old file or the complete new one.
</p>

<p>
    The API is stable. Version 1.0 freezes the public items, the directory tables, and the guarantees above for every 1.x release; the terms are in the <a href="docs/STABILITY-1.0.md">stability contract</a>.
</p>

## Features

### Application Directories
- **Eight kinds** &mdash; data, config, cache, logs, state, runtime, temp, documents
- **Three scopes** &mdash; `App::new` (this user), `App::global` (the machine), `App::portable` (one folder you choose)
- **Portable detection** &mdash; `App::auto` switches to a portable layout when a `.portable` file sits next to the executable
- **Overrides** &mdash; `with_dir` relocates one kind; `with_env("HIVEDB")` reads `HIVEDB_DATA_DIR`, `HIVEDB_LOGS_DIR`, and so on
- **Native conventions** &mdash; XDG and FHS on Linux and BSD, the `Library` domains on macOS, Known Folders on Windows
- **Distinct by construction** &mdash; no two kinds share a directory, so clearing the cache can never wipe data; overrides are checked too

### Hardened Sub-Paths
- **Either slash style** &mdash; `"a/b"`, `"a\\b"`, `"/a//b/"` all mean the same thing and produce the platform separator
- **Reject, don't repair** &mdash; traversal, absolute paths, reserved names and characters fail before any path is built
- **`sanitize`** &mdash; lossy repair for user-supplied names; its output is always accepted

### OS Folders
- **`home`, `documents`, `desktop`, `downloads`, `pictures`, `music`, `videos`** &mdash; redirect-aware
- **`root`, `temp`, `exe`, `exe_dir`** &mdash; plus `os()`, a compile-time constant
- **`expand_home`** &mdash; `~` and `~/rest` for paths read from configuration; `$VARS` are deliberately left alone
- **Resolved once** &mdash; cached for the life of the process; later calls cost about a nanosecond

### Guarded File Operations
- **`make_dir`, `make_file`, `exists`, `list`, `remove`, `copy`, `move_to`, `find`**
- **Never overwrites**, **never follows links**, **refuses catastrophic deletes**
- **Atomic copies** &mdash; staged under a temporary name, renamed into place when complete
- **`write_atomic`** &mdash; replaces a file through a flushed temporary and a rename; keeps Unix permissions and symbolic links

<hr>

## Installation

```toml
[dependencies]
path-lib = "1.0"
```

<hr>

## Quick Start

```rust
use path_lib::{App, Kind};

// Per-user directories, relocatable with HIVEDB_DATA_DIR and friends.
let app = App::new("HiveDB")?.with_env("HIVEDB")?;

// Base directories, resolved once.
println!("{}", app.data().display());     // ~/Library/Application Support/HiveDB on macOS

// Hardened sub-paths: any slash style, always inside the base.
let db = app.data_at("/big\\sloppy//floppy/")?;
assert!(app.data_at("../../etc/passwd").is_err());

// Create on demand (0700 on Unix for user scope).
let wal = app.create(Kind::Data, "db/wal")?;

// Replace a file atomically: readers see the old or the new contents.
path_lib::write_atomic(app.config_at("hivedb.toml")?, "port = 7000\n")?;

// Machine-wide, for services.
let svc = App::global("HiveDB")?;
println!("{}", svc.config().display());   // /etc/HiveDB on Linux
# Ok::<(), path_lib::Error>(())
```

### Where Things Go

| Kind      | Linux (user)                | macOS (user)                          | Windows (user)               |
|-----------|-----------------------------|---------------------------------------|------------------------------|
| data      | `~/.local/share/<app>`      | `~/Library/Application Support/<app>` | `%LOCALAPPDATA%\<app>`       |
| config    | `~/.config/<app>`           | `~/Library/Preferences/<app>`         | `%APPDATA%\<app>`            |
| cache     | `~/.cache/<app>`            | `~/Library/Caches/<app>`              | `%LOCALAPPDATA%\<app>\cache` |
| logs      | `~/.local/state/<app>/logs` | `~/Library/Logs/<app>`                | `%LOCALAPPDATA%\<app>\logs`  |

| Kind      | Linux (global)     | macOS (global)                        | Windows (global)             |
|-----------|--------------------|---------------------------------------|------------------------------|
| data      | `/var/lib/<app>`   | `/Library/Application Support/<app>`  | `%ProgramData%\<app>`        |
| config    | `/etc/<app>`       | `/Library/Preferences/<app>`          | `%ProgramData%\<app>\config` |
| cache     | `/var/cache/<app>` | `/Library/Caches/<app>`               | `%ProgramData%\<app>\cache`  |
| logs      | `/var/log/<app>`   | `/Library/Logs/<app>`                 | `%ProgramData%\<app>\logs`   |

The full tables, including state, runtime, temp, and documents, are in the [API reference](docs/API.md#directory-tables).

### User Input

```rust
let app = path_lib::App::new("HiveDB")?;

// Reject: for paths your code builds.
let p = app.documents_at("exports/2026")?;

// Repair: for names a user typed.
let name = path_lib::sanitize("Q3: final?.pdf");   // "Q3_ final_.pdf"
let p = app.documents_at(&name)?;
# Ok::<(), path_lib::Error>(())
```

<hr>

## Documentation

- [API reference](docs/API.md) &mdash; every public item, the directory tables, and the validation rules.
- [Stability contract](docs/STABILITY-1.0.md) &mdash; what 1.x guarantees and what may still change.
- [Performance](docs/PERFORMANCE.md) &mdash; benchmark methodology and results.
- [Developer guidelines](docs/GUIDELINES.md) &mdash; the engineering bar this project is built to.
- [Release notes](docs/release/) &mdash; what shipped, when, and why.

<br>

## Contributing

Contributions are welcome under the project's dual license. Before opening a pull request, please make sure:

1. `cargo fmt --all -- --check` passes.
2. `cargo clippy --all-targets --all-features -- -D warnings` is clean.
3. `cargo test --all-features` passes.
4. New public items include documentation and at least one example.
5. The [API reference](docs/API.md) and [CHANGELOG](CHANGELOG.md) are updated alongside code changes.

<br>

<!-- LICENSE
############################################# -->
<div id="license">
    <h2>License</h2>
    <p><b>path-lib</b> is <b>dual-licensed</b> under either of:</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; see <a href="./LICENSE-APACHE" title="Apache-2.0 License">LICENSE-APACHE</a> or <a href="http://www.apache.org/licenses/LICENSE-2.0" title="Apache-2.0 License" target="_blank">apache.org/licenses/LICENSE-2.0</a></li>
        <li><b>MIT License</b> &mdash; see <a href="./LICENSE-MIT" title="MIT License">LICENSE-MIT</a> or <a href="https://opensource.org/licenses/MIT" title="MIT License" target="_blank">opensource.org/licenses/MIT</a></li>
    </ul>
    <p>at your option.</p>
    <br>
    <h3>Contribution</h3>
    <p>Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in <b>path-lib</b> by you, as defined in the <b>Apache-2.0 license</b>, shall be <b>dual-licensed</b> as above, without any additional terms or conditions.</p>
    <p>Unless required by applicable law or agreed to in writing, software distributed under the Licenses is distributed on an <b>"AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND</b>, either express or implied.</p>
</div>

<!-- FOOT COPYRIGHT
################################################# -->
<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>JAMES GOBER.</strong></sup>
</div>
