#![cfg(unix)]

//! Adversarial filesystem tests for the native store.
//!
//! Every attack is performed on real directories and real symlinks through the
//! public API.  The store must refuse to traverse a symlink, must never write
//! through one, must not hang on non-regular files, and must detect a bucket
//! that was renamed or replaced while an operation was in flight.

mod support;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kala_artifacts::{ArtifactError, ContentAddressedArtifactStore};
use support::{bucket_root, object_path, objects_root, sha256_hex, store, temporary_files};

fn expect_error_matching(result: Result<(), ArtifactError>, needle: &str) {
    match result {
        Ok(()) => panic!("operation unexpectedly succeeded"),
        Err(error) => {
            let message = error.to_string();
            assert!(
                message.contains(needle),
                "error {message:?} does not contain {needle:?}"
            );
        }
    }
}

#[test]
fn a_symlinked_bucket_never_redirects_reads_or_writes() {
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"attacked payload";
    let digest = sha256_hex(payload);

    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(&bucket).unwrap();
    std::fs::remove_dir(&bucket).unwrap();
    std::os::unix::fs::symlink(&outside, &bucket).unwrap();

    expect_error_matching(store.put_bytes(payload).map(|_| ()), "symlink");
    expect_error_matching(store.get_bytes(&digest).map(|_| ()), "symlink");
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
fn a_bucket_that_is_a_regular_file_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let digest = sha256_hex(b"file bucket");
    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(objects_root(&store)).unwrap();
    std::fs::write(&bucket, b"not a directory").unwrap();
    expect_error_matching(store.put_bytes(b"file bucket").map(|_| ()), "directory");
    expect_error_matching(store.get_bytes(&digest).map(|_| ()), "must be a directory");
}

#[test]
fn a_symlinked_object_is_never_read_as_an_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"safe artifact";
    let digest = store.put_bytes(payload).unwrap();
    let path = object_path(&store, &digest);
    std::fs::remove_file(&path).unwrap();
    let outside = directory.path().join("outside.bin");
    std::fs::write(&outside, payload).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();

    expect_error_matching(store.get_bytes(&digest).map(|_| ()), "symlink");
    expect_error_matching(store.put_bytes(payload).map(|_| ()), "symlink");
    assert_eq!(std::fs::read(&outside).unwrap(), payload);
}

#[test]
fn an_object_that_is_a_directory_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let digest = sha256_hex(b"directory object");
    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(bucket.join(&digest)).unwrap();
    expect_error_matching(store.get_bytes(&digest).map(|_| ()), "regular file");
}

#[test]
fn a_fifo_object_is_refused_without_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let digest = sha256_hex(b"fifo object");
    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(&bucket).unwrap();
    let fifo = bucket.join(&digest);
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo failed");

    // `O_NONBLOCK` means this call fails fast instead of waiting for a writer.
    expect_error_matching(store.get_bytes(&digest).map(|_| ()), "regular file");
    expect_error_matching(store.contains(&digest).map(|_| ()), "regular file");
    assert!(temporary_files(&objects_root(&store)).is_empty());
}

#[test]
fn the_objects_directory_identity_is_checked_on_every_operation() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"before replacement";
    let digest = store.put_bytes(payload).unwrap();

    // Replace the objects directory with a real, empty directory.
    let objects = objects_root(&store);
    let relocated = store.root().join("objects-relocated");
    std::fs::rename(&objects, &relocated).unwrap();
    std::fs::create_dir(&objects).unwrap();
    match store.get_bytes(&digest) {
        Err(ArtifactError::ObjectRootReplaced) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    match store.put_bytes(b"after replacement") {
        Err(ArtifactError::ObjectRootReplaced) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(std::fs::read_dir(&objects).unwrap().next().is_none());

    // A symlink replacement is refused as a symlink instead.
    std::fs::remove_dir(&objects).unwrap();
    std::os::unix::fs::symlink(&relocated, &objects).unwrap();
    expect_error_matching(store.get_bytes(&digest).map(|_| ()), "symlink");
    expect_error_matching(store.put_bytes(b"x").map(|_| ()), "symlink");
}

#[test]
fn the_root_handle_keeps_working_when_the_path_is_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"root swap";
    let digest = store.put_bytes(payload).unwrap();

    let outside = directory.path().join("outside-root");
    std::fs::create_dir(&outside).unwrap();
    let relocated = directory.path().join("artifacts-relocated");
    std::fs::rename(store.root(), &relocated).unwrap();
    std::os::unix::fs::symlink(&outside, store.root()).unwrap();

    // The held root handle still addresses the original directory; nothing is
    // written through the symlink.
    assert_eq!(store.get_bytes(&digest).unwrap(), payload);
    let second = store.put_bytes(b"second").unwrap();
    assert_eq!(store.get_bytes(&second).unwrap(), b"second");
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    assert_eq!(
        std::fs::read(relocated.join("objects").join(&digest[..2]).join(&digest)).unwrap(),
        payload
    );
}

