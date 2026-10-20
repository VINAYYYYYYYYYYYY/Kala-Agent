# kala-artifacts

Secure, immutable, content-addressed artifact storage compatible with
`kala_agent.persistence.artifacts.ContentAddressedArtifactStore`.

Objects live at `<root>/objects/<first-two-digest-hex>/<digest>`.  Digest,
key, size-limit, error and layout semantics match the Python implementation,
including the `objects/<ab>/<digest>` `key_for` shape and the opaquely
`FileNotFoundError(digest)` style for a missing object.

This crate is a member of the root `backend/` workspace and uses its single
`Cargo.lock`; the former standalone lock was a redundant generated file and was
removed during workspace consolidation. `kala-json` is consumed read-only as a
path dependency for the shared `Sha256Digest` hex type and layout contract.
`kala-studio` and `kala-contracts` remain separate workspaces under independent
review.

## Public API

```rust
use kala_artifacts::{ArtifactError, ContentAddressedArtifactStore, DEFAULT_MAX_OBJECT_SIZE};

let store = ContentAddressedArtifactStore::new("state/artifacts", DEFAULT_MAX_OBJECT_SIZE)?;
let digest = store.put_bytes(b"payload")?;          // lowercase SHA-256 hex
assert_eq!(store.get_bytes(&digest)?, b"payload");
assert_eq!(store.get_text(&digest)?, "payload");
assert_eq!(store.get_size(&digest)?, 7);
assert!(store.contains(&digest)?);
assert_eq!(store.key_for(&digest)?, format!("objects/{}/{digest}", &digest[..2]));
# Ok::<(), ArtifactError>(())
```

- `new(root, max_object_size)` — open or create the store.  The final root
  component is created `0o700` on Unix (never chmodded when it already
  exists); the `objects` directory is created `0o700`; temporary files are
  created `0o600` with `O_EXCL`.
- `from_artifact_dir(dir, max)` / `from_settings(dir, max)` — pass the
  application's already-derived artifact directory; the native backend keeps
  `Settings` in its configuration layer.
- `put_bytes`, `put_text`, `get_bytes`, `get_text`, `contains`, `get_size`,
  `key_for`, `root`, `max_object_size`.
- `normalize_digest` — Python-compatible `str.strip()` + full Unicode
  lowercasing + 64 hex validation.

`ArtifactError` keeps Python's conditions distinguishable: invalid digest,
too-large input, stored-too-large, digest mismatch, symlink/reparse refusal,
non-regular object, replaced object root, publication failure, temporary-name
exhaustion, opaque not-found (payload is the normalized digest), invalid UTF-8,
I/O and unsupported platform.

## Security model

- **Linux/Unix:** every component is opened relative to an already-open
  directory descriptor with `O_NOFOLLOW`, `O_CLOEXEC`, `O_DIRECTORY`; object
  reads use `O_NONBLOCK` (a FIFO cannot hang the caller); temporary creation
  uses `O_EXCL`; publication is a same-directory `linkat` that fails on an
  existing name; a final `lstat`/`fstat` identity check refuses a publication
  whose bucket was renamed or replaced in flight.  The root and `objects`
  handles are held for the life of the store and the `objects` path is
  re-validated before every operation.
- **Windows:** the root and `objects` directories are opened with
  `CreateFileW(..., FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)`
  and checked for `FILE_ATTRIBUTE_REPARSE_POINT`.  Every child is created or
  opened with `NtCreateFile` and an `OBJECT_ATTRIBUTES.RootDirectory` pointing
  at the validated parent, always with `FILE_OPEN_REPARSE_POINT`; identities
  are `FILE_ID_INFO` (falling back to `BY_HANDLE_FILE_INFORMATION`).  Every
  bucket handle selected by the read path, the successful existing-directory
  `FILE_OPEN_IF` path, or the write-access fallback reopen is re-checked for
  reparse attributes and directory type before use.  Publication flushes the
  temporary file and renames it by handle with `ReplaceIfExists = FALSE`, then
  re-reads the bucket identity by file ID.
- **Windows rename target:** `FILE_RENAME_INFO.RootDirectory` is set to the
  opened bucket handle and the name is the bare digest.  Microsoft documents
  that a relative name with a non-NULL `RootDirectory` is resolved against that
  directory handle, while a relative name with `RootDirectory == NULL` is
  resolved against the process current directory.  Binding the target this way
  keeps publication inside the opened bucket regardless of the process cwd or
  later renames of the root/bucket paths.
- **Every read** re-hashes the returned bytes and fails with
  `DigestMismatch` if they do not match the requested digest.
- The store is `Send + Sync` and holds no cross-process lock; each mutation is
  individually atomic, so any number of threads or processes may share one
  root without coordinating.

### Windows guarantee differences (stated, not hidden)

- NTFS has no portable directory `fsync`.  Data is flushed with
  `FlushFileBuffers` on the temporary file; the directory flush is attempted
  and `ACCESS_DENIED`/`INVALID_HANDLE`/`NOT_SUPPORTED` are tolerated, so
  entry-name durability depends on the filesystem's own metadata ordering.
- Directory/file DACLs are inherited from the parent directory.  This crate
  does not loosen or forge an ACL and does not claim the Python `0o700`/`0o600`
  mode; on Windows the effective guarantee is whatever ACL the application
  data directory already carries.  On Unix the created modes are also subject
  to the process `umask`, exactly like `os.mkdir(mode=...)`.
- The public `root()` path strips the Windows extended-length `\\?\` prefix to
  match Python's `Path.resolve()` display form; operations never re-resolve
  that path, they use the validated handles.

## Tests

Run from `backend/` (root workspace):

```bash
cargo fmt --all --check
cargo test -p kala-artifacts --locked
cargo test -p kala-artifacts --locked --release
cargo clippy -p kala-artifacts --all-targets --locked -- -D warnings
cargo build -p kala-artifacts --release --locked
cargo check -p kala-artifacts --locked --all-targets --target x86_64-pc-windows-msvc
```

The Linux suite contains 40 accepted tests. The integration suite covers API
semantics and boundaries, symlinked root/objects/bucket/object attacks,
non-regular files and FIFOs without blocking, root/bucket/object replacement
during operations, a bucket-swap race, failed publication cleanup, thread and
process concurrency with byte-level evidence, and live Python cross-reader
tests in both directions (`tests/python_oracle.py` launches the repository
virtual environment; Python is test-only and the library never calls it).

`tests/windows_security.rs` contains the Windows reparse-point tests.  They
compile under the `x86_64-pc-windows-msvc` target check; genuine execution
requires a Windows host and symlink privilege (Developer Mode,
`SeCreateSymbolicLinkPrivilege`, or elevation) and has not been performed in
this development environment.  The hostile-fixture tests are explicitly
`#[ignore]`d, so a normal run does not report them as executed green evidence;
opt in with `cargo test --test windows_security -- --ignored` on a capable
Windows host.  A fixture that cannot be created then fails the test instead of
silently skipping.

`tests/windows_production.rs` covers the unprivileged Windows production path:
ordinary and idempotent put/get, concurrent identical/distinct writers, a
child process whose cwd is unrelated to the store, and bucket rename/swap
behavior.  It is cross-compiled here and remains pending execution on a
Windows host.

