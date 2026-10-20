//! Shared helpers for the integration tests.
//!
//! The tests write STEP files and oracle reports into a private temporary
//! directory. The guard removes it on success *and* on failure (including a
//! panic), so a failing differential never leaves scratch files behind.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A self-cleaning temporary directory under the platform temp directory.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a unique directory named `<prefix>-<pid>-<counter>`.
    pub fn new(prefix: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the test temporary directory can be created");
        Self { path }
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Join a file name inside the directory.
    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
