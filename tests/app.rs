//! Integration tests for `App` against the real OS and filesystem.

#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::path::Path;

use common::{Sandbox, symlink_dir};
use path_lib::{App, Error, Kind, Scope};

fn assert_send_sync<T: Send + Sync + Clone>() {}

#[test]
fn test_app_is_send_sync_clone() {
    assert_send_sync::<App>();
    assert_send_sync::<Kind>();
    assert_send_sync::<Scope>();
}

#[test]
fn test_user_app_dirs_are_absolute_distinct_and_named() {
    let app = App::new("PathLibTest").unwrap();
    assert_eq!(app.scope(), Scope::User);
    let home = path_lib::home().unwrap();
    let dirs: Vec<&Path> = Kind::ALL.iter().map(|k| app.dir(*k)).collect();
    for (i, d) in dirs.iter().enumerate() {
        assert!(d.is_absolute(), "{d:?}");
        assert!(
            d.components().any(|c| c.as_os_str() == "PathLibTest"),
            "{d:?} must contain the app name"
        );
        for other in &dirs[i + 1..] {
            assert_ne!(d, other);
        }
    }
    // User-scope data, config, cache, and documents live under home on
    // every platform (temp and runtime may not).
    for k in [Kind::Data, Kind::Config, Kind::Cache, Kind::Documents] {
        assert!(app.dir(k).starts_with(home), "{k}: {:?}", app.dir(k));
    }
}

