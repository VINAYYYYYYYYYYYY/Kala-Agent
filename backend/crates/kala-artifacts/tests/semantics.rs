//! API semantics of the native content-addressed artifact store.
//!
//! These tests exercise the production code paths (real directories, real
//! handles, real hashing); no internal helper is mocked.

mod support;

use std::collections::BTreeMap;

use kala_artifacts::{ArtifactError, ContentAddressedArtifactStore, DEFAULT_MAX_OBJECT_SIZE};
use support::{
    assert_exact_store_contents, bucket_root, object_path, objects_root, sha256_hex, store,
    temporary_files,
};

#[test]
fn default_size_limit_matches_python() {
    let directory = tempfile::tempdir().unwrap();
    let store =
        ContentAddressedArtifactStore::new(directory.path(), DEFAULT_MAX_OBJECT_SIZE).unwrap();
    assert_eq!(store.max_object_size(), DEFAULT_MAX_OBJECT_SIZE);
}

#[test]
fn configuration_is_exposed_and_directories_are_private() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let store = ContentAddressedArtifactStore::new(&root, 32).unwrap();
    assert_eq!(store.max_object_size(), 32);
    assert!(store.root().is_absolute());
    #[cfg(unix)]
    assert_eq!(store.root(), root.canonicalize().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let root_mode = std::fs::metadata(store.root())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let objects_mode = std::fs::metadata(objects_root(&store))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(root_mode, 0o700, "root directory is created 0o700");
        assert_eq!(objects_mode, 0o700, "objects directory is created 0o700");
    }
}

#[cfg(unix)]
#[test]
fn constructor_rejects_a_symlinked_root() {
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let linked = directory.path().join("linked");
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    let error = ContentAddressedArtifactStore::new(&linked, 16).unwrap_err();
    assert!(
        matches!(error, ArtifactError::RootSymlink),
        "unexpected error: {error}"
    );
}

#[cfg(unix)]
#[test]
fn constructor_rejects_a_symlinked_objects_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    std::fs::create_dir(&root).unwrap();
    let elsewhere = directory.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("objects")).unwrap();
    let error = ContentAddressedArtifactStore::new(&root, 16).unwrap_err();
    assert!(
        matches!(error, ArtifactError::ObjectRootSymlink),
        "unexpected error: {error}"
    );
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
}

#[test]
fn constructor_rejects_non_directory_roots_and_object_roots() {
    let directory = tempfile::tempdir().unwrap();
    let plain = directory.path().join("plain");
    std::fs::write(&plain, b"x").unwrap();
    let nested = plain.join("nested").join("deeper");
    assert!(matches!(
        ContentAddressedArtifactStore::new(&nested, 16),
        Err(ArtifactError::RootNotDirectory(_)) | Err(ArtifactError::Io(_))
    ));

    let root = directory.path().join("other");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("objects"), b"x").unwrap();
    let error = ContentAddressedArtifactStore::new(&root, 16).unwrap_err();
    assert!(
        matches!(error, ArtifactError::ObjectRootNotDirectory(_)),
        "unexpected error: {error}"
    );
}

#[test]
fn key_for_matches_the_python_layout_and_rejects_invalid_digests() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 32);
    let digest = sha256_hex(b"layout");
    assert_eq!(
        store.key_for(&digest).unwrap(),
        format!("objects/{}/{digest}", &digest[..2])
    );
    assert_eq!(
        store.key_for(&digest.to_uppercase()).unwrap(),
        format!("objects/{}/{digest}", &digest[..2])
    );
    assert_eq!(
        store
            .key_for(&format!("  \u{1c}{digest} \u{3000}"))
            .unwrap(),
        format!("objects/{}/{digest}", &digest[..2])
    );
    assert!(matches!(
        store.key_for("not-a-digest"),
        Err(ArtifactError::InvalidDigest)
    ));
    assert!(store.key_for(&"z".repeat(64)).is_err());
    assert!(store.key_for("").is_err());
    assert!(store.get_bytes("../outside").is_err());
    assert!(store.contains("").is_err());
}

