#![cfg(windows)]

//! Windows reparse-point security tests.
//!
//! These exercise the production `NtCreateFile`/`FILE_OPEN_REPARSE_POINT`
//! paths through real symlinks.  Creating a symlink on Windows requires
//! Developer Mode, SeCreateSymbolicLinkPrivilege, or elevation, so the hostile
//! tests are explicitly `#[ignore]`d: a normal `cargo test` run does not claim
//! them as green evidence.  Opt in on a capable Windows host with
//! `cargo test --test windows_security -- --ignored`; if a required fixture
//! still cannot be created the test fails instead of silently returning.
//! There is no Windows host in this development environment, so genuine
//! execution of this file is pending and is reported as such; `cargo check
//! --target x86_64-pc-windows-msvc` proves it compiles.

mod support;

use std::path::Path;

use kala_artifacts::ArtifactError;
use support::{bucket_root, object_path, sha256_hex, store};

fn make_symlink(target: &Path, link: &Path, directory: bool) {
    let result = if directory {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    result.unwrap_or_else(|error| {
        panic!(
            "required reparse fixture could not be created: {error}. This ignored test \
             needs Windows Developer Mode, SeCreateSymbolicLinkPrivilege, or elevation; \
             do not record a skipped run as passing evidence."
        )
    });
}

#[test]
#[ignore = "requires Windows Developer Mode, SeCreateSymbolicLinkPrivilege, or elevation"]
fn windows_rejects_a_symlinked_bucket() {
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"windows bucket attack";
    let digest = sha256_hex(payload);
    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(&bucket).unwrap();
    let relocated = directory.path().join("bucket-relocated");
    std::fs::rename(&bucket, &relocated).unwrap();
    make_symlink(&outside, &bucket, true);
    match store.put_bytes(payload) {
        Err(error) => assert_eq!(
            error.to_string(),
            ArtifactError::ObjectPathSymlink.to_string()
        ),
        Ok(_) => panic!("publication through a symlinked bucket must fail"),
    }
    match store.get_bytes(&digest) {
        Err(error) => assert_eq!(
            error.to_string(),
            ArtifactError::ObjectPathSymlink.to_string()
        ),
        Ok(_) => panic!("read through a symlinked bucket must fail"),
    }
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
#[ignore = "requires Windows Developer Mode, SeCreateSymbolicLinkPrivilege, or elevation"]
fn windows_rejects_a_symlinked_object() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"windows object attack";
    let digest = store.put_bytes(payload).unwrap();
    let path = object_path(&store, &digest);
    std::fs::remove_file(&path).unwrap();
    let outside = directory.path().join("outside.bin");
    std::fs::write(&outside, payload).unwrap();
    make_symlink(&outside, &path, false);
    match store.get_bytes(&digest) {
        Err(error) => assert_eq!(error.to_string(), ArtifactError::ObjectSymlink.to_string()),
        Ok(_) => panic!("a symlinked object must never be read"),
    }
    match store.put_bytes(payload) {
        Err(error) => assert_eq!(error.to_string(), ArtifactError::ObjectSymlink.to_string()),
        Ok(_) => panic!("publication over a symlinked object must fail"),
    }
}

#[test]
fn windows_objects_identity_is_checked() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 1024);
    let payload = b"identity";
    let digest = store.put_bytes(payload).unwrap();
    let objects = store.root().join("objects");
    let relocated = store.root().join("objects-relocated");
    std::fs::rename(&objects, &relocated).unwrap();
    std::fs::create_dir(&objects).unwrap();
    match store.get_bytes(&digest) {
        Err(ArtifactError::ObjectRootReplaced) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}
