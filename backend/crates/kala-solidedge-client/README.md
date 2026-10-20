# kala-solidedge-client

Native ConstructionScript lowering and loopback bridge client for the Kala
Agent Solid Edge integration. This is a **standalone crate** with its own
workspace and `Cargo.lock`; it is not a member of the accepted root backend
workspace and it does not edit that workspace, the Python adapters, the shared
fixtures, or `studio/**`.

The crate is the Windows production path described by ADR-0010 and ADR-0008:
it lowers the shared `ConstructionScript` into bridge protocol v1 operations
and talks to the local bridge over an authenticated loopback connection.

**It computes no geometry.** Solid Edge performs every operation. The
closed-form oracle, the volume model, the STEP writer and the measurement code
stay in the Python geometry worker and are deliberately not ported here.

## Public API

```rust
use kala_contracts::load_construction_json_str;
use kala_solidedge_client::{BridgeClient, build_request, new_request_id};

let script = load_construction_json_str(
    r#"{"summary":"plate","components":[{"name":"plate","steps":[
       {"op":"sketch","profile":"rectangle","width":20,"height":20},
       {"op":"extrude","length":10}]}]}"#,
)?
.expect("a valid construction script");

// The request id is transport correlation only. Keep it across retries so an
// ambiguous submission never becomes a second build.
let request = build_request(&script, &new_request_id(), Some("cal-1"), 120.0, None)?;
let client = BridgeClient::new("http://127.0.0.1:8797", token, 30.0, 300.0)?;
let accepted = client.submit(&request)?;
let status = client.wait(&accepted.build_id, 120.0, 1.0)?;
```

### Lowering — `compile`

- `compile_operations(&ConstructionScript, Option<&AnalysisScript>, Option<&BTreeMap<String, f64>>)`
  → `Result<Vec<BridgeOp>, SolidEdgeClientError>`.
- `build_request(&ConstructionScript, request_id, calibration_id, timeout_seconds, density_kg_m3)`
  → `Result<BuildRequest, SolidEdgeClientError>`.
- `build_request_with(...)` — the same, with the analysis script and the fillet
  radii, so a caller can prove they are refused rather than dropped.
- `new_request_id() -> String` — 32 lowercase hexadecimal characters.
- `summary(&ConstructionScript) -> String` — the normalized summary.
- `ASSEMBLY_NAME`, `DEFAULT_BUILD_TIMEOUT_SECONDS`, `MAX_SUMMARY_CHARS`.

The refusals carry the messages of the Python compiler, character for
character. A refusal happens **before** a socket is opened.

### Client — `client`

- `BridgeClient::new(base_url, token, timeout, artifact_timeout)`,
  `BridgeClient::from_token_file(path, base_url, timeout, artifact_timeout)`.
- `health() -> Result<Health, _>`, `submit(&BuildRequest) -> Result<BuildAccepted, _>`,
  `status(build_id) -> Result<BuildStatus, _>`, `cancel(build_id) -> Result<BuildStatus, _>`,
  `wait(build_id, timeout, interval)`, `wait_with(build_id, timeout, interval, clock, sleep)`.
- `download(build_id, &Artifact, output_dir) -> Result<Download, _>` — streams
  the bytes to a temporary file, hashes them while they are written, and only
  publishes them under their final name once the declared size and the
  declared SHA-256 both match.
- `validate_bridge_url(&str) -> Result<String, _>` and `BridgeUrl`.
- `MAX_JSON_BYTES` (1 MiB), `MAX_ARTIFACT_BYTES` (256 MiB),
  `DEFAULT_TIMEOUT_SECONDS` (30 s), `DEFAULT_ARTIFACT_TIMEOUT_SECONDS` (300 s).

The transport is loopback-only: `http://127.0.0.1[:port]` and nothing else. A
redirect is refused rather than followed. Every response is bounded in size
and every request is bounded in time. A lost submission is reported as
*ambiguous*, never as a failure, so the caller keeps the request id.

### Wire types — `protocol`

- `BridgeOp` — the fourteen protocol v1 operations.
- `BuildRequest` — `new`, `to_wire_json`, `to_canonical_value`,
  `canonical_request_bytes`, `request_digest`, `from_json_bytes`.
