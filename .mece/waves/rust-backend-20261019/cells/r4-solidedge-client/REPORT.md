# r4-solidedge-client cell report — native lowering and loopback bridge client

Date: 2026-10-19
Cell: `r4-solidedge-client` (wave `rust-backend-20261019`)
Crate: `backend/crates/kala-solidedge-client`
Implementer: OpenCode Go **MiniMax M3.1 Flash (Free)** (worker `r4-solidedge-client`)
Status: implemented and validated in isolation; ready for independent review.
No delegation, provider/model switch, commit, push, publish, deploy, source
deletion, credential read, root-manifest edit, or packaging cutover.

## Scope and ownership

Owned and changed only:

```
backend/crates/kala-solidedge-client/Cargo.toml
backend/crates/kala-solidedge-client/Cargo.lock
backend/crates/kala-solidedge-client/.gitignore
backend/crates/kala-solidedge-client/README.md
backend/crates/kala-solidedge-client/src/lib.rs
backend/crates/kala-solidedge-client/src/errors.rs
backend/crates/kala-solidedge-client/src/protocol.rs
backend/crates/kala-solidedge-client/src/compile.rs
backend/crates/kala-solidedge-client/src/client.rs
backend/crates/kala-solidedge-client/src/http.rs
backend/crates/kala-solidedge-client/src/journal.rs
backend/crates/kala-solidedge-client/src/paths.rs
backend/crates/kala-solidedge-client/src/auth.rs
backend/crates/kala-solidedge-client/tests/client_mock_bridge.rs
backend/crates/kala-solidedge-client/tests/compile_differential.rs
backend/crates/kala-solidedge-client/tests/journal_compatibility.rs
backend/crates/kala-solidedge-client/tests/request_contract.rs
backend/crates/kala-solidedge-client/tests/support/mod.rs
backend/crates/kala-solidedge-client/tests/oracle/compile_oracle.py
backend/crates/kala-solidedge-client/tests/oracle/journal_oracle.py
backend/crates/kala-solidedge-client/tests/oracle/request_oracle.py
.mece/waves/rust-backend-20261019/cells/r4-solidedge-client/REPORT.md
```

Not touched: `backend/Cargo.toml` and the root `backend/Cargo.lock`; the
accepted crates `kala-contracts`, `kala-json`, `kala-budget` (read-only path
dependencies); the Python product, the `adapters/solidedge` and
`adapters/solidedge-bridge` packages (read-only references); the shared
fixtures and generators under `tests/fixtures/` (read-only);
`studio/**`; every other cell's crate and report. The uncommitted changes
visible in `git status` for `src/kala_agent/**` and `studio/**` belong to the
concurrent Python and Studio cells and were left alone.

The crate is a **standalone workspace** with its own `Cargo.lock`, like
`kala-contracts`, `kala-process`, `kala-kernel` and the other independent
cells. It is deliberately *not* added to the root workspace manifest.

## What was built

Rust is the connection and lowering layer only, exactly as ADR-0010 and
ADR-0008 require. **No geometry, no volume, no bound, no surface and no STEP
computation was ported.** `oracle.py` and every other geometry computation
stay in the Python geometry worker.

### `compile` — ConstructionScript → bridge operations

A native mirror of `adapters/solidedge/src/kala_agent_solidedge/compile.py`.

- `compile_operations(&ConstructionScript, Option<&AnalysisScript>,
  Option<&BTreeMap<String, f64>>) -> Result<Vec<BridgeOp>, SolidEdgeClientError>`.
- `build_request(&ConstructionScript, request_id, calibration_id,
  timeout_seconds, density_kg_m3)` and the full `build_request_with(...)`,
  which accepts an analysis script and fillet radii so a caller can prove they
  are refused rather than silently dropped.
- `new_request_id()` returns 32 lowercase hexadecimal characters
  (`uuid4().hex` shape), from the operating system CSPRNG.

