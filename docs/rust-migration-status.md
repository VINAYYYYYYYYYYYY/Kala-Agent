# Rust migration status

> **What this document is.** The honest, per-module state of the port of
> Kala's own backend code from Python to Rust. Every named Python module gets
> a row: where it lives in Rust now, its status, its measured test count, the
> differential and review evidence behind that status, and whether it stays in
> Python on purpose.
>
> **What this document is not.** It is not a release announcement. The
> **Linux desktop cutover has happened** (ADR-0010 phase 2: the shipped
> package runs only Rust binaries and calls the real OpenCascade), and the
> AppImage is built and smoke-tested — but there is **no Windows runtime**,
> **no Solid Edge build**, **no code signing**, **no publication**, and **no
> speed or readiness number** anywhere in this file. The Python reference is
> still in the tree and is still the oracle.
>
> Measured on this host (Linux, CPython 3.14.8, rustc/cargo 1.99.1) on
> **2026-10-20**. Every command in §6.1 was re-run in this pass; §6.2 lists
> the ones that were **not** run, and they are marked as such.

---

## 1. How to read this

### 1.1 Status vocabulary

| Status | Meaning |
|---|---|
| **accepted** | The module is ported, the live differential against the repository Python is green, and the crate's own gates pass. The crate is *not* necessarily wired into a running service. |
| **accepted, on the product path** | As above, and the module is wired into the **shipped desktop app**. Its Python original is retired from the package and survives only as a development fallback or as the differential oracle (§3.1). |
| **accepted, difference declared** | As above, and the port differs from Python in a way that is written down (§4). The difference is intentional or reviewed, not an accident. |
| **follow-up** | The port exists but is not finished: a command, a route, a review finding or a repair is still open (§5). |
| **Python only (on purpose)** | The module stays in Python. §3 says why. |
| **parked** | The code is in the tree, is not deleted, and is not on any default path. |

### 1.2 Test counts

Counts are the **release-profile** test counts of the crate as it stands on
2026-10-20, measured by `cargo test --locked --release` in the crate's own
directory (§6). A crate's count is the sum of its unit tests, its integration
tests and its doc tests; the split is given where it is known. A count is
**not** a quality score and it is not comparable across crates.

### 1.3 Differential evidence

A **differential** is a live comparison against the repository's Python, run
as `<repo>/.venv/bin/python -I` with `PYTHONPATH=src` (the Python source is
inserted by the oracle script itself, so the isolation is real). Python is a
**test oracle only**: no crate in the product path invokes an interpreter.
Each differential is reported as *cases* and *mismatches*.

A **review** is an independent adversarial probe of an accepted crate, in its
own cell, with its own test files. A review that recommends **REPAIR** is
recorded as such; a review that recommends **ACCEPT** is recorded as such.

---

## 2. The module map

### 2.1 The root `backend/` workspace — seven members, one `Cargo.lock`

`cargo test --workspace --locked --release` → **449 passed, 0 failed, 0 ignored**.

| Python module | Rust crate / module | Status | Tests (release) | Differential evidence | Review |
|---|---|---|---:|---|---|
| *(the canonical JSON domain: `json.dumps`/`json.loads` as every module uses them)* | `kala-json` — `parse_strict`, `canonical_dumps`, `canonical_digest`, `CanonicalValue`, `JsonText`, `Limits` | accepted | 33 (20 unit, 3 corpus, 7 deep, 3 differential) | corpus `canonical_json.json` 66 cases; live Python differential, 3 tests, 0 mismatches | `deep_json` REVIEW-1 |
| `src/kala_agent/persistence/chain.py` (the digest preimage and the chain rule) | `kala-json` — `chain_preimage`, `chain_digest`, `payload_digest`, `verify_chain` | accepted | *(the 33 above)* | corpus `event_chain.json` 5 exact preimages + genesis; chain head digest reproduced byte-for-byte from a Python-written directory | `deep_json` REVIEW-1 |
| `src/kala_agent/persistence/database.py` (schema v2, migrations, `Database`, `SnapshotStore`) | `kala-persistence` — `Database`, `SnapshotStore`, `WorkflowSnapshot` | accepted | 65 (crate total; `schema_migration` 16, `journal_security` 13, `timestamp_compat` 3, …) | Python↔Rust cross-reader (3 tests), deep-JSON cross-reader, journal fixture; 0 mismatches | `persistence` REVIEW-1, REVIEW-2 |
| `src/kala_agent/persistence/journal.py` (`EventJournal`, `Event`, `ChainVerification`) | `kala-persistence` — `EventJournal`, `Event`, `ChainVerification` | accepted | *(the 65 above)* | as above; the whole chain is verified on every read and before every append | `persistence` REVIEW-1, REVIEW-2 |
| `src/kala_agent/kernel/budget.py` (Decimal, counters, durations, the ledger) | `kala-budget` — `Decimal`, `Counter`, `Duration`, `RunBudget`, `BudgetLedger` | accepted | 65 (42 lib, 15 ledger, 3 differential, 3 unicode, 1 far-exponent, 1 fixture) | corpus `decimal_budget.json` 85/85 cases; live differential against CPython's `decimal`, 0 mismatches | `budget` REVIEW-1, REVIEW-2 |
| `src/kala_agent/studio/geometry/{limits,primitives,meshes,stl}.py` (analytic preview meshes, binary STL) | `kala-geometry` | accepted, **on the default path** — the `native-geometry` feature enables `native-primitives` (§2.3) | 32 (16 unit, 8 validation, 4 fixture, 3 invariants, 1 differential) | corpus `geometry_stl.json` 6 meshes + 3 byte-exact STL digests; live Python differential, 0 mismatches | `geometry` REVIEW-1 |
| `src/kala_agent/studio/geometry/step.py` (the OCP STEP export) | `kala-step` — `export_step_primitive`, `StepExporter`, `probe_capability` | accepted, **on the default path** — the `native-geometry` feature enables `native-primitives` (§2.3) | 35 (16 native export, 9 lib, 7 CLI, 1 OCP differential, 1 FreeCAD differential, 1 doc) | independent OCP **and** FreeCAD re-import of the emitted solid; 28 structural cases | `step` (round 1) |
| `src/kala_agent/persistence/artifacts.py` (the content-addressed store) | `kala-artifacts` — `ContentAddressedArtifactStore` | accepted | 44 (17 semantics, 12 security, 8 concurrency, 4 lib, 3 cross-reader) | Python↔Rust cross-reader of the same store; 0 mismatches | `artifacts` REVIEW-1, REVIEW-2 |
| `src/kala_agent/studio/**` — the *public* Studio server (`server.py`, `cli.py`, `settings.py`, `providers/**`, `boundary.py`, `middleware.py`, `errors.py`, `schemas.py`, `static.py`) | `kala-studio` — the **front door and the native geometry backend** (the HTTP boundary, the settings/token stores, the static UI, the chat providers, the CAD-provider routing, and the design, drawing, section, import, journal, intent and CAD-provider routes) | accepted, **on the product path** (§3.1) — the Python geometry worker is now a **development-only opt-in** | 175 | `python_differential.py` **268 cases, 0 mismatches**; `worker_differential.py` **292 cases, 0 mismatches, 0 known gaps**; `python_provider_dto_oracle.py` 20 cases, 0 failures; `replay_binary_corpus.py` **56 cases, 0 mismatches** (§5) | `studio` REVIEW-1…4; `r1-worker-mode`; `r2-studio-proxy`; `r2-followup`; `r2-chat-providers`; `r2-datadir-fix`; `delivery` REVIEW-1/2; `rust-e2e-qa`; `cad-r2`; `cad-r3`; `native-journal`; `native-design`; `packaging` |