- `Health`, `BuildAccepted`, `BuildStatus`, `BuildResult`, `PartResult`,
  `RangeBox`, `StepTrace`, `Artifact`, `ErrorResponse`.
- `Plane`, `JobState`, `ArtifactFormat`, `ErrorCode`.
- `validate_procedure(&[BridgeOp]) -> Result<(), ProtocolViolation>`.
- `PROTOCOL_VERSION`, `MAX_PARTS`, `MAX_OPERATIONS`, `MIN_SIZE_MM`,
  `MAX_SIZE_MM`, `MAX_OFFSET_MM`, `MAX_ANGLE_DEG`, `MIN_TIMEOUT_SECONDS`,
  `MAX_TIMEOUT_SECONDS`, `MAX_DENSITY_KG_M3`, `MAX_TRACE_STEPS`,
  `MAX_ARTIFACTS`, `MAX_WARNINGS`, `REVOLVE_AXIS`.
- `sha256_hex(&[u8]) -> String`.

The canonical encoding is the Python one: sorted keys, no whitespace, ASCII
escapes, and CPython's shortest round-trip float `repr`. The request digest is
the SHA-256 of the canonical request bytes with `request_id` removed, so a
retry of the same semantic request has the same digest.

### Journal and paths — `journal`, `paths`

- `Journal::new(path)`, `read`, `find_by_digest`, `find_exact`,
  `find_by_request`, `append`, `update`; `JournalEntry` with the eight record
  fields.
- `JOURNAL_FILENAME` (`journal.jsonl`), `MAX_ENTRIES` (200),
  `MAX_DIGEST_CHARS`, `MAX_ID_CHARS`, `MAX_TARGET_CHARS`, `MAX_SCRIPT_CHARS`.
- `resolve_token_file`, `resolve_bridge_url`, `resolve_output_dir`,
  `default_token_file`, `default_output_dir`, `local_app_data`,
  `is_windows`, `safe_build_id`, `build_output_dir`.
- `DEFAULT_BRIDGE_URL` (`http://127.0.0.1:8797`),
  `BRIDGE_URL_ENVIRONMENT_KEY` (`KALA_SOLIDEDGE_BRIDGE_URL`),
  `TOKEN_FILE_ENVIRONMENT_KEY` (`KALA_SOLIDEDGE_TOKEN_FILE`),
  `TOKEN_FILENAME` (`bridge.token`), `DATA_DIR_PARTS`, `RUNS_DIR_PARTS`.

The journal record is one JSON object per line, keys sorted, no whitespace,
ASCII escaped — the Python client's format, byte for byte. Either client can
read the other's records.

### Errors — `errors`

`SolidEdgeClientError` carries an `ErrorKind` (`Compile`, `Client`,
`Authentication`, `Protocol`, `Network`, `Artifact`), a bounded message, and
the bridge's `ErrorCode` when the failure came from a typed error envelope.
Messages are normalized exactly like the Python base exception: newlines
become spaces, whitespace runs collapse, and the result is truncated to 500
code points.

## What this crate does not claim

Nothing here is evidence about Solid Edge. There is no Windows host, no Solid
Edge installation, and no native build in this repository. The client is
tested against the bridge contract and an in-process mock bridge only.

## Tests

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo test --release --locked
cargo check --locked --target x86_64-pc-windows-gnu
```

The differential tests run the live repository Python in an isolated
interpreter (`python -I`) with the repository sources on the import path:

- `tests/compile_differential.rs` — every script of
  `tests/fixtures/construction_parity.json` and every case of the Python
  compile tests, compared with the Python lowering as canonical JSON, byte for
  byte. The refusals are compared message for message.
- `tests/request_contract.rs` — the canonical request bytes, the request
  digest, and the wire body against the Python bridge contract.
- `tests/journal_compatibility.rs` — the journal records written by one client
  and read by the other, in both directions, byte for byte.
- `tests/client_mock_bridge.rs` — the client against an in-process mock
  bridge: authentication, version mismatch, the error envelope, timeouts,
  oversize responses, the job lifecycle, and the artifact verification.

Set `KALA_PYTHON` to choose a different interpreter; the default is the
repository's `.venv/bin/python`.
