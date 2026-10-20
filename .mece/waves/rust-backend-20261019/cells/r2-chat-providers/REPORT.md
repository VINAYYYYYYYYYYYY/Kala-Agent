# r2-chat-providers — every CAD provider through the Rust chat front door, and the cold-reload area key

Cell: `r2-chat-providers`, wave `rust-backend-20261019`. Date: 2026-10-20.
Read before starting: `../rust-e2e-qa/REPORT.md` (the FAIL),
`../r2-studio-proxy/REPORT.md`, `../r2-followup/REPORT.md`, ADR-0011 and
`../CONVENTIONS.md`.

**Status: done.** The Rust front door's chat now passes the selected CAD
provider (`opencascade`, `freecad` or `solidedge`) to the worker unchanged;
the worker alone decides whether it can build, its refusal is relayed in the
Python source's own wording, and no other provider is ever tried. The live
differential now includes the REAL FreeCAD chat turn and matches the live
Python server **case for case (292 cases, 0 mismatches, 0 known gaps)**, and
the shared Python cold-reload bug is fixed with a test that fails on the old
reader.

No commit, push, deploy, runtime cutover, delegation or credential read was
performed. No Windows runtime, speed or readiness claim is made. Only the
owned paths below were edited.

---

## 1. The two defects

### 1.1 Chat refused every provider except OpenCascade

`chat_design.rs::finish_design` hard-coded OpenCascade as the only
chat-buildable provider:

```rust
if cad_provider != CAD_PROVIDER_OPENCASCADE {
    turn.text = join_text(&[&turn.text, &provider_unavailable_message(cad_provider)]);
    return Ok(());
}
```

`providers/cad` and the no-provider reply both reported FreeCAD as available
on this host, and the worker built FreeCAD correctly through `/design`, so the
front door contradicted its own provider facts (the rust-e2e-qa P1 finding).
`tests/chat_with_design.rs` pinned the refusal text.

### 1.2 A journal-restored design lost its surface area

