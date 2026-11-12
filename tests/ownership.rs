//! Owner and permission checks on directories that already exist, links
//! in the middle of a sub-path, and kinds nested inside each other.

#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::path::Path;

use common::Sandbox;
use path_lib::{App, Error, Kind, Others};

fn refused<T: std::fmt::Debug>(r: path_lib::Result<T>) -> &'static str {
    match r {
        Err(Error::Refused { reason, .. }) => reason,
        other => panic!("expected Refused, got {other:?}"),
    }
}

// --- Windows: access control lists ---------------------------------------

#[cfg(windows)]
fn icacls(path: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("icacls")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "icacls {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Creates `link` as a junction to `target`; junctions need no privilege.
#[cfg(windows)]
fn junction(target: &Path, link: &Path) {
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(out.status.success(), "mklink /J: {out:?}");
}

#[cfg(windows)]
#[test]
fn test_windows_ensure_private_refuses_a_preexisting_open_dir() {
    let sb = Sandbox::new("own-win-open");
    let app = App::portable("demo", sb.path()).unwrap();
    std::fs::create_dir_all(app.logs()).unwrap();
    let _ = icacls(app.logs(), &["/grant", "*S-1-1-0:(OI)(CI)(F)"]);
    assert!(refused(app.ensure_private(Kind::Logs)).contains("change"));
    assert!(refused(app.ensure(Kind::Logs)).contains("change"));
    // The directory was left as it was, not repaired behind the caller.
    assert!(icacls(app.logs(), &[]).contains("Everyone"));
}

#[cfg(windows)]
#[test]
fn test_windows_ensure_private_refuses_a_dir_users_can_read() {
    let sb = Sandbox::new("own-win-read");
    let app = App::portable("demo", sb.path()).unwrap();
    // Start from a private directory, so the test does not depend on the
    // ACL of this machine's temp directory.
    let _ = app.ensure_private(Kind::State).unwrap();
    let _ = icacls(app.state(), &["/grant", "*S-1-5-32-545:(OI)(CI)(RX)"]);
    // Others may read: fine for ensure, not for ensure_private.
    let _ = app.ensure(Kind::State).unwrap();
    assert!(refused(app.ensure_private(Kind::State)).contains("read"));
}

#[cfg(windows)]
#[test]
fn test_windows_inherited_write_for_users_is_refused() {
    // Entries that only reach new children still count on a directory.
    let sb = Sandbox::new("own-win-inherit");
    let app = App::portable("demo", sb.path()).unwrap();
    std::fs::create_dir_all(app.cache()).unwrap();
    let _ = icacls(app.cache(), &["/grant", "*S-1-5-11:(OI)(CI)(IO)(M)"]);
    assert!(refused(app.ensure(Kind::Cache)).contains("change"));
}

#[cfg(windows)]
#[test]
fn test_windows_planted_app_root_refuses_nested_kinds() {
    // The global layout on Windows nests config inside data
    // (%ProgramData%\<app>\config); reproduce it under temp.
    let sb = Sandbox::new("own-win-root");
    let root = sb.join("svc");
    let app = App::global("svc")
        .unwrap()
        .with_dir(Kind::Data, &root)
        .unwrap()
        .with_dir(Kind::Config, root.join("config"))
        .unwrap()
        .with_dir(Kind::State, root.join("state"))
        .unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let _ = icacls(&root, &["/grant", "*S-1-5-11:(OI)(CI)(M)"]);
    assert!(refused(app.ensure(Kind::Config)).contains("change"));
    assert!(refused(app.ensure_private(Kind::State)).contains("change"));
    assert!(!root.join("config").exists() && !root.join("state").exists());
}

#[cfg(windows)]
#[test]
fn test_windows_global_ensure_grants_users_read_but_not_write() {
    let sb = Sandbox::new("own-win-global");
    let root = sb.join("svc");
    let app = App::global("svc")
        .unwrap()
        .with_dir(Kind::Data, &root)
        .unwrap()
        .with_dir(Kind::Config, root.join("config"))
        .unwrap();
    let config = app.ensure(Kind::Config).unwrap();
    let acl = icacls(config, &[]);
    assert!(acl.contains("BUILTIN\\Users:(OI)(CI)(RX)"), "{acl}");
    assert!(!acl.contains("(I)"), "inherits from the parent: {acl}");
    assert!(!acl.contains("WD") && !acl.contains("AD"), "{acl}");
    path_lib::check_owner(config, Others::MayRead).unwrap();
    assert!(path_lib::check_owner_private(config).is_err());
    // Running ensure again accepts what it created.
    let _ = app.ensure(Kind::Config).unwrap();
    let _ = app.ensure(Kind::Data).unwrap();
}

#[cfg(windows)]
#[test]
fn test_windows_user_scope_dirs_pass_their_own_checks() {
    let sb = Sandbox::new("own-win-user");
    let app = App::portable("demo", sb.path()).unwrap();
    for kind in Kind::ALL {
        let dir = app.ensure(kind).unwrap();
        path_lib::check_owner_private(dir).unwrap();
        let _ = app.ensure_private(kind).unwrap();
    }
    let keys = app.create_private(Kind::Data, "keys/main").unwrap();
    path_lib::check_owner_private(keys).unwrap();
}

#[cfg(windows)]
#[test]
fn test_windows_create_refuses_an_intermediate_junction() {
    let sb = Sandbox::new("own-win-junction");
    let outside = sb.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let app = App::portable("demo", sb.join("app")).unwrap();
    let data = app.ensure(Kind::Data).unwrap();
    junction(&outside, &data.join("a"));
    let _ = refused(app.create(Kind::Data, "a"));
    let _ = refused(app.create(Kind::Data, "a/b"));
    let _ = refused(app.create_private(Kind::Data, "a/b"));
    assert!(!outside.join("b").exists(), "followed the junction");
}

#[cfg(windows)]
#[test]
fn test_windows_open_checked_refuses_a_file_others_can_change() {
    let sb = Sandbox::new("own-win-file");
    let file = sb.join("app.env");
    path_lib::write_atomic_private(&file, "A=1\n").unwrap();
    let _ = path_lib::open_checked(&file, Others::NoAccess).unwrap();
    let _ = icacls(&file, &["/grant", "*S-1-1-0:(M)"]);
    assert!(refused(path_lib::open_checked(&file, Others::MayRead)).contains("change"));
    assert!(refused(path_lib::check_owner(&file, Others::MayRead)).contains("change"));
}

// --- Unix: modes and owners ------------------------------------------------

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
#[test]
fn test_unix_ensure_refuses_a_preexisting_writable_dir() {
    let sb = Sandbox::new("own-unix-open");
    let app = App::portable("demo", sb.path()).unwrap();
    std::fs::create_dir_all(app.temp()).unwrap();
    set_mode(app.temp(), 0o777);
    assert!(refused(app.ensure(Kind::Temp)).contains("write"));
    assert!(refused(app.ensure_private(Kind::Temp)).contains("write"));
    set_mode(app.temp(), 0o755);
    let _ = app.ensure(Kind::Temp).unwrap();
    assert!(refused(app.ensure_private(Kind::Temp)).contains("read"));
    set_mode(app.temp(), 0o700);
    let _ = app.ensure_private(Kind::Temp).unwrap();
}

#[cfg(unix)]
#[test]
fn test_unix_planted_app_root_refuses_nested_kinds() {
    let sb = Sandbox::new("own-unix-root");
    let root = sb.join("svc");
    let app = App::global("svc")
        .unwrap()
        .with_dir(Kind::Data, &root)
        .unwrap()
        .with_dir(Kind::State, root.join("state"))
        .unwrap();
    std::fs::create_dir_all(&root).unwrap();
    set_mode(&root, 0o777);
    assert!(refused(app.ensure(Kind::State)).contains("write"));
    assert!(!root.join("state").exists());
    set_mode(&root, 0o755);
    let state = app.ensure(Kind::State).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(state).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o022, 0, "{mode:o}");
}

