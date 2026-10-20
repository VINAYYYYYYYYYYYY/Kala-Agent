# Studio round 3 worker

T3 task:
`node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-studio-r3-go-retry46`
Child thread: same name.
Model: opencode-go/deepseek-v4.1-flash, variant **max**.
Implementation sub-agent session; GPT-6.1-Sol remains the reviewer/verifier.
Predecessor Studio rounds are terminal with no live turns; the STEP cell's
source is read-only and consumed as a path dependency; fixture/Python/
packaging paths were not edited.

State: implementation complete for TASK-R3; awaiting source-diff review and
independent verification.

Delivered:
- `src/geometry.rs`: `NativeStepExporter` adapter over `kala-step`, real
  default via `default_step_exporter()`, `StepError::Invalid` → 422;
  `UnavailableStepExporter` kept for the truthful alternate build/tests.
- `src/proxy.rs` (new): CPython `getproxies_environment` /
  `proxy_bypass_environment` port; `ReqwestTransport` resolves the proxy per
  request from the process environment with per-proxy cached clients.
- `src/tls.rs` (new): Windows current-user+local-machine `CA`/`ROOT` store
  loading through `schannel`, `SSL_CERT_FILE` parsing, platform-independent
  `add_der_roots` assembly; verification never disabled.
- `src/settings.rs`: code-point (`kala_json::JsonText`) stored keys,
  escaped-lone-surrogate preservation and byte-compatible writes,
  Python-semantics fallback parse.
- `src/json_input.rs` + `src/json_diagnostics.rs` (new): Python-compatible
  malformed-JSON error positions.
- `src/providers/{mod,remote}.rs`: stored-key precedence, surrogate
  credential → sanitized 500, proxy/transport updates.
- Tests: `tests/step_export_ocp.rs` + `tests/step_ocp_oracle.py` (real HTTP
  bodies re-imported by test-only OCP), updated corpus/replay/differential
  accounting, settings surrogate route test, proxy/TLS integration cases,
  STEP validation/overflow contract test.

Checks (from `backend/crates/kala-studio`):
- `cargo fmt --check` pass.
- `cargo test --locked` / `--release` — 103 tests, 0 failed.
- `cargo test --locked --no-default-features` — 101 tests, 0 failed
  (truthful unavailable-step build).
- `cargo clippy --all-targets --locked -- -D warnings` pass (default and
  no-default-features).
- `cargo build --release --locked` pass.
- `tests/replay_binary_corpus.py` — 48 accounted: 44 exact + 2 structural
  STEP + 2 body-cap config cases.
- `tests/python_differential.py` — 248 cases, 0 mismatches.
- `tests/python_provider_dto_oracle.py` — 20 cases, 0 failures.
- HTTP STEP OCP re-import — 5 cases, 0 failures (exact solids, mm, metrics
  vs live Python within 1e-9 relative).
- Python baseline Studio tests — 181 passed.
- Windows-gated `windows_store_certificates` body compiles for
  `x86_64-pc-windows-msvc` in a scratch crate; full-crate cross-compile
  blocked by the missing C toolchain for `ring` (no Windows runtime proof).

No Python/product/fixture/packaging file was edited; no commit/push/deploy;
no live model or credential call. Unresolved gaps are enumerated in
REPORT.md §9 (Windows runtime proof, Windows registry proxy, lone-surrogate
request details, settings large-integer class, one narrow duplicate-key +
surrogate fallback combination, OCP byte structure difference, pre-existing
MIME/network-class/CLI wording gaps).