`src/kala_agent/studio/design/service.py` wrote the measured payload as
`surface_area_mm2` (`_measured_payload`, unchanged) but the cold-reload reader
looked for `area_mm2` (`_measured_from_payload`). Every design restored from
the journal therefore had `surface_area_mm2: null` (the UI card's "—") after a
restart. Both front doors were affected because both use this worker.

---

## 2. Changed paths

| Path | Change |
|---|---|
| `backend/crates/kala-studio/src/chat_design.rs` | `finish_design` passes the selected provider through; the old refusal is gone. New `DesignFailure` reads the worker's error envelope and renders the source's chat refusal. `compile_design` returns `Result<Result<Value, DesignFailure>, BackendError>` and always sends the operator's provider. The older-worker retry-without-provider is removed. Four new unit tests. |
| `backend/crates/kala-studio/tests/chat_with_design.rs` | The pinned-refusal test is replaced by `the_stitch_sentence_matches_python_for_every_cad_provider`: exact Python reference text and design values for FreeCAD and OpenCascade, exact Solid Edge refusal, no fallback. The FreeCAD/Solid Edge blocks skip loudly on a host where the worker reports that provider differently. |
| `backend/crates/kala-studio/tests/worker_differential.py` | Three ADR-0011 chat cases with the Stitch sentence (freecad, opencascade, solidedge), compared against the live Python public server; a documented volatile-generated-digest normalisation for those chat cases. |
| `backend/crates/kala-studio/README.md` | The chat section now states the pass-through rule and the relay of the worker's refusal. |
| `src/kala_agent/studio/design/service.py` | **Reader only.** `_measured_area_from_payload` reads the written `surface_area_mm2` and falls back to the legacy `area_mm2`; the written payload format is untouched. |
| `tests/unit/studio_design/test_service.py` | New cold-reload test against the journal's own canonical `payload_json` plus a written/legacy reader test. |
| `tests/unit/studio_design/test_design_api.py` | New HTTP cold-restart test (two applications, same data dir) and an area assertion in the existing end-to-end test. |
| `.mece/.../r2-chat-providers/REPORT.md`, `evidence/live_chat_differential.json` | This report and the socket-level live evidence. |

No other Rust crate, root manifest/lock, Python test outside `tests/unit/studio_design/`,
shared fixture, adapter or `studio/**` file was edited.

---

## 3. Bug 1: the provider is passed through, the worker decides

### 3.1 The new flow

```rust
let Some(cad_provider) = cad_provider else { /* ask the operator; compile nothing */ };
let outcome = compile_design(backend, intent, cad_provider).await?;
match outcome {
    Ok(design) => { /* measured summary, design on the turn */ }
    Err(failure) => { turn.text = join_text(&[&turn.text, &failure.chat_message(cad_provider)]); }
}
```

`compile_design` always inserts the selected provider into the design request
(it replaces anything the intent might carry under that name), forwards the
request to the worker's `POST /internal/v1/design` and returns the worker's
answer. There is no provider allow-list and no local availability check in the
chat path any more.

### 3.2 The refusal is the worker's refusal, in the source's wording

The worker's public design route answers a provider that cannot build with

```
HTTP 503
{"code":"provider_unavailable",
 "message":"the solidedge CAD provider is not available",
 "details":["the solidedge CAD provider is not available",
            "reason: <the worker's probe reason>",
            "available: <ids that can build here>"]}
```

Python's chat route renders exactly those facts
(`_provider_unavailable_chat_message`). `DesignFailure::chat_message` rebuilds
the same sentence:

```
The {provider} CAD provider is not available: {reason}. Available: {available}. No design was built.
```

using the worker's `reason:` and `available:` details, falling back to the
worker's message and `none` if a detail is missing. The front door therefore
adds no availability fact of its own and never says "not available in this
studio build" any more.

On this host the Solid Edge turn is byte-identical to Python's:

```
… Compiling and measuring with OpenCASCADE. The solidedge CAD provider is not
available: Solid Edge build requires the Windows bridge (not reachable); this
host is linux, so the ADR-0008 bridge cannot run here. Available: opencascade,
freecad. No design was built.
```

### 3.3 No fallback, in any direction

* The selected provider is always in the request; a worker that does not know
  the field now answers its own refusal instead of the front door retrying
  without the selector (the old retry could have let an older worker build
  with its default provider).
* A refused provider leaves `design` null; no other provider is attempted.
* A `cad_provider` that is not one of the three is still refused by the
  request schema with the source's enum detail (unchanged, still in the
  differential).

### 3.4 FreeCAD through chat is a real build

`chat_with_design.rs` and the live differential use the rust-e2e-qa Stitch
sentence. Through the Rust front door with `cad_provider: "freecad"` the turn
returns design `a9d04f8ff14e…`, `provider: "freecad"`,
`provider_version: "1.1.4"`, `measured_by: "FreeCAD"`, volume
**35571.416047 mm³**, area **11889.101731 mm²**, envelope 60 × 45 × 40 mm,
one valid solid, `verified: true` (5/5 checks) and the view mesh
(568 triangles / 586 vertices / 1 solid). The reply text ends

```
L-bracket 60 x 45 x 40: B-Rep verified; measured by FreeCAD. Measured volume
35571.416 mm^3, surface area 11889.102 mm^2, envelope 60.000 x 45.000 x 40.000
mm. design_id a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7.
```

and is byte-identical to the live Python server's reply.

---

## 4. Bug 2: the cold-reload area key

### 4.1 The fix (reader only)

```python
def _measured_area_from_payload(value: Mapping[str, object]) -> float | None:
    area = _finite_or_none(value.get("surface_area_mm2"))
    if area is None:
        area = _finite_or_none(value.get("area_mm2"))
    return area
```

The written payload is untouched (`_measured_payload` still writes
`surface_area_mm2`), and no event is rewritten, so existing journals restore
correctly. The legacy `area_mm2` key is only consulted when the written key is
absent or not finite, so a payload that carries both keeps the real value.

### 4.2 The tests

