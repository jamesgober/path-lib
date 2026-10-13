//! Integration tests for the filesystem operations, against the real
//! filesystem.

#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::fs;
use std::path::Path;

use common::{Sandbox, symlink_dir, symlink_file};
use path_lib::Error;

// --- make_dir / make_file / exists -------------------------------------

#[test]
fn test_make_dir_nested_creates_all_and_is_idempotent() {
    let sb = Sandbox::new("mkdir");
    let p = sb.join("a/b/c");
    path_lib::make_dir(&p).unwrap();
    path_lib::make_dir(&p).unwrap();
    assert!(p.is_dir());
}

#[test]
fn test_make_dir_over_file_returns_already_exists() {
    let sb = Sandbox::new("mkdir-file");
    let f = sb.write("f", "x");
    assert!(matches!(
        path_lib::make_dir(&f),
        Err(Error::AlreadyExists { .. })
    ));
}

#[test]
fn test_make_file_creates_parents_and_never_overwrites() {
    let sb = Sandbox::new("mkfile");
    let f = sb.join("x/y/z.txt");
    path_lib::make_file(&f).unwrap();
    assert_eq!(fs::read(&f).unwrap().len(), 0);
    fs::write(&f, "keep").unwrap();
    assert!(matches!(
        path_lib::make_file(&f),
        Err(Error::AlreadyExists { .. })
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), "keep");
}

#[test]
fn test_exists_reports_files_dirs_and_missing() {
    let sb = Sandbox::new("exists");
    let f = sb.write("f", "x");
    assert!(path_lib::exists(&f));
    assert!(path_lib::exists(sb.path()));
    assert!(!path_lib::exists(sb.join("nope")));
}

#[test]
fn test_exists_true_for_broken_symlink() {
    let sb = Sandbox::new("exists-link");
    let link = sb.join("dangling");
    if !symlink_file(&sb.join("missing-target"), &link) {
        return;
    }
    assert!(path_lib::exists(&link));
}

// --- list ---------------------------------------------------------------

#[test]
fn test_list_is_sorted_and_shallow() {
    let sb = Sandbox::new("list");
    let _ = sb.write("b.txt", "");
    let _ = sb.write("a.txt", "");
    let _ = sb.write("c/d.txt", "");
    let got = path_lib::list(sb.path()).unwrap();
    let names: Vec<_> = got
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a.txt", "b.txt", "c"]);
}