The supported subset and the refusals are the Python ones, with the same
messages, character for character: helix and threads, fuse, compound,
analysis, fillets, a rectangle revolved on `XZ`, a duplicate placement, an
unconsumed sketch, an extrusion without a length, an unknown plane or
profile, and the wire operation cap. Every refusal happens **before a socket
is opened**. The Rust messages reproduce CPython's `repr()` for the quoted
identifiers (`'plate'`, `'fuse'`).

The construction script itself is not restated: `kala-contracts` already
models `ConstructionScript`/`ComponentScript`/`ComponentStep`/`AssemblyStep`
with the same validation, the same profile aliases and the same defaults, and
this crate consumes it read-only.

### `protocol` — bridge protocol v1

A native mirror of `adapters/solidedge-bridge/src/kala_solidedge_bridge/protocol.py`:
`BridgeOp` (the fourteen operations), `BuildRequest`, `Health`,
`BuildAccepted`, `BuildStatus`, `BuildResult`, `PartResult`, `RangeBox`,
`StepTrace`, `Artifact`, `ErrorResponse`, `Plane`, `JobState`,
`ArtifactFormat`, `ErrorCode`, every bound constant, and the procedure
validator with the Pydantic-style messages.

Two encodings matter and both are byte-compatible with Python:

- **canonical JSON** — sorted keys, no whitespace, ASCII escapes, CPython's
  shortest round-trip float `repr` (`kala_json::canonical_dumps`). This is
  the one deterministic digest input.
- **wire JSON** — the Pydantic field order and escaping of
  `model_dump_json()`.

`BuildRequest::request_digest()` is the SHA-256 of the canonical request
bytes with `request_id` removed, so a retry of the same semantic request has
the same digest. `canonical_request_bytes()` and `request_digest()` are
compared against the live Python contract in `tests/request_contract.rs`.

### `client` — the loopback bridge client

- `BridgeClient::new(base_url, token, timeout, artifact_timeout)` and
  `BridgeClient::from_token_file(path, ...)`.
- `health`, `submit`, `status`, `cancel`, `wait`, `wait_with` (injected
  clock and sleeper), `download`.
- `validate_bridge_url` and `BridgeUrl`.

Properties, all exercised by the tests:

- **Loopback only.** `http://127.0.0.1[:port]` and nothing else. A userinfo,
  a query, a fragment, a base path, another host, another scheme, a
  non-numeric port or a port outside `1..=65535` is refused before a socket
  exists. The normalized base URL is returned.
- **Auth.** The bearer token comes from the token file the bridge wrote,
  resolved exactly as the Python client resolves it (flag →
  `KALA_SOLIDEDGE_TOKEN_FILE` → the Windows
  `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token` default). The token is
  read as ASCII, trimmed, and never empty. It never appears in a message, a
  URL, a journal record or a log line; the mock-bridge test asserts the token
  is absent from the 401 message. `auth::token_matches` is the constant-time
  comparison of the bridge's `auth.py`.
- **No redirects.** A 3xx answer is a `ProtocolError`; the client never
  leaves loopback.
- **Bounded.** 1 MiB for a JSON document, 256 MiB for an artifact, one
  overall deadline per request, one per download, and one per polling loop.
  `Content-Length` is a hint, never a proof of size: the body is capped while
  it is read.
- **Typed errors.** The bridge's error envelope is mapped exactly like the
  Python client: 401 and 403 are authentication failures, 409 is "busy or
  conflicting", 503 is "not ready", 413 is "too large", 404 is "no such
  build or artifact", a `calibration_required` code is "the host is not
  calibrated", and anything else is "the bridge refused the request (status)".
  An untyped error document is still a typed refusal. The stable wire code is
  kept on the error (`wire_code()`).
- **Ambiguity is honest.** A failed submission is reported as *may or may not
  have reached the bridge*; the caller keeps the request id instead of
  resubmitting.
- **Artifacts are published atomically.** The bytes are streamed to a
  temporary file in the caller's directory, hashed while they are written,
  and renamed into place only after the declared size and the declared
  SHA-256 both match. A mismatch discards the file.

