# Native Rust Studio service — cell report (round 4)

**Status: round-4 implementation complete; ready for GPT-6.1-Sol source-diff
review and independent verification. Native STEP remains independently
accepted and is unchanged; this round repairs the two REVIEW-3 settings
data-preservation defects and removes the unused Windows trust-store
duplicate.**
The nine `/api/studio` routes, boundary middleware, settings/token stores,
static UI serving, deterministic local provider, fixed-endpoint remote
provider and desktop CLI stay as implemented in rounds 1–3 (see
`SESSION.md`/`SESSION-R2.md`/`SESSION-R3.md` and the review trail in
`REVIEW-1.md`, `TASK-R2.md`, `TASK-R3.md`, `REVIEW-2.md`, `REVIEW-3.md`).
Round 3 delivered, inside `backend/crates/kala-studio/**` only:

1. the production `StepExporter` adapter over the reviewed `kala-step`
   native analytic B-rep writer, wired as the real default (`/health`
   `step_available: true`, real ISO 10303-21 bytes from `/export` and
   eligible `/handoff` calls), with the `UnavailableStepExporter` seam kept
   as a compile-time/test alternate;
2. real HTTP STEP re-import verification with the test-only OpenCascade
   oracle (valid exact solids, millimetres, bounds/volume/area, native vs
   Python exporter);
3. honest corpus/replay/differential accounting for structural STEP parity;
4. Python-compatible proxy environment semantics (lowercase-vs-uppercase,
   ignored `ALL_PROXY`, CGI `HTTP_PROXY` suppression, `NO_PROXY`, fixed
   endpoints);
5. Windows `CA`+`ROOT` trust-store loading plus `SSL_CERT_FILE`, without ever
   disabling certificate/hostname verification;
6. settings that preserve valid existing keys containing escaped lone
   surrogates via `kala-json` code points, byte-for-byte like
   `json.dumps(..., ensure_ascii=True)`;
7. a CPython-scanner-compatible `json_invalid` position walk that repairs the
   previously documented malformed-JSON offsets (partial literals, invalid
   escapes, malformed numbers, unterminated strings, trailing commas,
   control characters).

Round 4 delivered, inside `backend/crates/kala-studio/**` only:

1. a CPython `json.loads` projection for the settings store
   (`src/settings_json.rs`) that accepts duplicate object keys with
   last-wins semantics at every level, decodes keys and string values to
   `kala_json::JsonText` code points (escaped lone surrogates survive even in
   the surviving provider value), accepts CPython's `NaN`/`Infinity`/
   `-Infinity` constants and numbers of any magnitude, and is iterative
   (no parser depth cap of its own). The REVIEW-3 file
   `{"api_keys":{},"api_keys":{"openai":"synthetic-review-key"},"unused":"\ud800"}`
   now yields the surviving key; an unrelated save/clear rewrites it
   byte-for-byte like Python;
2. CPython's default 4,300-digit integer-conversion limit as its own failure
   class, in scan order: reads on health/providers/settings/chat become the
   sanitized 500, a settings write is a 422 with the exact CPython
   `ValueError` message and leaves the original file untouched (the reviewed
   strict `kala-json` parser is untouched and still rejects duplicates for
   hash/journal use);
3. removal of the unused duplicated `tls::configure_windows_roots`; the
   Windows-gated trust-store surface is now only `windows_store_certificates`
   plus the small `configure_roots` branch, both used. The full-crate
   Windows cross-check remains blocked by the missing Windows SDK/C toolchain
   for `ring` (recorded in §5), and no Windows runtime claim is made.

No Python runtime, OCP, Kala core, SQLite or Linux sandbox is used by the
service. No Python product/fixture/packaging file was edited, no commit,
push, deploy, packaging cutover or model switch was performed, and no
performance/RSS claim is made.

**Session identity for independent verification:** T3 task
`node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-studio-r4-go-review48`
(child thread of the same name), model `opencode-go/deepseek-v4.1-flash`,
variant **max**. See `LAUNCH-R4.md` and `SESSION-R4.md`. Round 3 was the
previous terminal worker (`…studio-r3-go-retry46`, see `SESSION-R3.md`).