#[test]
fn put_get_roundtrip_is_content_addressed_and_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 64);
    let payload = b"payload";
    let digest = store.put_bytes(payload).unwrap();
    assert_eq!(digest, sha256_hex(payload));
    assert_eq!(store.get_bytes(&digest).unwrap(), payload);
    assert_eq!(store.get_size(&digest).unwrap(), payload.len() as u64);
    assert!(store.contains(&digest).unwrap());
    assert_eq!(store.put_bytes(payload).unwrap(), digest);
    assert_eq!(store.put_bytes(&payload[..]).unwrap(), digest);

    let mut expected = BTreeMap::new();
    expected.insert(digest, payload.to_vec());
    assert_exact_store_contents(&store, &expected);
}

#[test]
fn publication_is_immutable_and_never_overwrites() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 64);
    let digest = store.put_bytes(b"original").unwrap();
    let path = object_path(&store, &digest);
    let original_bytes = std::fs::read(&path).unwrap();
    assert_eq!(original_bytes, b"original");

    // A corrupt existing object is refused instead of silently replaced.
    std::fs::write(&path, b"tampered").unwrap();
    let error = store.put_bytes(b"original").unwrap_err();
    assert!(
        matches!(error, ArtifactError::DigestMismatch),
        "unexpected error: {error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"tampered");
    let error = store.get_bytes(&digest).unwrap_err();
    assert!(matches!(error, ArtifactError::DigestMismatch));

    // Restore and refill.
    std::fs::write(&path, &original_bytes).unwrap();
    assert_eq!(store.put_bytes(b"original").unwrap(), digest);
    assert_eq!(store.get_bytes(&digest).unwrap(), b"original");
}

#[test]
fn failed_publication_leaves_no_temporary_file_and_no_partial_object() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 64);
    let payload = b"atomic artifact";
    let digest = sha256_hex(payload);

    // A directory squatting on the destination name makes the hard link fail.
    let bucket = bucket_root(&store, &digest);
    std::fs::create_dir_all(&bucket).unwrap();
    std::fs::create_dir(bucket.join(&digest)).unwrap();
    let error = store.put_bytes(payload).unwrap_err();
    assert!(
        matches!(
            error,
            ArtifactError::NonRegular | ArtifactError::PublicationFailed
        ),
        "unexpected error: {error}"
    );
    assert!(temporary_files(&objects_root(&store)).is_empty());

    // Remove the squatter and publish normally.
    std::fs::remove_dir_all(&bucket).unwrap();
    assert_eq!(store.put_bytes(payload).unwrap(), digest);
    assert_eq!(store.get_bytes(&digest).unwrap(), payload);
}

#[test]
fn missing_objects_and_buckets_report_opaque_not_found() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 64);
    let missing = sha256_hex(b"absent");
    match store.get_bytes(&missing) {
        Err(ArtifactError::NotFound(message)) => assert_eq!(message, missing),
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(!store.contains(&missing).unwrap());
    assert!(matches!(
        store.get_size(&missing),
        Err(ArtifactError::NotFound(_))
    ));

    // An empty bucket directory without the object is still "missing".
    std::fs::create_dir_all(bucket_root(&store, &missing)).unwrap();
    assert!(!store.contains(&missing).unwrap());
    assert!(matches!(
        store.get_bytes(&missing),
        Err(ArtifactError::NotFound(_))
    ));
}