* `test_a_cold_reload_restores_the_surface_area_from_the_journal_bytes` reads
  the journal's own canonical `payload_json` for the
  `provider.build.completed` event (the record a cold reload reads), asserts
  the stored bytes carry `surface_area_mm2` and not `area_mm2`, then opens a
  second `DesignService` on the same data directory and asserts the loaded
  outcome keeps the area (11 889.101731 mm²) and volume.
* `test_the_measured_reader_keeps_the_legacy_area_key_as_a_fallback` covers
  the written key, the legacy key, a written null with a legacy value, and an
  empty payload.
* `test_a_restarted_server_restores_the_measured_area_from_the_journal` does
  the same through the HTTP route: two applications on the same data
  directory, the second one cold, re-posts the same design and gets
  `cached: true` with the area intact.

Both cold-reload tests were run against the **old** reader as a control:
they fail with `assert None == 11889.101731097317`, and pass with the fix.

---

## 5. Public API (Rust)

```rust
pub struct DesignFailure {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub details: Vec<String>,
}
impl DesignFailure {
    pub fn from_response(response: &WorkerResponse) -> Self;
    pub fn is_provider_unavailable(&self) -> bool;
    pub fn summary(&self) -> String;
    pub fn chat_message(&self, provider: &str) -> String;
}
pub async fn compile_design(backend: &dyn GeometryBackend, intent: &Value, cad_provider: &str)
    -> Result<Result<Value, DesignFailure>, BackendError>;
```

Removed: `provider_unavailable_message`, `design_failure_message` and the
private `refuses_provider_field`. `CAD_PROVIDER_OPENCASCADE` remains (it is
the fallback row of `available_cad_providers`). The changed `compile_design`
return type and the removed helpers are the only public API changes; nothing
outside the crate depends on them.

---

## 6. Gates

### 6.1 Rust (all commands from `backend/`)

| Gate | Result |
|---|---|
| `cargo fmt -p kala-studio -- --check` | **pass** |
| `cargo clippy -p kala-studio --all-targets --locked -- -D warnings` | **pass**, 0 warnings |
| `cargo clippy -p kala-studio --all-targets --all-features --locked -- -D warnings` | **pass**, 0 warnings |
| `cargo test -p kala-studio --locked` (debug, default) | **148 passed**, 0 failed |
| `cargo test -p kala-studio --locked --all-features` (debug) | **164 passed**, 0 failed |
| `cargo test -p kala-studio --locked --release` | **148 passed**, 0 failed |
| `cargo build --release -p kala-studio --locked` | **pass** |
| `cargo build --release --workspace --locked` | **pass** |
| `cargo test --workspace --locked --no-run` | **compiles** |
| `cargo check -p kala-studio --target x86_64-pc-windows-gnu` | **blocked**: `ring`'s build script needs `x86_64-w64-mingw32-gcc`, which is not installed. The blocker is the missing C toolchain, as recorded by r2/r2-followup; no Windows claim is made. |

Per-target counts, default debug build: 100 lib unit tests, 7
`chat_and_remote`, 5 `chat_with_design`, 6 `cli_launch`, 1 `float_vectors`,
5 `security_boundaries`, 12 `settings_token`, 6 `step_import_limit`,
1 `tls_trust`, 5 `worker_lifecycle`. The all-features build adds 6 lib unit
tests, 1 `corpus_fixture`, 6 `geometry_exports`, 2 `json_encoding` and
1 `step_export_ocp`. The 4 new `chat_design` unit tests are included in the
lib counts (r2-followup: 144/160; now 148/164).

### 6.2 Python (from the repository root, `.venv`)

| Gate | Result |
|---|---|
| `pytest tests/unit/studio_design -q` | **230 passed**, 0 failed |
| `pytest tests/unit -q` | **1555 passed**, 0 failed, 18 warnings |
| `pytest tests/security -q` | **189 passed**, 0 failed |
| `pytest tests/integration -q` | **12 passed**, 0 failed |
| `ruff check` (service.py + the two test files) | **pass** |
| `ruff format --check` (same files) | **pass** |
| `mypy --python-version 3.14` (same files) | **Success: no issues found in 3 source files** |
| `mypy` with the repo config (service.py) | **Success: no issues found** |

