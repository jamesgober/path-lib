//! # path-lib
//!
//! CROSS-PLATFORM PATHS FOR RUST
//!
//! Where should an application keep its data, config, cache, and logs?
//! The answer differs on every operating system, differs again for a
//! per-user tool versus a machine-wide service, and gets subtly wrong the
//! moment a folder is redirected or an environment variable is missing.
//! `path-lib` answers it once, correctly, with no dependencies.
//!
//! # Application directories
//!
//! ```
//! use path_lib::App;
//!
//! let app = App::new("HiveDB")?;          // this user
//! let svc = App::global("HiveDB")?;       // the whole machine
//!
//! let data = app.data();                  // &Path, resolved once
//! let db = app.data_at("db/main")?;       // hardened sub-path
//! # let _ = (svc, data, db);
//! # Ok::<(), path_lib::Error>(())
//! ```
//!
//! | Kind      | Linux (user)                  | macOS (user)                            | Windows (user)                 |
//! |-----------|-------------------------------|-----------------------------------------|--------------------------------|
//! | data      | `~/.local/share/<app>`        | `~/Library/Application Support/<app>`   | `%LOCALAPPDATA%\<app>`         |
//! | config    | `~/.config/<app>`             | `~/Library/Preferences/<app>`           | `%APPDATA%\<app>`              |
//! | cache     | `~/.cache/<app>`              | `~/Library/Caches/<app>`                | `%LOCALAPPDATA%\<app>\cache`   |
//! | logs      | `~/.local/state/<app>/logs`   | `~/Library/Logs/<app>`                  | `%LOCALAPPDATA%\<app>\logs`    |
//! | state     | `~/.local/state/<app>`        | `.../Application Support/<app>/state`   | `%LOCALAPPDATA%\<app>\state`   |
//! | runtime   | `$XDG_RUNTIME_DIR/<app>`      | `.../Application Support/<app>/run`     | `%LOCALAPPDATA%\<app>\run`     |
//! | temp      | `~/.cache/<app>/tmp`          | `$TMPDIR/<app>`                         | `%TEMP%\<app>`                 |
//! | documents | `~/Documents/<app>`           | `~/Documents/<app>`                     | `Documents\<app>`              |
//!
//! | Kind      | Linux (global)          | macOS (global)                        | Windows (global)               |
//! |-----------|-------------------------|---------------------------------------|--------------------------------|
//! | data      | `/var/lib/<app>`        | `/Library/Application Support/<app>`  | `%ProgramData%\<app>`          |
//! | config    | `/etc/<app>`            | `/Library/Preferences/<app>`          | `%ProgramData%\<app>\config`   |
//! | cache     | `/var/cache/<app>`      | `/Library/Caches/<app>`               | `%ProgramData%\<app>\cache`    |
//! | logs      | `/var/log/<app>`        | `/Library/Logs/<app>`                 | `%ProgramData%\<app>\logs`     |
//! | state     | `/var/lib/<app>/state`  | `.../Application Support/<app>/state` | `%ProgramData%\<app>\state`    |
//! | runtime   | `/run/<app>`            | `/var/run/<app>`                      | `%ProgramData%\<app>\run`      |
//! | temp      | `/var/tmp/<app>`        | `/var/tmp/<app>`                      | `%ProgramData%\<app>\temp`     |
//! | documents | `/srv/<app>`            | `/Users/Shared/<app>`                 | `%PUBLIC%\Documents\<app>`     |
//!
//! XDG variables (`XDG_DATA_HOME`, `XDG_CONFIG_HOME`, ...) and
//! `user-dirs.dirs` are honoured on Linux and the BSDs. Windows folders
//! come from the Known Folder API, so OneDrive-redirected Documents
//! resolve correctly. [`App::portable`] puts everything under one folder
//! of your choosing instead, and [`App::auto`] picks a portable layout
//! next to the executable when a [`PORTABLE_MARKER`] file (`.portable`)
//! is there, and per-user directories otherwise.
//!
//! Individual directories can be relocated after construction.
//! [`App::with_dir`] replaces one kind's directory with an absolute path;
//! [`App::with_env`] does the same from `<PREFIX>_<KIND>_DIR` environment
//! variables, which is how services are usually reconfigured in
//! containers. Neither lets two kinds share a directory.
//!
//! ```
//! use path_lib::{App, Kind};
//!
//! let cache = std::env::temp_dir().join("hivedb-cache");
//! let app = App::new("HiveDB")?
//!     .with_dir(Kind::Cache, &cache)?     // explicit
//!     .with_env("HIVEDB")?;               // HIVEDB_DATA_DIR, ...
//! # let _ = app;
//! # Ok::<(), path_lib::Error>(())
//! ```
//!
//! # Hardened sub-paths
//!
//! Every `*_at` accessor, and the free function [`join`], accepts a
//! relative path in either slash style and rejects, before building
//! anything, input that could escape the base directory or that means
//! different things on different systems: `..`, drive letters, control
//! characters, Windows-reserved characters and their Unicode look-alikes,
//! bidirectional overrides, device names such as `CON`, and trailing dots
//! or spaces. For names typed by end users, [`sanitize`] repairs instead
//! of rejecting.
//!
//! ```
//! # let app = path_lib::App::portable("Demo", std::env::temp_dir())?;
//! assert!(app.data_at("/big\\sloppy//floppy/").is_ok());
//! assert!(app.data_at("../../etc/passwd").is_err());
//!
//! let name = path_lib::sanitize("Q3: final?.pdf");   // "Q3_ final_.pdf"
//! assert!(app.documents_at(&name).is_ok());
//! # Ok::<(), path_lib::Error>(())
//! ```
//!
//! # OS folders
//!
//! [`home`], [`documents`], [`desktop`], [`downloads`], [`pictures`],
//! [`music`], [`videos`], [`root`], [`temp`], [`exe`], and [`exe_dir`]
//! return cached `&'static Path` values. [`os()`] is a compile-time
//! constant: the operating system never needs detecting at runtime.
//! [`expand_home`] turns `~` and `~/rest` from configuration values into
//! paths under the home directory; it deliberately expands nothing else.
//!
//! # Filesystem operations
//!
//! [`make_dir`], [`make_file`], [`exists`], [`list`], [`remove`],
//! [`copy`], [`move_to`], and [`find`] never overwrite, never follow
//! symbolic links while walking a tree, and refuse to remove or move
//! roots, the home directory, and the user's standard folders. [`App`]
//! exposes the common ones scoped to its directories.
//!
//! [`write_atomic`] is the one operation meant to overwrite: it replaces a
//! file through a flushed temporary sibling and a rename, so readers see
//! either the old contents or the complete new contents, never a partial
//! write.
//!
//! ```
//! let file = std::env::temp_dir().join("path-lib-doc-crate").join("app.conf");
//! path_lib::write_atomic(&file, "port = 7000\n")?;
//! assert_eq!(std::fs::read_to_string(&file).unwrap(), "port = 7000\n");
//! # path_lib::remove(file.parent().unwrap())?;
//! # Ok::<(), path_lib::Error>(())
//! ```
//!
//! # Stability
//!
//! The public API, the directory tables, and the behaviour described
//! above are frozen for every 1.x release. The terms are in
//! `docs/STABILITY-1.0.md` in the repository.
//!
//! # Platform support
//!
//! Linux, macOS, and Windows are tested in CI. FreeBSD and the other
//! BSDs use the Linux (XDG) rules with `/var/run` for global runtime
//! files. Other targets fail to compile with an explanatory message
//! rather than resolving paths that would be wrong.

#![deny(warnings)]
#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]
#![deny(unused_results)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]
#![deny(clippy::dbg_macro)]
#![deny(clippy::unreachable)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::missing_safety_doc)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod app;
mod error;
mod join;
mod kind;
mod ops;
mod os;
mod platform;
mod rules;

pub use app::{App, PORTABLE_MARKER};
pub use error::{Error, PREVIEW_CHARS, Reason, Result};
pub use join::{clean, join, sanitize};
pub use kind::{Kind, Scope};
pub use ops::{
    MAX_WALK_DEPTH, copy, exists, find, list, make_dir, make_file, move_to, remove, write_atomic,
};
pub use os::{
    Os, desktop, documents, downloads, exe, exe_dir, expand_home, home, music, os, pictures, root,
    temp, videos,
};
pub use rules::{MAX_DEPTH, MAX_INPUT_BYTES, MAX_SEGMENT_BYTES};