### 2.2 The standalone crates — own workspace, own lock

| Python module | Rust crate / module | Status | Tests (release) | Differential evidence | Review |
|---|---|---|---:|---|---|
| `src/kala_agent/contracts/{models,construction}.py` | `kala-contracts` | accepted, **on the product path** (linked by the `native-geometry` feature) | 50 | live differential **486 cases** (253 accepted, 233 rejected, 18 bytes-entry), 0 mismatches, all round trips match | `contracts` REVIEW-1, REVIEW-2, REVIEW-3 |
| `src/kala_agent/kernel/{policy,workflow,planning}.py` | `kala-kernel` — `policy`, `workflow`, `planning` | accepted, difference declared (stricter restore, §4.4) | 152 (129 cell tests + 23 independent review probes) | 13 live differential tests: 220 random policy operations, 180 random workflow operations, 160 random checkpoint resolutions, exact event payload/order comparison, no normalization of hash, error or array order | `kernel`; `kernel-review` (**REPAIR**, D1 — the source now carries the actor on the non-approved path; see §5) |
| `src/kala_agent/kernel/memory.py` | `kala-memory` | accepted | 85 | live differential **77 scenarios / 2,716 operations**, 0 unexplained mismatches, byte-for-byte canonical JSON | `memory-r1` |
| `src/kala_agent/persistence/repositories.py` | `kala-repositories` (all six repositories over the accepted schema) | accepted | 33 | cross-reader (3 tests); Python `test_persistence.py` 16 and `test_revision_commit.py` 26 stay green | `repositories` REVIEW-1 |
| `src/kala_agent/tools/protocol.py` (the bounded JSON domain, the envelopes, the workspace rules, the worker bootstrap) | `kala-tools` — `protocol` | accepted | 229 (crate total) | the crate's registry/simulator corpus and the live executor differential (below) | `tools-protocol-r1` |
| `src/kala_agent/tools/registry.py` | `kala-tools` — `registry` | accepted | *(the 229 above)* | **1,064 comparisons** across the registry and simulator corpus, 0 unexplained mismatches | `tools-registry-r1` |
| `src/kala_agent/tools/simulator.py` | `kala-tools` — `simulator` | accepted | *(the 229 above)* | as above | `tools-registry-r1` |
| `src/kala_agent/tools/executor.py` | `kala-tools` — `executor` | accepted, difference declared (the dead `timeout_seconds`, §4.6) | *(the 229 above)* | **23-case live executor differential**: same status, code, diagnostic, result, artifacts, provenance and digests; 0 unexplained mismatches | `tools-executor-r1` |
| `src/kala_agent/tools/sandbox.py` | `kala-sandbox` (the strict Linux jail, the preflight probe, the PID-namespace worker jail) | accepted, difference declared (fail-closed, §4.5) | 60 (30 security, 10 PID-namespace, 15 unit, 5 differential) | **182 cases, 0 unexplained mismatches**; the corpus mixes hand-written valid and hostile shapes | `sandbox-r1` (**D11, a real security regression**: the production worker jail had no PID namespace) → `sandbox-r2` (repaired) |
| *(the process supervision and sandbox boundary that `tools/executor.py` uses)* | `kala-process` | accepted | 46 (33 process, 10 sandbox, 2 lib, 1 live Python oracle) | 1 live Python oracle test | `process` (round 2, after 5 review repairs) |
| `src/kala_agent/config.py` | `kala-api` — `config` | accepted | 124 (crate total) | **113-case** configuration differential (precedence, validation messages, every field value and every `(kind, field, message)` triple); 0 mismatches | `api-core-r1` |
| `src/kala_agent/api/boundary.py` | `kala-api` — `boundary` | accepted | *(the 124 above)* | **240 cases** over the host, origin, credential, method and size gates | `api-core-r1` |
| `src/kala_agent/api/envelope.py` | `kala-api` — `envelope` | accepted | *(the 124 above)* | as above | `api-core-r1` |
| `src/kala_agent/api/middleware.py` | `kala-api` — `middleware` | accepted | *(the 124 above)* | as above | `api-core-r1` |
| `src/kala_agent/api/errors.py` | `kala-api` — `errors` | accepted | *(the 124 above)* | **17** validation-detail cases + **17** status-table cases + **9** framework-error cases + **393** ASGI-level cases + **31** real-loopback-socket cases | `api-core-r1`; `api-errors-fix` (the `HTTP_500_BAD_REQUEST` defect and its 19 Python regression tests) |
| `src/kala_agent/api/schemas.py` (and the application views it embeds) | `kala-api` — `schemas`, `views`, `validation`, `datetime`, `patterns` | accepted, difference declared (extra-field order, §4.2) | *(the 124 above)* | **560-case** differential (128 accepted, 432 refused), **0 mismatches**; the 8 order-only cases are counted, not reported as mismatches; every 422 body is compared byte for byte | `schemas-r1` |
| `src/kala_agent/api/app.py` (every route, the body layer, the answers) + `src/kala_agent/api/dependencies.py` | `kala-api` — `routes`, `jsoninput`, `wire` | accepted, difference declared (7 differences, §4.3) | *(the 124 above)* | **96-case** route differential over a real socket, **0 unexplained mismatches**; 95 of 96 compared strictly (status, `content-type`, `content-length`, `location`, exact body bytes) | `api-routes-r1` |
| `src/kala_agent/cli/main.py` — the `serve` launch contract | `kala-api/server` — the `kala-api-server` binary | accepted | 10 (8 launch contract, 1 unit, 1 differential) | the 96-case route differential, run against the native server | `api-routes-r1` |
| `src/kala_agent/application.py` (the composition root) | `kala-application` — `KalaApplication` and its DTOs | accepted, difference declared (D1, §4.1) | 53 (33 cell tests + 20 independent review probes) | main scenario (71 recorded operations, 59 events, 64 snapshots, 11 approvals, 6 artifacts), a **52-case** deterministic corpus, **11 DTOs** compared as exact transport bytes, and the **cross-reader in both directions** (Python writes → Rust reads → Python reads → Rust reads): **0 unexplained mismatches** | `application-r1`; `application-review` (**ACCEPT**, D1 recorded) |
| `src/kala_agent/cli/{main,__init__,__main__.py` (12 subcommands) | `kala-cli` — the `kala-agent` binary | **follow-up**: `serve` does not launch (§5) | 22 | **262-step** live differential: 143 help documents (byte-identical at 11 terminal widths) + 119 behaviour steps; 0 unexplained mismatches; the two data directories are compared structurally at the end | `cli-r1` |
| `src/kala_agent/runtime/model_gateway.py` | `kala-runtime` — `gateway` | accepted | 112 (crate total) | **26 mock-provider cases** (results, ledger bytes, digests, transcripts, manifests, wire traffic) + a **3,324-entry** randomized deterministic corpus; 0 mismatches | `gateway-r1` |
| `src/kala_agent/runtime/agent.py` | `kala-runtime` — `agent` | accepted | *(the 112 above)* | **36-case** live differential against the repository's Python agent; 0 unexplained mismatches | `agent-r1` |
| `src/kala_agent/domains/**` (the three packages: `mechanical`, `building`, `wing`) | `kala-domains` | accepted, difference declared (the native identity, §4.7) | 29 (15 domain contract, 5 lib, 5 source-mutation, 4 differential) | **67** valid parameter cases + **51** invalid-input cases compared by first field and exact Pydantic message + **35** raw model-JSON documents + **19** workflow-score cases; 0 mismatches | `domains` REVIEW-1, REVIEW-2 |
| `src/kala_agent/studio/design/{templates,intent}.py` + the design half of `studio/providers/local.py` | `kala-design-intent` | accepted, **on the product path** (the native geometry backend's intent contract) | 54 (40 contract, 8 lib, 6 differential) | **1,721 cases**: 908 template cases (valid + an infeasible randomized grid), 563 local design prompts, 58 model replies, 192 raw intent payloads; 0 mismatches | `design-intent-r1` |
| `adapters/solidedge/src/kala_agent_solidedge/compile.py` + `adapters/solidedge-bridge/.../protocol.py` | `kala-solidedge-client` (the `ConstructionScript` lowering and the loopback bridge client, protocol v1) | accepted as a **client only** — no live Solid Edge proof (§5) | 79 (45 unit, 33 lowering, 1 doc) | **54-case** compile differential against the live Python adapter; 0 mismatches | `r4-solidedge-client` |
| `src/kala_agent/studio/design/providers/**` — the *connection* half of the ADR-0011 provider model | `kala-cad-providers` (the contract, a real `FreeCADCmd` provider, a Solid Edge bridge probe) | accepted, **on the product path** (§3.1) — the connections are the product's CAD provider connections; the `opencascade` builder is `kala-cad` | 40 (23 FreeCAD unit, 1 live FreeCAD differential, 3 Solid Edge differential, 13 Solid Edge mock bridge) | a live FreeCAD differential: **43 builds** (28 frozen corpus + 15 randomized), **0 refusals**, 0 mismatches, every STEP re-imported by the Python importer | `providers-r1`; `freecad-multi-abi` |
| `src/kala_agent/studio/design/{compiler,measure,mesh,drawing,importer}.py` + the OCP construction compiler and its STEP writer — the ADR-0009 geometry core | `kala-cad` — the **phase-2 Rust→OCCT adapter**: one narrow C++ shim (`src/ffi/adapter.cpp`) over the real **OpenCascade 7.9.3** SDK. It **calls OCCT and never rewrites it**. | accepted, **on the product path** (§3.1) | 118 (75 after the core, +43 from the HLR drawing and the section) | a live OCP differential: **60 cases** (28 frozen fixture + 8 boundary + 24 deterministic random), 89 parts, 16 refusals, 0 mismatches; a live drawing differential: 13 shapes, 55 views, 55 sections, 15 refusals, **0 mismatches**, 0 mm sampled point movement | `cad-r1` (cancelled) → `cad-r2` → `cad-r3` |
| `src/kala_agent/studio/session_journal.py` — the Studio session journal and the artifact store | `kala-studio-journal` | accepted, **on the product path** (§3.1) | 60 (2 unit, 37 semantics, 10 security, 6 live differential, 1 corpus differential, 3 append-shape, 1 doc) | 6 live interleaved differential tests + a 240-operation seeded corpus differential: the same file, the same rows, the same chain digests, the same tamper verdicts, **byte for byte** | `native-journal` |

### 2.3 Parked crates: none

The geometry area is **no longer parked**. ADR-0010 §3 parked three crates;
phase 2 released all three onto the product path:

| Rust crate | Was | Now |
|---|---|---|
| `kala-cad` | **parked** (the interrupted OCCT FFI port) | the **phase-2 adapter**, on the default path: `kala-studio`'s `native-geometry` feature links it |
| `kala-geometry` | parked behind the non-default `native-primitives` feature | on the default path: `native-geometry` enables `native-primitives`, and the analytic meshes answer the primitive preview/export/handoff routes |
| `kala-step` | parked behind the non-default `native-primitives` feature | as above; the analytic STEP writer answers the primitive export route |

The old feature alias `native-step` is retired (it is an alias of
`native-primitives` and is not a default feature). The explicit portable
build is now `cargo build -p kala-studio --no-default-features`, which links
no kernel and reports `design_available: false` instead of failing to build.

---

## 3. What stays Python, on purpose

These are not unfinished work. They are the places where Rust would be the
wrong tool, and each one has a stated reason.

### 3.1 The cutover that has happened (ADR-0010 phase 2)

| Aspect | Before (phase 1) | Now (phase 2) |
|---|---|---|
| The geometry | the front door supervised **one Python geometry worker** and proxied the geometry, design, journal, intent and import routes to it | the front door serves those routes **itself**, from the native crates |
| The worker | was started by default | is started **only** when `KALA_STUDIO_GEOMETRY_WORKER` names one (the development fallback) |
| The default feature | `default = []` (no native geometry at all) | `default = ["native-geometry"]` (the native geometry backend) |
| The desktop package | shipped a frozen OCP worker alongside the Rust binary | ships **only Rust binaries**; the packaged child environment has `PYTHONPATH`, `PYTHONHOME`, `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and `KALA_STUDIO_GEOMETRY_WORKER` removed |

The packaged app is therefore **one runtime**. The only Python left inside it
is the FreeCAD adapter, and that runs inside the user's own FreeCAD, which is
**not** bundled. The Linux deliverable is the unsigned AppImage of §5.1.

### 3.2 The Python surfaces that stay

| What stays Python | Where it lives | Why it stays |
|---|---|---|
| **The FreeCAD adapter** | `adapters/freecad/**` | It runs **inside FreeCAD**. It is a FreeCAD module that drives `FreeCADCmd` through a documented entry point; the product never imports FreeCAD into its own process. A Rust re-implementation would have to re-implement FreeCAD's scripting API, which is the thing being integrated with. The **connection** to it is Rust (`kala-cad-providers`), and its runtime dependencies are vendored per CPython ABI so the installed app needs no virtualenv. |
| **The Solid Edge bridge** | `adapters/solidedge-bridge/**` | It runs **on Windows**, against a COM runtime and a licensed Solid Edge installation that only exists there. The **protocol** and the **client** are Rust (`kala-solidedge-client`); the bridge that talks to COM is not, and cannot be exercised on this host at all. |
| **The geometry worker, as a development fallback** | `src/kala_agent/studio/**` — the OCP compiler, the measurement, the hidden-line drawing, the section, the view-only STEP importer, the strict intent parser, the session journal, the CAD provider builds | **Retired from the packaged app; a development-only opt-in.** OpenCascade stayed C++ throughout: phase 1 was the Python/OCP worker, phase 2 is the narrow C++ adapter called from Rust (`kala-cad`), and phase 2 is what ships. The Python worker is still in the tree as the **development fallback** — it starts only when `KALA_STUDIO_GEOMETRY_WORKER` names one — and as the differential oracle. |
| **The Python reference, everywhere** | `src/kala_agent/**`, `tests/**` | The Python tree is the oracle for every differential in §2. It is deleted **only** at the very end, after every Rust part passes its differential, and **only** with the user's explicit approval. Nothing has been deleted. |

One further Python surface is **not** "still to be ported" but *deliberately
outside* the Rust scope: the **Solid Edge bridge's** COM side (above). The
Studio session journal is no longer in this list — it is ported
(`kala-studio-journal`) and byte-compatible with the Python one.

---

## 4. The intentional differences, in one place

Every difference below was written down by the cell that made it. Nothing here
is a discovered defect except **D1**, which is a reviewed, accepted finding.

### 4.1 `application` D1 — an oversized foreign event payload answers 400, not 500

| | Python | Rust |
|---|---|---|
| decision | refuses | refuses |
| exception | `JsonEnvelopeError` (a `RuntimeError`/`ValueError`, **not** an `ApplicationError`) | `ApplicationError` |
| code | none | `application_error` |
| message | `JSON string exceeds the bounded length` | `the event payload cannot be canonicalized as JSON: JSON string exceeds the bounded length` |
| HTTP status | **500** `internal_error` | **400** |

Both sides **fail closed**: the listing is refused, nothing is written, and
nothing is silently continued. The underlying reason text is identical; only
the wrapper and the status differ. It is reachable only from a foreign writer
that stored an event payload above the 65,536-byte bound — every event the
root writes itself is far below it (the largest is `revision.prepared` at
≈2.4 kB).

**Status: open.** `application-review` recommended either the one-line fix in
`src/application.rs::event_view` or a declaration in the cell report. The
current source still carries the wrapper, so this is recorded here as an
undeclared difference rather than a fixed one. *Severity: low; not blocking.*

### 4.2 `schemas` — the order of the undeclared fields

Python reports the unknown keys of a rejected body in the order the **input**
carries them; the native layer reports them in the order of their **code
points**, because the transport's JSON value model keeps an object's keys
sorted. The failures themselves are identical. The differential counts this
for **8 of its 560 cases** and does not report them as mismatches. A transport
that parses a body with an order-preserving parser can restore the input
order; the schemas module cannot, because it reads a value and not a text.

Two smaller ones travel with it: the keys of a `parameters` object are sorted
in the wire document (the canonical JSON and the digest are unchanged), and a
nested contract model is validated by the accepted `kala-contracts` crate, so
a nested timestamp without an offset is a `value_error` there and a
`timezone_aware` in Pydantic.

### 4.3 `api-routes` — the seven declared differences

| # | Difference |
|---|---|
| 7.1 | **The number format of a floating-point number.** The live API writes its answers with Pydantic's Rust serializer; CPython's `json.dumps` writes the scientific form for the same value (`2.36e-05` vs `0.0000236`). `kala_api::wire::encode_answer` reproduces the **API's** form, because the API layer must match the API. The contracts layer keeps CPython's form, because the application layer compares against `json.dumps`. Both are correct for their own layer. |
| 7.2 | **The JSON reader of a body.** `jsoninput::parse` reads a body the way CPython's `json.loads` does (a repeated key is last-wins; `NaN`/`Infinity` are numbers). A body that carries a lone surrogate is rendered with U+FFFD, because a Rust string cannot carry one; Python keeps it. |
| 7.3 | **A body nested deeper than CPython's recursion limit.** CPython refuses it with a `RecursionError`, which the API answers with a **500**; the native reader accepts it and answers with the **422** of the model. The reader is iterative and fails closed at any depth. |
| 7.4 | **A request a parser refuses before the application.** A duplicated `Host` header is refused by the Python server's parser (400) before the application sees it; the native server hands it to the application, which refuses it with `421 invalid_host`. The difference belongs to the parsers, not to the routes. |
| 7.5 | **The duplicated views of the application.** `kala-api/src/views.rs` still carries a second model of the application views. The **route answers** use the DTOs of the composition root and not that module, but the accepted response models of `schemas-r1` embed it, and a DTO of the composition root is write-only (it cannot validate a document). Removing it is a change of the `schemas-r1` cell, not of this one. See §5. |
| 7.6 | **No OpenAPI document and no documentation pages.** `/openapi.json`, `/docs` and `/docs/oauth2-redirect` are the routes of the framework, not of `app.py`. They are **not ported**: the document is generated by FastAPI from the signatures of the handlers and the JSON schema of every model. The corpus does not carry them and no claim is made about them. |
| 7.7 | **The framing of the two servers.** Each server adds its own `date`, `server` and `connection` headers. They belong to the servers, not to the application, and the differential compares the application's headers only. |

### 4.4 `kernel` — a stricter workflow restore

Native restore **rejects** forged history, unknown history/attempt/completed
steps, over-definition replanning, inconsistent pending approvals and
non-string `last_error`/`reason` fields. Python accepts these documents. Valid
Python exports are accepted by the native restore and re-serialize
byte-identically in both directions; the independent review measured **6
native-only rejections** and **no case where the native side accepted a
document Python rejected**.

### 4.5 `sandbox` — fail-closed instead of fail-open

The Python module runs a worker **without** the isolation controls it could
not establish and *reports* them. The native production path **refuses**: a
specification without `pid_namespace` is refused, and a control that could not
be established refuses the worker. `native_sandbox_spec` used to *drop* the
`pid_namespace` control; a caller that asks for the control now gets an
**error** instead of a weaker jail. This is the repair `sandbox-r2` made after
`sandbox-r1` was found to have shipped a real security regression.

### 4.6 `tools` — the executor's dead `timeout_seconds`

Python's `execute` accepts a `timeout_seconds` override and **never reads
it**: the deadline is the smaller of the deadline the call declares and the
deadline of the descriptor, capped by the profile. The port keeps the
parameter and the behaviour, so a caller that relied on the override is not
surprised by a different deadline. The override is dead in both
implementations.

### 4.7 The native identity digests

The accepted `kala-domains` crate computes **native Rust** evaluator and
package digests from its own source tree. It deliberately does not reproduce
the historical Python source-file digests. Consequently these differ from
Python: the package digest in `DomainPackageView`, the package digest in the
evaluation report and in the spec attributes, the report artifact digest and
name, `spec_digest`, the revision `state_digest` and `action_digest`, the
commit `request_id`, the `DomainPackageManifest` digest and its
`schema_digests`, and every journal event digest whose payload carries one of
those.

Parameter-schema digests and `parameters_digest` **are** byte-identical and
are asserted exactly. Stored Python revisions stay readable and immutable; a
Python-prepared commit is not silently re-approved under native identity
because its re-derivation changes identity. The `domain_provenance.json`
fixture records the **legacy** Python identity and says so.

### 4.8 The native geometry backend (ADR-0010 phase 2)

Every difference below was declared by the cell that made it. The geometry,
the measurements, the checks, the drawing and the section are **identical**;
the differences are in the *identity* of the kernel and in the work it
repeats.

1. **The OpenCascade binding differs.** The native build links the **system
   OCCT 7.9.3**; the Python oracle imports the `cadquery-ocp` binding
   **8.0.1.1**. The reported `opencascade` provider version therefore names
   a different kernel binding. The live differentials replace that one field
   with a placeholder **for the `opencascade` provider only**; the FreeCAD
   and Solid Edge identities are still compared exactly.
2. **The display mesh vertex order differs.** The two OpenCascade versions
   assemble the same STEP file with its faces in a different order, so the
   imported triangulation is the same geometry in a different order. The
   differentials compare the mesh by an **order-independent fingerprint**:
   the sorted `(position, normal)` vertices and the sorted triangles, plus
   the exact vertex count, the triangle count, the bounds, the solids and
   the warnings. The two fingerprints are equal — the same 586 vertices, the
   same 568 triangles, the same bounds — and only the array order differs.
3. **The generated artifact digests are volatile by construction.** The
   OpenCascade STEP writer and the FreeCAD FCStd writer embed the wall clock
   of the write. The harnesses mask `step_sha256` / `native_sha256` and check
   that they are still 64-character lowercase hexadecimal digests.
4. **The design request's field-level validation kind is approximated.** The
   provider refusals (missing provider, unknown provider) are exact. A
   field-level intent error reports the reviewed `kala-design-intent` message
   with the `value_error` kind, where Pydantic would name a more specific
   kind (`greater_than`, `string_type`, …). The message text and the
   location are the same; the corpus and the differentials do not exercise
   that path.
5. **The native cache holds the plain view data, not the kernel handle.**
   `kala-cad`'s `Shape` is deliberately `!Send + !Sync`, so the design cache
   cannot own it. A drawing or a section therefore **re-imports the stored
   STEP artifact** instead of reusing a cached shape. The answer is
   identical; the work is not. (This is the "the view is the provider's
   export" rule applied to the drawing.)
6. **The STEP export is not byte-stable between two exports.** The writer
   embeds the wall clock and its own entity numbering. The geometry, the
   units, the solid count, the volume and the bounds are the contract; the
   raw bytes are not. `normalise_step_bytes` (a fixed `FILE_NAME` timestamp
   and a zeroed translator counter) is the corpus' comparison form.
7. **There is no `step_available: false` mode at runtime.** The
   `native-geometry` feature links the kernel; a build without it
   (`--no-default-features`) is the explicit portable mode, and the capability
   probe reports `design_available: false` when the kernel cannot execute.
8. **The worker's development default is gone.** `WorkerCommand::
   from_environment` still knows a development default, but the front door no
   longer starts a worker unless `KALA_STUDIO_GEOMETRY_WORKER` names one.
9. **`Shape` is not `Send` or `Sync`, and the geometry work is serialised.**
   OCCT mutation is thread-affine for one handle, so `kala-cad` owns exactly
   one kernel handle per Rust value and the Studio's own geometry lock
   serialises the design, drawing and section work — exactly like the
   Python `_GEOMETRY_LOCK`.

### 4.9 The rest, by crate

Each crate's own report lists its complete list; the ones with a user-visible
consequence are:

- **`kala-cli`** — `serve` starts nothing (§5.2); the extra-field parameter
  error reports only the first key in canonical order (the accepted
  `kala-domains` crate's `reject_extra` returns on the first sorted key, and
  the CLI cannot preserve insertion order through a `CanonicalObject`); a lone
  surrogate in a parameter value gets the coercion wording of the accepted
  crate; a `>4,300`-digit integer in `--parameters-json` is a clean refusal
  where Python raises an uncaught `ValueError`; `int()` accepts ASCII digits
  only, where Python also accepts non-ASCII Unicode decimal digits; the deep
  JSON threshold is the measured CPython 3.14 value on this machine and is
  platform-dependent; non-UTF-8 argv/environment is lossy-decoded where Python
  uses `surrogateescape`; on a real TTY Python queries the terminal while the
  native line keeps `COLUMNS`/80.
- **`kala-application`** — constructor failures from the
  database, journal or artifact layers are returned as one `ApplicationError`
  (code `application_error`) where Python raises concrete classes
  (`JournalIntegrityError`, a SQLite exception); the **message is preserved**
  and the process still fails closed. The journal-chain failure is mapped
  back to Python's `local_failure`/`JournalIntegrityError` because its
  message is deterministic. A `value_error` context in `kala-api` carries the
  raised text, which the API never renders.
- **`kala-tools`** — the approval-binding internal failure is reported as a
  failed run with the internal error code, because a `ToolResult` is the only
  value the port returns; four validation messages exist in the port but are
  unreachable there.
- **`kala-process`** — Python's dynamic kwargs, duck-typed values and
  arbitrary callback exceptions cannot be expressed.
- **`kala-budget`** — Pydantic's multi-line validation text is not reproduced;
  the error **class categories** are.
- **`kala-sandbox`** — 14 differences in total (native probe helper, supplied
  interpreter facts, native child paths, type-checked specification fields,
  non-finite JSON values, a refused lone surrogate, the recursion limit, the
  exact `round`, a non-blocking network probe, the `verify_controls_with`
  rule, the PID-namespace repair of §4.5, a bounded probe output, the
  `os.strerror` wording and no import-time C library load).

---

## 5. What remains

Each item below is **open**. None of them is a claim that something is done.

### 5.1 The Windows build, the Windows run and the release

**The Linux cutover has happened; the Windows one has not.**

* **Done, and measured.** The desktop package launches **only** the native
  Rust `kala-studio`; the Python geometry worker is retired from the package
  and survives only as a development opt-in. The Linux deliverable is built
  and smoke-tested:

  | Field | Value |
  |---|---|
  | Path | `studio/release/kala-studio-0.1.0-x86_64.AppImage` |
  | Size | **163 642 516 bytes** (156.1 MiB) |
  | SHA-256 | `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e` |
  | Signature | **none** — the AppImage is unsigned |

  The smoke ran from the extracted image under a literal `env -i`: the backend
  starts with `PATH=""`, the health document is `status: ok` with
  `design_available: true`, `step_available: true` and
  `geometry_worker {state: ready, version: 7.9.3}`, the OpenCascade L-bracket
  is **35571.416047113315 mm³** with bounds `[-30,0,0] … [30,45,40]`, the
  drawing is `front(7) top(11) right(8)`, the XZ section at `y = 4` is
  **2321.4601836602546 mm² = 2400 − 25π** (exact, 1 outer + 1 inner loop), the
  STEP round trip exports 55 554 bytes and re-imports one solid with equal
  bounds, and the **FreeCAD** provider (the host's own FreeCAD 1.1.4, not
  bundled) builds the same L-bracket at **35571.416047 mm³** with
  `measured_by: FreeCAD`.

* **Not done.** `packaging/windows/build-windows.ps1` is written and reviewed
  but **has never been executed**: this host has no Windows and no `pwsh`.
  The script is a definition, not Windows-runtime proof. The user's Windows
  machine runs the build and the post-install checklist.
* **Not done.** The AppImage is **unsigned** and **not published**. The
  release workflow (`.github/workflows/studio-release.yml`) is written but was
  not pushed or run.
* **Not done.** `xvfb-run` is absent on the build host, so the Electron
  **window** was never rendered; the packaged **backend** is verified by
  extraction (`--appimage-extract`, no FUSE, no X server).
* The old release rule (`metadata.nativeStep === true`) is **replaced** by the
  native-design readiness report; a stage that lacks the bundled FreeCAD
  adapter is refused, and publication is forced to `never`.

The install and build instructions are in
[studio-install.md](studio-install.md); the packaging contract is in
[rust-studio.md](rust-studio.md).

### 5.2 `kala-agent serve` has not landed

The `cli-serve` cell has a brief but **no report and no landed change**. The
`kala-cli` crate still refuses `serve` with `application_error` and
`SERVE_ROUTE_LAYER_GAP_MESSAGE`: *"the serve command cannot start the native
core API: kala-api publishes the route router and its Application port, but no
accepted server entry point had landed when this port was built
(api-routes-r1), so it refuses rather than launch an unaccepted server."*

The pieces do exist: `kala-api` publishes `routes::{Application, Routes,
Lifespan}` and the connection layer, and `backend/crates/kala-api/server` is a
standalone workspace with `LocalApplication` and a `kala-api-server` binary
that carries the launch contract. `build_serve_target` and `build_serve_plan`
(the loopback validation and the token minting) are complete and tested in
`kala-cli`. **Wiring the command is the one remaining step**, and it is not
done.

### 5.3 The Windows checks blocked on MinGW

The `x86_64-pc-windows-gnu` target **is** installed, but the mingw C compiler
is not, so the crates whose tree carries a C dependency (`libsqlite3-sys`,
`ring`) fail in their build scripts with `failed to find tool
"x86_64-w64-mingw32-gcc"`. That blocks the Windows cross-check for
`kala-application`, `kala-cli`, `kala-api/server`, `kala-runtime`,
`kala-memory` and `kala-studio`.

The other crates **do** pass a Windows compile check, as their cell reports
record (these were **not** re-run in this pass):

| Check | Crates |
|---|---|
| `x86_64-pc-windows-gnu` | `kala-api`, `kala-tools`, `kala-sandbox`, `kala-process`, `kala-contracts`, `kala-design-intent`, `kala-solidedge-client`, `kala-cad-providers`, `kala-domains` |
| `x86_64-pc-windows-msvc` | `kala-kernel`, `kala-repositories`, `kala-json`, `kala-geometry`, `kala-budget`, `kala-step`, `kala-artifacts`, `kala-process`, `kala-tools`, `kala-cad-providers` |
| not verified on Windows | `kala-persistence` — `libsqlite3-sys`'s bundled SQLite needs MSVC `cl.exe`/`lib.exe` |

This is a **missing cross toolchain on this host, not a code error**. No
Windows runtime behaviour is claimed for any crate. The wave's rule is that
the user installs `mingw-w64-gcc` and the `windows-check` cell runs
afterwards; that cell has **not filed a report**, so the cross-check is
still open. Two further notes from the cutover wave:

* `kala-cad` cross-checks cleanly for **both** Windows targets with
  `--no-default-features` (the portable surface). The **OCCT-linked** build
  needs a Windows OCCT SDK (`OCCT_INCLUDE_DIR` / `OCCT_ROOT` / `VCPKG_ROOT`),
  which is what `packaging/windows/build-windows.ps1` supplies from vcpkg.
* A MinGW link against an **MSVC** OCCT distribution is not supported; the
  Windows build must use an OCCT build for the same toolchain.

### 5.4 The live Solid Edge proof

There is none, on any host. `kala-solidedge-client` and the bridge protocol
are fixture-tested (54-case compile differential); the `ConstructionScript`
lowering and the loopback client are proven against contract fixtures. **No
Solid Edge build has ever been executed.** On Linux the provider refuses
honestly with a reason. There is no Windows host, no Solid Edge installation
and no COM runtime here. The bridge milestone that has landed has **no
production native executor**, so even a reachable Windows bridge reports no
build capability.

### 5.5 The `views.rs` duplication

`kala-api/src/views.rs` still carries a second model of the application
views, beside the DTOs of `kala-application`. The **route answers use the
DTOs of the composition root**, so the duplication is not on the answer path;
but the accepted response models of `schemas-r1` embed it, and a DTO of the
composition root is **write-only** — it is constructed by the root and cannot
be read back from a document, so it cannot validate one. Removing the module
is a change of the `schemas-r1` cell, whose 560-case differential validates
those models against the live Pydantic models.

### 5.6 The open findings

| Finding | Where | State |
|---|---|---|
| **D1** — the oversized foreign event payload answers 400, not 500 | `kala-application/src/application.rs::event_view` | **open**; §4.1. Low severity, fails closed either way. |
| **kernel D1** — policy `decide()` dropped the accountable actor from a non-approved decision | `kala-kernel` | `kernel-review` recommended **REPAIR**. The current source passes the actor on that path; no separate repair-acceptance report was filed, so the closure is recorded here as "the source carries the fix", not as an accepted re-review. |
| ~~`replay_binary_corpus.py` is red — 56 cases, 5 mismatches~~ | `backend/crates/kala-studio/tests/replay_binary_corpus.py` | **closed (2026-10-20)**. The harness now normalises the corpus' own declared volatile paths and the native identity; it reports **56 cases, 0 mismatches** (§5.7). |
| ~~`corpus_fixture.rs` fails in the parked build~~ | `backend/crates/kala-studio/tests/corpus_fixture.rs` | **closed (2026-10-20)**. The test now follows the feature that actually links the STEP exporter and asserts the regenerated 56-case corpus; it passes in the default build (2 tests). |
| ~~the six new Studio routes are not in the 268-case differential~~ | the Rust front door | **closed (2026-10-20)**. The routes are answered natively and the 292-case worker differential covers them; the six routes are in the corpus and in the 292 cases. |

### 5.7 The corpus replay, precisely

`backend/crates/kala-studio/tests/replay_binary_corpus.py` replays the frozen
corpus against the real release binary. It was **red** while the front door
proxied the geometry (56 cases, 5 mismatches); the native geometry backend
brought the declared normalisation with it, and the harness now reports:

```
release binary cases: 56
exact: 48
fields (the corpus declares the volatile paths): 4
  of which host evidence (compared against the current host probe): 1
step structural (real ISO 10303-21 solid): 2
skipped for library body-limit config: 2
  skip studio.negative.oversized_declared: requires 16,384-byte library config; covered by corpus_fixture
  skip studio.negative.oversized_chunked: requires 16,384-byte library config; covered by corpus_fixture
corpus digest: 53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28
mismatches: 0
```

The four `fields` cases are the corpus' **own declared volatile paths** (the
STEP digest, the journal head and the chain digests — the STEP writer embeds
the wall clock of the write) and the one **host-evidence** case
(`studio.cad_providers`, the recorded CAD-provider probe of the machine that
generated the corpus) is compared against the current host's probe. The two
skips need the 16 384-byte library body-limit configuration and are covered
by `corpus_fixture`. The corpus digest is unchanged.

The two live differentials are green as well: `python_differential.py` is
**268 cases, 0 mismatches** and `worker_differential.py` is **292 cases, 0
mismatches, 0 known gaps**.

### 5.8 The test-data hygiene work

The isolation of the test suites from the user's real data directory is
recorded in [studio-limitations.md](studio-limitations.md#test-hygiene-the-users-data-directory),
together with the one incident in this migration and its assessment.

### 5.9 The packaging residuals

1. **The AppImage is unsigned and unpublished.** No certificate was read and
   no signing material exists in the repository. Publication is forced to
   `never` by the packaging helper.
2. **The Electron window was never rendered.** `xvfb-run` is absent on the
   build host, so the smoke verifies the packaged **backend** and the
   **served UI root** by extraction (`--appimage-extract`, no FUSE, no X
   server), not the Chromium shell.
3. **A real FreeCAD on Python 3.11–3.13 was never run.** The AppImage
   vendors the adapter dependencies for `cp311`, `cp312`, `cp313` and
   `cp314` and the runtime picks the matching set, but the only *real*
   FreeCAD exercised here is the host's **1.1.4 on Python 3.14.8**; the
   `cp311`–`cp313` imports were proven with matching CPython builds from
   `uv`. A FreeCAD on an unsupported Python is reported as
   provider-**unavailable** with its exact version, never as a crash.
4. **The OCCT closure is host-built.** The 23 staged `libTK*.so*` libraries
   come from the build host's OCCT 7.9.3. A distribution built elsewhere
   stages its own closure; the staging code path is the same.
5. **The `mod/Kala` FreeCAD addon is staged, not installed.** The user copies
   it into FreeCAD's user `Mod` directory by hand.
6. **The Windows stage is a definition.** `build-windows.ps1` vendors the
   four `win_amd64` ABI sets and stages the vcpkg `TK*.dll` closure next to
   `kala-studio.exe`, and it verifies the ABI match, the vendored packages
   and the `_pydantic_core*.pyd` presence — all reviewed, none executed.

---

## 6. The evidence, and how to re-run it

### 6.1 Commands re-run in this pass

Every command below was executed on this host on 2026-10-20 with the
conventions' data isolation in force: a temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`, and
explicit temporary `--data-dir` values. **The user's real data directory was
not opened, read, written or listed by any of them.**

| Command | Result |
|---|---|
| `cd backend && cargo test --workspace --locked --release` | **449 passed**, 0 failed, 0 ignored |
| `cd backend/crates/kala-cad && cargo test --locked --release` | **118 passed** (the phase-2 OCCT adapter) |
| `cd backend/crates/kala-studio-journal && cargo test --locked --release` | **60 passed** (the native session journal) |
| `cd backend/crates/kala-cad-providers && cargo test --locked --release` | **40 passed** (the CAD provider connections) |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py` | **56 cases, 0 mismatches**; `exact: 48`, `fields: 4`, `step structural: 2`, `skipped: 2`; corpus digest `53dcb9da…` (§5.7) |
| `cd studio && npm run typecheck` | **exit 0** |
| `cd studio && npm test` | **43 files, 628 tests passed** |
| `cd studio && npm run build` | **exit 0** (the pre-existing `three` >500 kB warning) |
| `env -i … node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad` | **PASS** — the extracted AppImage, `PATH=""` in phase 1, the host FreeCAD in phase 2 (§5.1) |
| `sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage` | `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`, 163 642 516 bytes |
| the workspace root `cargo test --workspace --locked --release` per-crate split | `kala-json` 33, `kala-persistence` 65, `kala-geometry` 32, `kala-budget` 65, `kala-step` 35, `kala-artifacts` 44, `kala-studio` 175 (of which the new `native_geometry` suite is 10) |

The counts of the **other** standalone crates in §2.2 are **not** re-run in
this pass; they stand as filed in their cell reports:

| Crate | Tests (release, as filed) |
|---|---|
| `kala-contracts` | 50 |
| `kala-process` | 46 |
| `kala-kernel` | 152 |
| `kala-memory` | 85 |
| `kala-tools` | 229 |
| `kala-sandbox` | 60 |
| `kala-api` (+ `kala-api/server`) | 124 (+ 10) |
| `kala-design-intent` | 54 |
| `kala-domains` | 29 |
| `kala-repositories` | 33 |
| `kala-solidedge-client` | 79 |
| `kala-application` | 53 |
| `kala-cli` | 22 |
| `kala-runtime` | 112 |

> **The `kala-application` lock file.** `cargo test --locked` in
> `backend/crates/kala-application` **failed** with *"cannot update the lock
> file … because `--locked` was passed to prevent this"*. The crate's local
> `Cargo.lock` predated `serde_json`, which `api-routes-r1` added to
> `kala-api`. Re-running the same command **without** `--locked` refreshed
> that one lock file, and `--locked` works again (53 passed). No other file
> was changed. The crate's own documented gate is `cargo test --locked`; a
> fresh clone would hit the same refusal until the lock is refreshed once.

### 6.2 Commands **not** run in this pass

| Command | Why |
|---|---|
| `cargo fmt --check` / `cargo clippy --all-targets --all-features -- -D warnings` per crate | **Not run** in this pass. The gate results in the cell reports (§2) stand as filed; the counts in §2 are the release test runs, which were re-run. |
| `cargo check --target x86_64-pc-windows-gnu` | **Not run.** The mingw C toolchain is absent on this host (§5.3). The `kala-api` and `kala-json` Windows checks that earlier cells recorded as passing are not re-verified here. |
| any Windows build, MSVC run, COM run or Solid Edge build | **Not run**; none is available on this host. `packaging/windows/build-windows.ps1` was **not** executed. |
| `packaging/windows/build-windows.ps1` | **Not run**; there is no Windows and no `pwsh` on this host (§5.1). The script is a definition, not a proof. |
| `npm run check:native-readiness` in `studio/` | **Not run** in this pass; the readiness report of the built stage stands as recorded in [rust-studio.md](rust-studio.md) and in `studio/release/native-readiness.json`. |
| the AppImage **build** (`node scripts/desktop-pack.cjs --linux AppImage`) | **Not run** in this pass. The artifact was built by the packaging cell; this pass verified the **existing** artifact (hash, size and the `env -i` smoke). |
| the Electron **window** rendering | **Not exercised.** `xvfb-run` is absent on this host; the packaged backend was verified by extraction instead. |
| a real FreeCAD on Python 3.11–3.13 | **Not run.** The official FreeCAD 1.1.4 AppImage ships `py311`; the `cp311`–`cp313` adapter imports were proven with matching CPython builds, not with a real FreeCAD. |
| any benchmark | **Not run.** No speed, memory or readiness number is claimed anywhere in this document. |

### 6.3 Where the evidence lives

| Evidence | Path |
|---|---|
| The cell report of every port | `.mece/waves/rust-backend-20261019/cells/<cell>/REPORT.md` |
| The independent reviews | `.mece/waves/rust-backend-20261019/cells/<cell>/REVIEW*.md` |
| The design-turn wave | `.mece/waves/design-turns-20261019/cells/<cell>/REPORT.md` |
| The cutover wave (the phase-2 cells and the packaging) | `.mece/waves/cutover-20261020/cells/<cell>/REPORT.md` |
| The corpus and its self-check | `backend/compat/`, `backend/fixtures/corpus/v1/` |
| The front-door evidence | `backend/crates/kala-studio/README.md` |
| The OCCT adapter's evidence | `backend/crates/kala-cad/README.md` |
| The packaging contract | `packaging/linux/README.md`, `packaging/windows/README.md` |
| The user-facing Studio documents | `docs/studio.md`, `docs/studio-limitations.md`, `docs/rust-studio.md` |

---

## 7. The short version

- **Kala's own backend logic is ported.** 25 crate directories sit under
  `backend/crates/` — 7 members of the root workspace and 18 standalone
  crates, one of them (`kala-api/server`) nested inside `kala-api`. Between
  them they cover the canonical JSON, the persistence, the contracts, the
  kernel, the memory, the budget, the repositories, the domains, the tools,
  the sandbox, the process boundary, the model gateway, the agent, the core
  API, the composition root, the CLI, the Studio front door, the design
  intent, the **OpenCascade adapter**, the **Studio session journal**, the CAD
  provider connections and the Solid Edge client.
- **The Linux desktop cutover has happened (ADR-0010 phase 2).** The shipped
  app runs **only Rust binaries** and calls the **real OpenCascade 7.9.3**
  through a narrow C++ shim (`kala-cad`). The Python geometry worker is
  retired from the package and survives only as a development opt-in and as
  the oracle.
- **The Linux deliverable exists**: an **unsigned** AppImage,
  `studio/release/kala-studio-0.1.0-x86_64.AppImage`, 163 642 516 bytes,
  sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`,
  smoke-tested end to end from the extracted image under a literal `env -i`.
- **The differentials are green**: 292 / 268 / 20 / 56 at **0 mismatches**
  against the live Python reference with the native backend, and every
  declared difference is written down (§4).
- **Two things are deliberately still Python**: the FreeCAD adapter (it runs
  inside FreeCAD) and the Solid Edge bridge (it runs on Windows). The
  geometry worker is a third, but as a **development fallback** only.
- **What is open**: the **Windows build and run** (the script exists and has
  never been executed), `kala-agent serve`, the Windows cross-checks blocked
  on MinGW, the live Solid Edge proof, the `views.rs` duplication, the
  application D1, **code signing**, **publication**, and the Electron
  window-rendering path.
- **The Python reference is intact** and is still the oracle.
