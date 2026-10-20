//! Thread and process concurrency, with byte-level evidence.
//!
//! Nothing in the store coordinates writers; the tests hold the same line as
//! the Python suite: concurrent identical/distinct writers converge on exactly
//! one verified object per digest, readers never observe a partial object, and
//! a second process can complete writes a first process started.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use kala_artifacts::ContentAddressedArtifactStore;
use support::{assert_exact_store_contents, sha256_hex, store};

const THREAD_WORKERS: usize = 8;
const ROUNDS: usize = 40;
const SHARED_PAYLOADS: usize = 4;
const DISTINCT_PAYLOADS: usize = 32;
const PROCESS_WORKERS: usize = 4;

fn shared_payload(index: usize) -> Vec<u8> {
    format!("shared-artifact-{}", index % SHARED_PAYLOADS).into_bytes()
}

fn distinct_payload(index: usize) -> Vec<u8> {
    format!("distinct-artifact-{}", index % DISTINCT_PAYLOADS).into_bytes()
}

fn worker_payload(worker: usize, round: usize) -> Vec<u8> {
    format!("worker-{worker}-{round}").into_bytes()
}

fn expected_payloads(
    workers: usize,
    rounds: usize,
    include_worker: bool,
) -> BTreeMap<String, Vec<u8>> {
    let mut expected = BTreeMap::new();
    for worker in 0..workers {
        for round in 0..rounds {
            for payload in [shared_payload(round), distinct_payload(round)] {
                expected.insert(sha256_hex(&payload), payload);
            }
            if include_worker {
                let payload = worker_payload(worker, round);
                expected.insert(sha256_hex(&payload), payload);
            }
        }
    }
    expected
}

#[test]
fn concurrent_threads_publish_every_object_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(store(directory.path(), 1_000_000));
    let barrier = Arc::new(Barrier::new(THREAD_WORKERS));
    let mut handles = Vec::new();
    let mut reported: Vec<String> = Vec::new();
    for worker in 0..THREAD_WORKERS {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let mut collected = Vec::new();
            barrier.wait();
            for round in 0..ROUNDS {
                collected.push(store.put_bytes(&shared_payload(round)).unwrap());
                collected.push(store.put_bytes(&distinct_payload(round)).unwrap());
                collected.push(
                    store
                        .put_text(&String::from_utf8(worker_payload(worker, round)).unwrap())
                        .unwrap(),
                );
            }
            collected
        }));
    }
    for handle in handles {
        reported.extend(handle.join().unwrap());
    }
    let expected = expected_payloads(THREAD_WORKERS, ROUNDS, true);
    assert_eq!(reported.len(), THREAD_WORKERS * ROUNDS * 3);
    assert_eq!(
        reported
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        expected
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    );
    assert_exact_store_contents(&store, &expected);
}

fn process_worker_from_environment() -> Option<()> {
    let root = std::env::var_os("KALA_ARTIFACT_PROCESS_ROOT")?;
    let worker: usize = std::env::var("KALA_ARTIFACT_PROCESS_WORKER")
        .ok()?
        .parse()
        .ok()?;
    let store = ContentAddressedArtifactStore::new(PathBuf::from(root), 1_000_000).unwrap();
    for round in 0..ROUNDS {
        store.put_bytes(&shared_payload(round)).unwrap();
        store.put_bytes(&distinct_payload(round)).unwrap();
        store
            .put_text(&String::from_utf8(worker_payload(worker, round)).unwrap())
            .unwrap();
    }
    Some(())
}

#[test]
fn process_worker() {
    // Runs as a helper when the parent sets the environment.  A normal test run
    // has no such variable and this is a no-op passing test.
    let _ = process_worker_from_environment();
}

#[test]
fn concurrent_processes_publish_every_object_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    ContentAddressedArtifactStore::new(&root, 1_000_000).unwrap();
    let executable = std::env::current_exe().expect("test executable");
    let mut children = Vec::new();
    for worker in 0..PROCESS_WORKERS {
        let child = Command::new(&executable)
            .args(["--exact", "process_worker", "--nocapture"])
            .env("KALA_ARTIFACT_PROCESS_ROOT", &root)
            .env("KALA_ARTIFACT_PROCESS_WORKER", worker.to_string())
            .spawn()
            .expect("worker process starts");
        children.push(child);
    }
    for mut child in children {
        let status = child.wait().expect("worker process finishes");
        assert!(status.success(), "worker process failed: {status}");
    }
    let store = ContentAddressedArtifactStore::new(&root, 1_000_000).unwrap();
    let expected = expected_payloads(PROCESS_WORKERS, ROUNDS, true);
    assert_exact_store_contents(&store, &expected);
}

#[test]
fn readers_never_observe_a_partially_published_object() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(store(directory.path(), 1_000_000));
    let payloads: Vec<Vec<u8>> = (0..16)
        .map(|index| format!("partial-publication-{index}").into_bytes())
        .collect();
    let digests: Vec<String> = payloads.iter().map(|payload| sha256_hex(payload)).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let stop = Arc::clone(&stop);
        let payloads = payloads.clone();
        let digests = digests.clone();
        readers.push(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                for (payload, digest) in payloads.iter().zip(digests.iter()) {
                    if let Ok(bytes) = store.get_bytes(digest) {
                        assert_eq!(bytes, *payload);
                    }
                }
            }
        }));
    }
    for payload in &payloads {
        store.put_bytes(payload).unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    for reader in readers {
        reader.join().unwrap();
    }
    let expected: BTreeMap<String, Vec<u8>> = digests
        .iter()
        .cloned()
        .zip(payloads.iter().cloned())
        .collect();
    assert_exact_store_contents(&store, &expected);
}