#[cfg(unix)]
#[test]
fn test_unix_create_refuses_an_intermediate_symlink() {
    let sb = Sandbox::new("own-unix-link");
    let outside = sb.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let app = App::portable("demo", sb.join("app")).unwrap();
    let data = app.ensure(Kind::Data).unwrap();
    std::os::unix::fs::symlink(&outside, data.join("a")).unwrap();
    let _ = refused(app.create(Kind::Data, "a/b"));
    let _ = refused(app.create_private(Kind::Data, "a/b"));
    assert!(!outside.join("b").exists(), "followed the link");
}

#[cfg(unix)]
#[test]
fn test_unix_root_owned_dirs_are_trusted() {
    // `/` is owned by root and not writable by others on every Unix.
    path_lib::check_owner("/", Others::MayRead).unwrap();
    assert!(refused(path_lib::check_owner_private("/")).contains("read"));
}

#[cfg(unix)]
#[test]
fn test_unix_open_checked_refuses_writable_and_special_files() {
    let sb = Sandbox::new("own-unix-file");
    let file = sb.write("app.env", "A=1\n");
    set_mode(&file, 0o600);
    let _ = path_lib::open_checked(&file, Others::NoAccess).unwrap();
    set_mode(&file, 0o644);
    let _ = path_lib::open_checked(&file, Others::MayRead).unwrap();
    assert!(refused(path_lib::open_checked(&file, Others::NoAccess)).contains("read"));
    set_mode(&file, 0o666);
    assert!(refused(path_lib::open_checked(&file, Others::MayRead)).contains("write"));
    let link = sb.join("link.env");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let _ = refused(path_lib::open_checked(&link, Others::MayRead));
    let fifo = sb.join("fifo.env");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if made.is_ok_and(|s| s.success()) {
        let _ = refused(path_lib::open_checked(&fifo, Others::MayRead));
    }
}

