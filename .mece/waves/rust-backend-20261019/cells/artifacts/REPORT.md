# Cell report — secure native content-addressed artifact store

- Cell: `.mece/waves/rust-backend-20261019/cells/artifacts`
- Brief: `.mece/waves/rust-backend-20261019/cells/artifacts/TASK.md`
- Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**.
  Original round: T3 task
  `node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-artifacts-r1-go-retry46`,
  child thread
  `thread:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-artifacts-r1-go-retry46`
  (`LAUNCH-RETRY46.md`); original worker session
  `ses_eab67d309ffeSQsjPk3vMTFZ7a`.
  Repair round 2: T3 task
  `node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-artifacts-r2-go-review47`,
  child thread
  `thread:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-artifacts-r2-go-review47`
  (`LAUNCH-R2.md`).
- Reviewer: GPT-6.1-Sol (not run by this worker)
- Date: 2026-10-19
- No delegation, no commit, no push, no deploy, no model/provider switch. The
  original terminal result was consumed for round 2; no duplicate live
  predecessor work and no child turns were started.

## Delivered

A new standalone crate, `backend/crates/kala-artifacts/**`, plus
`REPORT.md`, `SESSION.md` and the round-2 `SESSION-R2.md`. Nothing else was
modified: the accepted
`backend/Cargo.toml`, `backend/Cargo.lock`, `kala-json`, `kala-persistence`,
`kala-studio`/`kala-contracts` (other workers) and all Python/product/test/
packaging/frontend files are untouched. The crate is its own workspace with a
local `Cargo.lock`; `kala-json` is consumed read-only as a path dependency for
the shared `Sha256Digest` type.

| Path | Purpose |
|---|---|
| `backend/crates/kala-artifacts/Cargo.toml` | standalone `[workspace]`, local lock; deps `sha2`, `kala-json` (path), `getrandom`, `thiserror`; Unix `rustix`+`libc`; Windows `windows-sys` (Wdk/NT + Win32 FS) |
| `backend/crates/kala-artifacts/Cargo.lock` | local resolved lock (`--locked` verified) |
| `src/lib.rs` | public store, digest normalization, persistence of layout/limits/error contract |
| `src/error.rs` | typed `ArtifactError` covering every Python failure condition |
| `src/sys/mod.rs` + `src/sys/unix.rs` | descriptor-relative Unix implementation (`rustix`) |
| `src/sys/windows.rs` | native Windows implementation (`windows-sys`, `NtCreateFile`, `NtCreateFile`-relative handles) |
| `src/sys/unsupported.rs` | fail-closed stub for any other target |
| `tests/semantics.rs` | API semantics, layout, limits, immutability, cleanup, durability |
| `tests/security.rs` | symlink/reparse/non-regular/replacement/race adversarial tests |
| `tests/concurrency.rs` | thread + real multi-process concurrency, atomic reads |
| `tests/cross_reader.rs` + `tests/python_oracle.py` | live Python↔Rust cross-reader storage |
| `tests/windows_production.rs` | Windows production-path put/get, idempotence, concurrency, unrelated cwd, bucket rename/swap (compile-checked; execution pending) |
| `tests/windows_security.rs` | Windows reparse-point tests (explicitly `#[ignore]`d, opt-in on a privileged Windows host) |
| `README.md` | API, security model, Windows guarantee differences and rename-target evidence, test commands |

## Round 2 repair (review-1 P2 / rename semantics)

Scope: `backend/crates/kala-artifacts/**` only; no root manifest/lock, other crate,
Python, fixture, packaging, frontend or user file was edited. Linux behavior was
not changed except for the Windows code path and tests.

### Every selected Windows bucket handle is validated

`sys/windows.rs` now selects a handle (read branch, successful
`FILE_OPEN_IF`, or write-access fallback reopen) and then calls one
unconditional `validate_bucket_directory` per selected handle before
`Bucket` is constructed.  The fallback helper validates the readable handle it
uses to classify the failure, drops it, reopens with write access, and validates
the final writable handle itself; the outer `open_bucket` validates the result
again.  This removes the previous gap where the successful `FILE_OPEN_IF`
handle and the fallback's second `FILE_OPEN` handle were used without the
reparse/directory check.  `ensure_objects` was reshaped the same way: exactly
one final reparse/directory check on whichever handle the branch selected.
The public symlink refusal contract is unchanged (`ObjectPathSymlink` before
`ObjectParentNotDirectory`).

A `#[cfg(test)]` hook is the only test seam: it runs between the readable open
and the writable reopen so a unit test can replace the bucket deterministically.
It is compiled out of normal and integration-test builds.

### Rename target semantics resolved with documentation evidence