The 230 studio-design tests include the freecad-marked module (8 tests,
19 s of real `FreeCADCmd` subprocesses); no test was skipped.

### 6.3 The live differentials (the final release binary)

| Harness | Result |
|---|---|
| `tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** (r2-followup: 289/0/0; +3 provider chat cases) |
| `tests/python_differential.py` | **268 cases, 0 mismatches** |
| `tests/replay_binary_corpus.py` | **48 cases, 0 mismatches** |
| `tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |

The 292 cases are the 289 previous cases plus
`design.chat_with_freecad`, `design.chat_with_opencascade` and
`design.chat_with_solidedge`, all with the Stitch sentence. Each is compared
against the **live Python public server** (status, body and stable/security
headers). The reply text, the design id, the provider, the version, every
measured value, the checks, the view and the script digest are byte-exact;
only the generated STEP/FCStd digests are compared as present strings for
these chat cases, because both writers embed their build time (section 9.1).

### 6.4 The socket-level live proof

`evidence/live_chat_differential.json` is a separate run with two real
loopback servers — `python -m kala_agent.studio` and the release
`kala-studio` supervising its own Python worker — each with its own data
directory. For all three providers:

| Field | FreeCAD | OpenCascade | Solid Edge |
|---|---|---|---|
| HTTP status (Python / Rust) | 200 / 200 | 200 / 200 | 200 / 200 |
| Reply text identical | **yes** | **yes** | **yes** |
| Design present | yes | yes | **no** (refused, no fallback) |
| `provider` / `provider_version` | freecad / 1.1.4 | opencascade / 8.0.1.1 | — |
| `measured_by` | FreeCAD | OpenCascade (opt-in) | — |
| `volume_mm3` | 35571.416047 | 35571.416047113315 | — |
| `surface_area_mm2` | 11889.101731 | 11889.101731097317 | — |
| `verified` / checks | true / 5 of 5 passed | true / 5 of 5 passed | — |
| view (triangles / vertices / solids) | 568 / 586 / 1 | 568 / 586 / 1 | — |

The two generated artifact digests (STEP, FCStd) differ between the two
processes because each embeds its build time; they are recorded in the
evidence file, not compared (section 9.1).

---

## 7. Security evidence

* **The request carries no new authority.** The provider is a closed enum in
  the chat request (`opencascade | freecad | solidedge`); anything else is the
  source's 422 before the design hook runs. The front door forwards the
  string; the worker validates it again as its own `DesignRequest` enum.
* **The refusal cannot be forged by a provider or a model.** It is built only
  from the worker's own 503 error envelope, and only when the status is 503
  and the code is `provider_unavailable`. A remote chat model cannot inject a
  CAD provider or an availability fact: the parsed `DesignIntent` schema has
  `additionalProperties: false` and no `provider` property, and
  `compile_design` always overwrites the provider with the operator's choice.
* **No silent substitution.** A refused provider leaves `design: null` and
  the turn says so; no other provider is compiled and no measurement is
  invented.
* **No new route, token or secret.** The worker boundary, the launch token,
  the `/proc` token assertions and the body limits are unchanged. Nothing is
  logged from the chat turn except the same bounded facts as before (the
  journal still carries the prompt digest, never the prompt).
* **The Python fix reads only.** It does not change how data is written, so a
  journal cannot gain a new field or lose one through this change; it only
  stops discarding the area the journal already stored.

---

## 8. Intentional differences and limits

1. **STEP/FCStd digests are build-time artifacts.** The FreeCAD and
   OpenCascade STEP writers write the build second into `FILE_NAME`, and the
   FreeCAD FCStd zip carries the build time; two independent builds of the
   identical script differ at that byte (reproducible with two consecutive
   builds). The new chat cases therefore compare those two generated digests
   as present strings and compare everything else exactly. The pre-existing
   differential cases keep their comparison unchanged.