#[test]
fn test_list_missing_returns_not_found() {
    let sb = Sandbox::new("list-missing");
    assert!(matches!(
        path_lib::list(sb.join("nope")),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn test_list_on_file_returns_io_error() {
    let sb = Sandbox::new("list-file");
    let f = sb.write("f", "");
    assert!(matches!(path_lib::list(&f), Err(Error::Io { .. })));
}

// --- remove -------------------------------------------------------------

#[test]
fn test_remove_file_dir_and_missing() {
    let sb = Sandbox::new("remove");
    let f = sb.write("f.txt", "x");
    let _ = sb.write("d/e/f.txt", "x");
    assert!(path_lib::remove(&f).unwrap());
    assert!(path_lib::remove(sb.join("d")).unwrap());
    assert!(!path_lib::remove(sb.join("d")).unwrap());
    assert!(sb.names("").is_empty());
}

#[test]
fn test_remove_refuses_protected_paths() {
    let root = if cfg!(windows) { "C:\\" } else { "/" };
    for p in [root, if cfg!(windows) { "C:\\Windows" } else { "/etc" }] {
        assert!(
            matches!(path_lib::remove(p), Err(Error::Refused { .. })),
            "{p}"
        );
    }
    let home = path_lib::home().unwrap();
    assert!(matches!(path_lib::remove(home), Err(Error::Refused { .. })));
    assert!(matches!(
        path_lib::remove(path_lib::documents().unwrap()),
        Err(Error::Refused { .. })
    ));
    assert!(matches!(
        path_lib::remove(path_lib::temp()),
        Err(Error::Refused { .. })
    ));
}

#[test]
fn test_remove_refuses_home_written_differently() {
    let home = path_lib::home().unwrap();
    // Trailing separator and a `.` segment must not bypass the guard.
    let sneaky = format!(
        "{}{}.{}",
        home.display(),
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    assert!(matches!(
        path_lib::remove(&sneaky),
        Err(Error::Refused { .. })
    ));
    if cfg!(any(windows, target_os = "macos")) {
        let upper = home.to_string_lossy().to_uppercase();
        assert!(matches!(
            path_lib::remove(&upper),
            Err(Error::Refused { .. })
        ));
    }
}

#[test]
fn test_remove_symlink_removes_link_not_target() {
    let sb = Sandbox::new("remove-link");
    let target = sb.join("target");
    let _ = sb.write("target/keep.txt", "keep");
    let link = sb.join("link");
    if !symlink_dir(&target, &link) {
        return;
    }
    assert!(path_lib::remove(&link).unwrap());
    assert!(!path_lib::exists(&link));
    assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "keep");
}

#[test]
fn test_remove_dir_containing_symlink_leaves_target_intact() {
    let sb = Sandbox::new("remove-tree-link");
    let outside = sb.join("outside");
    let _ = sb.write("outside/keep.txt", "keep");
    let _ = sb.write("tree/file.txt", "x");
    if !symlink_dir(&outside, &sb.join("tree/escape")) {
        return;
    }
    assert!(path_lib::remove(sb.join("tree")).unwrap());
    assert_eq!(
        fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep"
    );
}

// --- copy ---------------------------------------------------------------

#[test]
fn test_copy_file_returns_byte_count() {
    let sb = Sandbox::new("copy-file");
    let src = sb.write("a.txt", "hello");
    let n = path_lib::copy(&src, sb.join("deep/b.txt")).unwrap();
    assert_eq!(n, 5);
    assert_eq!(fs::read_to_string(sb.join("deep/b.txt")).unwrap(), "hello");
    assert_eq!(fs::read_to_string(&src).unwrap(), "hello");
}

#[test]
fn test_copy_tree_copies_everything_and_leaves_no_staging() {
    let sb = Sandbox::new("copy-tree");
    let _ = sb.write("src/a.txt", "aa");
    let _ = sb.write("src/x/b.txt", "bbb");
    path_lib::make_dir(sb.join("src/empty")).unwrap();
    let n = path_lib::copy(sb.join("src"), sb.join("dst")).unwrap();
    assert_eq!(n, 5);
    assert_eq!(fs::read_to_string(sb.join("dst/x/b.txt")).unwrap(), "bbb");
    assert!(sb.join("dst/empty").is_dir());
    assert_eq!(sb.names(""), ["dst", "src"]);
}

#[test]
fn test_copy_never_overwrites() {
    let sb = Sandbox::new("copy-exists");
    let a = sb.write("a", "new");
    let b = sb.write("b", "old");
    assert!(matches!(
        path_lib::copy(&a, &b),
        Err(Error::AlreadyExists { .. })
    ));
    assert_eq!(fs::read_to_string(&b).unwrap(), "old");
}

#[test]
fn test_copy_missing_source_returns_not_found() {
    let sb = Sandbox::new("copy-missing");
    assert!(matches!(
        path_lib::copy(sb.join("nope"), sb.join("dst")),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn test_copy_into_itself_is_refused_without_side_effects() {
    let sb = Sandbox::new("copy-self");
    let _ = sb.write("src/a.txt", "a");
    let r = path_lib::copy(sb.join("src"), sb.join("src/inner/copy"));
    assert!(matches!(r, Err(Error::Refused { .. })));
    assert!(
        !sb.join("src/inner").exists(),
        "refused copy must not create parents"
    );
}

#[test]
fn test_copy_tree_with_symlink_is_refused_and_rolled_back() {
    let sb = Sandbox::new("copy-link");
    let _ = sb.write("src/a.txt", "a");
    let _ = sb.write("elsewhere/secret.txt", "s");
    if !symlink_dir(&sb.join("elsewhere"), &sb.join("src/link")) {
        return;
    }
    let r = path_lib::copy(sb.join("src"), sb.join("dst"));
    assert!(matches!(r, Err(Error::Refused { .. })));
    assert!(!sb.join("dst").exists());
    assert_eq!(
        sb.names(""),
        ["elsewhere", "src"],
        "staging dir must be cleaned up"
    );
}

#[test]
fn test_copy_destination_without_name_is_refused() {
    let sb = Sandbox::new("copy-noname");
    let a = sb.write("a", "x");
    assert!(path_lib::copy(&a, sb.join("..")).is_err());
}

// --- move_to ------------------------------------------------------------

#[test]
fn test_move_file_and_dir() {
    let sb = Sandbox::new("move");
    let f = sb.write("a.txt", "a");
    path_lib::move_to(&f, sb.join("b/a.txt")).unwrap();
    assert!(!f.exists());
    assert_eq!(fs::read_to_string(sb.join("b/a.txt")).unwrap(), "a");
    path_lib::move_to(sb.join("b"), sb.join("c")).unwrap();
    assert!(sb.join("c/a.txt").is_file());
    assert!(!sb.join("b").exists());
}

#[test]
fn test_move_never_overwrites() {
    let sb = Sandbox::new("move-exists");
    let a = sb.write("a", "new");
    let b = sb.write("b", "old");
    assert!(matches!(
        path_lib::move_to(&a, &b),
        Err(Error::AlreadyExists { .. })
    ));
    assert_eq!(fs::read_to_string(&b).unwrap(), "old");
    assert!(a.exists());
}

#[test]
fn test_move_into_itself_and_protected_refused() {
    let sb = Sandbox::new("move-self");
    let _ = sb.write("d/a", "a");
    assert!(matches!(
        path_lib::move_to(sb.join("d"), sb.join("d/sub")),
        Err(Error::Refused { .. })
    ));
    let home = path_lib::home().unwrap();
    assert!(matches!(
        path_lib::move_to(home, sb.join("stolen-home")),
        Err(Error::Refused { .. })
    ));
}

#[test]
fn test_move_missing_source_returns_not_found() {
    let sb = Sandbox::new("move-missing");
    assert!(matches!(
        path_lib::move_to(sb.join("nope"), sb.join("x")),
        Err(Error::NotFound { .. })
    ));
}

// --- find ---------------------------------------------------------------

#[test]
fn test_find_matches_files_and_dirs_recursively_sorted() {
    let sb = Sandbox::new("find");
    let _ = sb.write("logs/2026-10-01.log", "");
    let _ = sb.write("logs/old/2026-09-30.log", "");
    let _ = sb.write("notes.txt", "");
    path_lib::make_dir(sb.join("archive.log")).unwrap();
    let got = path_lib::find(sb.path(), "*.log").unwrap();
    let rel: Vec<_> = got
        .iter()
        .map(|p| p.strip_prefix(sb.path()).unwrap().to_path_buf())
        .collect();
    assert_eq!(
        rel,
        [
            Path::new("archive.log").to_path_buf(),
            Path::new("logs").join("2026-10-01.log"),
            Path::new("logs").join("old").join("2026-09-30.log"),
        ]
    );
}

#[test]
fn test_find_rejects_bad_patterns() {
    let sb = Sandbox::new("find-bad");
    for p in ["", "a/b", "a\\b", "x:y", "a\0"] {
        assert!(
            matches!(path_lib::find(sb.path(), p), Err(Error::InvalidPath { .. })),
            "{p:?}"
        );
    }
}

#[test]
fn test_find_missing_root_returns_not_found() {
    let sb = Sandbox::new("find-missing");
    assert!(matches!(
        path_lib::find(sb.join("nope"), "*"),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn test_find_does_not_follow_symlink_loops() {
    let sb = Sandbox::new("find-loop");
    let _ = sb.write("a/file.txt", "");
    if !symlink_dir(&sb.join("a"), &sb.join("a/loop")) {
        return;
    }
    let got = path_lib::find(sb.path(), "file.txt").unwrap();
    assert_eq!(got.len(), 1);
}

// --- concurrency --------------------------------------------------------

#[test]
fn test_make_dir_concurrently_from_many_threads() {
    let sb = Sandbox::new("concurrent");
    let target = sb.join("shared/deep/dir");
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let t = target.clone();
            std::thread::spawn(move || path_lib::make_dir(&t))
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    assert!(target.is_dir());
}

#[test]
fn test_copy_concurrently_to_distinct_targets() {
    let sb = Sandbox::new("concurrent-copy");
    let _ = sb.write("src/a.txt", "abc");
    let src = sb.join("src");
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let (s, d) = (src.clone(), sb.join(&format!("dst{i}")));
            std::thread::spawn(move || path_lib::copy(&s, &d))
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap().unwrap(), 3);
    }
    assert_eq!(sb.names("").len(), 9, "no staging leftovers");
}
