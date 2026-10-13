//! Shared test scaffolding.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A unique scratch directory under the system temp dir, deleted on drop.
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join("path-lib-tests")
            .join(format!("{label}-{}-{n}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn join(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let p = self.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, contents).unwrap();
        p
    }

    /// Names in `dir` (non-recursive), sorted.
    pub fn names(&self, rel: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.join(rel))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Test cleanup: a leftover directory under temp is harmless and
        // must not mask the real test result.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Creates a directory symlink. Returns `false` where the platform or
/// account does not allow it (Windows without Developer Mode), so tests
/// that need links can skip instead of failing spuriously.
pub fn symlink_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }
}

/// Creates a file symlink, with the same skip rule as [`symlink_dir`].
pub fn symlink_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}
