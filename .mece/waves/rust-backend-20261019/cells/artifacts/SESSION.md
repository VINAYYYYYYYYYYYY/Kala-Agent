# Cell session notes — secure native content-addressed artifact store

- Session: OpenCode Go `opencode-go/deepseek-v4.1-flash` variant **max**,
  session id `ses_eab67d309ffeSQsjPk3vMTFZ7a` (direct worker invocation; no T3
  child task ID was provided to this worker).
- Reviewer (not run here): GPT-6.1-Sol.
- Date: 2026-10-19. Working tree root: `/home/v9i/Projects/Kala-Agent`.
- Constraints honored: no delegation/subagent, no commit/push/deploy, no
  root-manifest/lock edits, no other-crate edits, no Python/product/test/
  packaging/frontend edits, no fixture/generator edits, temporary directories
  only.

## What was read first

- `WAVE.md`, `SYNTHESIS.md`, `REVIEW.md` for the migration state, model policy
  and the persistence review guidance (stored objects use the larger
  `CanonicalJsonObject` domain; digests must not silently weaken).
- The actual Python source of record:
  `src/kala_agent/persistence/artifacts.py`,
  `src/kala_agent/contracts/models.py` (`_normalize_digest`, `Sha256Digest`),
  `src/kala_agent/config.py` (`Settings.artifact_dir`),
  `src/kala_agent/application.py` (how the store is constructed/used).
- The actual Python tests: `tests/unit/test_artifacts_branches.py`,
  `tests/security/test_artifact_concurrency.py`,
  `tests/unit/test_persistence.py` (artifact sections),
  `tests/integration/test_mechanical_loop.py` (digest/size assertions), and
  ADR-0004 (layout, modes, hard-link publication, durability claims).
- Accepted native crates for reuse/contract: `backend/crates/kala-json`
  (`Sha256Digest`, chain/limits conventions) and `kala-persistence` (test and
  error-reporting style, standalone-workspace precedent for excluded crates).

Critical reading of the brief: the Python implementation is genuinely
descriptor-relative on Linux, but it re-opens `<root>/objects` by path each
operation and only `lstat`s the bucket *after* publication. The native
implementation keeps the root handle and an `objects` identity and validates
both, which is strictly stronger; that deliberate difference is recorded as a
review question in `REPORT.md` (item 7).

## Order of work

1. Inspected cache/registry and targets; `rustix 1.1.5`, `windows-sys 0.61.2`,
   `getrandom 0.3.4`, `sha2 0.10.9`, `thiserror 2.0.21` and `tempfile 3.27.0`
   were already cached. Added the `x86_64-pc-windows-msvc` rust-std target with
   `rustup target add` (tooling only; no product change) so the Windows module
   could be type-checked.
2. Created the standalone crate and implemented the platform-neutral API,
   digest normalization, layout/limits/errors.
3. Implemented `sys::unix` with `rustix` (`openat`/`mkdirat`/`statat`/
   `fstat`/`linkat`/`unlinkat`/`fsync`, `O_NOFOLLOW|O_EXCL|O_NONBLOCK|…`).
4. Implemented `sys::windows` with `windows-sys`: `CreateFileW` with
   `FILE_FLAG_OPEN_REPARSE_POINT`, `NtCreateFile` with a parent
   `RootDirectory`, `FileAttributeTagInfo`/`FileIdInfo` identity,
   `FlushFileBuffers`, `SetFileInformationByHandle(FileRenameInfo, no
   replace)` and `FileDispositionInfo` cleanup.
5. Wrote the integration suites (semantics, security, concurrency, live Python
   cross-reader, Windows-only reparse tests) and iterated against failures.
6. Hardened flaky points: a classifier for `ENOTDIR` vs `ELOOP` symlink
   refusals, a deterministic planted-symlink seed for the bucket-swap race
   (30/30 repeats), tolerant recoveries in the object-swap reader test, and a
   final 5× full-suite repeat.
7. Ran the full verification matrix, then wrote `REPORT.md`/`SESSION.md`.

## Touched paths (exact)

New, owned:

```
backend/crates/kala-artifacts/Cargo.toml
backend/crates/kala-artifacts/Cargo.lock
backend/crates/kala-artifacts/README.md
backend/crates/kala-artifacts/src/lib.rs
backend/crates/kala-artifacts/src/error.rs
backend/crates/kala-artifacts/src/sys/mod.rs
backend/crates/kala-artifacts/src/sys/unix.rs
backend/crates/kala-artifacts/src/sys/windows.rs
backend/crates/kala-artifacts/src/sys/unsupported.rs
backend/crates/kala-artifacts/tests/support/mod.rs
backend/crates/kala-artifacts/tests/semantics.rs
backend/crates/kala-artifacts/tests/security.rs
backend/crates/kala-artifacts/tests/concurrency.rs
backend/crates/kala-artifacts/tests/cross_reader.rs
backend/crates/kala-artifacts/tests/python_oracle.py
backend/crates/kala-artifacts/tests/windows_security.rs
.mece/waves/rust-backend-20261019/cells/artifacts/REPORT.md
.mece/waves/rust-backend-20261019/cells/artifacts/SESSION.md
```

Untouched (verified by `git status --short backend`): `backend/Cargo.toml`,
`backend/Cargo.lock`, `backend/crates/kala-json/**` (read-only path dep),
`backend/crates/kala-persistence/**`, `kala-studio`, `kala-contracts`, all
Python sources/tests, fixtures, packaging and studio/frontend files. Cargo
build output lives in git-ignored `target/` directories.

Deletions/moves: none. A temporary (not delivered) attempt to add a
`cargo check` on the excluded `kala-artifacts` path is not part of the
workspace; the parent manifest was never edited.

## Verification actually run

```bash
cd backend/crates/kala-artifacts
cargo fmt --all --check
cargo test --locked
cargo test --locked --release
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
cargo check --locked --all-targets --target x86_64-pc-windows-msvc
cargo clippy --locked --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

Results: fmt clean; 40 native tests pass in debug and release (4 unit, 17
semantics, 12 security, 4 concurrency including 4 spawned process workers, 3
live Python cross-reader); clippy clean on both targets; release build clean;
Windows all-target check clean. Flake evidence: bucket-swap test 30/30,
full suite 5/5. Python reference
`tests/unit/test_artifacts_branches.py + tests/security/test_artifact_concurrency.py`
passes 59/59 unchanged.

## Open items handed to review

- Windows runtime execution (including reparse-point tests) is pending a
  Windows host; only cross-compilation was possible.
- Windows directory fsync and ACL guarantees are explicitly weaker and are
  documented rather than claimed.
- The stricter root/objects identity semantics (`ObjectRootReplaced`, held
  root handle) need a reviewer decision.
- No performance measurement or cutover is claimed; the crate is not yet a
  member of the shared `backend` workspace (integration is a later cell).