The pre-repair code passed `RootDirectory = NULL` with a bare digest name.  That
is **not** guaranteed to target the opened bucket: Microsoft's Win32
`FILE_RENAME_INFO` documentation says `RootDirectory` is "Set to NULL in the
common case. If FileName specifies a relative name, this field can be a handle
to the directory relative to which the new name is resolved", and its
`FileName` member documents a relative path as "A path relative to the process's
current directory".  The WDK `FILE_RENAME_INFORMATION` documentation is
explicit: a relative file name requires `RootDirectory` to contain "a handle to
the target directory for the rename operation", and `RootDirectory` is "a handle
for the root directory under which the file will reside after it is renamed".

Sources (fetched in this session from this host):

- <https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info>
  (`RootDirectory`, `FileName` member text quoted above; page updated 2026-05-15).
- <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information>
  (`RootDirectory` / `FileName` remarks quoted above).
- <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle>
  confirms `FileRenameInfo` uses `FILE_RENAME_INFO`.

`rename_by_handle` now takes the bucket `Dir`, sets
`FILE_RENAME_INFO.RootDirectory = bucket_handle`, passes the bare digest as the
simple relative name, keeps `ReplaceIfExists = FALSE` for the
`FileRenameInfo` class, and sizes the buffer as
`sizeof(FILE_RENAME_INFO) + FileNameLength` with an explicit NUL terminator.
The target is therefore the opened bucket independent of the process cwd and of
root/bucket path renames; no absolute path is re-resolved.  A Wine source
cross-check was also read: Wine's `FileRenameInformation` handler builds
`OBJECT_ATTRIBUTES` with `info->RootDirectory` and passes it to
`set_fd_name_info`, i.e. the same handle-relative contract.

### Added Windows coverage

- `tests/windows_production.rs` (Windows-only, unprivileged): ordinary
  put/get/idempotent put, concurrent identical/distinct writers/readers, a
  child process whose cwd is unrelated to the store, and a bucket
  rename/swap/replacement-on-disk scenario.
- `sys/windows.rs` unit tests: successful existing-directory `FILE_OPEN_IF`
  reopen, write-access fallback replacement at the reopen boundary, held-bucket
  rename publication refusal with temporary cleanup, `FILE_RENAME_INFO` layout;
  one symlink-at-fallback test is `#[ignore]`d with the other hostile fixtures.
- `tests/windows_security.rs`: symlink bucket/object tests are now explicitly
  `#[ignore]`d and opt-in via `cargo test --test windows_security -- --ignored`.
  If the host still cannot create the fixture, the test panics with an
  actionable message; a normal run reports them as ignored, never as green.

## API (Python → native mapping)

| Python | Rust |
|---|---|
| `ContentAddressedArtifactStore(root, max_object_size=1_000_000_000)` | `ContentAddressedArtifactStore::new(root, u64)` (default constant `DEFAULT_MAX_OBJECT_SIZE`) |
| `from_settings(settings, max_object_size=…)` | `from_artifact_dir(dir, max)` / `from_settings(dir, max)` — the native config layer passes `Settings::artifact_dir`; there is no Python-shaped settings type in this crate |
| `root` / `max_object_size` | `root() -> &Path` / `max_object_size() -> u64` |
| `key_for(digest)` | `key_for(&str) -> Result<String>` — exactly `objects/<ab>/<digest>` |
| `put_bytes/put_text/get_bytes/get_text/contains/get_size` | same names; `put_text`/`get_text` convert UTF-8; failures are `Result<_, ArtifactError>` |
| `ValueError/FileNotFoundError/NotADirectoryError/OSError/UnicodeDecodeError` | `ArtifactError::{InvalidDigest, TooLarge, StoredTooLarge, DigestMismatch, ExistingIntegrity, RootSymlink, RootNotDirectory, ObjectRootSymlink, ObjectRootNotDirectory, ObjectRootReplaced, ObjectPathSymlink, ObjectParentNotDirectory, ObjectSymlink, ObjectOpen, NonRegular, PublicationFailed, TemporaryNameExhausted, NotFound(String), InvalidUtf8, Io, UnsupportedPlatform}`; `NotFound` displays exactly the normalized digest, matching `FileNotFoundError(digest)` |

Digest normalization matches Python `_normalize_digest`: `str.strip()` with the
full CPython whitespace set (including `U+001C..=U+001F`), full Unicode
lowercasing, then exactly 64 `[0-9a-f]` characters. Key/layout, `0o700`/`0o600`
creation modes, 8 attempt/16-random-byte temporary names
(`.artifact-<32-hex>.tmp`), inclusive size limit, re-hash-on-every-read and the
immutable no-overwrite publication order all match the actual Python source and
tests read for this cell (`src/kala_agent/persistence/artifacts.py`,
`tests/unit/test_artifacts_branches.py`,
`tests/security/test_artifact_concurrency.py`,
`tests/unit/test_persistence.py`, ADR-0004).