#[test]
fn size_limits_are_inclusive_and_apply_to_reads() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 8);
    let exact = store.put_bytes(b"12345678").unwrap();
    assert_eq!(store.get_size(&exact).unwrap(), 8);
    assert!(matches!(
        store.put_bytes(b"123456789"),
        Err(ArtifactError::TooLarge)
    ));

    // A stored object that exceeds the current limit is refused on read.
    let small_store = ContentAddressedArtifactStore::new(store.root(), 4).unwrap();
    assert!(matches!(
        small_store.get_bytes(&exact),
        Err(ArtifactError::StoredTooLarge)
    ));
    assert!(matches!(
        small_store.contains(&exact),
        Err(ArtifactError::StoredTooLarge)
    ));
}

#[test]
fn zero_limit_allows_only_the_empty_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 0);
    let digest = store.put_text("").unwrap();
    assert_eq!(digest, sha256_hex(b""));
    assert_eq!(store.get_bytes(&digest).unwrap(), b"");
    assert_eq!(store.get_size(&digest).unwrap(), 0);
    assert!(matches!(
        store.put_bytes(b"x"),
        Err(ArtifactError::TooLarge)
    ));
    assert!(store.contains(&digest).unwrap());
}

#[test]
fn text_roundtrips_unicode_and_reports_invalid_utf8() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 64);
    let text = "καλός — 漢字 🎯";
    let digest = store.put_text(text).unwrap();
    assert_eq!(digest, sha256_hex(text.as_bytes()));
    assert_eq!(store.get_text(&digest).unwrap(), text);

    let binary = store.put_bytes(b"\xff\xfe").unwrap();
    match store.get_text(&binary) {
        Err(ArtifactError::InvalidUtf8(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn boundary_sizes_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let max = 4096u64;
    let store = store(directory.path(), max);
    let payloads: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0],
        b"a".repeat(255),
        (0..=255u8).collect(),
        (0..max as usize).map(|index| (index % 251) as u8).collect(),
    ];
    let mut expected = BTreeMap::new();
    for payload in &payloads {
        let digest = store.put_bytes(payload).unwrap();
        assert_eq!(store.get_bytes(&digest).unwrap(), *payload);
        expected.insert(digest, payload.clone());
    }
    // Every payload maps to a distinct digest, including the empty one.
    assert_eq!(expected.len(), payloads.len());
    assert_exact_store_contents(&store, &expected);

    let mut oversized = vec![0u8; max as usize + 1];
    oversized[max as usize] = 1;
    assert!(matches!(
        store.put_bytes(&oversized),
        Err(ArtifactError::TooLarge)
    ));
}

#[test]
fn durability_after_store_reopen_and_no_temporary_residue() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let digests = {
        let store = ContentAddressedArtifactStore::new(&root, 4096).unwrap();
        let mut digests = Vec::new();
        for payload in [b"durability".as_slice(), b"", b"third payload"] {
            digests.push((store.put_bytes(payload).unwrap(), payload.to_vec()));
        }
        digests
    };
    // A brand new store handle (new descriptors) must see the published bytes.
    let reopened = ContentAddressedArtifactStore::new(&root, 4096).unwrap();
    for (digest, payload) in &digests {
        assert_eq!(reopened.get_bytes(digest).unwrap(), *payload);
        assert!(reopened.contains(digest).unwrap());
    }
    assert!(temporary_files(&objects_root(&reopened)).is_empty());
}

#[test]
fn contains_reports_a_corrupt_object_instead_of_false() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path(), 64);
    let digest = store.put_bytes(b"verify me").unwrap();
    std::fs::write(object_path(&store, &digest), b"tampered now").unwrap();
    match store.contains(&digest) {
        Err(ArtifactError::DigestMismatch) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn from_artifact_dir_matches_new() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("configured").join("artifacts");
    let store = ContentAddressedArtifactStore::from_artifact_dir(&root, 16).unwrap();
    assert!(store.root().is_absolute());
    #[cfg(unix)]
    assert_eq!(store.root(), root.canonicalize().unwrap());
    let digest = store.put_bytes(b"configured").unwrap();
    assert_eq!(store.get_bytes(&digest).unwrap(), b"configured");
}