## 1. Changed paths (all inside the round-4 ownership)

Round-4 changed paths under `backend/crates/kala-studio/`:

```
src/settings_json.rs        NEW: CPython json.loads projection for settings
                            (last-wins duplicates, code-point strings,
                            non-finite constants, digit-limit class,
                            iterative walk)
src/settings.rs             primary reader uses the projection; new
                            SettingsReadError { Decode, IntegerLimit };
                            read/write/clear regression tests
src/server.rs               settings read failures log the source exception
                            name (UnicodeDecodeError vs ValueError); all four
                            read routes still map either class to the
                            sanitized 500; writes keep the 422 class
src/lib.rs                  module declaration
src/tls.rs                  removed the unused configure_windows_roots copy
tests/settings_token.rs     real-socket duplicate+surrogate save/clear and
                            integer-limit read/write/boundary regressions
tests/python_differential.py  twenty new live-Python settings cases
                            (duplicates, surrogate fields/values, mixed
                            deep+non-finite, integer-limit reads/writes,
                            4,300-digit boundary)
README.md                   round-4 settings/verification/gap text
REPORT.md / SESSION-R4.md   this evidence
```

`Cargo.toml` and `Cargo.lock` are unchanged (sha256
`5de2012628a80d534632b50caf48a0ad99cb8d065cde85b6cb9e7ef74634a8e6`); no
dependency was added.


### Round-3 changes (carried forward; retained for the review trail)

Modified under `backend/crates/kala-studio/` in round 3:

```
Cargo.toml                 features (default "native-step"), optional
                           kala-step path dep, rustls normal dep,
                           cfg(windows) schannel dep, serde dev-dep
Cargo.lock                 regenerated local lock (sha256 5de20126…)
src/geometry.rs            NativeStepExporter adapter, StepError::Invalid,
                           default_step_exporter(), truthful seam kept
src/server.rs              default exporter, 422 mapping for adapter
                           Invalid, environ+stored credential plumbing
src/cli.rs                 default exporter for the real binary
src/lib.rs                 module/doc updates
src/settings.rs            JsonText code-point store, surrogate-preserving
                           reader/writer, Python-semantics parse fallback
src/json_input.rs          normalise_python_values (constants + non-finite
                           literals → 0) with the existing offset scanner
src/schemas.rs             json_invalid position scanner for serde failures
src/providers/mod.rs       stored-aware configured checks, dispatch carries
                           the stored keys
src/providers/remote.rs    stored credential precedence (surrogate → 500),
                           per-request Python proxy resolution, per-proxy
                           cached clients, TransportError::Internal
README.md                  round-3 behaviour and gaps
```

New under `backend/crates/kala-studio/`:

```
src/proxy.rs               CPython getproxies_environment /
                           proxy_bypass_environment port + unit tests
src/tls.rs                 Windows CA+ROOT store enumeration, SSL_CERT_FILE
                           parsing, add_der_roots root assembly
src/json_diagnostics.rs    CPython-scanner-compatible error offsets
tests/step_export_ocp.rs   real HTTP STEP reimport via test-only OCP
tests/step_ocp_oracle.py   test-only OCP oracle (never product code)
```

Tests updated: `tests/common/mod.rs` (native default + explicit
unavailable helper), `tests/geometry_exports.rs`, `tests/corpus_fixture.rs`,
`tests/settings_token.rs`, `tests/tls_trust.rs`,
`tests/python_differential.py`, `tests/replay_binary_corpus.py`.

Docs updated: this `REPORT.md` and `SESSION-R3.md`.

Read-only and untouched: `backend/Cargo.toml`, `backend/Cargo.lock`,
`backend/crates/kala-json/**`, `kala-geometry/**`, `kala-step/**`,
`kala-budget/**`, `kala-persistence/**`, `backend/fixtures/**`,
`backend/compat/**`, all Python product/tests, `Rust+buid123/`,
`rust-implementation.md`, docs, adapters, packaging, desktop/frontend. The
crate builds with its own lock/target.

