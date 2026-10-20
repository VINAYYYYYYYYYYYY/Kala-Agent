//! Shared integration-test helpers.
//!
//! Every helper here is test-only. Python is spawned only by the oracle and
//! cross-reader tests; the library itself never calls Python.

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};
use tempfile::TempDir;

pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/v1")
}

pub fn fixture_json(name: &str) -> serde_json::Value {
    let path = fixture_dir().join(name);
    let data = fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&data)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

pub fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    let python3 = Path::new("python3");
    if Command::new(python3)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return python3.to_path_buf();
    }
    panic!("no Python oracle available; set KALA_PYTHON to the repository virtual environment")
}

pub fn python_oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/python_oracle.py")
}

/// Run the persistence oracle script and return its stdout JSON.
pub fn run_oracle(args: &[&str], stdin: &[u8]) -> serde_json::Value {
    let output = run_oracle_raw(args, stdin);
    serde_json::from_slice(&output)
        .unwrap_or_else(|error| panic!("oracle JSON: {error}; output={output:?}"))
}

pub fn run_oracle_raw(args: &[&str], stdin: &[u8]) -> Vec<u8> {
    let mut child = Command::new(python_executable())
        .arg(python_oracle_script())
        .args(args)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Python oracle");
    child
        .stdin
        .as_mut()
        .expect("oracle stdin")
        .write_all(stdin)
        .expect("write oracle input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// A copied fixture database plus the directory that keeps it alive.
pub struct FixtureCopy {
    pub directory: TempDir,
    pub path: PathBuf,
}

pub fn copy_fixture_database() -> FixtureCopy {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("schema_v2.sqlite3");
    fs::copy(fixture_dir().join("schema_v2.sqlite3"), &path).expect("copy fixture database");
    FixtureCopy { directory, path }
}

/// A UTC timestamp fixed to whole seconds, matching the corpus clock.
///
/// Adding a duration rather than formatting the value into the seconds field
/// keeps callers that use 60 or more from accidentally creating a leap second.
pub fn fixed_time(seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .expect("fixed epoch")
        .with_timezone(&Utc)
        + chrono::TimeDelta::seconds(seconds)
}