#[test]
fn a_same_digest_conflict_is_never_overwritten_even_under_concurrency() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(store(directory.path(), 4096));
    let payload = Arc::new(vec![7u8; 512]);
    let digest = sha256_hex(&payload);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let payload = Arc::clone(&payload);
        handles.push(std::thread::spawn(move || {
            for _ in 0..32 {
                assert_eq!(store.put_bytes(&payload).unwrap(), sha256_hex(&payload));
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let mut expected = BTreeMap::new();
    expected.insert(digest, payload.as_ref().clone());
    support::assert_exact_store_contents(&store, &expected);
}

#[test]
fn a_bucket_swapped_while_writes_are_in_flight_never_escapes() {
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside-buckets");
    std::fs::create_dir(&outside).unwrap();
    let store = Arc::new(store(directory.path(), 1_000_000));
    let bucket = "cd";
    let payloads: Vec<Vec<u8>> = {
        let mut payloads = Vec::new();
        let mut index = 0usize;
        while payloads.len() < 32 {
            let payload = format!("bucket-swap-probe-{index}").into_bytes();
            index += 1;
            if sha256_hex(&payload).starts_with(bucket) {
                payloads.push(payload);
            }
        }
        payloads
    };
    let stop = Arc::new(AtomicBool::new(false));
    let refusals = Arc::new(Mutex::new(0usize));
    let published = Arc::new(Mutex::new(Vec::<String>::new()));

    // Deterministic evidence: with the bucket planted as a symlink before the
    // writers start, a publication is refused and nothing is written outside.
    let planted_bucket = store.root().join("objects").join(bucket);
    std::os::unix::fs::symlink(&outside, &planted_bucket).unwrap();
    assert!(
        store.put_bytes(&payloads[0]).is_err(),
        "publication through a planted symlink must fail"
    );
    *refusals.lock().unwrap() += 1;

    let attacker_root = store.root().to_path_buf();
    let attacker_stop = Arc::clone(&stop);
    let attacker = {
        let outside = outside.clone();
        std::thread::spawn(move || {
            let bucket = attacker_root.join("objects").join(bucket);
            let relocated = attacker_root.join("objects").join("bucket-relocated");
            while !attacker_stop.load(Ordering::SeqCst) {
                if bucket.is_symlink() {
                    let _ = std::fs::remove_file(&bucket);
                } else if bucket.exists() {
                    let _ = std::fs::rename(&bucket, &relocated);
                }
                let _ = std::os::unix::fs::symlink(&outside, &bucket);
                std::thread::yield_now();
            }
        })
    };

    let mut writers = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let payloads = payloads.clone();
        let refusals = Arc::clone(&refusals);
        let published = Arc::clone(&published);
        let stop = Arc::clone(&stop);
        writers.push(std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while !stop.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                for payload in &payloads {
                    let digest = sha256_hex(payload);
                    match store.put_bytes(payload) {
                        Ok(stored) => {
                            assert_eq!(stored, digest);
                            published.lock().unwrap().push(digest);
                        }
                        Err(_) => *refusals.lock().unwrap() += 1,
                    }
                    std::thread::yield_now();
                }
            }
        }));
    }
    for writer in writers {
        writer.join().unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    attacker.join().unwrap();

    assert!(
        *refusals.lock().unwrap() > 0,
        "the attacker must have been observed"
    );
    assert!(
        std::fs::read_dir(&outside).unwrap().next().is_none(),
        "no bytes may land outside the store"
    );
    for digest in published.lock().unwrap().iter() {
        if let Ok(bytes) = std::fs::read(object_path(&store, digest)) {
            assert_eq!(sha256_hex(&bytes), *digest);
        }
    }

    // Clean the attack up so the store can be inspected logically.
    let objects_bucket = store.root().join("objects").join(bucket);
    if objects_bucket.is_symlink() {
        std::fs::remove_file(&objects_bucket).unwrap();
    }
}