2. **A provider refusal longer than the worker's detail bound.** The worker's
   503 envelope truncates each detail to 256 characters; Python's chat text
   can carry a 512-character reason. On this host the Solid Edge reason is
   far shorter and the refusal is byte-identical. A pathological >256-character
   reason would be relayed truncated — the worker's bound, not a new one.
3. **A `design_unavailable` refusal keeps the front door's older wording.**
   Only the `provider_unavailable` refusal is rebuilt in the source's chat
   wording; a worker without the OCP kernel still answers the turn with the
   front door's pre-existing "The design could not be built: …" rather than
   the source's `DESIGN_UNAVAILABLE_MESSAGE`. That path cannot build on this
   host and was not part of this cell.
4. **Stale docs outside the cell's ownership.** `docs/cad-providers.md`
   ("Honest limitations" item 2) and `docs/rust-studio.md` still say the Rust
   front door cannot build FreeCAD. They are outside the owned paths and were
   not edited; the crate README is current.
5. **No Windows evidence.** The cross-check is blocked by the missing C
   toolchain that `ring` needs, and Solid Edge has no live proof on this host.
   The Solid Edge refusal is proven, not a Solid Edge build.
6. **No runtime cutover.** The Python Studio remains the reference oracle; the
   Rust front door is still a migration artifact, not the production server.
   No speed claim is made; every chat turn still costs the worker's
   `intent/parse` hop and a real provider build is a real external process.

---

## 9. Evidence and reproduction

Product/test files are listed in section 2. The raw gates and probes are in
`/tmp/opencode/r2-chat-providers/`:

| File | Contents |
|---|---|
| `rust_gates.log` | the full fmt/clippy/test/build/Windows-check chain with exit codes |
| `rust_differentials.log` | the four live differential summaries on the release binary |
| `python_gates.log` | pytest/ruff/mypy summaries |
| `worker_diff_final.log` | the final 292-case worker differential with the provider chat cases |
| `python_differential_final.log`, `replay_binary_corpus_final.log`, `python_provider_dto_oracle_final.log` | the other three live differentials on the final release binary |
| `worker_diff_after2.log`, `baseline_worker_diff.log` | the intermediate 292/0/0 run and the pre-change 289/0/0 baseline |
| `python_chat_reference.log` | the live Python chat answers for all three providers (reference text) |
| `probe_step_bytes.py`, `step_1.step`, `step_2.step` | the reproducible STEP timestamp difference |
| `probe_fcstd.py`, `native_1.fcstd`, `native_2.fcstd` | the reproducible FCStd archive-timestamp difference |
| `live_socket_differential.py` | the two-server socket-level differential |
| `.mece/.../evidence/live_chat_differential.json` | its final evidence document (texts, designs, digests) |

Commands used for the live runs (from the repository root):

```bash
.venv/bin/python backend/crates/kala-studio/tests/worker_differential.py
.venv/bin/python backend/crates/kala-studio/tests/python_differential.py
.venv/bin/python backend/crates/kala-studio/tests/replay_binary_corpus.py
.venv/bin/python backend/crates/kala-studio/tests/python_provider_dto_oracle.py
.venv/bin/python /tmp/opencode/r2-chat-providers/live_socket_differential.py \
  .mece/waves/rust-backend-20261019/cells/r2-chat-providers/evidence/live_chat_differential.json
```

---

## 10. Honesty notes

* The geometry and every number are measured by the worker's provider — real
  `FreeCADCmd` 1.1.4 for the FreeCAD case, the OCP kernel for OpenCascade.
  Rust never fabricates a mesh, a measurement, an availability fact or a
  refusal.
* The FreeCAD parity is Linux-only evidence from this host; no Windows, COM,
  Solid Edge build or packaging claim is made.
* The `148`/`164` test counts above are from the final source after the last
  edit; the logs record each command's own count. The Windows cross-check is
  reported as blocked, not as passing.
* Other cells were editing `backend/crates/kala-runtime` and some Python and
  `studio/**` files concurrently. None of their files was touched; the
  workspace-wide build and `--no-run` compile include their current state, and
  they passed.
* The only behavioural change on the Python side is the reader; the payload
  format and every existing journal remain valid.