The transport is a small purpose-built loopback HTTP/1.1 client
(`src/http.rs`), not a general HTTP stack: it sends exactly one
`Authorization` header and no `Origin` header, which is what the bridge
requires, and it never logs the token.

### `journal` and `paths`

`journal.rs` is a native mirror of the Python `journal.py`: the same file name
(`journal.jsonl`), the same eight fields, the same limits (200 records, 64
digest characters, 64 id characters, 512 target characters, 65 536 script
characters, 32 status characters, 200 summary characters), the same
skip-the-malformed-line rule, the same newest-last ordering, the same
`find_by_digest` / `find_exact` / `find_by_request` lookups, the same
`append` / `update` semantics, and the same atomic rewrite. The record is one
JSON object per line, keys sorted, no whitespace, ASCII escaped — the
Python client's bytes, not merely its meaning.

`paths.rs` is a native mirror of the Python `paths.py`: the same default
bridge URL, the same environment keys, the same Windows data and run
directories, the same "no Linux default" rule, and the same `safe_build_id`
guard (no separators, no drive or ADS colon, no dot-only name, no reserved
Windows device name, no trailing dot or space).

## Public API

The exhaustive list is in `backend/crates/kala-solidedge-client/README.md`.
In short:

- `compile`: `compile_operations`, `build_request`, `build_request_with`,
  `new_request_id`, `summary`, `ASSEMBLY_NAME`,
  `DEFAULT_BUILD_TIMEOUT_SECONDS`, `MAX_SUMMARY_CHARS`.
- `client`: `BridgeClient` (`new`, `from_token_file`, `base_url`, `timeout`,
  `artifact_timeout`, `health`, `submit`, `status`, `cancel`, `wait`,
  `wait_with`, `download`), `Download`, `BridgeUrl`, `validate_bridge_url`,
  `MAX_JSON_BYTES`, `MAX_ARTIFACT_BYTES`, `DEFAULT_TIMEOUT_SECONDS`,
  `DEFAULT_ARTIFACT_TIMEOUT_SECONDS`.
- `protocol`: `BridgeOp`, `BuildRequest`, `Health`, `BuildAccepted`,
  `BuildStatus`, `BuildResult`, `PartResult`, `RangeBox`, `StepTrace`,
  `Artifact`, `ErrorResponse`, `Plane`, `JobState`, `ArtifactFormat`,
  `ErrorCode`, `ProtocolViolation`, `validate_procedure`, `sha256_hex`, and
  every bound constant.
- `journal`: `Journal`, `JournalEntry`, `JOURNAL_FILENAME`, `MAX_ENTRIES`,
  `MAX_DIGEST_CHARS`, `MAX_ID_CHARS`, `MAX_TARGET_CHARS`, `MAX_SCRIPT_CHARS`.
- `paths`: `resolve_token_file`, `resolve_bridge_url`, `resolve_output_dir`,
  `default_token_file`, `default_output_dir`, `local_app_data`,
  `home_directory`, `is_windows`, `safe_build_id`, `build_output_dir`,
  `environment_snapshot`, `Environment`, `DEFAULT_BRIDGE_URL`,
  `BRIDGE_URL_ENVIRONMENT_KEY`, `TOKEN_FILE_ENVIRONMENT_KEY`,
  `TOKEN_FILENAME`, `DATA_DIR_PARTS`, `RUNS_DIR_PARTS`.
- `auth`: `read_token_file`, `token_matches`, `MIN_TOKEN_CHARS`,
  `MAX_TOKEN_CHARS`.
- `errors`: `SolidEdgeClientError` with `ErrorKind` (`Compile`, `Client`,
  `Authentication`, `Protocol`, `Network`, `Artifact`), `kind()`,
  `message()`, `wire_code()`, `is_compile()`, `is_network()`,
  `MAX_MESSAGE_CHARS`.

## Test counts and results

`cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings`
clean; `cargo test --locked` and `cargo test --release --locked` green;
`cargo check --locked --target x86_64-pc-windows-gnu` green (the
`x86_64-pc-windows-gnu` target **is** installed on this host).