## Security design

Linux/Unix (production path):
- Root is opened once with `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC` and held; the
  `objects` path is re-opened and its `st_dev`/`st_ino` compared before every
  operation, so a renamed/replaced objects directory is refused
  (`ObjectRootReplaced`) and a symlinked one is refused (`ObjectRootSymlink`).
- Buckets are opened with `openat` from the held `objects` descriptor with
  `O_NOFOLLOW|O_DIRECTORY`; a symlink/`ENOTDIR` result is re-classified with
  `lstat` so the correct symlink vs not-a-directory refusal is reported.
- Objects are opened with `O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC`,
  `fstat`-checked for a regular file (a FIFO/socket/device/directory is
  refused), read with a `max+1` bound, and re-hashed.
- Publication: `O_EXCL` temporary file (`0o600`), `fsync` file, same-directory
  `linkat` (atomic, refuses an existing name), directory `fsync` (documented
  errno tolerance), regular-file re-check, temporary unlink in all paths, then
  a final `lstat`/`fstat` bucket identity check. A conflicting existing object
  is itself read and digest-verified; it is never overwritten.
- The root handle keeps working even if the root path is swapped; that is
  stronger than the path-based Python behavior and is asserted by a test.

Windows (required product target, compile-checked here):
- Root and `objects` are opened with `CreateFileW(OPEN_EXISTING,
  FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT)` and checked for
  `FILE_ATTRIBUTE_REPARSE_POINT`; a denied write access falls back to a
  read-only handle so reads still work.
- Every child is created/opened with `NtCreateFile` and
  `OBJECT_ATTRIBUTES.RootDirectory` = validated parent handle, always with
  `FILE_OPEN_REPARSE_POINT`; attributes are re-read and reparse points refused.
  Identities use `FileIdInfo` (`FILE_ID_INFO`), falling back to
  `GetFileInformationByHandle`.
- Every selected bucket handle — read branch, successful existing-directory
  `FILE_OPEN_IF`, and the write-access fallback reopen — passes the same final
  reparse + directory validation before `Bucket` exists (round 2).
- Publication writes a `FILE_CREATE` temporary, `FlushFileBuffers`, then
  renames by handle with `SetFileInformationByHandle(FileRenameInfo,
  ReplaceIfExists = FALSE, RootDirectory = opened bucket handle)` — atomic and
  no-overwrite; the target is handle-relative rather than cwd-relative — then
  it re-checks the object is a regular non-reparse file, deletes a conflicting
  temporary by handle, and finally re-reads the bucket identity. No
  check-then-open path canonicalization is used for any component.

## Checks actually run

### Round 1 (original delivery, this Linux host)

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo test --locked` (debug) | 40 tests, 0 failed; 4 spawned process-worker runs; includes live Python cross-reader |
| `cargo test --locked --release` | same 40 tests, 0 failed |
| `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo build --locked --release` | clean |
| `cargo check --locked --all-targets --target x86_64-pc-windows-msvc` | clean (cross-compile check only) |
| `cargo clippy --locked --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | clean |
| `cargo test --test security a_bucket_swapped…` ×30 | 30/30 pass after the deterministic planted-symlink seed |
| full `cargo test --locked` ×5 | 5/5 pass (flake-hardening evidence) |
| Python reference suite: `.venv/bin/python -m pytest tests/unit/test_artifacts_branches.py tests/security/test_artifact_concurrency.py -q` | 59 passed (unchanged Python behavior) |

### Round 2 (repair, this Linux host, same standalone crate)

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo test --locked` (debug) | 40 tests, 0 failed; same suites as round 1 |
| `cargo test --locked --release` | 40 tests, 0 failed (4 unit + 4 concurrency, including real process workers + 3 live Python cross-reader + 12 security + 17 semantics) |
| `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo build --locked --release` | clean |
| `cargo check --locked --all-targets --target x86_64-pc-windows-msvc` | clean (cross-compile check only; includes the new Windows-only tests) |
| `cargo clippy --locked --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | clean |

Round-2 test inventory: the Linux-executable count remains 40.  Windows-only
definitions added or retained: 5 `tests/windows_production.rs` (one is a
spawned-child helper marked `#[ignore]`), 3 `tests/windows_security.rs` (two
hostile-fixture tests marked `#[ignore]`), and 5 `sys/windows.rs` unit tests
(one symlink-fixture test marked `#[ignore]`).  None of the Windows-only tests
ran here; the Windows all-target checks compile them.  Python is used only as a
test oracle; the library has no Python/subprocess/fallback path.