## 2. STEP integration

- **Adapter.** `geometry::NativeStepExporter` implements the existing
  `StepExporter` trait by calling `kala_step::export_step_primitive` with
  the same validated `PrimitiveKind`/parameters the route already produced.
  `kala_step::StepError::Invalid` maps onto the route's real 422
  `invalid_request` branch (Python `ValueError` text), `Export` onto the
  sanitized 500. The writer ignores `workdir` by design, so no temporary
  file exists.
- **Default.** `geometry::default_step_exporter()` returns the native
  adapter when the default `native-step` feature is on (library default,
  `StudioConfig::default()`, the CLI and the test harness), and
  `UnavailableStepExporter` when the feature is off. The unavailable path is
  still exercised by `TestServer::start_unavailable_step`, the dedicated
  real-socket test, the mock plumbing test, and the 42-exact/6-dependency
  corpus mode under `--no-default-features`.
- **Contracts preserved.** Request validation, the wide-integer
  `OverflowError` sanitized-500 branch, flat error envelopes, download
  filenames/media types, `x-studio-handoff` length consistency and the
  security header set are unchanged; the new test
  `step_export_keeps_validation_overflow_and_error_contracts` asserts them
  for the STEP branch (422 invalid dimension, 422 non-finite, 500 overflow,
  409 unsupported handoff capability).
- **Real bytes.** Every HTTP STEP body is a deterministic ISO 10303-21 file
  (`ISO-10303-21;` … `END-ISO-10303-21;\n`, exactly one
  `MANIFOLD_SOLID_BREP`, ASCII) with millimetre units. A second export of the
  same request is byte-identical.
- **No product dependency.** `ldd target/release/kala-studio` resolves only
  `libgcc_s.so.1`, `libm.so.6`, `libc.so.6` and the loader; the binary
  contains no `python`, `STEPControl` or `BRepPrimAPI` strings.

### 2.1 Test-only OCP re-import evidence

`tests/step_export_ocp.rs` starts the service on a real loopback socket,
captures `POST /api/studio/export` for box/cylinder/sphere (defaults plus a
non-cube) and one `POST /api/studio/handoff` (solidworks), and runs
`tests/step_ocp_oracle.py` under the repository virtualenv. Measured on this
host with CPython 3.14.8 and OCP 8.0.1.1.0, 5 cases, 0 failures:

- `IFSelect_RetDone` read, transfer roots > 0, `BRepCheck_Analyzer` valid;
- exactly 1 solid, 1 shell, `BRepCheck_Shell::Closed` and `Orientation`
  `BRepCheck_NoError`, 0 free edges;
- exact analytic surface histograms (`plane×6`; `cylinder×1 + plane×2`;
  `sphere×1`);
- volume/area within `1e-9` relative of the analytic value **and** of the
  live Python OCP exporter for the same primitive; bounds within `1e-6` mm
  (`Bnd_Box` pads by the 1e-7 shape tolerance). This is what proves the
  millimetre unit is not misread.

The oracle is mandatory: a missing interpreter/OCP fails the test; the only
opt-out is the loudly reported `KALA_STUDIO_SKIP_OCP_ORACLE=1`.

## 3. Corpus, replay and differential accounting

No fixture was rewritten, skipped or re-classified. The two STEP downloads
carry `body_contract: "structural"` and `byte_equality_contract: false` in
the frozen fixture itself, so they are asserted against the fixture's own
stable fields (ISO prefix, one manifold solid, trailer, stable headers, a
self-consistent `x-studio-handoff` length); their informational OCP
timestamp/digest are explicitly not compared.

