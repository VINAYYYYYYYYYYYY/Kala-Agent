# Cell downloads-cancel — native-file download, real cancellation, honest readiness wording, client deadline

Wave `completeness-20261021` · Cell `downloads-cancel` · 2026-10-21

Items: **B11 + U-03** (artifact download), **B3 (CAD half) + U-06** (cancel really
cancels), **B10** (readiness wording) of `../audit-backend/AUDIT.md` /
`../audit-ui/AUDIT.md` (B10 is the `native-readiness`/`desktop-pack` wording), and
**addendum 4** (client deadline vs the new 1 800 s build budget).

No commit was made. Every shell exported `RUSTUP_HOME=/home/v9i/.rustup`,
`CARGO_HOME=/home/v9i/.cargo`, a temporary `HOME`, `TMPDIR`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` under
`/tmp/dc` (and `/tmp/dc2` for the final worker-proxy differential).
`/home/v9i/.local/share/kala-agent/**` was never read or written. No credential
or settings file was read, no secret was printed, and no real `claude`, `codex`
or `opencode` CLI was run. The only real external program used is the host's
`FreeCADCmd` 1.1.4, as a build provider (the same use the design-budget cell
made). All temporary trees were removed at the end; `df -h /tmp` was checked.

---

## 1. Download the provider's files (B11 + U-03)

### Change

**Route.** `GET /api/studio/design/{design_id}/artifact/{kind}`

* `design_id` is validated as a 64-character lowercase hex sha256; `kind` is an
  exact allow-list (`script`, `step`, `fcstd`, `stl`). Both are validated at the
  front door **before** any store access (a bad id is 422, an unknown kind is
  422, a kind the design did not store is 404).
* It carries the same gates as the other stored-design reads: the Host guard,
  the same-origin guard, the launch bearer (route-level, like `/drawing` and
  `/journal`), and the standard `{code, message, details}` envelope.
* It serves the **exact stored bytes**. The journal's content-addressed store
  already verifies the digest on read, and the service re-hashes the bytes and
  refuses a mismatch (`500 artifact_digest_mismatch`) before a byte is sent.
* The hard cap is **64 MiB**; a larger object is `413 artifact_too_large`.
* Headers: `content-type` (`application/json` for the script,
  `application/step`, `application/octet-stream` for FCStd, `model/stl`),
  `content-disposition: attachment; filename="<label-slug>-<digest8>.<ext>"`
  (`.FCStd` for the native document), `x-studio-design-id`,
  `x-studio-digest`, `x-studio-artifact-kind`.
* A missing kind is `404 artifact_not_found` with
  `details: ["design_id=<id>; kind=<kind>"]`.

The allow-list, the cap, the content types and the filename rules live in one
non-feature-gated module, `backend/crates/kala-studio/src/artifacts.rs`, so the
route table works in every build; the Python reference mirrors them in
`src/kala_agent/studio/design/artifacts.py` (identical slug algorithm, extension
map and cap). The worker proxy gained
`GET /internal/v1/design/{design_id}/artifact/{kind}` so the development Python
worker still answers when attached.

**UI.** A `download` button now sits next to every digest chip:

* the design card (`DesignAgentCard`) — each chip whose kind is downloadable
  (`script`, `step`, `fcstd`; an unknown native format offers no button);
* the export panel (`ExportPanel`) — script, step and native chips.

Both use `getDesignArtifact(connection, designId, kind)` in `studio/src/api/client.ts`
(a bounded request, `DESIGN_EXPORT_TIMEOUT_MS`), save the blob under the
backend's `content-disposition` filename, and write the existing local ledger
receipt (`Saved <filename>` plus `<bytes> · sha256 <digest8>`). The download
receipt is shown in the panel and in the journal ledger; a refusal is shown
verbatim (`code: message`).

### Proof

Rust (`cargo test -p kala-studio`):

* `artifacts::tests::every_artifact_kind_has_a_stable_content_type_and_extension`
* `artifacts::tests::a_hostile_label_cannot_escape_the_filename`
* `native_geometry::tests::the_artifact_route_downloads_the_exact_stored_bytes` —
  the script bytes equal the stored object byte for byte; the STEP bytes equal
  the stored provider STEP object; filename, content type and digest headers
  are asserted.
* `native_geometry::tests::the_artifact_route_refuses_a_kind_the_design_did_not_store`
  — `fcstd`/`stl` are 404 `artifact_not_found`, `bogus` is 422, a bad id is 422,
  a wrong bearer is 401.
* `native_geometry::tests::the_artifact_route_refuses_a_tampered_store_object` —
  a rewritten store object is `500 artifact_digest_mismatch`, not a download.

Python (`pytest tests/unit`):

* `tests/unit/studio_design/test_design_api.py::test_artifact_download_serves_the_exact_stored_bytes`
* `…::test_artifact_download_refuses_a_kind_the_design_did_not_store`
* `…::test_artifact_download_validates_the_design_id_and_the_bearer`
* `…::test_artifact_download_verifies_the_journalled_digest`
* `tests/unit/studio_design/test_design_artifacts.py` (5 cases: kinds, content
  types, filename shape, hostile labels, bounded slug)

UI (`npm test`): `test/designApi.test.ts` (3 artifact cases), `test/DesignAgentCard.dom.test.tsx`
(3), `test/designExport.dom.test.tsx` (3), all green.

**The required curl of a real FCStd download** (release binary, real
`FreeCADCmd` 1.1.4, FreeCAD L-bracket 60×45×40 wall 6, label
`Downloads cancel bracket`):

```
design_id 29d93a939708fb13ba9242220ebfc234a6f5e2a12dff54603cba6da27d40ccd7
verified True
step_sha256   863b20c94917cf65cc1835ebf9f0ed8842f6710779871a06ba852d7c7c1c36fe
native_sha256 6b6815c211870b1fc101c4f5bcc7ed5a1e72cf6cf46b3db501ce8d2b593caaa9 (fcstd)

$ curl -D - -H "Authorization: Bearer $TOK" \
    http://127.0.0.1:8799/api/studio/design/$DID/artifact/fcstd
HTTP/1.1 200 OK
content-disposition: attachment; filename="downloads-cancel-bracket-6b6815c2.FCStd"
content-length: 17734
x-studio-digest: 6b6815c211870b1fc101c4f5bcc7ed5a1e72cf6cf46b3db501ce8d2b593caaa9
x-studio-artifact-kind: fcstd
content-type: application/octet-stream

$ sha256sum download.FCStd
6b6815c211870b1fc101c4f5bcc7ed5a1e72cf6cf46b3db501ce8d2b593caaa9  download.FCStd
$ file download.FCStd
Zip archive data, at least v2.0 to extract   (an FCStd is a zip container)
$ stat -c %s download.FCStd
17734

$ curl -D - .../artifact/step
HTTP/1.1 200 OK
content-disposition: attachment; filename="downloads-cancel-bracket-863b20c9.step"
x-studio-digest: 863b20c94917cf65cc1835ebf9f0ed8842f6710779871a06ba852d7c7c1c36fe
content-type: application/step

$ curl .../artifact/stl            → 404 {"code":"artifact_not_found", …}
$ curl (no bearer) .../artifact/fcstd → 401
```

The served FCStd digest equals `artifacts.native_sha256` exactly, and the
filename is the real label slug plus the first eight digest characters.

---

## 2. Cancel really cancels (B3 CAD half + U-06)

### Change

**Cancellation is threaded from the request into the pipeline.**

* `NativeGeometryBackend::design` creates a `kala_process::CancelToken` and
  holds a **cancel-on-drop guard** across the blocking design task. A client
  disconnect drops the hyper request future; the guard sets the token; the
  blocking task (which cannot be dropped) observes it. A completed turn
  disarms the guard, so a successful design never records a cancellation.
  This one seam covers both `POST /design` and the design turn inside
  `POST /chat` (`chat_design::compile_design` calls the same backend method).
* `NativeDesignService::design_cancellable(intent, provider, &CancelToken)`
  checks the token at every phase boundary: validation → intent, probe → build
  request, build → import, import → measure/journal, and before each
  `artifact.stored`/`design.*` event of the post-build record. `design()`
  keeps its old signature and passes a never-cancelled token.
* The FreeCAD provider is now killable: `CadProvider::build_cancellable` (a
  default that falls back to the uninterruptible `build`) is overridden by
  `FreeCadProvider`, which passes the token through to
  `kala_process::supervise`. The supervisor polls the token on every slice and
  kills the provider's **own process group**. A killed build returns
  `CadProviderError::Cancelled`, never a "build refused".
* After a cancellation the service writes **one** `design.cancelled` event
  (`{design_id, reason}`, no prompt, no credential, no provider output) and
  returns `DesignError::Cancelled`. No later `design.*`, `provider.build.*` or
  `artifact.stored` event is written; the half-built design is discarded and
  never journalled as a design.
* The in-process OCCT build is **not** interruptible mid-operation; the token is
  honoured at the next phase boundary (worst case one 60 s compile). This is
  documented in `design_cancellable` and here.
* The Python reference cannot cancel a synchronous thread in the FastAPI pool;
  its `DesignService.design` docstring now says so plainly and points at the
  native service as the shipped path (honest boundary, no fake cancel).

**UI.** The cancel line is now
`Cancelled — the backend stopped the turn.` (and the local ledger records
`Turn cancelled — the backend stopped it`), instead of "Request cancelled",
which implied the work had stopped when it had not.

### Proof

Rust (`cargo test -p kala-studio`, `tests/design_cancel.rs`):

* `a_pre_cancelled_turn_records_only_its_cancellation` — the journal contains
  exactly one event, `design.cancelled`; no `design.intent`, no build event.
* `a_client_disconnect_kills_the_freecad_process_group` — a fake `FreeCADCmd`
  writes its pid and its child's, then sleeps 300 s. The test aborts the socket
  once the driver is running; the **whole tree is gone** and the journal is
  `design.intent, provider.build.requested, design.cancelled` with no
  completion/artifact/measure/check/reject event.
* `a_client_disconnect_cancels_a_design_turn_sent_through_chat` — the same,
  through `POST /chat`: one `design.cancelled`, no `chat.turn`, no build
  completion or artifact.

The audit's experiment against the **release binary and the real host
FreeCADCmd 1.1.4**, with the 100-hole plate (202 steps, ≈59 s when complete),
aborted after two seconds of real build work:

```
freecad provider: True 1.1.4
build started after 0.07s; FreeCADCmd pids: [2308558]
process tree before abort: [2308558]
survivors after 0.05s: []
journal types: ['design.intent', 'provider.build.requested', 'design.cancelled']
RESULT {"survivors": [], "gone_after_s": 0.05, "types": ["design.intent",
        "provider.build.requested", "design.cancelled"],
        "cancelled_reason": "the FreeCAD build was cancelled; the FreeCADCmd
        process group was stopped"}
PASS
```

So the FreeCAD process tree is gone **0.05 s** after the abort (limit 2 s), and
no build event was written after the abort. The same test asserts the process
pids exist before the abort, so it cannot pass vacuously.

UI: `test/ChatPanel.dom.test.tsx` asserts the new sentence and that a timeout
does not produce it.

---

## 3. Honest readiness wording (B10)

### Change

* `studio/scripts/native-readiness.cjs`
  * the header, the analysis and the printed report now name the category
    **geometry-backend seam (answered natively by default; the Python worker is
    a development opt-in)**;
  * the JSON fields are `geometrySeamRouteMethodCount` / `geometrySeamRouteMethods`;
    the old `proxiedRouteMethodCount` / `proxiedRouteMethods` remain as a
    compatible alias with identical values;
  * the printed line no longer says "proxied route-methods (Python geometry
    worker)";
  * the classifier parser understands the new two-parameter
    `strip_prefix` + `split_once` arm, so the artifact route is counted from the
    real source instead of being guessed.
* `studio/scripts/desktop-pack.cjs` reads the new field (falling back to the
  alias) and warns: "N route-method(s) go through the geometry-backend seam; the
  default build answers them from the native kala-studio backend (the supervised
  Python worker is a development opt-in)."
* `scripts/native-readiness.d.cts` / `scripts/desktop-pack.d.cts` declare the
  new fields and the alias.

### Proof

`node studio/scripts/native-readiness.cjs --report-only` (current tree):

```
[native-readiness] native Studio front door covers all 16 current Python route-methods and DTO capabilities (7 answered directly, 11 through the geometry-backend seam, answered natively by default)
[native-readiness] geometry-backend seam route-methods (answered natively by default; Python worker is a development opt-in): 11
…
  - GET /api/studio/design/{}/artifact/{}
…
[native-readiness] READY for current-feature packaging
```

`studio/test/nativeBackendPackaging.test.ts` (45 cases) asserts the new lists
and counts (7 direct + 11 seam = 18 native route-methods), the alias equality,
the absence of "proxied" from the summary/report, and the reworded
`desktopPack.decidePackaging` warning for both the new and the legacy field.

---

## 4. Client deadline vs the new backend budget (addendum)

### Change

* Rust `/health` publishes `design_turn_budget_seconds`, computed from the real
  constants: `2 × (STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS 180 + FreeCAD
  DEFAULT_TIMEOUT_SECONDS 1 800) = 3 960` (66 min). The build deadline is
  restated as an integer in `providers/remote.rs` and a unit test pins it to
  `kala_cad_providers::DEFAULT_TIMEOUT_SECONDS` and checks the OpenCascade
  compile budget fits inside it.
* Python `/health` publishes the same field from
  `providers.dispatch.design_turn_budget_seconds()` (same formula, same 3 960),
  placed at the same DTO field position so the two services emit byte-identical
  health documents.
* The Studio client derives its deadline from the value:
  `chatTurnTimeoutMs(budget) = budget × 1000 + 5 min`, with a documented
  **70-minute** fallback when the field is missing or unusable
  (`FALLBACK_CHAT_TURN_TIMEOUT_MS`); `CHAT_TURN_TIMEOUT_MS` remains as the
  fallback alias. The value is stored by `getHealth` in the server slice and
  read by `postChat`.

### Proof

* Rust `providers::remote::tests::the_design_turn_budget_is_built_from_the_real_provider_budgets`
  (3 960; equals 2 × (180 + 1 800); the restated deadline is the provider's
  default) and `native_geometry::tests::the_native_backend_builds_a_design_without_python`
  (`/health` → `design_turn_budget_seconds == 3960`).
* Python `tests/unit/test_studio_server.py::test_health_publishes_the_real_design_turn_budget`
  (constant identity, the field, and its DTO position).
* UI `test/apiClient.test.ts`: `chatTurnTimeoutMs` fallback/values,
  `stores the backend design-turn budget and falls back when it is absent`,
  `uses the backend-reported budget as the chat deadline` (a 1 s budget ends
  the turn at 1 s + 5 min, not at the fallback), and the fallback-deadline case.

---

## Gates

All commands ran with the temporary environment described at the top.

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | **clean** |
| `cargo clippy -p kala-studio --all-targets -- -D warnings` | **clean** |
| `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings` | **clean** |
| `cargo check -p kala-studio --no-default-features` | **clean** |
| `cargo test -p kala-studio --no-fail-fast` | **289 passed, 0 failed** (22 test binaries) |
| `cargo test -p kala-cad-providers --lib --test freecad_unit --test freecad_differential --test freecad_cut_guard --test solidedge_differential --test solidedge_mock_bridge` | **49 passed, 0 failed** |
| `pytest tests/unit -q` | **1667 passed, 0 failed** (42 s) |
| `python_differential.py` | **268 cases, 0 mismatches** |
| `worker_differential.py` | **295 cases, 0 mismatches, 0 known gaps** |
| `worker_differential.py --backend worker` (the Python worker proxy) | **295 cases, 0 mismatches, 0 known gaps** |
| `python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| `generate_corpus.py --check` | **determinism check passed** |
| `self_check.py` | **OK** — 59 Studio cases, 16 routes |
| `npm run typecheck` (studio) | **exit 0** |
| `npm test` (studio) | **53 files, 769 tests passed** |
| `npm run build` (studio) | **exit 0** |
| `cargo build --release -p kala-studio` | **pass** (11 702 528 bytes) |

`cargo test -p kala-cad-providers` in full would run the live
`freecad_budget` binary (a 1 000 s 500-step build, design-budget's measurement
binary); it was excluded from this cell's run because the provider change here
is the cancellation hook, and the five binaries above exercise the provider.
That is an honest scope note, not a skipped failure.

## Differentials

| Differential | Before | After | Mismatches | Id set |
|---|---|---|---|---|
| `python_differential.py` | 268 | **268** | **0** | unchanged, fingerprint `1c97fe873e4c82fedfc446cabce84849a7d8e84842355ed15688877ba5aefead` |
| `worker_differential.py` | 292 | **295** | **0** (0 known gaps) | +3 new ids `design.artifact.script`, `design.artifact.fcstd`, `design.artifact.bogus` (set digest `2beb706a5962298727011b67d30dcfc10a71d33330e600777f5cd1cdcb61806d`); full 292-id fingerprint `a74fe9f5c55cea9a0b801603f2048587010ea3d87365e534c7da0c8bc1a82d88` |
| `python_provider_dto_oracle.py` | 20 | **20** | **0** | unchanged |

Why the worker differential grows by 3 and the Python differential does not:
the artifact route needs a stored design id, and only
`worker_differential.py` drives the ADR-0009/0011 design routes. The three new
checks compare the same design id on both sides, the **exact script bytes**,
the 404 envelope for `fcstd` (not stored by OpenCascade), and the 422 envelope
for an unknown kind. The same run with `--backend worker` (the front door
proxying to the supervised Python worker) also passes 295/0, so the mirrored
internal route is exercised, not just the native one. STEP is not compared
cross-process because both writers
embed a timestamp and per-process entity numbering; its exact-bytes contract is
covered by the in-tree tests above (download bytes equal the stored object whose
digest the design reports).

## Digests

* Corpus (regenerated; `--check` passes):
  * `backend/fixtures/corpus/v1/studio_http.json` `3eebaedd641df52f45945b2266f579514a0be035cf77dfdd48100003456b9c59`
  * `backend/fixtures/corpus/v1/manifest.json` `2dbf37ea24f3b76dbdfce9aa29e3e3f1785415f0ccebef02274cafe60dbfd5f6` (recorded `corpus_sha256` `c0003bae333f27a079adcf3e86f2618741a6606334a17ffe3fafe727fcd6c330`)
  * `backend/fixtures/corpus/v1/canonical_json.json` `0c2a4bd95ce0bd7d0c0951382fc8d56aa46f9417d503bb3deb920daf28d90533`
  * `source_state.python_sources.tree_digest` `2deb2169baa71dbfc7c0da8515b31ebea7f66a1f349984b30a3936734283cb40` (99 product Python files)
* `backend/target/release/kala-studio` (built by this cell, default features):
  `86843bbb5a0f1a0bc5e8ff9327d72e069151795e8c0a0e51ce51027075b95fd0`
  (11 702 528 bytes)
* Readiness (current sources): Python `db4b34ba1dd4bef4c3713f9882a9d7b72212216caf6e95517eaddb9876704beb`
  (3 files), native `93394dfd243e1dea7542af4d71449184c18f70b4419a915540439c3122b510e2`
  (57 files), 16/18 route-methods, 7 direct + 11 seam.

## Files touched

Rust: `backend/crates/kala-studio/src/artifacts.rs` (**new**),
`src/lib.rs` (one module line), `src/server.rs` (route, validation, bearer,
health field), `src/geometry_backend.rs` (the `artifact` seam + worker proxy),
`src/native_geometry/mod.rs` (artifact route, cancel-on-drop guard),
`src/native_geometry/service.rs` (artifact download, cancellation, `design.cancelled`),
`src/native_geometry/providers.rs` (`build_cancellable`),
`src/providers/remote.rs` (budget constant + test), `src/providers/mod.rs` (exports),
`tests/native_geometry.rs` (artifact tests, health budget),
`tests/design_cancel.rs` (**new**), `tests/corpus_fixture.rs` (digest, counts,
artifact seam route), `tests/worker_differential.py` (3 new cases).
`kala-cad-providers`: `src/base.rs` (`CadProviderError::Cancelled`,
`build_cancellable`), `src/freecad.rs` (cancellable build, `run_bounded_with_cancel`).

Python: `src/kala_agent/studio/design/artifacts.py` (**new**),
`design/service.py` (artifact download, error types, honest cancellation note),
`server.py` (public route, health field, envelopes), `internal_api.py` (worker
route), `schemas.py` (health field), `providers/dispatch.py` (budget function),
`tests/unit/studio_design/test_design_api.py`,
`tests/unit/studio_design/test_design_artifacts.py` (**new**),
`tests/unit/test_studio_server.py`, `tests/unit/studio_worker/test_worker_http.py`
(internal route list), corpus inputs (`corpuslib/http_cases.py`, `self_check.py`,
`README.md`).

UI: `src/api/client.ts` (`getDesignArtifact`, dynamic chat deadline, health
budget), `src/types/studio.ts`, `src/state/store.ts`, `src/components/chat/DesignAgentCard.tsx`,
`src/components/chat/designCard.ts`, `src/components/chat/ChatPanel.tsx`,
`src/components/export/ExportPanel.tsx`, `src/saveBlob.ts` (**new**),
`src/styles/chat.css`, `src/styles/studio.css`, `scripts/native-readiness.cjs`,
`scripts/desktop-pack.cjs`, `scripts/native-readiness.d.cts`,
`scripts/desktop-pack.d.cts`, and the matching tests.

Not touched: docs/**, the credential/settings code, the Python CLI runtime, and
`kala-studio-journal` / `kala-artifacts` (their public API was enough).

## Gaps and honest limits

1. **The Python reference cannot cancel a disconnected thread.** Its `/design`
   route is synchronous in the FastAPI thread pool, and a thread cannot be
   dropped by the event loop. The artifact download is mirrored exactly; the
   cancellation is native-only (the shipped path). The limitation is stated in
   `DesignService.design`'s docstring and here.
2. **The OCCT in-process build is not interruptible mid-operation.** A cancel is
   honoured at the next phase boundary; the worst case is one 60 s compile that
   finishes before the turn stops. The FreeCAD (out-of-process) build is killed
   promptly, which is the long case (up to 1 800 s).
3. **A cancelled design is discarded, not journalled as a design.** That is the
   decision the audit flagged as needing one; the journal keeps
   `design.intent`/`provider.build.requested` (they happened) plus one
   `design.cancelled`, and never claims a completion the backend did not reach.
4. **`design.cancelled` is a new event type.** The Python `JournalEventType`
   enum does not list it because the Python reference never writes it; the
   journal accepts any safe identifier and the UI ledger shows the row. A later
   cell can add it to the enum if a Python cancel path ever exists.
5. **The 64 MiB download cap is independent of the STEP-import ceiling.** If
   `KALA_STEP_IMPORT_MAX_BYTES` is raised above 64 MiB, a larger stored artifact
   downloads as a 413; the cap is the task's contract, not an accident.
6. **The corpus replay asserts the seam refusal, not the artifact body**, for
   the two valid-kind artifact cases, because that replay attaches no geometry
   worker. The exact 404/200 bodies are covered by the live worker differential
   and the in-tree tests.
7. **Windows is not executed here.** The FreeCAD kill goes through
   `kala_process`'s Windows supervisor; the runtime-fixes cell already
   type-checked the CLI process-group path on a Windows target, but no Windows
   host ran this cell.
8. **A real 500-step FreeCAD build was not rebuilt.** The download experiment
   used a real small FreeCAD part; the 500-step timing is design-budget's
   measurement (1 009.5 s) that sized the 1 800 s deadline this cell now
   publishes.
9. **Files outside the literal ownership list** (`src/artifacts.rs`,
   `src/lib.rs`, `src/geometry_backend.rs`, the two `.d.cts` declarations) were
   required to add the route and its types; they are listed above so the
   conductor can review them deliberately.