| Suite | Tests | Debug | Release |
| --- | ---: | --- | --- |
| `src/` unit tests | 45 | pass | pass |
| `tests/client_mock_bridge.rs` | 26 | pass | pass |
| `tests/compile_differential.rs` | 1 (54 cases) | pass | pass |
| `tests/journal_compatibility.rs` | 4 | pass | pass |
| `tests/request_contract.rs` | 2 | pass | pass |
| doc test | 1 | pass | pass |
| **total** | **79** | **79 pass** | **79 pass** |

Zero failed, zero ignored in both profiles.

### Differential lowering (byte-exact)

`tests/compile_differential.rs` runs the live repository Python in an
isolated interpreter with the repository sources on the import path:

```text
PYTHONPATH=adapters/solidedge/src:adapters/solidedge-bridge/src:src \
  .venv/bin/python -I backend/crates/kala-solidedge-client/tests/oracle/compile_oracle.py
```

(`-I` ignores `PYTHONPATH`, so the oracle inserts the same three paths
itself; the variable is still exported as the cell task requires.)

The corpus is **54 cases**:

- **28** from the shared parity fixture `tests/fixtures/construction_parity.json`
  (read-only). The Rust side reads the fixture file itself and compiles
  those scripts; the oracle compiles the same scripts. The Rust side also
  asserts that the model it loaded is the model the oracle loaded, so a
  re-serialisation that changed the script would fail loudly.
- **26** from the Python compile tests
  (`adapters/solidedge/tests/test_compile.py`). The oracle imports the live
  test module and rebuilds the cases with the test module's own
  `script()` and `rectangle_component()` helpers, so these are literally the
  Python suite's scripts, not a re-typed copy.

Result: **47 compiled cases and 7 refused cases, all identical to Python.**

- The 47 compiled cases are compared as canonical JSON, **byte for byte**,
  against `json.dumps([op.model_dump(mode="json") for op in ops],
  sort_keys=True, separators=(",", ":"), ensure_ascii=True)`.
- The 7 refused cases are compared **message for message**:

```text
component 'wheel' revolves a rectangle on XZ; the component-local Y axis is not the profile axis, so this case is refused
helix and thread features are not supported by the Solid Edge integration
assembly operation 'fuse' is not supported; version 1 supports only positioned parts, not fuse or compound
assembly operation 'compound' is not supported; version 1 supports only positioned parts, not fuse or compound
analysis operations (material, fix, force, pressure, mesh, solve) are not supported by the Solid Edge integration
fillets are not supported by the Solid Edge integration: plate
component 'a' is placed more than once
```

The comparison is strict in both directions: a Rust refusal where Python
compiled, and a Python refusal where Rust compiled, both fail the test.

### Request bytes and digest

`tests/request_contract.rs` compares, against the live Python contract, the
canonical request bytes, the request digest, and the wire body. All three
are **identical**, including a request with an `XZ` sketch, a reversed
extrusion, a 300-second timeout and a density of 7800 kg/m³. This is what
makes the journal identity (`request_digest`) interchangeable between the two
clients.

### Journal interchange

`tests/journal_compatibility.rs` proves the interchange in both directions:

- the Python `Journal` writes a file, the Rust `Journal` reads it and finds
  the same two records with the same fields;
- the Rust `Journal` writes the same records and the **file bytes are
  identical** to the Python-written file;
- the Python `Journal` reads the Rust-written file and reports the same
  records.

### Mock bridge

`tests/client_mock_bridge.rs` runs the client against an in-process mock
bridge (a real loopback HTTP server) that implements the `protocol.py`
semantics: it records the request, requires a bearer token, and answers
protocol v1 documents. Covered:

- health is authenticated and typed; the `Authorization` header is exactly
  `Bearer <token>`; no `Origin` header is sent;
- a 401 is an authentication error and the token does not leak into the
  message; a 403 likewise;
- a 302 is refused, not followed, and only one request is made;
- a document larger than 1 MiB is refused before it is parsed;
- a non-protocol document and a `protocol_version: 2` document are protocol
  errors;