| Surface | Accounting |
|---|---|
| Library corpus (`tests/corpus_fixture.rs`, real socket) | **48/48: 46 exact + 2 structural STEP**; every health body (including `step_available: true`) is byte-exact |
| Release binary replay (`tests/replay_binary_corpus.py`) | 48 cases: **44 exact + 2 structural STEP + 2 skipped**; the two skips are the 16,384-byte body-cap configuration cases that the library replays exactly at that limit |
| Live Python differential (`tests/python_differential.py`) | **268 cases, 0 mismatches** (248 in round 3); STEP downloads compared structurally with both sides re-checked as real ISO solids |
| Monkeypatched provider oracle (`tests/python_provider_dto_oracle.py`) | **20 cases, 0 failures** (round-2 evidence stays green) |
| Python baseline Studio tests (unmodified source) | **181 passed** |

Round-3 additions to the live differential: 49 malformed-JSON cases (the
enumerated partial literals, invalid `\q`/`\x`/`\u12` escapes, malformed
numbers `1.`/`1e`/`1e+`/`--1`/`01`/`0x1`/`1.e5`, unterminated strings,
trailing/double commas, extra data, raw control characters, `NaN`/`Infinity`
boundaries, constant-before-later-error, BOM-only), 5 STEP structural cases
plus an invalid-dimension STEP case, and the 8-case escaped-surrogate
settings sequence whose final `studio-settings.json` bytes are compared
between the Python and native stores.

Round-4 additions to the live differential (20 cases): the duplicate +
surrogate settings sequence (read/health/providers, unrelated write, read,
clear, read; final file bytes compared), the mixed duplicate + surrogate +
deep + non-finite valid file (read, write, read, clear, read), and the
4,301-digit integer file (settings/health/providers reads as sanitized 500s,
the refused 422 write with the exact CPython detail, a second read) followed
by the accepted 4,300-digit boundary and its clear. No fixture was touched.

## 4. Proxy environment compatibility

`src/proxy.rs` ports `urllib.request.getproxies_environment()` and
`proxy_bypass_environment()`; `ReqwestTransport` disables reqwest's own
environment matcher (`no_proxy()`) and resolves the proxy per request from
the process environment (Python builds a fresh opener per request), caching
one client per proxy URL. Endpoint URLs stay the hard-coded
OpenAI/OpenRouter constants; no environment value can change them.

Verified:

- lowercase `https_proxy` wins over an uppercase `HTTPS_PROXY`; an empty
  lowercase value removes the uppercase one (real local proxy capture);
- `ALL_PROXY`/`all_proxy` are recorded but never selected for HTTPS;
- uppercase `HTTP_PROXY` is suppressed when `REQUEST_METHOD` is present,
  lowercase `http_proxy` survives (unit test of the exact pass order);
- `NO_PROXY` `*`, comma lists, leading dots, case-insensitivity and the
  CPython `_splitport` rules (unit tests derived from live `proxy_bypass`
  output, plus the local `NO_PROXY=localhost` bypass over a real proxy);
- malformed proxy URLs surface as the sanitized 500 class, not a silent
  direct connection.

## 5. Windows TLS CA+ROOT trust

`src/tls.rs` reproduces `SSLContext.load_default_certs()` semantics:

- **Unix/macOS unchanged:** the reviewed `rustls-tls-native-roots` default
  (OS store; OpenSSL defaults plus `SSL_CERT_FILE`/`SSL_CERT_DIR`) is kept.
- **Windows:** the built-in set is replaced by explicitly assembled DER
  roots from the current-user **and** local-machine **`CA` and `ROOT`**
  stores through the safe, supported `schannel` CryptoAPI binding (filtered
  by server-auth usage and time validity like `rustls-native-certs` does for
  `ROOT`), plus PEM/DER certificates from `SSL_CERT_FILE`. Damaged entries
  are skipped (CPython warns and continues). Certificate and hostname
  verification are never disabled.
- **Executed evidence on Linux:** `add_der_roots` is the exact assembly the
  Windows branch uses and is platform-independent; `tests/tls_trust.rs`
  runs it against the synthetic local HTTPS server — the supplied CA is
  trusted, a CA outside the supplied set is refused, hostname mismatch is
  refused, and a damaged DER entry is skipped rather than fatal.
