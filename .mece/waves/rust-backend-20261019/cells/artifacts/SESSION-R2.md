# Cell session notes — Windows review repair, round 2

- Task: `.mece/waves/rust-backend-20261019/cells/artifacts/TASK-R2.md`
- Review being answered: `REVIEW-1.md` (GPT-6.1-Sol, narrow Windows repair)
- Model: OpenCode Go `opencode-go/deepseek-v4.1-flash`, variant **max** (no
  model/provider switch, no delegation/subagent).
- T3 task:
  `node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-artifacts-r2-go-review47`;
  child thread:
  `thread:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-artifacts-r2-go-review47`
  (`LAUNCH-R2.md`).
- Original round task/child (now recorded in `REPORT.md`):
  `…rust-backend-20261019-artifacts-r1-go-retry46`
  (`LAUNCH-RETRY46.md`).
- Date: 2026-10-19. Working tree root: `/home/v9i/Projects/Kala-Agent`.
- Constraints honored: no commit/push/deploy/cutover; no root manifest/lock or
  other-crate edits; no Python/product/test/fixture/packaging/frontend/user
  edits; temporary directories only. Owned scope only:
  `backend/crates/kala-artifacts/**` and
  `.mece/waves/rust-backend-20261019/cells/artifacts/{REPORT.md,SESSION-R2.md}`.

## Read first

`TASK.md`, `REPORT.md`, `REVIEW-1.md`, `TASK-R2.md`, `LAUNCH-RETRY46.md`,
`LAUNCH-R2.md`, and the current `src/sys/windows.rs`, `src/lib.rs`,
`src/sys/mod.rs`, `src/error.rs`, `README.md`, all integration tests and the
shared test support module.

## What changed (source)

1. **Final bucket validation.**
   - Added `validate_bucket_directory(&Dir)` (reparse check, then directory
     check, preserving `ObjectPathSymlink` before `ObjectParentNotDirectory`).
   - `Dir::open_bucket` now produces a handle from exactly one branch (read
     `FILE_OPEN`, create `FILE_OPEN_IF`, or write-access fallback reopen) and
     always calls `validate_bucket_directory` before constructing `Bucket`.
   - Extracted `reopen_bucket_for_write` for the create fallback.  It validates
     the readable handle used for classification, drops it, reopens with write
     access, and validates the **final writable** handle itself.
   - `Dir::ensure_objects` reshaped to the same pattern: one unconditional final
     reparse + directory check on the selected handle.
   - `#[cfg(test)] thread_local` hook `BUCKET_WRITE_REOPEN_HOOK` stages a
     deterministic replacement between the readable open and the writable
     reopen.  It is not compiled into normal/integration builds.
2. **Rename target semantics.**
   - `rename_by_handle` now takes the bucket `Dir` and sets
     `FILE_RENAME_INFO.RootDirectory = bucket handle`, with the bare digest as
     the simple relative `FileName`.
   - `ReplaceIfExists = FALSE` remains the field used for the `FileRenameInfo`
     class (not `FileRenameInfoEx`).
   - Buffer sizing changed to the documented
     `sizeof(FILE_RENAME_INFO) + FileNameLength` shape, with an explicit NUL
     terminator stored after the name; `FileNameLength` excludes the NUL.
3. **Windows tests.**
   - New `tests/windows_production.rs`: ordinary/idempotent put/get, concurrent
     identical and distinct writers/readers, a spawned child process whose cwd
     is unrelated to the store, and bucket rename/swap/replacement.
   - New `sys/windows.rs` unit tests: `FILE_OPEN_IF` existing-directory reopen,
     fallback-boundary replacement, held-bucket rename publication refusal plus
     temporary cleanup, and `FILE_RENAME_INFO` layout; one symlink-at-fallback
     test is ignored/opt-in.
   - `tests/windows_security.rs`: symlink bucket/object tests are `#[ignore]`d
     and opt-in; fixture creation failure panics instead of silently returning.
4. **Docs.** `README.md` records the final-handle validation and the
   handle-relative rename target; `REPORT.md` records the round-2 repair,
   evidence URLs, results and remaining gaps.