#[test]
fn an_object_swapped_for_a_symlink_is_never_read_from_outside() {
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside-object.bin");
    std::fs::write(&outside, b"attacker controlled").unwrap();
    let store = Arc::new(store(directory.path(), 1_000_000));
    let payload = Arc::new(b"swapped artifact".to_vec());
    let digest = store.put_bytes(&payload).unwrap();
    let target = object_path(&store, &digest);
    let stop = Arc::new(AtomicBool::new(false));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));

    let swapper_stop = Arc::clone(&stop);
    let swapper_target = target.clone();
    let swapper_outside = outside.clone();
    let swapper_store = Arc::clone(&store);
    let swapper_payload = Arc::clone(&payload);
    let swapper = std::thread::spawn(move || {
        while !swapper_stop.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&swapper_target);
            let _ = std::os::unix::fs::symlink(&swapper_outside, &swapper_target);
            let _ = swapper_store.put_bytes(&swapper_payload);
            std::thread::yield_now();
        }
    });

    let mut readers = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let digest = digest.clone();
        let failures = Arc::clone(&failures);
        readers.push(std::thread::spawn(move || {
            let payload = b"swapped artifact".to_vec();
            for _ in 0..500 {
                // Any refusal is safe: the object was swapped, removed or
                // replaced while the read was in flight.
                if let Ok(bytes) = store.get_bytes(&digest) {
                    if bytes != payload {
                        failures
                            .lock()
                            .unwrap()
                            .push("read bytes from a replaced object".to_string());
                    }
                }
            }
        }));
    }
    for reader in readers {
        reader.join().unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    swapper.join().unwrap();

    assert!(failures.lock().unwrap().is_empty());
    if target.is_symlink() {
        std::fs::remove_file(&target).unwrap();
    }
    let restored = store.put_bytes(&payload).unwrap();
    assert_eq!(store.get_bytes(&restored).unwrap(), *payload);
}

#[test]
fn no_silent_chmod_of_an_existing_root_is_performed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    std::fs::create_dir(&root).unwrap();
    let mut permissions = std::fs::metadata(&root).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&root, permissions).unwrap();

    let store = ContentAddressedArtifactStore::new(&root, 16).unwrap();
    let mode = std::fs::metadata(store.root())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o755,
        "the store must not silently tighten an existing root"
    );
    let digest = store.put_bytes(b"existing mode").unwrap();
    assert!(store.contains(&digest).unwrap());
}

#[test]
fn writing_to_a_read_only_bucket_reports_an_error() {
    let directory = tempfile::tempdir().unwrap();
    if unsafe { libc_geteuid() } == 0 {
        // root bypasses permission bits; the container runs as a normal user.
        return;
    }
    let store = store(directory.path(), 1024);
    let payload = b"read only bucket";
    let digest = sha256_hex(payload);
    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(&bucket).unwrap();
    let mut permissions = std::fs::metadata(&bucket).unwrap().permissions();
    permissions.set_mode(0o500);
    std::fs::set_permissions(&bucket, permissions).unwrap();
    let result = store.put_bytes(payload);
    let mut permissions = std::fs::metadata(&bucket).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&bucket, permissions).unwrap();
    assert!(result.is_err());
    assert!(!store.contains(&digest).unwrap());
}

unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}