- **Windows compile cross-check (round 4):** the duplicate unused
  `configure_windows_roots` was removed; the remaining `cfg(windows)` surface
  is the `windows_store_certificates()` body (the only Windows-API code) plus
  the small `configure_roots` branch, which only calls the platform-independent
  `add_der_roots`/`environment_certificates` helpers. The exact store body was
  recompiled for `x86_64-pc-windows-msvc` in a scratch crate against the same
  `schannel` 0.1.30/windows-sys 0.61.2 versions as the workspace lock with
  `RUSTFLAGS=-D warnings`: **clean**.
  (The rustup `x86_64-pc-windows-msvc`/`x86_64-pc-windows-gnu` std targets
  were installed for this check; no repository file changed.)
  A full-crate Windows check is blocked on this Linux host by the missing
  cross C toolchain for `ring` (`cc-rs: failed to find tool "lib.exe"` for
  MSVC, `x86_64-w64-mingw32-gcc` for GNU), so the full binary was not
  cross-compiled and **no Windows runtime execution is claimed**.

## 6. Settings: CPython `json.loads` semantics

Round 4 replaces the settings fallback with a dedicated CPython-compatible
projection (`src/settings_json.rs`) and keeps the reviewed parsers only as a
safety net when the projection reports malformed input. Measured behaviour:

- **Duplicates are last-wins at every level.** Python accepts
  `{"api_keys":{},"api_keys":{"openai":"synthetic-review-key"},"unused":"\ud800"}`
  and reads `openai` from the last `api_keys` member. Native now returns the
  same map. Duplicate provider keys inside the surviving object are last-wins
  too, including a later non-string value (which skips only that provider,
  like `isinstance(candidate, str)` in the source). The REVIEW-3
  "unrelated save loses the key" scenario is covered by a real-socket test
  and by the live differential (`settings.dup_surrogate_*`).
- **Escaped lone surrogates survive in ignored fields and in the surviving
  provider value.** Keys and values are decoded to `kala_json::JsonText`
  code points; `"\ud83d\ude00"` still combines to one supplementary code
  point and `"\ud83d\u0041"` stays two code points, exactly like CPython.
  Validation keeps Python `str.strip`/`len`/`ord`/`isspace` semantics, and
  the renderer re-emits `json.dumps(indent=2, sort_keys=True,
  ensure_ascii=True)` bytes, so an unrelated save/clear rewrites the file
  byte-for-byte like Python (`settings.dup_surrogate_write/clear` and the
  final settings-bytes comparison in the differential).
- **Extended documents stay accepted.** `NaN`/`Infinity`/`-Infinity` and
  overflowing float literals are values, and the walk is iterative, so a
  deep unrelated value (400 levels in the tests) does not discard the keys.
  The mixed duplicate + surrogate + deep + non-finite document is exercised
  by a unit test and the live `settings.mixed_valid_*` sequence.
- **The 4,300-digit integer conversion limit is a distinct failure class.**
  `json.loads` raises `ValueError` (not `JSONDecodeError`) at the moment an
  over-long integer token converts, so the source does not return `{}`:
  `GET` health/providers/settings/chat are the sanitized 500, and
  `POST /settings` is a 422 whose detail is the exact CPython text
  (`Exceeds the limit (4300 digits) for integer string conversion: value has
  4301 digits; use sys.set_int_max_str_digits() to increase the limit`).
  The write refuses before the atomic replacement, leaving the original file
  byte-identical. The parser reports this in Python's scan order, so a big
  integer before a later syntax error still wins, while an earlier syntax
  error stays `JSONDecodeError`/`{}`. Exactly 4,300 digits is accepted.
  Verified by `settings::tests::integer_limited_settings_match_the_source_
  failure_class`, the real-socket
  `integer_limited_settings_match_the_source_failure_class_on_the_routes`
  test, and the live `settings.integer_limit_*` / `settings.at_limit_*`
  cases.
- **No weakening of the strict parser.** `backend/crates/kala-json` is
  untouched and still rejects duplicates for hash/journal use; the settings
  projection only feeds the credential store. A malformed document still
  falls back to the reviewed `serde_json`/`kala-json` readers, and a document
  no parser accepts is `{}`, exactly like `except json.JSONDecodeError`.
