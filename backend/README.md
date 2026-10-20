# Kala Agent Rust backend workspace

Standalone Rust libraries for the Kala Agent backend migration, plus the
Rust **Studio front door**. The root `backend/` workspace has **seven
members** and one authoritative `Cargo.lock`; a further set of
**standalone crates** sits beside it, each with its own workspace and
lock, under independent review.

> **The per-module status of the whole port — every Python module, its Rust
> home, its tests, its differential and review evidence, the declared
> differences and what is still open — is in
> [`docs/rust-migration-status.md`](../docs/rust-migration-status.md).**
> This README is the workspace map; that document is the status record.

| Workspace member | Accepted capability | Tests (release) |
|---|---|---:|
| `kala-json` | Canonical CPython-compatible JSON, strict parsing, SHA-256 digests, journal event-chain preimages/verification | 33 |
| `kala-persistence` | SQLite schema v2, append-only verified event journal, canonical snapshots, stored-object JSON domain | 65 |
| `kala-geometry` | Analytic box/cylinder/sphere meshes, validation/defaults, deterministic binary STL | 32 |
| `kala-budget` | Arbitrary-precision Decimal, counters, microsecond durations, run-budget ledger | 65 |
| `kala-step` | Exact analytic AP214 B-rep STEP export for the three Studio primitives | 35 |
| `kala-artifacts` | Secure, immutable content-addressed artifact store (Unix production path; Windows code compiled but not executed here) | 44 |
| `kala-studio` | The Studio **service**: the HTTP boundary, the settings/token stores, the static UI, the chat providers, the CAD-provider routing **and the native geometry backend** (ADR-0010 phase 2) | 175 |

**449 tests** in the root workspace, 0 failed, 0 ignored, on this Linux
host (`cargo test --workspace --locked --release`; the debug run reports the
same 449).

| Standalone crate (own workspace + lock) | Capability | Tests (release) |
|---|---|---:|
| `kala-contracts` | Typed Rust contracts: the frozen Pydantic surface of `kala_agent.contracts` | 50 |
| `kala-process` | Bounded process supervision and the strict Linux sandbox boundary | 46 |
| `kala-kernel` | Policy, workflow and planning state machinery | 152 |
| `kala-memory` | Bounded working and durable memory | 85 |
| `kala-tools` | Tool protocol, JSON-schema validation, envelopes, workspace path rules, worker bootstrap, the child environment, the registry, the simulator **and the executor** | 229 |
| `kala-sandbox` | The strict Linux jail, the preflight probe, the PID-namespace worker jail, the refusal policy | 60 |
| `kala-api` | The core API connection layer, the DTOs, the routes and the answers | 124 |
| `kala-api/server` | The `kala-api-server` binary and its launch contract (nested inside `kala-api`) | 10 |
| `kala-application` | The composition root: `KalaApplication`, every operation and DTO | 53 |
| `kala-cli` | The `kala-agent` command line (12 subcommands); `serve` does not launch yet | 22 |
| `kala-design-intent` | The Studio design surface: templates, intent, the strict parser | 54 |
| `kala-domains` | The domain declaration and arithmetic layer | 29 |
| `kala-repositories` | Typed repositories over the accepted persistence schema | 33 |
| `kala-solidedge-client` | Native `ConstructionScript` lowering and the loopback bridge client (protocol v1) | 79 |
| `kala-cad-providers` | Native CAD provider **connections**: the ADR-0011 contract, a real `FreeCADCmd` provider, a Solid Edge bridge probe, and the per-ABI FreeCAD Python selection | 40 |
| `kala-cad` | The **phase-2 Rust→OCCT adapter**: one narrow C++ shim over the real OpenCascade 7.9.3 SDK — compile, measure, tessellate, STEP write/read, HLR drawings and exact-area sections | 118 |
| `kala-studio-journal` | The Studio session journal and the content-addressed artifact store, byte-compatible with the Python journal | 60 |
| `kala-runtime` | The native **model gateway** (provider shaping, egress policy, budget reservation, response validation, secret redaction) **and the agent runtime** | 112 |

It is **not a complete backend** for the non-Studio product, and the Windows
delivery is still future work. The **composition root, the core API wiring
and the CLI are ported** and differentially green, but nothing composes them
into a running service; the Windows/COM executor has never been executed.

## The Studio service (ADR-0010 phase 2)

`kala-studio` used to be a native nine-route Studio HTTP service that
proxied its geometry to a supervised Python worker. After the cutover it is
the **Studio service itself**:

| Concern | Owner |
|---|---|
| HTTP boundary, Host/Origin/bearer/body gates | **Rust** |
| `/health`, `/integrations`, `/providers`, `/settings`, `/chat` | **Rust** |
| CAD provider selection and the "never default to opencascade" rule | **Rust** |
| `/preview`, `/export`, `/handoff` | **Rust** — the reviewed analytic primitives (`kala-geometry`, `kala-step`) |
| `/design*`, `/journal*`, `/providers/cad`, `/import` | **Rust** — the native geometry backend, the native journal and the native provider connections |
| Geometry, measurement, drawing, section, the STEP import | **Rust**, through `kala-cad` — a narrow C++ adapter that **calls** the real OpenCascade 7.9.3 and never rewrites it |
| The Python geometry worker | **a development-only fallback**: it starts only when `KALA_STUDIO_GEOMETRY_WORKER` names one |

Geometry is **never re-implemented in Rust**: `kala-cad` calls OCCT. The
Python reference server and the Python worker remain in the tree as the
parity oracle and the development fallback; nothing was deleted.