#[test]
fn test_global_app_dirs_are_absolute_distinct_and_outside_home() {
    let app = App::global("PathLibTest").unwrap();
    assert_eq!(app.scope(), Scope::Global);
    let home = path_lib::home().unwrap();
    for k in Kind::ALL {
        let d = app.dir(k);
        assert!(d.is_absolute());
        if k != Kind::Documents {
            assert!(!d.starts_with(home), "{k}: {d:?}");
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn test_global_linux_paths_follow_fhs() {
    let app = App::global("hivedb").unwrap();
    assert_eq!(app.data(), Path::new("/var/lib/hivedb"));
    assert_eq!(app.config(), Path::new("/etc/hivedb"));
    assert_eq!(app.logs(), Path::new("/var/log/hivedb"));
    assert_eq!(app.runtime(), Path::new("/run/hivedb"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_paths_follow_library_domains() {
    let user = App::new("HiveDB").unwrap();
    let home = path_lib::home().unwrap();
    assert_eq!(user.data(), home.join("Library/Application Support/HiveDB"));
    let global = App::global("HiveDB").unwrap();
    assert_eq!(
        global.data(),
        Path::new("/Library/Application Support/HiveDB")
    );
}

#[cfg(windows)]
#[test]
fn test_windows_paths_use_known_folders() {
    let app = App::new("HiveDB").unwrap();
    let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
    let roaming = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap());
    assert_eq!(app.data(), path_lib::clean(local.join("HiveDB")));
    assert_eq!(app.config(), path_lib::clean(roaming.join("HiveDB")));
    let global = App::global("HiveDB").unwrap();
    let program_data = std::path::PathBuf::from(std::env::var_os("ProgramData").unwrap());
    assert_eq!(global.data(), path_lib::clean(program_data.join("HiveDB")));
}

#[test]
fn test_invalid_names_rejected_by_every_constructor() {
    for bad in ["", "../x", "a/b", "CON", "x:", "~me", "trailing."] {
        assert!(
            matches!(App::new(bad), Err(Error::InvalidName { .. })),
            "{bad:?}"
        );
        assert!(
            matches!(App::global(bad), Err(Error::InvalidName { .. })),
            "{bad:?}"
        );
        assert!(
            matches!(App::portable(bad, "/tmp"), Err(Error::InvalidName { .. })),
            "{bad:?}"
        );
    }
}

#[test]
fn test_at_accessors_are_confined_and_platform_separated() {
    let app = App::new("PathLibTest").unwrap();
    for kind in Kind::ALL {
        let p = app.at(kind, "/big\\sloppy//floppy/nipple-slaps/").unwrap();
        assert!(p.starts_with(app.dir(kind)));
        assert_eq!(
            p,
            app.dir(kind)
                .join("big")
                .join("sloppy")
                .join("floppy")
                .join("nipple-slaps")
        );
        assert!(app.at(kind, "../escape").is_err());
    }
    assert_eq!(app.data_at("x").unwrap(), app.at(Kind::Data, "x").unwrap());
    assert_eq!(
        app.config_at("x").unwrap(),
        app.at(Kind::Config, "x").unwrap()
    );
    assert_eq!(
        app.cache_at("x").unwrap(),
        app.at(Kind::Cache, "x").unwrap()
    );
    assert_eq!(app.logs_at("x").unwrap(), app.at(Kind::Logs, "x").unwrap());
    assert_eq!(
        app.state_at("x").unwrap(),
        app.at(Kind::State, "x").unwrap()
    );
    assert_eq!(
        app.runtime_at("x").unwrap(),
        app.at(Kind::Runtime, "x").unwrap()
    );
    assert_eq!(app.temp_at("x").unwrap(), app.at(Kind::Temp, "x").unwrap());
    assert_eq!(
        app.documents_at("x").unwrap(),
        app.at(Kind::Documents, "x").unwrap()
    );
}

#[test]
fn test_portable_ensure_all_creates_every_kind() {
    let sb = Sandbox::new("app-ensure");
    let app = App::portable("Port", sb.path()).unwrap();
    app.ensure_all().unwrap();
    app.ensure_all().unwrap();
    for k in Kind::ALL {
        assert!(app.dir(k).is_dir(), "{k}");
    }
}

#[cfg(unix)]
#[test]
fn test_portable_ensure_creates_private_dirs() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::new("app-mode");
    let app = App::portable("Port", sb.path()).unwrap();
    let dir = app.ensure(Kind::Data).unwrap();
    let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode & 0o077,
        0,
        "user directories must not be group/world accessible: {mode:o}"
    );
}

#[test]
fn test_ensure_refuses_planted_symlink() {
    let sb = Sandbox::new("app-symlink");
    let app = App::portable("Port", sb.path()).unwrap();
    let elsewhere = sb.join("attacker");
    std::fs::create_dir_all(&elsewhere).unwrap();
    if !symlink_dir(&elsewhere, app.cache()) {
        return;
    }
    assert!(matches!(
        app.ensure(Kind::Cache),
        Err(Error::Refused { .. })
    ));
}

#[test]
fn test_create_list_find_remove_round_trip() {
    let sb = Sandbox::new("app-crud");
    let app = App::portable("Port", sb.path()).unwrap();
    let db = app.create(Kind::Data, "db/main").unwrap();
    assert!(db.is_dir());
    path_lib::make_file(db.join("a.wal")).unwrap();
    path_lib::make_file(db.join("b.wal")).unwrap();

    assert_eq!(
        app.list(Kind::Data, "db").unwrap(),
        std::slice::from_ref(&db)
    );
    assert_eq!(app.find(Kind::Data, "*.wal").unwrap().len(), 2);
    assert!(app.remove(Kind::Data, "db").unwrap());
    assert!(!db.exists());
    assert!(app.create(Kind::Data, "../../escape").is_err());
    assert!(app.remove(Kind::Data, "..").is_err());
}

#[test]
fn test_remove_empty_sub_clears_kind_directory() {
    let sb = Sandbox::new("app-clear");
    let app = App::portable("Port", sb.path()).unwrap();
    let _ = app.create(Kind::Cache, "a/b").unwrap();
    assert!(app.remove(Kind::Cache, "").unwrap());
    assert!(!app.cache().exists());
    assert!(app.data().parent().unwrap().exists(), "siblings untouched");
}

#[test]
fn test_portable_relative_root_is_pinned_to_cwd_at_construction() {
    let app = App::portable("Port", "relative-root").unwrap();
    assert!(app.data().is_absolute());
    assert!(app.data().starts_with(std::env::current_dir().unwrap()));
}

#[test]
fn test_display_lists_all_dirs() {
    let app = App::new("PathLibTest").unwrap();
    let text = app.to_string();
    assert_eq!(text.lines().count(), 1 + Kind::ALL.len());
    assert!(text.contains(&app.data().display().to_string()));
}

#[test]
fn test_app_construction_is_consistent_across_threads() {
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| App::new("PathLibTest").unwrap()))
        .collect();
    let apps: Vec<App> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(apps.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn test_with_dir_and_with_env_compose_with_ensure() {
    let sb = Sandbox::new("app-override");
    let app = App::portable("Port", sb.path())
        .unwrap()
        .with_dir(Kind::Logs, sb.join("elsewhere/logs"))
        .unwrap()
        .with_env("PATH_LIB_TEST_UNSET_PREFIX")
        .unwrap();
    let logs = app.ensure(Kind::Logs).unwrap();
    assert!(logs.is_dir());
    assert!(logs.ends_with(Path::new("elsewhere").join("logs")));
}

#[test]
fn test_with_dir_rejects_relative_and_duplicate() {
    let app = App::new("PathLibTest").unwrap();
    let config = app.config().to_path_buf();
    assert!(app.clone().with_dir(Kind::Data, "relative").is_err());
    assert!(app.with_dir(Kind::Data, config).is_err());
}

#[test]
fn test_auto_without_marker_next_to_test_binary_is_user_scope() {
    let marker = path_lib::exe_dir().unwrap().join(path_lib::PORTABLE_MARKER);
    if marker.exists() {
        return;
    }
    assert_eq!(App::auto("PathLibTest").unwrap().scope(), Scope::User);
}

#[test]
fn test_dirs_matches_dir_for_every_kind() {
    let app = App::new("PathLibTest").unwrap();
    assert_eq!(app.dirs().count(), Kind::ALL.len());
    for (kind, dir) in app.dirs() {
        assert_eq!(app.dir(kind), dir);
    }
}

#[test]
fn test_expand_home_end_to_end() {
    let home = path_lib::home().unwrap();
    assert_eq!(path_lib::expand_home("~").unwrap(), home);
    assert_eq!(
        path_lib::expand_home("~/x/y").unwrap(),
        home.join("x").join("y")
    );
}