- **Legacy behaviour retained.** Missing/unreadable files are `{}`;
  non-UTF-8 files keep the source `UnicodeDecodeError` class; atomic
  replacement, `0600`, symlink refusal and the no-size-cap read are
  unchanged.

## 7. Malformed-JSON diagnostic offsets

`src/json_diagnostics.rs` is a CPython-scanner-compatible walk
(iterative, no recursion) that returns CPython's first-error character
offset for partial literals (`tru`→0, `[tru]`→1), invalid escapes
(`\q`→backslash, `\u12`→`u`), unterminated strings (opening quote), malformed
numbers (`1.`/`1e`/`1e+`→1, `--1`→0, `01`→1, `1.e5`→1) and trailing commas
(the comma). It is used only when `serde_json` already rejected the body, so
rejection, status and the reviewed constant-token paths are unchanged; the
`kala-json` hash parser and canonical bytes were not touched. All 49 live
differential cases (including the previously documented mismatches) now
compare byte-identical 422 bodies with the live Python app.

## 8. Checks actually run

Host: Linux x86-64 (7.2.8-9-omarchy), rustc/cargo 1.99.1, CPython 3.14.8 in
the repository venv, OCP 8.0.1.1.0 for the test-only oracle. Crate
`kala-studio` 0.1.0, edition 2024. Locked deps include hyper 1.12.0,
tokio 1.53.3, reqwest 0.12.28, rustls 0.23.46, rustls-native-certs 0.8.5,
serde_json 1.0.154, plus path-reviewed `kala-json` 0.1.0,
`kala-geometry` 0.1.0 and `kala-step` 0.1.0; `schannel` 0.1.30 is a
`cfg(windows)` dependency.

