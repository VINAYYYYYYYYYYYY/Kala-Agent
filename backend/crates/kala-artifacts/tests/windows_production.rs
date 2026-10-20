#![cfg(windows)]

//! Windows production-path tests.
//!
//! These tests use the public store API and therefore exercise the real
//! `CreateFileW`/`NtCreateFile`/`SetFileInformationByHandle` implementation on
//! a Windows host.  They cover ordinary put/get, idempotent publication,
//! concurrent identical/distinct writers, a process cwd unrelated to the
//! store (including the directory-relative rename target), and bucket
//! rename/swap behavior.  They need no symlink privilege.  Genuine execution is
//! pending because this development host is Linux; the Windows all-target
//! check/clippy builds are the evidence available for this round.

mod support;

use std::process::Command;
use std::sync::Arc;

use kala_artifacts::{ArtifactError, ContentAddressedArtifactStore};
use support::{bucket_root, object_path, objects_root, sha256_hex, store, temporary_files};

#[test]
fn ordinary_put_get_and_idempotent_put_use_production_paths() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 4096);
    let payload = b"windows production round trip";
    let digest = store.put_bytes(payload).unwrap();

    assert_eq!(digest, sha256_hex(payload));
    assert_eq!(store.get_bytes(&digest).unwrap(), payload);
    assert_eq!(store.get_size(&digest).unwrap(), payload.len() as u64);
    assert!(store.contains(&digest).unwrap());
    assert_eq!(
        std::fs::read(object_path(&store, &digest)).unwrap(),
        payload
    );

    // The second put takes the successful existing-bucket `FILE_OPEN_IF`
    // branch and must publish idempotently without a temporary file.
    assert_eq!(store.put_bytes(payload).unwrap(), digest);
    assert!(temporary_files(&objects_root(&store)).is_empty());
}

#[test]
fn concurrent_identical_and_distinct_puts_and_gets() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        ContentAddressedArtifactStore::new(directory.path().join("artifacts"), 1_000_000).unwrap(),
    );
    let shared = Arc::new(vec![0x5au8; 4096]);
    let shared_digest = sha256_hex(&shared);

    let mut workers = Vec::new();
    for worker in 0..4u8 {
        let store = Arc::clone(&store);
        let shared = Arc::clone(&shared);
        let shared_digest = shared_digest.clone();
        workers.push(std::thread::spawn(move || {
            let distinct = vec![worker; 1024];
            let distinct_digest = sha256_hex(&distinct);
            for _ in 0..8 {
                assert_eq!(store.put_bytes(&shared).unwrap(), shared_digest);
                assert_eq!(store.put_bytes(&distinct).unwrap(), distinct_digest);
                assert_eq!(store.get_bytes(&shared_digest).unwrap(), *shared);
                assert_eq!(store.get_bytes(&distinct_digest).unwrap(), distinct);
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }

    assert_eq!(store.get_bytes(&shared_digest).unwrap(), *shared);
    assert!(temporary_files(&objects_root(&store)).is_empty());
}

#[test]
fn put_get_with_process_cwd_unrelated_to_the_store() {
    // The rename target must be the opened bucket handle, never the process
    // cwd, so this runs in a child process whose cwd is a different directory.
    let executable = std::env::current_exe().expect("test executable");
    let output = Command::new(executable)
        .args(["--exact", "cwd_child_put_get", "--ignored", "--nocapture"])
        .env("KALA_WINDOWS_CWD_CHILD", "1")
        .output()
        .expect("cwd child test starts");
    assert!(
        output.status.success(),
        "cwd child test failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("KALA_WINDOWS_CWD_CHILD_OK"),
        "the cwd child test did not run (zero matched tests?)\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "spawned by put_get_with_process_cwd_unrelated_to_the_store"]
fn cwd_child_put_get() {
    assert!(
        std::env::var_os("KALA_WINDOWS_CWD_CHILD").is_some(),
        "this helper must be spawned by the parent cwd test"
    );
    let work = tempfile::tempdir().unwrap();
    let unrelated_cwd = work.path().join("unrelated-cwd");
    std::fs::create_dir(&unrelated_cwd).unwrap();
    let original_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&unrelated_cwd).unwrap();

    let root = work.path().join("store");
    let store = ContentAddressedArtifactStore::new(&root, 1024).unwrap();
    let payload = b"cwd must not affect the rename target";
    let digest = store.put_bytes(payload).unwrap();
    assert_eq!(store.get_bytes(&digest).unwrap(), payload);

    let object = root.join("objects").join(&digest[..2]).join(&digest);
    assert_eq!(std::fs::read(&object).unwrap(), payload);
    assert!(
        !unrelated_cwd.join(&digest).exists(),
        "publication must not rename into the process cwd"
    );
    assert!(
        std::fs::read_dir(&unrelated_cwd).unwrap().next().is_none(),
        "the unrelated cwd must stay empty"
    );
    std::env::set_current_dir(original_cwd).unwrap();
    println!("KALA_WINDOWS_CWD_CHILD_OK");
}

#[test]
fn bucket_rename_and_swap_does_not_redirect_later_operations() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 4096);
    let payload = b"bucket rename and swap";
    let digest = store.put_bytes(payload).unwrap();
    let bucket = bucket_root(&store, &digest);
    let relocated = store.root().join("bucket-relocated");

    // Moving the bucket away makes the object unreachable by digest; a later
    // put must create a fresh bucket at the canonical path rather than writing
    // through the old location.
    std::fs::rename(&bucket, &relocated).unwrap();
    match store.get_bytes(&digest) {
        Err(ArtifactError::NotFound(found)) => assert_eq!(found, digest),
        other => panic!("unexpected result: {other:?}"),
    }
    std::fs::create_dir(&bucket).unwrap();
    assert_eq!(store.put_bytes(payload).unwrap(), digest);
    assert_eq!(std::fs::read(bucket.join(&digest)).unwrap(), payload);
    assert_eq!(std::fs::read(relocated.join(&digest)).unwrap(), payload);

    // Replacing the bucket name with a regular file must be refused by both
    // the read and create paths; this covers the final selected-handle check.
    std::fs::remove_file(bucket.join(&digest)).unwrap();
    std::fs::remove_dir(&bucket).unwrap();
    std::fs::write(&bucket, b"not a directory").unwrap();
    match store.get_bytes(&digest) {
        Err(ArtifactError::ObjectParentNotDirectory) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    match store.put_bytes(payload) {
        Err(ArtifactError::ObjectParentNotDirectory) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(temporary_files(&objects_root(&store)).is_empty());
}