Environment: `cargo 1.99.1 / rustc 1.99.1`; installed targets
`x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc` (rust-std added this
session for the Windows check); container Linux host, non-root.

## Unresolved platform blockers and honest limits

1. **No Windows host.** Windows code paths (`sys/windows.rs`,
   `tests/windows_production.rs`, `tests/windows_security.rs`) are verified only
   by `cargo check`/`clippy` for `x86_64-pc-windows-msvc`; genuine execution,
   NTFS rename behavior and reparse-point attack tests are **pending**. No
   Windows runtime proof is claimed.  Round-1 review independently confirmed
   both Windows all-target checks compile; runtime proof remains the one honest
   gap.
2. **No portable directory fsync on Windows.** Data is flushed on the file;
   the directory flush is attempted and `ACCESS_DENIED`/`INVALID_HANDLE`/
   `NOT_SUPPORTED` are tolerated. Entry-name durability therefore depends on
   NTFS metadata ordering, stated in `README.md` rather than papered over.
3. **Windows ACLs.** Modes `0o700/0o600` are not translated to a DACL; the
   store inherits the parent directory ACL and does not claim a Windows
   permission guarantee. On Unix the created modes are further subject to the
   process `umask`, the same as `os.mkdir(mode=…)`.
4. **`~user` expansion.** Unix implements it via `getpwnam_r` (like
   `posixpath.expanduser`); Windows intentionally leaves `~user` literal,
   matching `ntpath.expanduser`. `~` and `~/`/`~\` match on both platforms
   (`HOME` with `getpwuid_r` fallback on Unix, `USERPROFILE`/`HOMEDRIVE`+
   `HOMEPATH` on Windows).
5. **Window size type.** Python accepts an arbitrary non-negative `int`; the
   native API uses `u64` (the default 1e9 and every realistic bound fit).
6. **Whole-object reads.** Like Python, `get_*`/`get_size`/`contains` read and
   verify the full object with a `max+1` bound; streaming/chunking is not
   implemented (ADR-0004 already lists large CAD/CAE artifacts as future
   work). No performance claim is made and none was measured.
7. **Stricter identity behavior (deliberate, accepted in round 1).** The held
   `objects` identity check adds `ObjectRootReplaced`, and the held root handle
   keeps working after a root path swap; both are stronger than the Python
   path-based behavior and are covered by adversarial tests.  REVIEW-1 accepted
   this strengthening and asked that it be documented; it is.
8. **Windows-only symlink tests need privilege.** They are now explicitly
   `#[ignore]`d and opt-in (`cargo test --test windows_security -- --ignored`);
   if the fixture cannot be created in an opted-in run, the test panics.  A
   normal run reports them as ignored, not as executed green evidence.
9. **Rename target semantics (resolved in round 2).** Relative `FileName` with
   `RootDirectory = NULL` is documented as cwd-relative, so the old code could
   not guarantee the target was the opened bucket.  `RootDirectory` now carries
   the bucket handle; see the quoted Microsoft documentation in the Round 2
   section.  Runtime behavior is still pending a Windows host, but the API
   contract and code shape are now the documented handle-relative form.
10. **Windows `FILE_OPEN_IF`/fallback boundary only indirectly executed.**
    The fallback-boundary test uses a `#[cfg(test)]` seam because creating the
    real trigger (an ACL-denied create followed by a concurrent replacement)
    needs a Windows host with controlled ACLs.  All selected handles still pass
    the same unconditional production validation; the seam only stages the
    replacement deterministically.

## Review focus suggestions (round 2)

- Re-run `cargo test --locked --release`, `cargo fmt --all --check`,
  `cargo clippy --locked --all-targets -- -D warnings`, and the Windows
  all-target `check`/`clippy` from `backend/crates/kala-artifacts`; the 3
  cross-reader tests require `.venv/bin/python` (`KALA_PYTHON` override).
- Inspect `open_bucket`/`reopen_bucket_for_write` and confirm every selected
  handle reaches `validate_bucket_directory` before `Bucket` is constructed,
  including the `FILE_OPEN_IF` success and fallback write reopen.
- Inspect `rename_by_handle`: `RootDirectory` must be the bucket handle,
  `FileName` the bare digest, `ReplaceIfExists = FALSE` for `FileRenameInfo`,
  and the buffer length `sizeof(FILE_RENAME_INFO) + FileNameLength`.
- Verify the `#[ignore]`/opt-in split is honest and that no Windows-only test is
  reported as executed on this Linux host.
- Verify no root manifest/lock or other crate/Python file changed (see
  `SESSION-R2.md` for the exact touched-path list).