| Command (crate directory) | Actual result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo test --locked` | pass: **115 tests** (75 lib + 40 integration) |
| `cargo test --locked --release` | pass: same **115 tests** |
| `cargo test --locked --no-default-features` | pass: **113 tests** (native-gated tests excluded; corpus runs its truthful 42+6 mode) |
| `cargo clippy --all-targets --locked -- -D warnings` | pass |
| `cargo clippy --all-targets --locked --no-default-features -- -D warnings` | pass |
| `cargo build --release --locked` | pass; ELF x86-64 `target/release/kala-studio` (6,941,744 bytes, sha256 `5b61ac8d7a5a4af9eeccca3c04049a1b22984dd6d1b65cad2f4d747c53131ddf`) |
| release-binary corpus replay | **48 accounted**: 44 exact + 2 structural STEP + 2 body-cap config cases |
| library corpus replay (16,384-byte cap) | **48 accounted**: 46 exact + 2 structural STEP (the 2 body-cap cases exact here) |
| live Python differential | **268 cases, 0 mismatches** (round 3: 248; 20 new settings cases) |
| monkeypatched provider DTO oracle | **20 cases, 0 failures** |
| HTTP STEP OCP re-import | **5 cases, 0 failures** (unchanged; see §2.1) |
| Python baseline Studio tests (unmodified) | **181 passed** |
| Windows-gated store code compile check | `x86_64-pc-windows-msvc` scratch check of the exact remaining `windows_store_certificates` body against schannel 0.1.30 with `RUSTFLAGS=-D warnings`: clean |
| full-crate Windows `cargo check` | **blocked, not claimed**: fails in `ring`'s `cc-rs` with `failed to find tool "lib.exe"` (missing Windows SDK/C toolchain); see §5 |
| release binary linkage/strings | `ldd` libc/libm/libgcc only; no python/OCP strings |

Auxiliary out-of-tree probe (not part of the repository test suite): 1,452
generated and single-point-mutated settings documents were compared against
live CPython `json.loads` (JSONDecodeError → malformed, ValueError →
integer-limit class, success → last-wins `api_keys` projection with code
points), including duplicate root/nested maps, escaped-surrogate
permutations, non-finite constants and the 4,300-digit boundary: **0
mismatches**. The exact REVIEW-3 probes were also re-run against the release
binary: the duplicate+surrogate file read `200` with configured
`[true, false]` and the unrelated write preserved the surviving key
byte-for-byte; the 4,301-digit file read the sanitized `500` and the write
was a `422` with the exact CPython detail while the original file stayed
byte-identical. This complements the in-repo unit matrix, real-socket tests
and the 268-case live differential.

## 9. Honest gaps and deviations

1. **Windows runtime proof absent.** No Windows host was available. The
   store enumeration compiles for the MSVC target but was never executed;
   the enterprise/group-policy/services/`USERS` store locations and
   `SSL_CERT_DIR` that CPython also reads on Windows are not loaded here
   (current-user + local-machine `CA`/`ROOT` + `SSL_CERT_FILE` are). The
   full-crate Windows cross-compile is blocked by the missing C toolchain
   for `ring` (exact errors recorded in §5). No COM/Solid Edge/installer
   claim is made.
2. **Windows registry proxy not implemented.** When no environment proxy is
   configured, CPython on Windows falls back to the WinINET registry
   settings (`getproxies_registry`); the native transport goes direct. The
   environment rules that TASK-R3 enumerated are exact.
3. **Lone-surrogate request bodies** remain `json_invalid` 422 instead of
   Pydantic's field-level `string_unicode`/`extra_forbidden` detail (status
   unchanged); provider replies and stored settings are handled as
   documented. `serde_json`'s 128-level syntactic recursion limit still
   differs from CPython's process-dependent threshold for HTTP request
   bodies.
4. **Settings depth beyond CPython's scanner stack.** The settings
   projection is iterative and has no depth cap of its own. CPython's C
   scanner raises `RecursionError` (not `JSONDecodeError`, so the source does
   not catch it) beyond its environment-dependent stack threshold; such an
   extreme-depth file would be a sanitized 500 in Python but is still read by
   the native store. Depth 400 and 5,000 documents are accepted by both.
5. **Settings integer limit is the CPython default.** The native store fixes
   the conversion limit at CPython's default 4,300 digits
   (`sys.int_info.default_max_str_digits`). A source interpreter launched
   with a non-default `PYTHONINTMAXSTRDIGITS` would raise at a different
   boundary; the shipped Studio does not set that variable.
6. **STEP bytes are not OCP bytes.** The native document is deterministic
   (fixed `FILE_NAME` timestamp, its own entity ordering) while OpenCascade
   embeds a timestamp; the fixture itself declares byte equality is not the
   contract, and the OCP re-import proves the same exact solid in
   millimetres.
7. **Static MIME subset, network error class names/reason phrases and the
   `clap`-vs-argparse help text** remain as documented after round 2;
   `recursion-limit` and surrogate request-body diagnostics are unchanged
   from the round-2 gap list.
8. No performance, RSS, startup or packaging claim is offered; the
   conductor runs the controlled benchmarks after worker quiescence.

## 10. Integration notes for conductor

- The crate owns only its `[workspace]`/`Cargo.lock`/`target`; the parent
  workspace and lock remain persistence-owned and untouched. `kala-step` is
  a new path dependency of this crate only; its own workspace/lock are
  unchanged and still buildable.
- To integrate, add `crates/kala-studio` to the parent workspace and wire
  the desktop spawn contract (`resources/backend/kala-studio[.exe]`,
  `--host 127.0.0.1 --port <port>`, `KALA_STUDIO_TOKEN`). Packaging remains
  deliberately uncut.
- Review focus: the new `src/settings_json.rs` CPython projection (last-wins
  duplicates, code-point strings, scan-order 4,300-digit `ValueError` class,
  iterative depth) and its `SettingsReadError` wiring; the STEP adapter and
  OCP re-import evidence (unchanged and accepted); the proxy resolver against
  the live `urllib` functions; `add_der_roots` plus the remaining
  Windows-gated store code; and the scanner positions in
  `json_diagnostics.rs`.