The desktop package that ships this is the **unsigned** Linux AppImage
(`studio/release/kala-studio-0.1.0-x86_64.AppImage`, 163 642 516 bytes,
sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`),
smoke-tested end to end from the extracted image with no Python on `PATH`.
The Windows NSIS build is **written but never executed** (no Windows and no
`pwsh` on the build host). See `docs/studio-install.md`.

### The launch contract

The **worker is opt-in**: the front door starts no Python process unless
`KALA_STUDIO_GEOMETRY_WORKER` names one. The rest of the contract is
unchanged and still tested.

- **Command.** `KALA_STUDIO_GEOMETRY_WORKER` is either a JSON argv array
  (`["/path/python","-m","kala_agent.studio","--worker"]`) or a single
  executable path, in which case the fixed module arguments are appended.
  Whitespace splitting is **not** performed; a value that names an
  executable *and* arguments is refused rather than silently re-parsed.
  **There is no development default any more** — an unset variable means no
  worker at all.
- **Token.** 32 fresh random bytes rendered as 64 hexadecimal characters,
  passed **only** through `KALA_WORKER_TOKEN`. It is never in argv and
  never logged; the lifecycle test reads `/proc/<pid>/cmdline` and asserts
  the token and the variable name are absent from it.
- **Lifeline.** `KALA_WORKER_STDIN_LIFELINE=1` is set, so the worker stops
  when the front door closes its stdin: a dead front door can never orphan
  a worker.
- **Readiness.** Exactly one line, `KALA_WORKER_READY {json}`, read with a
  bounded 30-second start timeout. The port, the version and the
  `design_available` flag come from it.
- **Restart.** Capped exponential backoff (250 ms → 5 s). The state is
  published in `/health` as `geometry_worker {state, version,
  design_available}`.
- **Degradation.** A worker that cannot start leaves the native routes
  working and the geometry routes answering `503
  geometry_worker_unavailable`. **No mesh, no STEP file and no
  measurement is ever fabricated.**
- **STEP import ceiling.** `KALA_STEP_IMPORT_MAX_BYTES` (default 64 MiB,
  allowed 1 B … 512 MiB) is resolved **once**, before the worker is
  launched; the front door enforces it on the public import route and
  hands the same number to the worker process. Every other route keeps the
  shared 256 KiB Studio limit. An unusable value refuses startup (exit 2).

### The live differential

The parity gate runs the **real release binary with the native geometry
backend** against the live Python reference:

| Harness | Result (re-run on this host) |
|---|---|
| `crates/kala-studio/tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** |
| `crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** |
| `crates/kala-studio/tests/replay_binary_corpus.py` | **56 cases, 0 mismatches** |
| `crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |

> **`replay_binary_corpus.py` is green.** It was red while the front door
> proxied the geometry (56 cases, 5 mismatches, all in the four cases the
> corpus regeneration had added). The native geometry backend brought the
> corpus' own declared normalisation with it, so the harness now reports
> `exact: 48`, `fields: 4` (the corpus' declared volatile paths), `step
> structural: 2`, `skipped: 2` (the 16 384-byte library body-limit
> configuration, covered by `corpus_fixture`) and **0 mismatches**, with the
> corpus digest unchanged at `53dcb9da…`. The exact analysis is in
> [`docs/rust-migration-status.md` §5.7](../docs/rust-migration-status.md).

Two differences are **declared** and normalised by the harnesses: the
`opencascade` provider version names the linked OpenCascade SDK (7.9.3)
rather than the `cadquery-ocp` binding the Python oracle imports (8.0.1.1),
and the display mesh is compared by an order-independent fingerprint (the
same vertex set, the same triangle set, the same counts and the same bounds
— only the array order differs). The FreeCAD and Solid Edge identities are
still compared exactly.

The 292 cases include three ADR-0011 chat cases (FreeCAD, OpenCascade and
Solid Edge) that compare the reply text, the design and every measured value
against the live Python server. The front door passes the selected provider
through unchanged; the native backend alone decides whether it can build. A
refusal is relayed in the source's own wording and no other provider is
tried.

The comparison is the status, the exact body bytes and the stable and
security headers. The normalisations are the Rust-only `geometry_worker`
health field (the Python server has no worker), the volatile journal fields
(two processes never share a chain digest), the generated provider-build
digests (the FreeCAD and STEP writers embed their build time), the STEP
downloads (compared structurally, because OpenCascade's writer embeds a
timestamp and its own entity numbering), and the two declared native
differences of §4.8 in the status document (the kernel identity and the
display-mesh vertex order).

The standalone runtime and tools crates carry their own live differentials:

| Harness | Result |
|---|---|
| `kala-runtime/tests/python_differential.rs` | **26 mock-provider cases**, 0 mismatches; the two request logs are identical |
| `kala-runtime/tests/corpus_differential.rs` | **3,324 deterministic corpus entries**, 0 mismatches |
| `kala-tools/tests/executor_differential.rs` | **23 executor cases**, 0 unexplained mismatches; same status, code, diagnostic, result, artifacts, provenance and digests |
| the `kala-tools` registry and simulator differentials | **1,064 comparisons**, 0 unexplained mismatches |

Run them from `backend/` with the repository venv (they drive the OCP
kernel):

```bash
../.venv/bin/python crates/kala-studio/tests/worker_differential.py
../.venv/bin/python crates/kala-studio/tests/python_differential.py
../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py
../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py
```

## Status

| Item | State |
|---|---|
| Workspace, edition 2024, resolver 3, declared MSRV 1.85 | present; only rustc/cargo 1.99.1 actually used |
| `kala-json` canonical JSON and chain | accepted; Python corpus byte parity and live differential |
| `kala-persistence` schema/journal/snapshots | accepted; full-chain verification on every read, Python/Rust cross-readers |
| `kala-geometry` meshes and STL | accepted on this Linux host; fixture digests, live Python differential |
| `kala-budget` Decimal/ledger | accepted; corpus cases, Unicode-digit cases, live differential |
| `kala-step` analytic STEP | accepted; native boundary audit, OCP and FreeCAD re-import |
| `kala-artifacts` CAS | accepted within stated platform limits; Linux adversarial/concurrency tests pass |
| `kala-contracts`, `kala-process` | accepted as standalone crates; still excluded from the root build |
| `kala-kernel`, `kala-memory`, `kala-sandbox` | accepted as standalone crates |
| `kala-api`, `kala-design-intent`, `kala-domains`, `kala-repositories` | accepted as standalone crates |
| `kala-solidedge-client`, `kala-cad-providers` | accepted as standalone crates; **no live Solid Edge proof** |
| `kala-tools` | accepted as a standalone crate **including the executor**: 229 tests, 23-case live executor differential, 1,064-comparison registry/simulator corpus |
| `kala-runtime` gateway **and** agent module | **accepted**: 112 tests (the gateway's 55 at acceptance, plus the agent's 57); 26-case mock-provider differential, 3,324-entry deterministic corpus and a 36-case live agent differential, 0 mismatches |
| `kala-application` composition root | **accepted**: 53 tests; a 52-case corpus, 11 DTOs compared as exact transport bytes and the Python↔Rust cross-reader in both directions, 0 unexplained mismatches. One reviewed difference (D1) is open — see the status document |
| `kala-api` + `kala-api/server` | **accepted**: 124 + 10 tests; a 580-case live differential, a 560-case schema differential and a 96-case route differential, 0 unexplained mismatches |
| `kala-cli` | **accepted** for the 11 launching subcommands: 22 tests, a 262-step live differential (143 help documents + 119 behaviour steps), 0 unexplained mismatches. `serve` does not launch yet |
| `kala-cad` | **accepted and on the product path** (the phase-2 Rust→OCCT adapter): 118 release tests, a live OCP differential (60 cases) and a live drawing differential (13 shapes, 55 views, 55 sections, 15 refusals), 0 mismatches |
| `kala-studio-journal` | **accepted and on the product path**: 60 release tests, 6 live interleaved differential tests and a 240-operation corpus differential; the same file, the same rows, the same chain digests, byte for byte |
| `kala-studio` service | **accepted and on the product path**: 175 release tests and a 292-case live differential with 0 mismatches; the default feature is `native-geometry` |
| Windows execution/COM proof | **none**; the studio crate's Windows check is **blocked by the missing MinGW C toolchain** |
| Linux desktop packaging | **done**: the unsigned AppImage (163 642 516 bytes, sha256 `b069b7d4…`) is built and smoke-tested from the extracted image |
| Windows desktop packaging | **written, never executed**: `packaging/windows/build-windows.ps1` has not run on any host |
| Runtime cutover of the Studio | **done for the desktop app** (ADR-0010 phase 2); the Python reference stays as the oracle |
| Performance/optimization claim | **none**; no benchmark was run |

> **The Studio was cut over; the core product was not.** `kala-studio` is
> the shipped Studio service: it computes its own geometry through the real
> OpenCascade, owns the session journal, and the desktop package runs **only
> Rust binaries**. `kala-application`, `kala-api` (+ `kala-api/server`) and
> `kala-cli` exist, are differentially green against the repository Python and
> pass their own gates — but nothing composes them into a running service,
> the Python console script is still the installed entry point, and
> `kala-agent serve` still refuses with the gap message because the
> `cli-serve` cell has a brief but no landed change. The full per-module
> record is in
> [`docs/rust-migration-status.md`](../docs/rust-migration-status.md).

## Workspace layout

```toml
members = [
    "crates/kala-json",
    "crates/kala-persistence",
    "crates/kala-geometry",
    "crates/kala-budget",
    "crates/kala-step",
    "crates/kala-artifacts",
    "crates/kala-studio",
]
exclude = [
    "crates/kala-contracts",
    "crates/kala-process",
    "crates/kala-solidedge-client",
    "crates/kala-cad",
    "crates/kala-cad-providers",
    "crates/kala-design-intent",
    "crates/kala-studio-journal",
]
resolver = "3"
```

```
backend/
  Cargo.toml                 workspace manifest, shared dependency versions/profile
  Cargo.lock                 the single authoritative lock for all seven members
  crates/kala-json/          canonical JSON, chain, digests
  crates/kala-persistence/   SQLite schema v2, journal, snapshots
  crates/kala-geometry/      analytic meshes and binary STL
  crates/kala-budget/        Decimal, counters, durations, ledger
  crates/kala-step/          exact analytic STEP writer (+ `kala-step` CLI)
  crates/kala-artifacts/     content-addressed artifact store
  crates/kala-studio/        the Studio service + the native geometry backend
  crates/kala-*/             the standalone crates (own workspace, own lock)
  fixtures/corpus/v1/        Python-generated compatibility corpus (read-only)
  compat/                    Python corpus generator/self-check (test-only)
```

The six library members inherit `version`, `edition`, `rust-version`,
`license`, `publish = false` and shared dependency versions from
`backend/Cargo.toml`. `kala-studio` declares the equivalent package
metadata explicitly because it was accepted as a standalone crate. The
release profile (`strip = "symbols"`) is defined only at the workspace
root.

## Accepted capabilities

### `kala-json` — canonical JSON and chain

```rust
use kala_json::{
    canonical_digest, canonical_dumps, chain_digest, chain_preimage,
    parse_strict, payload_digest, verify_chain,
    CanonicalValue, EventChainFields, JsonText, Limits, Sha256Digest,
};

let value = parse_strict(br#"{"b":1,"a":[1.0,-0.0,1180591620717411303424]}"#,
                         &Limits::default())?;
let bytes = canonical_dumps(&value, &Limits::default())?;
let digest = canonical_digest(&value, &Limits::default())?;
```

`CanonicalValue` distinguishes integers from floats, keeps arbitrary-precision
integers exact, preserves `-0.0`, and uses `JsonText` for code-point-faithful
strings. `EventChainFields` is a typed record of the eight chain-covered parts;
`chain_preimage` emits the domain and parts with 8-byte big-endian UTF-8 length
prefixes. `payload_digest` hashes persisted payload JSON bytes without
re-serializing them. `verify_chain` checks contiguous one-based sequences,
previous-digest linkage and optional stored digests.

### `kala-persistence` — schema, journal, snapshots

```rust
use kala_persistence::{
    AppendOptions, Database, EventFilter, EventJournal, SnapshotStore,
    canonicalize_json_object, persistence_limits,
};

let database = Database::open("state/kala.sqlite3")?;
let journal = EventJournal::new(&database)?;
let event = journal.append_with(
    "workflow.started",
    "workflow-1",
    r#"{"offline":true}"#,
    &AppendOptions::default().with_workflow_id("workflow-1"),
)?;
let snapshot = SnapshotStore::new(&database).save(
    "snapshot-1", "workflow-1", r#"{"state":"running"}"#, None,
)?;
let head = journal.verify_chain()?.head_event_digest;
```

`Database` applies the frozen schema-v2 migrations, keeps foreign keys and WAL,
and nests transactions as SAVEPOINTs; failed `COMMIT`/`RELEASE` and panics
unwind. `EventJournal` verifies the whole stored chain when created and before
every read/get, including old rows outside caller filters. Identifier
normalization strips CPython's full `str.strip()` whitespace set while chain
preimages hash the raw stored bytes. Stored chain strings are required to be
exactly 64 lowercase hex digits before conversion. `SnapshotStore` verifies
canonical bytes and the stored digest on every read.

### `kala-geometry` — analytic meshes and STL

Typed builders and Python-compatible JSON dispatch for `box`, `cylinder` and
`sphere`; `Mesh`, `Bounds`, `compute_bounds`, `signed_volume`,
`welded_edges_by_position_pairs`; deterministic binary STL with the Python
header/count/record layout. Every validation error's `Display` text matches the
sanitized Python `ValueError`. See `crates/kala-geometry/README.md` for the
public API and the host-libm bit-exactness boundary.

### `kala-budget` — Decimal, counters, durations, ledger

Exact finite sign/coefficient/exponent `Decimal` with CPython grammar and
default context; arbitrary-precision `Counter`; microsecond duration grid with
ties-to-even rounding; `RunBudget` and thread-safe `BudgetLedger` including
state import/export and Python error-class mapping. Float formatting delegates
to `kala_json::format_float`. See `crates/kala-budget/README.md`.

### `kala-step` — exact analytic STEP

`export_step_primitive` / `export_step` / `export_parameters`, the
`StepExporter`/`NativeStepExporter` seam, `probe_capability`, `self_check`, and
the `kala-step` CLI. The writer emits analytic AP214 B-rep (planes, cylinders,
spheres, conics, line/circle edges) with no tessellation, Python, OCP or other
native library in the product path. Correctness is checked by a native boundary
audit and independent OCP/FreeCAD re-import; see `crates/kala-step/README.md`.

### `kala-artifacts` — content-addressed store

`ContentAddressedArtifactStore` with Python-compatible
`objects/<ab>/<digest>` layout, digest normalization, inclusive size limit,
re-hash-on-read and immutable publication. The Unix implementation is
descriptor-relative (`rustix`) with symlink/replacement/FIFO refusals, atomic
hard-link publication and thread/process tests. The Windows implementation
(`windows-sys`, handle-relative `NtCreateFile` and `SetFileInformationByHandle`)
compiles for `x86_64-pc-windows-msvc` but has **not executed** on Windows. See
`crates/kala-artifacts/README.md` for the security model and the explicit
Windows guarantee differences.

### `kala-studio` — the Studio service

See [The Studio service](#the-studio-service-adr-0010-phase-2) above, the
crate's own `crates/kala-studio/README.md`, and `docs/studio.md` for the
user-facing description.

### `kala-cad` — the OpenCascade adapter

`backend/crates/kala-cad/` is the **phase-2 Rust→OCCT adapter**: one narrow
C++ shim (`src/ffi/adapter.cpp`) over the real OpenCascade 7.9.3 SDK. It
**calls OCCT and never rewrites it**. It owns the construction compiler, the
measurement, the tessellation and the STL writer, the STEP write and the
view-only read, the HLR drawings and the exact-area sections. `Shape` owns
exactly one kernel handle and is deliberately `!Send + !Sync`; no C++
exception crosses the C ABI. Its own README documents the build discovery
(CMake, pkg-config, explicit env, vcpkg/MSVC) and the Windows notes.

### `kala-studio-journal` — the session journal

`backend/crates/kala-studio-journal/` is the hash-chained Studio session
journal and the content-addressed artifact store. It writes the **same
bytes** as the Python journal, opens and extends a Python-written journal, and
detects the same tampering. Its own README documents the public API.

## What is still Python

The Rust migration is **not** complete. Two things stay in Python **on
purpose** rather than as unfinished work, and a third is a **development
fallback**:

| Area | Python home | Rust status |
|---|---|---|
| **The FreeCAD adapter** | `adapters/freecad/**` | **Python only, on purpose.** It runs *inside* FreeCAD; a Rust rewrite would have to re-implement FreeCAD's scripting API, which is the thing being integrated with. The **connection** is Rust (`kala-cad-providers`), and the app bundles the adapter with per-ABI vendored dependencies. |
| **The Solid Edge bridge** | `adapters/solidedge-bridge/**` | **Python only, on purpose.** It runs on Windows against a licensed Solid Edge installation and a COM runtime. The protocol and the **client** are Rust (`kala-solidedge-client`); the COM side is not, and cannot be exercised on this host. |
| **The geometry worker** | `src/kala_agent/studio/**` — the OCP compiler, the measurement, the HLR drawing, the section, the view-only STEP importer, the strict intent parser, the session journal, the CAD provider builds | **Retired from the packaged app; a development-only opt-in.** OpenCascade stayed C++: phase 2 is the narrow C++ adapter called from Rust (`kala-cad`), and that is what ships. The worker starts only when `KALA_STUDIO_GEOMETRY_WORKER` names one, and it is the differential oracle. |

Everything else below is ported and differentially green:

| Area | Python home | Rust status |
|---|---|---|
| The Studio design-turn engine (the OCP compiler, the measurement, the HLR drawing, the section, the view-only STEP importer, the service) | `src/kala_agent/studio/design/**` | **ported and on the product path**: `kala-cad` (the kernel) + `kala-studio/src/native_geometry/` (the service) |
| The CAD providers (OpenCascade opt-in builder, FreeCAD subprocess, Solid Edge bridge probe) | `src/kala_agent/studio/design/providers/**` | **ported and on the product path**: `kala-cad` (the `opencascade` builder) and `kala-cad-providers` (the `freecad` and `solidedge` connections) |
| The Studio session journal and artifact store | `src/kala_agent/studio/session_journal.py` | **ported and on the product path**: `kala-studio-journal`, byte-compatible with the Python file |
| The Studio server, the settings store, the static serving | `src/kala_agent/studio/**` | **on the product path**: `kala-studio` is the shipped Studio service |
| The core API (config, boundary, envelope, middleware, errors, schemas, app, dependencies) | `src/kala_agent/api/**` | ported in `kala-api` (124 tests) and `kala-api/server` (10); **not wired into a running process** |
| The application composition root | `src/kala_agent/application.py` | ported in `kala-application` (53 tests); **not wired into a running process** |
| The CLI | `src/kala_agent/cli/**` | ported in `kala-cli` (22 tests); **not wired**, and `serve` does not launch |
| The tools executor, registry wiring and simulator | `src/kala_agent/tools/**` | ported in `kala-tools` (229 tests) and differential-green; **no runtime cutover** — the product still runs the Python executor |
| The tool sandbox | `src/kala_agent/tools/sandbox.py` | ported in `kala-sandbox` (60 tests), fail-closed; **no runtime cutover** |
| The agent runtime | `src/kala_agent/runtime/**` | ported in `kala-runtime` (112 tests: the gateway and the agent); **no runtime cutover** |
| The domain packages | `src/kala_agent/domains/**` | ported in `kala-domains` (29 tests); the native identities still need downstream re-approval |
| The Solid Edge adapter | `adapters/solidedge/**` | Python only; the Rust **client** exists |

**Python is deleted only at the very end**, after every Rust part passes its
differential, and only with the user's explicit approval. Nothing has been
deleted.

## What is pending

| Item | State |
|---|---|
| **Runtime cutover of the Studio** | **Done (ADR-0010 phase 2).** The desktop package runs only Rust binaries, and the front door computes its own geometry, journal and provider connections. The Python reference stays as the oracle. |
| **Runtime cutover of the core product** | **Not performed.** `kala-application`, `kala-api` and `kala-cli` are ported but nothing composes them into a running service, and the Python console script is still the installed entry point. |
| **`kala-agent serve`** | **Not landed.** The `cli-serve` cell has a brief but no report and no change. `kala-cli` refuses `serve` with the gap message; the pieces (`kala-api`'s router and `Application` port, the `kala-api/server` binary, `build_serve_target`, `build_serve_plan`) exist and are tested. |
| **The Windows build and run** | **Not executed.** `packaging/windows/build-windows.ps1` is written and reviewed, but it has never run: this host has no Windows and no `pwsh`. The user's Windows machine runs the build and the post-install checklist. |
| **Code signing and publication** | **None.** The AppImage is unsigned and unpublished; the release workflow was written but not pushed or run. No certificate was read. |
| **Windows / Solid Edge proof** | No Windows host, no Solid Edge installation, no COM runtime. The Solid Edge client is fixture-tested only; **no Solid Edge build has been executed**, and the bridge still has no verified native build executor. |
| **MinGW for the Windows checks** | `cargo check -p kala-studio --target x86_64-pc-windows-gnu` is **blocked**: `ring`'s build script needs `x86_64-w64-mingw32-gcc`, which is not installed on this host. `x86_64-pc-windows-msvc` is blocked the same way (`lib.exe` missing). The blocker is the missing C toolchain, not the crate. The same blocker stops the Windows cross-check for `kala-application`, `kala-cli`, `kala-api/server`, `kala-runtime`, `kala-memory` and `kala-studio-journal`; `kala-api`, `kala-tools`, `kala-sandbox`, `kala-process` and the other pure-Rust crates **do** pass it. `kala-cad` cross-checks cleanly for both Windows targets with `--no-default-features`; the OCCT-linked build needs a Windows OCCT SDK. `kala-persistence` is not verified for Windows (the bundled SQLite needs MSVC `cl.exe`). |
| **The `views.rs` duplication** | `kala-api/src/views.rs` still carries a second model of the application views beside the DTOs of `kala-application`. The route answers use the DTOs; the accepted response models of `schemas-r1` embed the module, so removing it is a change of that cell. |
| **The open findings** | The application D1 (an oversized foreign event payload answers 400, not 500) is open. The red `replay_binary_corpus.py` and the `corpus_fixture.rs` failure are **closed** (56 cases, 0 mismatches). All three are analysed in [`docs/rust-migration-status.md`](../docs/rust-migration-status.md) §5.6 and §5.7. |
| **The Electron window path** | `xvfb-run` is absent on the build host, so the packaged **backend** and the served UI root are verified by extraction; the Chromium **shell** was never rendered. |
| **A real FreeCAD on Python 3.11–3.13** | The official FreeCAD 1.1.4 AppImage ships `py311`; the `cp311`–`cp313` adapter imports were proven with matching CPython builds, not with a real FreeCAD. An unsupported Python is reported as provider-unavailable with its version. |
| **Performance / release benchmarks** | None. No speed, memory or readiness claim is made. |

## Toolchain

- `cargo 1.99.1` / `rustc 1.99.1` were used for every check in this round.
- The workspace declares `edition = "2024"` and inherited `rust-version = 1.85`
  from the accepted slices. That is a declared floor, **not** a tested claim:
  no Rust 1.85 toolchain was available here. Run the suite on the declared
  MSRV before publishing an MSRV guarantee.
- No Windows host, MSVC toolchain, Solid Edge installation or COM runtime is
  available. Windows results below are compile/clippy checks only.

## Build and test

Run from `backend/`:

```bash
cargo fmt --all --check
cargo test --workspace --locked                 # debug
cargo test --workspace --locked --release       # release
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --release --locked
```

The root workspace runs **449 tests** (the `kala-step` doctest is included in
its 35): `kala-json` 33, `kala-persistence` 65, `kala-geometry` 32,
`kala-budget` 65, `kala-step` 35, `kala-artifacts` 44 and `kala-studio` 175.
Debug and release totals are identical, with zero failures and zero ignored
tests on this Linux host. The `kala-studio` suite includes real-socket
security/CLI tests, the live Python differential and the OCP STEP reimport;
the Python scripts are test oracles only, and the native binary/library never
invoke Python outside the (opt-in) supervised geometry worker.

The default build of `kala-studio` **is** the native geometry build
(`native-geometry` → `native-primitives`), and its whole suite is green,
including `tests/corpus_fixture.rs` (2 tests) and the new
`tests/native_geometry.rs` (10 tests over a real socket). The portable
surface is `cargo build -p kala-studio --no-default-features`, which links
no kernel and reports `design_available: false`.

The standalone crates are built from their own directories, each against its
own lock:

```bash
cd backend/crates/kala-kernel      && cargo test --locked --release
cd backend/crates/kala-tools       && cargo test --locked --release
cd backend/crates/kala-runtime     && cargo test --locked --release
cd backend/crates/kala-application && cargo test --locked --release
cd backend/crates/kala-api         && cargo test --locked --release
cd backend/crates/kala-api/server  && cargo test --locked --release
cd backend/crates/kala-cli         && cargo test --locked --release
cd backend/crates/kala-cad         && cargo test --locked --release
cd backend/crates/kala-cad-providers && cargo test --locked --release
cd backend/crates/kala-studio-journal && cargo test --locked --release
```

`kala-tools` reports **229 passed** (including the 23-case executor
differential and the 1,064-comparison registry/simulator corpus).
`kala-runtime` reports **112 passed** — the gateway (55 at acceptance) and the
agent module on top of it; the crate-wide command compiles and is green.
`kala-application` reports **53 passed** (33 cell tests and 20 independent
review probes), `kala-api` **124** and `kala-api/server` **10**, `kala-cli`
**22**.

The three cutover crates report, **re-run in this docs pass**:

| Crate | Tests (release) | Suites |
|---|---:|---|
| `kala-cad` | **118** | the OCP differential (60 cases) and the drawing differential (13 shapes, 55 views, 55 sections, 15 refusals), 0 mismatches |
| `kala-cad-providers` | **40** | 23 FreeCAD unit, 1 live FreeCAD differential (43 builds, 0 refusals), 3 Solid Edge differential, 13 Solid Edge mock bridge |
| `kala-studio-journal` | **60** | 2 unit, 37 semantics, 10 security, 6 live differential, 1 corpus differential, 3 append shape, 1 doc |

> **`kala-application` needed one lock refresh.** `cargo test --locked` in
> `backend/crates/kala-application` first refused with *"cannot update the
> lock file … because `--locked` was passed to prevent this"*: the crate's
> local `Cargo.lock` predated `serde_json`, which `api-routes-r1` added to
> `kala-api`. Running the same command once **without** `--locked` refreshed
> that one lock file and `--locked` works again. No other file changed. A
> fresh clone hits the same refusal until the lock is refreshed once.

Live Python parity is part of the suite, not an optional sidecar:

```bash
cargo test -p kala-json --test python_differential --locked
cargo test -p kala-persistence --test python_cross_reader --locked
cargo test -p kala-geometry --test python_differential --locked
cargo test -p kala-budget --test differential_python --locked
cargo test -p kala-step --test ocp_differential --test freecad_differential --locked
cargo test -p kala-artifacts --test cross_reader --locked
cargo test -p kala-studio --test python_differential --locked
cargo test -p kala-studio --test step_export_ocp --locked
cargo test -p kala-cad --test ocp_differential --test drawing_differential --locked
cargo test -p kala-cad-providers --test freecad_differential --locked
cargo test -p kala-studio-journal --test differential --test corpus_differential --locked
```

These tests use the repository `.venv` (Python 3.14.8, OCP 8.0.1.1.0) and, for
`kala-step`, the installed `FreeCADCmd` 1.1.4. A missing oracle fails the test;
explicit opt-out variables are documented in each crate README. On Windows,
set `KALA_PYTHON`, `KALA_GEOMETRY_PYTHON`, `KALA_STEP_ORACLE_PYTHON` and
`KALA_STUDIO_OCP_PYTHON` to absolute interpreter paths because the Linux
`.venv/bin/python` default does not exist there.

The corpus self-check can be run separately from the repository root:

```bash
.venv/bin/python backend/compat/self_check.py
```

Observed in the **previous** docs pass (not re-run in this pass): **exit 0 —
`self-check OK`**. The corpus is regenerated into a temporary directory and
compared; it matches the checked-in files, and the reported digest is
`9993939bfea5e01553438e680fe3a9ea4b24335ee9077da5b8dc453b5a987e8b`
with **56 Studio cases / 15 Studio routes** and **49 core cases / 17 core
routes**. The earlier red self-check was a stale corpus; the `compat-drift`
cell regenerated it and classified every hunk.

The named parity targets above (`cargo test -p kala-json --test
python_differential`, and so on) were covered by the full workspace run in
this pass; they were not invoked one by one.

### Windows cross-checks

The five original pure-Rust libraries pass `cargo check` and
`cargo clippy -D warnings` with `--all-targets --target
x86_64-pc-windows-msvc`:

```bash
cargo check --locked --offline --target x86_64-pc-windows-msvc --all-targets \
  -p kala-json -p kala-geometry -p kala-budget -p kala-step -p kala-artifacts
cargo clippy --locked --offline --target x86_64-pc-windows-msvc --all-targets \
  -p kala-json -p kala-geometry -p kala-budget -p kala-step -p kala-artifacts \
  -- -D warnings
```

A whole-workspace Windows check currently stops in `libsqlite3-sys`'s bundled
SQLite build: no MSVC `cl.exe`/`lib.exe` is installed here (`cc-rs` reports
`failed to find tool "lib.exe"`). `kala-persistence` therefore remains
unverified for Windows compilation. `kala-studio`'s Windows path additionally
needs MSVC plus NASM for `ring`; that cross-check was not executed here. No
Windows runtime behavior is claimed for any crate. The workflow in
`.github/workflows/studio.yml` describes the expected Windows toolchain but is
not a hosted-run proof.

## Compatibility rules

- Object keys are sorted by Unicode code point, then ASCII-escaped.
- Strings use CPython `ensure_ascii=True` escaping: short control escapes,
  `\u00xx` for DEL and BMP non-ASCII, UTF-16 surrogate pairs for supplementary
  characters, and preserved lone surrogate escapes.
- Floats use CPython shortest round-trip `repr`: decimal notation for decimal
  exponents `-4..=15`, scientific notation outside with explicit sign and at
  least two exponent digits, integral floats keep `.0`, and `-0.0` is kept.
- Integers are arbitrary precision. CPython's default 4,300-digit int/str
  conversion limit is enforced for parity.
- Duplicate keys (even equal values), non-finite numbers, invalid UTF-8, BOM,
  trailing input and bounded-size/depth/string/key/array violations are
  rejected with the Python `JsonEnvelopeError` messages.
- Parse depth is checked at the root as depth zero; arrays and object values
  increment depth. The object-entry count is not limited, matching Python's
  `_normalize_json` (only arrays use `MAX_COLLECTION_ITEMS`).
- Stored timestamps are read with CPython C `datetime.fromisoformat` forms
  (calendar/week/basic dates, arbitrary single-character separator, hour/
  minute/second forms, comma or dot fractions, offset seconds/fractions),
  truncated to microseconds and required to carry an offset.
- Chain-covered identifier and `occurred_at` strings are hashed from their raw
  stored bytes. Trimming/reformatting happens only in the typed view.
- Geometry is `f64`; STL packing narrows to `f32`. Construction follows the
  Python operation order, including `sin`/`cos`/`sqrt`; exact fixture bytes are
  a Linux x86-64/glibc contract and last-bit trig parity on Windows is
  unverified.
- `kala-budget` is a default-context Decimal/ledger slice, not a general
  Decimal library. Pydantic multi-line validation text is not reproduced;
  wrappers must own response formatting.
- STEP output is deterministic for identical inputs (fixed `FILE_NAME`
  timestamp). STEP bytes are not a cross-implementation byte contract; the
  structural re-import checks are the contract.
- The core API's 4xx branch echoes the raised detail, cut to 128 code points.
  The **studio** module applies a stricter rule and drops a credential-shaped
  or over-long detail. That difference is deliberate and unchanged.

## Security and proof boundaries

- `kala-artifacts` on Unix opens every component relative to held descriptors
  with `O_NOFOLLOW`/`O_DIRECTORY`/`O_CLOEXEC`, refuses symlinks, non-directories
  and non-regular files, publishes with `O_EXCL` + same-directory `linkat`, and
  re-hashes every read. The accepted adversarial tests cover symlinked
  roots/objects/buckets, path replacement, FIFOs, failed publication cleanup,
  same-digest races, threads and real processes.
- The Windows artifact path is compile/clippy-checked only. NTFS rename
  behavior, reparse-point execution proofs, hostile Windows fixtures and
  inherited ACL behavior remain pending; the Windows store does not claim the
  Unix permission modes or portable directory `fsync`.
- `kala-persistence` verifies the complete chain on reads and refuses
  non-lowercase stored digests; appends verify the tail as the Python source
  does. Typed repositories and migration from existing user databases are not
  implemented.
- The geometry worker is **not browser-facing**: it binds `127.0.0.1:0`,
  refuses any `Origin` header, requires the exact `Host`, and requires a
  per-launch bearer token that travels only through the environment. It is
  **opt-in** now: no worker is started unless `KALA_STUDIO_GEOMETRY_WORKER`
  names one, and the packaged app never starts one.
- The `kala-runtime` gateway is one egress choke point: a hosted provider
  without a policy engine is denied, every call needs a per-request
  approval bound to the request digest, the manifest is recorded before the
  dispatch, and a remote provider never receives an artifact payload. Its
  `Display`/`Debug` and transport errors never carry a credential.
- The `kala-tools` executor keeps the capability and side-effect checks,
  the strict jail (verified by its own preflight probe), the process-group
  containment, the artifact confinement and the worker pool cap. A tool
  cannot report a decision only the kernel can make, and an unrecognised
  artifact format is classified as a design binary. The 23-case live
  differential includes each misbehaviour entry point.
- The front door's own boundary is unchanged and still first: Host (421) →
  body size (413) → Origin (403) → bearer (401) → static → router. The
  native geometry routes are only reached after the front door's own gate;
  the worker, when it is used, is a second, independent gate behind it.
- The packaged Electron child environment is **sanitised**: `PYTHONPATH`,
  `PYTHONHOME`, `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and
  `KALA_STUDIO_GEOMETRY_WORKER` are removed, and the packaged binary
  resolver never consults a PATH Python or a repository virtualenv.
- The `kala-cad` C++ boundary is exception-safe: every entry point catches
  `Standard_Failure`, `std::exception` and `...`, writes a bounded status
  and a bounded exception class, and no exception crosses the C ABI. The
  kernel handle is owned by exactly one Rust value and is never shared
  between threads; the Studio's geometry lock serialises the kernel work.
- Live Python differentials and corpus fixtures are the parity evidence. No
  security review of the standalone Studio HTTP surface is part of this
  workspace; Studio is reviewed separately.
- No benchmark, speedup, memory, Windows runtime, COM or full-migration claim
  is made.

## Standalone crates and the excluded members

`crates/kala-contracts` and `crates/kala-process` each declare their own
`[workspace]` and lock, and are listed in this workspace's `exclude`. They are
**not members** of the root build and are not built by the commands above.
Their cells continue to own their manifests, locks, source and tests; this
round changed no file under either directory.

- `kala-contracts` is the standalone typed contracts crate.
- `kala-process` is the standalone bounded subprocess/sandbox crate.

The same applies to `kala-kernel`, `kala-memory`, `kala-tools`,
`kala-sandbox`, `kala-api` (and its nested `kala-api/server` workspace),
`kala-application`, `kala-cli`, `kala-design-intent`, `kala-domains`,
`kala-repositories`, `kala-solidedge-client`, `kala-cad-providers`,
`kala-cad`, `kala-studio-journal` and `kala-runtime`: each is a standalone
workspace with its own `Cargo.lock`, under its own cell. The `exclude` list
in `backend/Cargo.toml` carries all of them, because Cargo refuses a path
dependency on a second workspace root unless the root manifest excludes it.

`kala-studio` **is** a root member. Its native binary/UI staging and the
Electron packaged layout are documented in `docs/rust-studio.md`. The Python
Studio service and its tests remain unchanged as the reference oracle; this
workspace does not delete or replace them. The Python reference has **15**
public route-methods; the Rust front door answers **16** (the same 15 plus the
ADR-0011 view-only import) — and **all 16 are answered in Rust**, with the
Python geometry worker as an opt-in development fallback.
`npm run check:native-readiness` in `studio/` reports that split, and the
built stage's `studio/release/native-readiness.json` classifies the artifact
`release` / `releaseEligible: true`.

Root dependency/version edits affect the excluded crates they consume by path
(`kala-json`, `kala-geometry`, `kala-step`, `kala-budget`). Their locked
metadata still resolves; they must re-run their own suites before their next
acceptance.

## Known scope limits

- Canonical JSON, chain, persistence, the six libraries above, the typed
  contracts, the kernel, the memory, the budget, the repositories, the
  domain packages, the tool protocol, the sandbox, the tools registry,
  simulator and executor, the model gateway **and the agent runtime**, the
  core API connection layer **and its routes**, the **composition root**,
  the **CLI**, the CAD provider connections, the Solid Edge client, the
  design intent, the **OpenCascade adapter** and the **Studio session
  journal** are ported. The real Windows/COM executor, the Windows build and
  run, the code signing and the publication remain future cells.
- The **Studio** is cut over: the desktop package runs only Rust binaries and
  the front door computes its own geometry. The **core product** is not:
  nothing composes `kala-application`, `kala-api` and `kala-cli` into a
  service, the Python console script is still the installed entry point, and
  `kala-agent serve` still refuses with the gap message.
- The two things that stay in Python **on purpose** — the FreeCAD adapter
  (it runs inside FreeCAD) and the Solid Edge bridge (it runs on Windows) —
  plus the geometry worker as a **development fallback**, are listed in
  [What is still Python](#what-is-still-python).
- Journal appends validate the verified tail (as Python does); reads and `get`
  verify the whole chain. No head cache bypasses a read check.
- Deep stored JSON is accepted without a fixed native depth ceiling. CPython's
  recursive decoder limit is host-specific: where Python refuses a well-formed
  deep document, the accepted sets intentionally differ. Tool-protocol input
  remains capped at depth 48 and 1 MiB.
- Raw `Decimal` values and non-string object keys are outside the typed JSON
  value domain; callers must choose a representation before canonical encoding.
- Python normalization reports the first limit violation in insertion order;
  this implementation reports the first in code-point key order. The error set
  is identical, but a document with several independent violations can name a
  different one first.
- `cargo test` fails (rather than silently skips) when a required Python or
  FreeCAD oracle is unavailable.
- No Windows binary, MSVC runtime, COM proof, HTTP parity, code signing,
  publication or performance result is claimed here.
- The **declared differences** between the two implementations — the native
  domain-package identity, the schemas' extra-field order, the API routes'
  seven differences, the kernel's stricter restore, the sandbox's fail-closed
  behaviour, the executor's dead `timeout_seconds`, the application D1 and
  the **two native-geometry differences** (the OpenCascade binding identity
  and the display-mesh vertex order) — are listed in one place in
  [`docs/rust-migration-status.md` §4](../docs/rust-migration-status.md).