## Rename-target evidence used

Official Microsoft documentation fetched from this host during the session
(not Firecrawl):

- Win32 `FILE_RENAME_INFO`:
  <https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info>
  - `RootDirectory`: "Set to NULL in the common case. If FileName specifies a
    relative name, this field can be a handle to the directory relative to which
    the new name is resolved."
  - `FileName`: relative form is "A path relative to the process's current
    directory."
- WDK `FILE_RENAME_INFORMATION`:
  <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information>
  - `RootDirectory`: "a handle for the root directory under which the file will
    reside after it is renamed."
  - `FileName` relative form: "the RootDirectory member contains a handle to the
    target directory for the rename operation. The file name itself must be a
    simple file name."
- `SetFileInformationByHandle`:
  <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle>
  confirms `FileRenameInfo` uses `FILE_RENAME_INFO`.

Corroboration from Wine source (`dlls/ntdll/unix/file.c`, fetched):
`FileRenameInformation`/`FileRenameInformationEx` builds
`OBJECT_ATTRIBUTES` with `info->RootDirectory` and forwards
`attr.RootDirectory` to `set_fd_name_info`.  This is source-level corroboration,
not a Windows runtime execution claim.

Conclusion: `RootDirectory = NULL` + bare relative name is documented as
cwd-relative, so the old code did not meet "target is always the opened bucket".
The code now uses the documented handle-relative form.

## Verification actually run (Linux host)

From `/home/v9i/Projects/Kala-Agent/backend/crates/kala-artifacts`:

```bash
cargo fmt --all --check
cargo test --locked
cargo test --locked --release
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
cargo check --locked --all-targets --target x86_64-pc-windows-msvc
cargo clippy --locked --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

Results:

- `fmt`: clean.
- `cargo test --locked` (debug): 40 tests, 0 failed (4 lib, 4 concurrency
  including spawned process workers, 3 live Python cross-reader, 12 security,
  17 semantics).
- `cargo test --locked --release`: same 40 tests, 0 failed.
- Linux all-target clippy `-D warnings`: clean.
- Release build: clean.
- Windows all-target `check` (`x86_64-pc-windows-msvc`): clean, including new
  Windows-only tests and unit tests.
- Windows all-target clippy `-D warnings`: clean.
- `tests/windows_production.rs` and `tests/windows_security.rs` execute 0 tests
  on Linux by construction (`#![cfg(windows)]`); this is **not** execution
  evidence.

No Windows runtime is available here.  Windows production tests, the
fallback-boundary unit test, the held-bucket rename test and the symlink tests
are compiled but **not executed**.  No fake green claim is made.  The Linux
suite still passes unchanged, including the real Python cross-readers and
hostile symlink/bucket-swap tests accepted by REVIEW-1.

## Touched paths (exact, round 2)

Modified:

```
backend/crates/kala-artifacts/src/sys/windows.rs
backend/crates/kala-artifacts/README.md
backend/crates/kala-artifacts/tests/windows_security.rs
.mece/waves/rust-backend-20261019/cells/artifacts/REPORT.md
```

Added:

```
backend/crates/kala-artifacts/tests/windows_production.rs
.mece/waves/rust-backend-20261019/cells/artifacts/SESSION-R2.md
```

No other path was edited.  `Cargo.toml`, `Cargo.lock`, `src/lib.rs`,
`src/error.rs`, `src/sys/mod.rs`, `src/sys/unix.rs`, `src/sys/unsupported.rs`,
the Unix/API tests, `tests/python_oracle.py`, all Python/product files, root
manifests/locks and other crates are untouched.  Cargo output remains in
git-ignored `target/`.

## Open items handed back

- Windows runtime execution is still pending; no NTFS/rename runtime proof is
  claimed for this round.
- Symlink-hostile Windows tests are opt-in (`--ignored`) and fail loudly if a
  fixture cannot be created.
- Directory fsync and DACL limitations are unchanged and documented.
- The fallback-boundary test uses a `#[cfg(test)]` seam because the real ACL
  trigger needs a Windows host; the production validation it exercises is
  unconditional.
