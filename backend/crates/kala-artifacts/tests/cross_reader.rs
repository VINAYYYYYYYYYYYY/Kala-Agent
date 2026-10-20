//! Live cross-reader tests against the Python content-addressed store.
//!
//! The Rust library never calls Python; these integration tests launch the
//! repository virtual environment as an oracle, exactly like the accepted
//! kala-persistence cross-reader tests.  Both directions are covered: Python
//! reads and verifies objects written by Rust, and Rust reads, verifies and
//! extends a store created by Python.

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use kala_artifacts::{ArtifactError, ContentAddressedArtifactStore};
use support::{hex_encode, object_path, run_python, sha256_hex};

fn payload_set() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"cross reader".to_vec(),
        "καλός — 漢字 🎯".as_bytes().to_vec(),
        (0..=255u8).collect(),
        (0..1024).map(|index| (index % 251) as u8).collect(),
    ]
}

fn args(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

#[test]
fn python_reads_and_verifies_rust_written_objects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let store = ContentAddressedArtifactStore::new(&root, 4096).unwrap();
    let mut expected = BTreeMap::new();
    let mut digests = Vec::new();
    for payload in payload_set() {
        let digest = store.put_bytes(&payload).unwrap();
        assert_eq!(digest, sha256_hex(&payload));
        digests.push(digest.clone());
        expected.insert(digest, payload);
    }

    let mut command = vec!["read".to_string(), root_str(&root), "4096".to_string()];
    command.extend(digests.iter().cloned());
    let outcome = run_python(&args(&command), "");
    assert!(
        outcome.status.success(),
        "python oracle failed: {}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    let lines: Vec<&str> = outcome.stdout.lines().collect();
    assert_eq!(lines.len(), digests.len());
    for (line, digest) in lines.iter().zip(digests.iter()) {
        let fields: Vec<&str> = line.split('|').collect();
        assert_eq!(fields.len(), 4, "unexpected oracle line: {line}");
        assert_eq!(fields[0], "ok");
        assert_eq!(fields[1], hex_encode(&expected[digest]));
        assert_eq!(fields[2], expected[digest].len().to_string());
        assert_eq!(fields[3], "True");
    }

    // The Python store must also accept the Rust-created layout and reject an
    // idempotent re-publication of an existing object.
    let mut create = vec!["create".to_string(), root_str(&root), "4096".to_string()];
    create.extend(expected.values().map(|payload| hex_encode(payload)));
    let outcome = run_python(&args(&create), "");
    assert!(
        outcome.status.success(),
        "python oracle failed: {}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    let returned: Vec<String> = outcome
        .stdout
        .lines()
        .map(|line| line.split('|').nth(1).unwrap().to_string())
        .collect();
    assert_eq!(returned, expected.keys().cloned().collect::<Vec<String>>());
}

#[test]
fn rust_reads_verifies_and_extends_a_python_created_store() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("python-artifacts");
    let payloads = payload_set();
    let mut create = vec!["create".to_string(), root_str(&root), "4096".to_string()];
    create.extend(payloads.iter().map(|payload| hex_encode(payload)));
    let outcome = run_python(&args(&create), "");
    assert!(
        outcome.status.success(),
        "python oracle failed: {}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    let mut expected = BTreeMap::new();
    let mut digests = Vec::new();
    for line in outcome.stdout.lines() {
        let fields: Vec<&str> = line.split('|').collect();
        assert_eq!(fields.len(), 4, "unexpected oracle line: {line}");
        assert_eq!(fields[0], "ok");
        let digest = fields[1].to_string();
        let size: usize = fields[2].parse().unwrap();
        let key = fields[3].to_string();
        assert_eq!(key, format!("objects/{}/{digest}", &digest[..2]));
        digests.push((digest, size));
    }
    assert_eq!(digests.len(), payloads.len());

    let store = ContentAddressedArtifactStore::new(&root, 4096).unwrap();
    for ((digest, size), payload) in digests.iter().zip(payloads.iter()) {
        assert_eq!(store.get_bytes(digest).unwrap(), *payload);
        assert_eq!(store.get_size(digest).unwrap(), *size as u64);
        assert!(store.contains(digest).unwrap());
        assert_eq!(
            store.key_for(digest).unwrap(),
            format!("objects/{}/{digest}", &digest[..2])
        );
        expected.insert(digest.clone(), payload.clone());
    }

    // Extend the Python store from Rust and have Python read the extension.
    let extension = b"rust extension".to_vec();
    let extension_digest = store.put_bytes(&extension).unwrap();
    expected.insert(extension_digest.clone(), extension);
    let read = vec![
        "read".to_string(),
        root_str(&root),
        "4096".to_string(),
        extension_digest.clone(),
    ];
    let outcome = run_python(&args(&read), "");
    assert!(
        outcome.status.success(),
        "python oracle failed: {}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    let fields: Vec<&str> = outcome.stdout.trim_end().split('|').collect();
    assert_eq!(
        fields.len(),
        4,
        "unexpected oracle line: {}",
        outcome.stdout
    );
    assert_eq!(fields[0], "ok");
    assert_eq!(fields[1], hex_encode(&expected[&extension_digest]));

    support::assert_exact_store_contents(&store, &expected);
}

#[test]
fn rust_detects_a_python_created_object_tampered_after_publication() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("tampered-artifacts");
    let payload = b"python tamper probe".to_vec();
    let create = vec![
        "create".to_string(),
        root_str(&root),
        "4096".to_string(),
        hex_encode(&payload),
    ];
    let outcome = run_python(&args(&create), "");
    assert!(outcome.status.success(), "{}", outcome.stderr);
    let digest = outcome
        .stdout
        .trim_end()
        .split('|')
        .nth(1)
        .unwrap()
        .to_string();
    let store = ContentAddressedArtifactStore::new(&root, 4096).unwrap();
    assert_eq!(store.get_bytes(&digest).unwrap(), payload);

    std::fs::write(object_path(&store, &digest), b"tampered by test").unwrap();
    match store.get_bytes(&digest) {
        Err(ArtifactError::DigestMismatch) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    match store.put_bytes(&payload) {
        Err(ArtifactError::DigestMismatch) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

fn root_str(root: &Path) -> String {
    root.to_str().expect("temporary path is UTF-8").to_string()
}
