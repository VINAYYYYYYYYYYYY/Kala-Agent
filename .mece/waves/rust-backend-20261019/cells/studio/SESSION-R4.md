# Studio round 4 worker — settings data preservation

T3 task:
`node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-studio-r4-go-review48`
Child thread: same name.
Model: `opencode-go/deepseek-v4.1-flash`, variant **max**.
Implementation sub-agent session; GPT-6.1-Sol remains the reviewer/verifier.
The round-3 Studio worker is terminal with no live turns; the production STEP
adapter and its independent acceptance are untouched. No delegation, model
switch, commit, push, deploy or packaging cutover was performed.

State: implementation complete for TASK-R4; awaiting source-diff review and
independent verification.

Delivered (only `backend/crates/kala-studio/**` plus this cell's
`REPORT.md`/`SESSION-R4.md`):

- `src/settings_json.rs` (new): CPython `json.loads` projection for
  `studio-settings.json`. Full strict grammar evaluated iteratively,
  duplicate object keys last-wins at every level, keys and string values
  decoded to `kala_json::JsonText` code points (escaped lone surrogates
  survive, valid pairs combine), `NaN`/`Infinity`/`-Infinity` and numbers of
  any magnitude are values, and CPython's default 4,300-digit integer
  conversion limit is reported in scan order with the exact `ValueError`
  text. Only the last root `api_keys` object and the last supported provider
  string values are projected; everything else is validated and ignored.
- `src/settings.rs`: the projection is the primary reader; the reviewed
  `serde_json`/`kala-json` path remains only as a safety net for documents
  the projection reports malformed. New `SettingsReadError`
  (`Decode`/`IntegerLimit`) exposes the source exception name and message.
  Duplicate + surrogate read/rewrite/clear and integer-limit read/write
  unit regressions added.
- `src/server.rs`: health/providers/settings/chat read failures log the
  source class (`UnicodeDecodeError` vs `ValueError`) and still return the
  sanitized 500; settings writes refuse before the atomic replacement and
  return the source 422 detail. No response bytes changed for existing
  passing cases.
- `src/tls.rs`: removed the unused `configure_windows_roots` duplicate left
  after `configure_roots` moved to `add_der_roots`. The remaining
  Windows-gated surface is `windows_store_certificates` plus the small
  `configure_roots` branch, both used.
- `src/lib.rs`, `README.md`: module declaration and round-4 evidence text.
- `tests/settings_token.rs`: real-socket regressions for the REVIEW-3
  duplicate+surrogate file (reads, health, providers, unrelated write,
  clear, byte-exact rewrites) and the 4,301-digit file (four read routes
  sanitized 500, refused 422 write leaving bytes untouched, 4,300-digit
  boundary accepted). Synthetic credentials only.
- `tests/python_differential.py`: 20 new live-Python cases (duplicate +
  surrogate read/write/clear with final file-byte comparison; mixed
  duplicate + surrogate + deep + non-finite read/write/clear;
  integer-limit reads/write/read and the 4,300-digit boundary). No fixture
  changed.

Checks (from `backend/crates/kala-studio`):

- `cargo fmt --check` — pass.
- `cargo test --locked` — **115 passed** (75 lib + 40 integration).
- `cargo test --locked --release` — **115 passed**.
- `cargo test --locked --no-default-features` — **113 passed** (truthful
  unavailable-step build).
- `cargo clippy --all-targets --locked -- -D warnings` and the
  `--no-default-features` variant — pass.
- `cargo build --release --locked` — pass; `target/release/kala-studio`
  6,941,744 bytes, sha256
  `5b61ac8d7a5a4af9eeccca3c04049a1b22984dd6d1b65cad2f4d747c53131ddf`.
- `tests/replay_binary_corpus.py` — 48 accounted: 44 exact + 2 structural
  STEP + 2 body-cap configuration cases, 0 mismatches.
- `tests/python_differential.py` — **268 cases, 0 mismatches** (248 in
  round 3).
- `tests/python_provider_dto_oracle.py` — 20 cases, 0 failures.
- HTTP STEP OCP re-import (`tests/step_export_ocp.rs`, unchanged) — 5
  cases, 0 failures.
- Python baseline Studio tests (unmodified source:
  `tests/unit/test_studio_*.py tests/security/test_studio_security.py`) —
  **181 passed**.
- Auxiliary out-of-tree parser probe (not a repository test): 1,452
  generated/mutated settings documents compared against live CPython
  `json.loads` (malformed vs integer-limit vs last-wins projection) — **0
  mismatches**.
- Exact REVIEW-3 probes re-run against the release binary: duplicate +
  surrogate file `200`/configured `[true,false]` with the unrelated write
  preserving the key byte-for-byte; the 4,301-digit file sanitized `500` read
  and `422` write with the exact CPython detail, original bytes unchanged.
- Windows-gated store function: exact remaining
  `windows_store_certificates` body compiled for `x86_64-pc-windows-msvc`
  against schannel 0.1.30 with `RUSTFLAGS=-D warnings`: clean. Full-crate
  Windows `cargo check` still fails in `ring`'s `cc-rs` on the missing MSVC
  toolchain/SDK (`failed to find tool "lib.exe"`); no Windows runtime claim
  is made.
- `Cargo.toml`/`Cargo.lock` unchanged (lock sha256 `5de20126…`); no new
  dependency.

Residual gaps are those in `REPORT.md` §9: no Windows runtime proof, no
Windows registry proxy, lone-surrogate request-body *detail* (status
unchanged), settings depth beyond CPython's own scanner stack, the
configuration-dependent `PYTHONINTMAXSTRDIGITS` boundary, STEP byte
structure, static MIME subset / network error class names / `clap` help
wording, and no performance or packaging claim.