- submit posts the typed request and the recorded body parses back into an
  equal `BuildRequest`;
- a lost submission is ambiguous, and the message tells the caller to keep
  the request id;
- a slow bridge is a network error inside the deadline;
- status and cancel use the build id, and an unsafe build id is refused
  locally with no request made;
- the error envelope is mapped for 409 (busy), 503 (not ready), 400
  (`calibration_required`), 404 (no such build), and an untyped body;
- download publishes the verified file atomically; a digest mismatch and a
  size mismatch both discard the file; an unsafe artifact name is refused
  locally; an oversize declared artifact is refused before it is written; a
  download that exceeds the deadline is discarded;
- `wait` returns a terminal status without unbounded polling, and the
  polling deadline keeps the build id.

## Remaining gaps and honest limits

1. **No Solid Edge, no Windows, no native build.** There is no Windows host
   and no Solid Edge installation in this repository. Nothing in this cell is
   evidence about Solid Edge, about COM, about the STA worker, or about the
   bridge's own runtime. The client is tested against the wire contract and
   an in-process mock only, exactly as ADR-0010's Solid Edge track requires
   until a host exists.
2. **The bridge runtime itself is still Python.** This crate is the client
   half. The bridge (`adapters/solidedge-bridge`) is unchanged and still
   Python; a native bridge runtime is a separate, later decision.
3. **The wire body is byte-identical for the tested values.** The canonical
   encoding and the request digest are exact for every value the contract
   admits. The *wire* body uses Pydantic's field order with CPython's float
   `repr`; for a float that needs scientific notation (outside the contract's
   `0.001..=5000` range) Pydantic and CPython spell the exponent differently
   (`1e16` against `1e+16`). Both parse to the same double, and the contract
   bounds keep every real value out of that range, so no admitted request is
   affected.
4. **Two small, documented departures from the Python client.**
   - A URL port that is not a number, or one outside `1..=65535`, is a typed
     `ClientError` here; Python raises an untyped `ValueError` from
     `urlsplit`. A typed refusal is strictly better and the message names
     the port.
   - The Windows home-directory fallback uses `%USERPROFILE%` (then
     `%HOMEDRIVE%%HOMEPATH%`) from the injected environment instead of
     `Path.home()`, so the resolution is testable and never reads the real
     process environment behind the caller's back.
5. **The token-length rule differs on purpose.** The bridge's own `auth.py`
   refuses a token shorter than 16 characters; the Python *client* does not.
   This crate follows the client (it refuses only an empty token) so the two
   clients behave identically; `MIN_TOKEN_CHARS`/`MAX_TOKEN_CHARS` document
   the bridge's stricter rule.
6. **The `cancel` request sends no `Content-Type`.** Python's `urllib` adds
   `application/x-www-form-urlencoded` to a body-less POST; the bridge
   ignores the content type, and the Rust client sends `Content-Length: 0`
   and nothing else.
7. **No CLI.** The Python package owns `kala-solidedge`; this crate is a
   library. The CLI composition belongs to a later cell.
8. **The token file is never created by the client.** The bridge owns the
   token file; the client only reads it, and refuses to guess a Linux path.
9. **Single in-flight connection.** The client is synchronous and talks to
   one bridge at a time, like the Python client. The bridge itself admits one
   build at a time, so this is not yet a limit in practice.
10. **The differential depends on the repository Python.** The three
    differential suites need the repository `.venv` (or `KALA_PYTHON`). The
    library itself never invokes Python; Python is used only by the tests,
    as the oracle.

## Reproducing the checks

```text
cd backend/crates/kala-solidedge-client
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo test --release --locked
cargo check --locked --target x86_64-pc-windows-gnu
```

Toolchain: `cargo 1.99.1`, `rustc 1.99.1`, edition 2024, `rust-version
1.85`. The Windows target `x86_64-pc-windows-gnu` is installed on this host
and the check is green; it is a compile check only — no Windows binary was
run and no Windows behaviour is claimed.
