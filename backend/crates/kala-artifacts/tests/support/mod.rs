//! Shared integration-test helpers.
//!
//! All tests use temporary directories only.  The Python helper launches the
//! repository virtual environment as a live oracle; Python is never used by
//! the library itself.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_artifacts::ContentAddressedArtifactStore;

pub fn store(directory: &Path, max_object_size: u64) -> ContentAddressedArtifactStore {
    ContentAddressedArtifactStore::new(directory.join("artifacts"), max_object_size)
        .expect("store constructs")
}

pub fn objects_root(store: &ContentAddressedArtifactStore) -> PathBuf {
    store.root().join("objects")
}

pub fn bucket_root(store: &ContentAddressedArtifactStore, digest: &str) -> PathBuf {
    objects_root(store).join(&digest[..2])
}

pub fn object_path(store: &ContentAddressedArtifactStore, digest: &str) -> PathBuf {
    bucket_root(store, digest).join(digest)
}

pub fn temporary_files(root: &Path) -> Vec<PathBuf> {
    walk(root)
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".artifact-") && name.ends_with(".tmp"))
        })
        .collect()
}

pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&directory) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            let metadata = std::fs::symlink_metadata(&path);
            if metadata.as_ref().is_ok_and(|meta| meta.is_dir()) {
                stack.push(path.clone());
            }
            entries.push(path);
        }
    }
    entries
}

/// Assert no symlink, temporary file or unexpected bucket remains under
/// `<root>/objects`, and every stored file hashes to its own name.
pub fn assert_exact_store_contents(
    store: &ContentAddressedArtifactStore,
    expected: &std::collections::BTreeMap<String, Vec<u8>>,
) {
    let objects = objects_root(store);
    assert!(objects.is_dir(), "objects root must exist");
    for entry in walk(&objects) {
        assert!(
            !entry
                .symlink_metadata()
                .expect("entry metadata")
                .file_type()
                .is_symlink(),
            "no symlink may remain: {}",
            entry.display()
        );
    }
    assert!(
        temporary_files(&objects).is_empty(),
        "temporary files must be cleaned up"
    );

    let mut buckets: Vec<String> = std::fs::read_dir(&objects)
        .expect("objects readable")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    buckets.sort();
    let mut expected_buckets: Vec<String> = expected
        .keys()
        .map(|digest| digest[..2].to_string())
        .collect();
    expected_buckets.sort();
    expected_buckets.dedup();
    assert_eq!(buckets, expected_buckets);

    for (digest, payload) in expected {
        let path = object_path(store, digest);
        assert_eq!(std::fs::read(&path).expect("stored object"), *payload);
        assert_eq!(sha256_hex(payload), *digest);
        assert_eq!(store.get_bytes(digest).expect("digest reads"), *payload);
        assert_eq!(
            store.get_size(digest).expect("size reads"),
            payload.len() as u64
        );
        assert!(store.contains(digest).expect("contains"));
        assert_eq!(
            store.key_for(digest).expect("key"),
            format!("objects/{}/{digest}", &digest[..2])
        );
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn hex_decode(text: &str) -> Vec<u8> {
    assert!(text.len() % 2 == 0, "hex payload must have even length");
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(text.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let high = hex_digit(chunk[0]);
        let low = hex_digit(chunk[1]);
        output.push((high << 4) | low);
    }
    output
}

fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        other => panic!("invalid hex digit {other}"),
    }
}

pub struct PythonOutcome {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

pub fn python_executable() -> PathBuf {
    if let Some(configured) = std::env::var_os("KALA_PYTHON") {
        return PathBuf::from(configured);
    }
    repository_root().join(".venv/bin/python")
}

/// Run `tests/python_oracle.py` with the given arguments and stdin bytes.
pub fn run_python(args: &[&str], stdin: &str) -> PythonOutcome {
    let root = repository_root();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/python_oracle.py");
    let python = python_executable();
    assert!(
        python.exists(),
        "the Python oracle interpreter is required at {}",
        python.display()
    );
    let mut command = Command::new(&python);
    command
        .arg(&script)
        .args(args)
        .current_dir(&root)
        .env("PYTHONPATH", root.join("src"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("Python oracle starts");
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(stdin.as_bytes())
        .expect("request written");
    let output = child.wait_with_output().expect("Python oracle finishes");
    PythonOutcome {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}