// --- every platform ----------------------------------------------------------

#[test]
fn test_ensure_accepts_what_it_created_on_every_call() {
    let sb = Sandbox::new("own-again");
    let app = App::portable("demo", sb.path()).unwrap();
    for _ in 0..2 {
        app.ensure_all().unwrap();
        for kind in Kind::ALL {
            let _ = app.ensure_private(kind).unwrap();
        }
        let _ = app.create(Kind::Cache, "a/b/c").unwrap();
        let _ = app.create_private(Kind::State, "keys").unwrap();
    }
}

#[test]
fn test_remove_refuses_a_path_holding_another_kind() {
    let sb = Sandbox::new("own-nested");
    let root = sb.join("app");
    let app = App::portable("demo", &root)
        .unwrap()
        .with_dir(Kind::Data, root.join("d"))
        .unwrap()
        .with_dir(Kind::Logs, root.join("d").join("logs"))
        .unwrap();
    let _ = app.ensure(Kind::Data).unwrap();
    let logs = app.ensure(Kind::Logs).unwrap();
    std::fs::write(logs.join("today.log"), b"x").unwrap();
    std::fs::write(app.data().join("main.db"), b"x").unwrap();
    assert!(refused(app.remove(Kind::Data, "")).contains("logs"));
    assert!(refused(app.remove(Kind::Data, "logs")).contains("logs"));
    assert!(logs.join("today.log").is_file());
    // clear keeps the nested kind and removes the rest.
    assert_eq!(app.clear(Kind::Data).unwrap(), 1);
    assert!(logs.join("today.log").is_file());
    assert!(!app.data().join("main.db").exists());
    // Removing the nested kind itself is fine; the data directory still
    // holds the place of the logs, so removing it whole stays refused.
    assert!(app.remove(Kind::Logs, "").unwrap());
    let _ = refused(app.remove(Kind::Data, ""));
    assert_eq!(app.clear(Kind::Data).unwrap(), 0);
    assert!(app.remove(Kind::Data, "missing").is_ok_and(|gone| !gone));
}

#[test]
fn test_default_layout_remove_never_takes_another_kind() {
    // In the real layouts some kinds nest (Windows: everything in data;
    // Linux: logs in state). Whatever the platform, no remove may take
    // another kind's directory with it.
    let app = App::new("PathLibNestProbe").unwrap();
    for kind in Kind::ALL {
        let nested = Kind::ALL
            .iter()
            .any(|k| *k != kind && app.dir(*k).starts_with(app.dir(kind)));
        let r = app.remove(kind, "");
        if nested {
            let _ = refused(r);
        } else {
            assert!(matches!(r, Ok(false)), "{kind}: {r:?}");
        }
    }
}
