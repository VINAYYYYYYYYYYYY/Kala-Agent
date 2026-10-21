# Kala Studio limitations, engineering safety, packaging trust, testing

> Scope: what this wave explicitly does **not** ship, the scope of the
> chat-first agent, the CAD-provider model and the OpenCascade
> geometry/estimation workbench, what an engineer must still do by hand,
> the trust posture of the unsigned installer, and the procedure to test
> the desktop bundle honestly.
>
> This document is the source of truth for honesty claims. The README
> links to it; the studio backend and the desktop bundle must
> continue to honour every claim here.
>
> **Current state (2026-10-20).** The studio is **Rust-only** (ADR-0010
> phase 2). The Linux AppImage is **built and smoke-tested but unsigned**;
> the **Windows installer is a script that has never been executed**; the
> **MinGW cross-check is still pending** on a missing C toolchain;
> **Solid Edge needs a Windows host** and has never built a part; and
> **FreeCAD is detected on the system, never bundled**, with the adapter's
> dependencies vendored for **cp311–cp314**. The Python geometry worker is
> a **development-only opt-in**, not a product layer.

## Roadmap of limitations

The studio is a **preview build**. The following items are open and
will not be silently patched over:

### Native vendor integrations

- **SOLIDWORKS** — Windows COM automation. Requires a licensed
  installation of SOLIDWORKS on the host. The studio backend does not
  ship a COM bridge. The integration row reports
  `status: native_unavailable` with `platforms: ["windows"]`,
  `host_sdk_required: ["SOLIDWORKS"]`, and `notes: "Requires licensed
  SOLIDWORKS installation; COM automation not implemented in this
  wave"`.
- **Solid Edge** — Windows COM automation through the ADR-0008 bridge.
  This is the one vendor bridge that exists: the protocol, the bridge
  service and a native Rust client for protocol v1 are in the tree. It is
  **fixture-tested, not live-proven** — no Solid Edge build has ever been
  executed, and the bridge milestone that has landed has no production
  native executor. It needs a **Windows host with a licensed Solid Edge
  installation**; on any other host the provider refuses with the reason.
  See [cad-providers.md](cad-providers.md).
- **AutoCAD** — `.NET` plugin / ObjectARX / automation. Requires
  licensed AutoCAD and a registered plugin assembly. The studio
  backend does not ship one. Integration row reports
  `native_unavailable` with `host_sdk_required: ["AutoCAD",
  "AutoCAD .NET plugin"]`.
- **Fusion 360** — Add-in. Requires a Fusion installation and an
  installed add-in from this repository. The studio backend does not
  ship arbitrary external Python; the Fusion add-in lives in a
  separate, future distribution.

A future wave may publish a vendor bridge; until then the studio
correctly refuses any "launch / connect / drive" claim. The Solid Edge
bridge is the one that exists, and it is still refused as a *proven*
capability because no live build has run.

### Analysis solver integrations

The integration registry lists five solver names as **representative
target integrations**: ANSYS Mechanical, Abaqus, COMSOL, Simcenter
Nastran, and Altair OptiStruct. They are **not** a ranked top-five
and **not** evidence-backed market leaders. The five-name set is a
request artefact from the wave brief; the studio asserts no
market-share claim about any of them.

The studio does **not**:

- Launch any solver executable.
- Read any solver result file (`.frd`, `.odb`, `.csv`, `.txt`, etc.)
- Fabricate a maximum displacement, a maximum von Mises stress, a
  factor of safety, or a pass/fail verdict.
- Provide a placeholder stub function that returns plausible numbers
  when the solver is absent.

Every analysis handoff returns either a neutral geometry file (STL or
STEP) or an honest `analysis_solver_not_available` error. The studio
never substitutes a stand-in computation for a real one.

### Open-source tools

- **FreeCAD** is now a real **CAD provider**, reached through the native
  `kala-cad-providers` connection. The studio does not import FreeCAD into
  its own process: it runs `FreeCADCmd` as a subprocess with a scrubbed
  environment and drives the `adapters/freecad` distribution through its
  documented entry point. The probe finds `FreeCADCmd` / `freecadcmd` on
  `PATH` and runs `<exe> --version` with a 20 s timeout; the version
  reported on the dev host is **1.1.4**. **FreeCAD is detected, never
  bundled**: the installed app carries the adapter and its Python
  dependencies vendored per CPython ABI (**cp311, cp312, cp313, cp314**),
  puts only the matching directory on `sys.path`, and needs no virtualenv
  and no `PYTHONPATH`. A FreeCAD on an unsupported Python is reported as
  provider-**unavailable** with its exact version, never as a crash. See
  [cad-providers.md](cad-providers.md) for the full contract, the 28/28
  corpus result and the assembly-export semantics.
- **OpenSCAD** is reported separately with status `discovered` /
  `not_found`. The studio does not embed an OpenSCAD interpreter
  and does not auto-execute user-supplied `.scad` scripts. There is
  no CSG-only route.
- **OpenCascade.js (WASM)** is a port of OpenCascade compiled to
  WebAssembly that could, in a future wave, give the studio a
  browser-side B-rep kernel. It is **not** used in this wave; the
  preview path is server-side analytic triangulation, not a
  browser-side B-rep. See
  <https://github.com/donalffons/opencascade.js>.

### The CAD providers

- **There is no default CAD provider.** The composer starts with no CAD
  target; a design turn with no target asks the operator to choose and
  builds nothing. OpenCascade is labelled **opt-in** and is never
  selected automatically.
- **The provider that built the model is the only measuring authority.**
  A provider that reports no property is shown as `—` with the check
  status `not_reported`; Kala never substitutes a number from another
  engine. FreeCAD reports no centroid and no mass, and both stay `—`.
- **Solid Edge has no live proof.** The Windows bridge, the protocol-v1
  client and the `ConstructionScript` lowering are tested against
  contract fixtures. No Solid Edge build has been executed, so no Solid
  Edge part, mass property or export is claimed. On a non-Windows host
  the provider refuses with a reason rather than pretending.
- **The service and the Python reference share the same provider rule.**
  Chat and `POST /design` through the **native Studio service** pass the
  selected provider through unchanged, and the backend alone decides
  whether that provider can build here. FreeCAD builds there: the live
  differential includes the real FreeCAD chat turn (design
  `a9d04f8ff14e…`, volume `35571.416047 mm³`, area `11889.101731 mm²`,
  `verified: true`) and matches the Python server case for case. On Linux,
  Solid Edge refuses in the backend's own wording and the turn carries
  `design: null`; **no other provider is tried**. There is no fallback and
  no automatic selection. See
  [cad-providers.md](cad-providers.md#chat-and-design-through-the-front-door).
- **The Solid Edge bridge caps a procedure at 8 parts.** The script budget
  is 64 components, so a Solid Edge design can never use the whole
  budget: a script that compiles to more than 8 parts is refused before it
  reaches the bridge. The protocol's 600-operation cap is a second line of
  defence that a contract-valid script cannot reach (the maximum is 543
  operations).

### What is still slow, capped or unbounded

These are the limits that remain after the 2026-10-21 wave. Each one is
measured or configured, not estimated.

- **A 500-step FreeCAD build is slow.** Measured on FreeCAD 1.1.4 with
  the real `FreeCADCmd`: a 202-step plate with 100 pockets takes **58.8 s**
  in the provider (59.8 s through the Studio), and a 500-step plate with
  249 pockets takes **1 009.5 s (16.8 min)** idle and **1 250 s (20.8 min)**
  under load. The contract allows the 500-step design and the **1 800 s**
  FreeCAD deadline covers it, but with much less margin on a loaded host,
  and it is not a pleasant chat turn. The cost is superlinear in the
  number of pockets, because every pocket recomputes the body to measure
  what it removed.
- **The Solid Edge 8-part cap** is far below the 64-component script
  budget, so a Solid Edge design cannot use the whole design budget.
- **The Windows process-tree kill uses `taskkill /T /F`, not a Job
  Object.** Every CLI turn runs in its own process group, and a timeout,
  a failure or a client disconnect kills the **whole group**, not only the
  direct child. On Unix that is `kill_process_group(SIGKILL)` with a direct
  kill of the child as a backstop. On Windows the documented OS tool
  `taskkill /T /F /PID <pid>` walks the parent/child tree instead of a Job
  Object with `KILL_ON_JOB_CLOSE`. A Job Object would be the stronger
  mechanism (it is what the sandboxed tool runner uses), but it needs a
  Windows host to land and prove safely. **Neither the `.cmd` resolution
  nor the `taskkill` fallback has been executed on a Windows host from
  this repository**: the Windows-only fragments are type-checked against
  the `x86_64-pc-windows-msvc` target, and that is the honest limit of
  the proof.
- **The journal and the artifact store grow without bound.** Every design
  stores its script, its STEP and its native artefact (for FreeCAD, the
  `.FCStd`) in the content-addressed store, and the journal appends the
  turn's events. The files live under the studio data directory:

  ```
  <studio data dir>/studio-journal.sqlite3     the hash-chained journal
  <studio data dir>/artifacts/objects/<xx>/<digest>   the artefacts
  ```

  There is **no retention policy, no prune route and no size budget
  today**: nothing in the journal or the artifact store deletes anything,
  so the directory grows with every design. The in-memory design cache is
  only 32 entries. Reclaiming the space is a manual operation today
  (delete the files under the studio data directory). A retention policy
  (last N designs or a size budget) and a safe prune path are **not**
  implemented.
- **The live model name of a subscription CLI is unknown.** The CLIs do
  not report which model they picked, so the studio says
  `Codex CLI default model (chosen by Codex)` and points at the model
  picker, where pinning a model makes the reply name it. See
  [Provider honesty](#provider-honesty).
- **The artifact download cap is 64 MiB, and it is independent of the
  STEP-import ceiling.** The download route refuses anything larger than
  **64 MiB** with `413 artifact_too_large`, while the journal's own
  per-artifact ceiling is `max(64 MiB, KALA_STEP_IMPORT_MAX_BYTES)`. So a
  host that raises `KALA_STEP_IMPORT_MAX_BYTES` above 64 MiB can *store*
  a bigger STEP and still cannot *download* it. The cap is the download
  contract, not an accident.
- **A cancel cannot interrupt the in-process OpenCascade build.** The
  out-of-process FreeCAD build has its **whole process group** killed when
  the client closes the request, promptly. The in-process OCCT build
  cannot be interrupted inside a single compile, so the cancellation token
  is honoured at the **next phase boundary** — the worst case is one
  **60 s** compile that finishes first. And the **Python reference**
  cannot cancel a running synchronous build at all: its `/design` route
  runs inside the FastAPI thread pool, and a thread cannot be dropped by
  the event loop. The cancellation is **native-only**, and the native
  service is the shipped path. See
  [studio.md](studio.md#cancelling-a-turn).

### The geometry viewport

- The clay (3D) viewport uses a **narrow-field-of-view perspective
  camera** (38°), not an orthographic camera. The pencil (2D) mode uses a
  narrower 24° perspective that reads as orthographic. A real
  `OrthographicCamera` switch was **not** implemented, so a part
  photographed in clay mode shows the perspective foreshortening that a
  true orthographic projection would not have. The Ortho auxiliary tab is
  a separate matter: it is drawn from the kernel's hidden-line result, not
  from the camera.
- The sheet `SCALE` cell reads **`FIT`**, not `1:1`: the viewport frames
  the part to fit the sheet, so there is no fixed model-to-paper ratio and
  claiming one would be false.
- The Section A–A plane for the analytic primitives is the **mid-plane of
  the measured bounds**; for a design it is the kernel's section at the
  offset the design asks for. There is no operator-movable cutting plane
  and no arbitrary offset control.

### Geometry limits

- WebGL renders **triangle meshes**, not exact B-rep. A user looking
  at the viewport sees a tessellation, not a parametric solid.
- STEP export requires the **native kernel**. The default build links
  OpenCascade 7.9.3 through `kala-cad`, and the analytic primitives are
  written by `kala-step`. A **portable** build
  (`cargo build -p kala-studio --no-default-features`) links no kernel and
  reports `design_available: false`; the export route then answers `503`
  with an actionable message. The studio never ships a placeholder STEP.
- The preview mesh generators are covered by regression tests
  (`tests/unit/test_studio_geometry.py` for the Python reference, the
  `kala-geometry` crate for the native path). An independent check this
  wave computed positive (outward) signed volumes for a box, a
  cylinder, and a sphere, so triangles now wind consistently outward
  and the earlier winding-defect warning has been removed.
- Primitive counts are bounded. The studio backend rejects requests
  whose parameters would produce more triangles than the
  configuration allows; the limit is operator-tunable through
  `KALA_STUDIO_MESH_LIMIT`.

### Nominal geometry estimates

The workbench presents a nominal bounding envelope and nominal
analytical volume and surface-area estimates for the active
primitive. Their exact scope:

- **Nominal, closed-form, ideal-primitive values.** Volume is shown in
  `mm³` and `cm³`, surface area in `mm²` and `cm²`, and the bounding
  envelope (with its envelope volume) in `mm` / `mm³`. The values are
  the closed-form formulas for the ideal primitive (box `xyz` /
  `2(xy + yz + zx)`; cylinder `πr²h` / `2πr(r + h)`; sphere
  `(4/3)πr³` / `4πr²`) computed from the same validated parameters the
  preview mesh uses. Unit conversions are nominal: `1 cm³ = 1000 mm³`
  and `1 cm² = 100 mm²`.
- **Not solver output.** These numbers are a sizing aid. They are not
  measured from the tessellation, not a stress, displacement, factor
  of safety, fatigue life, mass, or certification, and never a
  fabricated FEA/CFD result.
- **Bounding envelope is server-verified.** The preview route returns
  the mesh and its `bounds`; the bounding envelope is re-derived from
  that server mesh, so a local approximation can be compared against
  the server result.
- **Exact B-rep confirmation needs the kernel.** Exact B-rep volume and
  area mass properties are confirmed with OpenCascade's `BRepGProp` by the
  native `kala-cad` adapter (the default build links the **system OCCT
  7.9.3**; the Python reference uses the `cadquery-ocp` binding
  **8.0.1.1**). The exported STEP shape round-trips to within `1e-7`
  relative of the closed-form volume. Without a linked kernel (a portable
  `--no-default-features` build), STEP export returns `503` and no exact
  mass property is claimed.
- **Tessellation is not exact.** The analytic preview mesh approximates
  curved surfaces. The sphere tessellated volume is `1.066 %` below the
  analytic sphere volume at the default resolution; the estimate table
  reports the analytic ideal value, not the tessellated one.

### Provider honesty

- The studio never claims to use a model the user has not configured.
  The `/api/studio/providers` route is the source of truth.
- The three **subscription** routes (`claude_code`, `codex_cli`,
  `opencode_cli`, plus the historical alias `codex`) run the operator's
  own CLI as a subprocess. A route is `configured` only when the probe
  found the binary **and** the CLI reported that it is signed in; the
  studio reports *that* the CLI is signed in, never *as whom*, because the
  probe reads only the exit status of the CLI's own status command.
- The live **model name of a subscription CLI is unknown**. The CLIs do
  not report which model they picked when the request names none, so the
  studio says exactly that — `Codex CLI default model (chosen by Codex)` —
  and never invents a model name. Pinning a model in the picker makes the
  reply name it.
- The availability probe is **cached and non-blocking**: a route answers
  from the cache, and a first probe that has not finished inside the 600 ms
  route budget is reported as `checking...`, never as "unavailable". The
  verdict lands in the background; the Studio loads the provider list once
  at boot, so a first render can show `checking...` until the workspace is
  reloaded.
- Subscription access and API access are distinct plans billed
  separately by the provider. The studio never converts a subscription
  turn into a paid API call and never reuses an inherited
  `OPENAI_API_KEY` for the subscription path.
- The studio exposes no arbitrary-provider escape hatch: the accepted
  provider identifiers are fixed in the request schema, and only
  `openai` and `openrouter` perform a real remote API call.
- The studio does not assert legal or terms-of-service claims about
  third-party products. Provider-side eligibility is the provider's
  decision, not the studio's.

## Engineering safety

Kala Studio is a chat-first Mechanical Engineering Agent with an
integrated OpenCascade geometry, estimation, and neutral-file handoff
workbench. It captures intent in chat, builds parametric geometry,
reports nominal estimates, and exports a neutral file. It is **not** an
engineering analysis tool. The following items must remain outside the
studio's scope until a future wave decides otherwise with explicit
evidence:

- **No FEA, no CFD, no solver of any kind.** The studio runs no finite
  element, no finite volume and no fluid solver. It never reports a
  displacement, a stress, a von Mises value, a factor of safety, a fatigue
  life or a pass/fail verdict, and it never fabricates one when a solver is
  absent.
- **Nominal estimates are not analysis.** The volume, surface-area,
  and bounding-envelope figures are closed-form values for the ideal
  primitive. They do not model material, load, constraint, mesh, or
  environment, and they carry no safety margin or certification.
- **A provider's measured volume is not an analysis result either.** A
  measured `35571.416047 mm³` is a geometric fact about a shape; it says
  nothing about whether the part will carry a load.
- **Tolerances are declared, not analysed.** A tolerance string
  (`"Ø10 H7"`, `"ISO 2768-m"`) is **what the operator said**. Kala Studio
  computes no fit class, no limit, no deviation, no stack-up and no GD&T
  call, and it never resolves a fit. The UI labels the list
  *"declared — not analysed"* and the empty state says plainly that the
  backend reports declared limits only.
- **"B-Rep Verified" means three things and nothing more**: the provider's
  measurement agrees with the declared envelope, the model is a valid
  solid, and it has the expected solid count. It is not a strength,
  fatigue or manufacturability verdict.
- **STEP geometry is not ready-to-solve analysis.** STEP is a
  boundary representation. To run analysis an engineer must still
  choose a meshing strategy, mesh the body, assign materials,
  apply loads, apply boundary conditions, choose a solver, run the
  solver, and interpret the result. The studio's role ends at
  exporting a file the next tool can read.
- **No material database.** The studio does not store a material
  catalogue and does not look up Young's modulus, Poisson's ratio,
  yield strength, density, or fatigue data. The user supplies
  material data to the next tool. A declared `material` string is a
  label, not a density, and no density is inferred from it.
- **No unit system beyond length.** The studio accepts dimensions in
  millimetres and treats them as nominal. It does not enforce a
  unit system on mass, force, pressure, temperature, or any other
  quantity. Exporting a STEP file does not commit the user to any
  particular unit interpretation downstream.
- **No tolerance stack-up, no GD&T.** These are engineering
  decisions, not parametric primitives.
- **No certification, no regulatory claim, no signed-off design.**
  Files produced by the studio carry no provenance beyond the
  operator's local environment. They are not engineering deliverables
  in any regulatory sense.
- **The session journal is a session log, not a revision system.** The
  studio now keeps its **own** SQLite journal and artifact store under
  its data directory, so a design turn, its provider build, its artefact
  digests and its measurements can be audited afterwards. That journal is
  **not** the kernel's revisioned, content-addressed store: the studio
  saves no revisions, marks no approvals and shares no state with the
  core `KalaApplication`, which remains the system of record. The
  journal's chain verifies its own integrity; it certifies nothing about
  the engineering.

## The native Studio service and the Python geometry worker

The studio has **one** back end on the product path, and the split that used
to exist is deliberately gone (ADR-0010 phase 2):

- The **native Studio service** (`backend/crates/kala-studio/`) owns the
  HTTP boundary, the settings and token stores, the static UI, the chat
  providers, the CAD-provider routing **and the geometry**. The geometry
  comes from the **real OpenCascade 7.9.3** through `kala-cad`, one narrow
  C++ adapter (`src/ffi/adapter.cpp`) that calls OCCT and never rewrites
  it. `kala-cad-providers` owns the provider connections,
  `kala-design-intent` owns the strict intent parser, and
  `kala-studio-journal` owns the hash-chained session journal and the
  artifact store.
- The **Python geometry worker** is a **development-only opt-in**. It is
  started **only** when `KALA_STUDIO_GEOMETRY_WORKER` names one, and there
  is **no development default** any more. It is not in the release bundle,
  and the packaged app starts no Python process at all.

Consequences a user must understand:

- **One runtime ships.** The boundary that used to carry the geometry is
  now in-process, behind the same authenticated loopback API with a fixed
  route list.
- The worker, when it is used at all, is **not browser-facing**: it binds
  `127.0.0.1:0`, refuses any `Origin` header, requires the exact `Host`,
  and requires a per-launch bearer token that is passed **only** through
  the environment, never in `argv` and never in a log.
- The worker is **restarted** with a capped backoff (250 ms → 5 s) if it
  dies. While it is down the geometry routes answer `503
  geometry_worker_unavailable` and the native routes keep working. **No
  mesh, no STEP file and no measurement is fabricated in that state.**
- **The split does not duplicate provider logic.** A chat or design turn
  carries the operator's CAD provider through unchanged; the backend alone
  decides whether it can build here. FreeCAD builds through the service,
  Solid Edge refuses honestly on Linux, and no fallback provider is tried.
  The live differential passes the real release binary against the live
  Python server: **295 / 268 / 20 / 56 cases, 0 mismatches**. See
  [The frozen corpus is not all contract](#the-frozen-corpus-is-not-all-contract).
- The **Python reference server** still exists and is still the parity
  oracle. Nothing was deleted; the Python tree survives as the oracle every
  differential compares against.
- The **native primitive geometry** crates (`kala-geometry`, `kala-step`)
  are **on the default path**: the `native-geometry` feature enables
  `native-primitives`, and the analytic meshes and the analytic STEP writer
  answer the primitive `preview` / `export` / `handoff` routes. The explicit
  portable build is `--no-default-features`, which links no kernel and
  reports `design_available: false`.
- **Two differences are declared, and only two.** The native build links
  the **system OCCT 7.9.3** while the Python oracle imports the
  `cadquery-ocp` binding **8.0.1.1** (the reported `opencascade` provider
  version therefore names a different binding), and the two OpenCascade
  versions assemble the same STEP file with its faces in a different order
  (the display mesh is compared by an order-independent fingerprint). The
  geometry, the measurements, the checks, the drawing and the section are
  identical. The full list is in
  [rust-migration-status.md §4.8](rust-migration-status.md).

## The core API status bug (fixed)

A latent defect in `src/kala_agent/api/errors.py` was found and fixed in
this wave. The handler compared an `HTTPException`'s status against
`status.HTTP_500_BAD_REQUEST`, a name Starlette does not define. Reading
the missing name raised an `AttributeError` **inside** the handler, so for
every status of 400 or more that was not 404, 405 or 413 the handler never
answered and the caller received a generic `500 internal_error` instead of
the status it asked for.

| Raised status | Before | After |
|---|---|---|
| 400, 401, 403, 409, 418, 429, 499 | `500 internal_error` | the same status, `request_rejected`, detail cut to 128 code points |
| 404 / 405 / 413 | unchanged | unchanged |
| 500 | `500 internal_error` | unchanged (byte-identical) |
| 501 … 599 | `500 internal_error` | the same status, `internal_error` |

The 500 case was byte-identical before and after, which is why the defect
hid so well.

Two honest notes:

- The fix was **one comparison**. It is covered by 19 new Python
  regression tests (13 of them fail on the pre-fix source) and the Rust
  port of the handler now follows the fixed behaviour, with a 591-case
  live differential that reports **0 mismatches** and is
  mutation-sensitive.
- The API module still echoes the detail of a 4xx (cut to 128 code
  points). The **studio** module applies a stricter rule and drops a
  credential-shaped or over-long detail. That policy difference was **not**
  changed by the fix; if the API module should refuse credential-shaped
  details too, that is a separate change to the same handler.

## The cold-reload surface-area bug (fixed)

A second latent defect was found and fixed in this wave, in the shared Python
geometry worker. `service.py` wrote the
measured payload as `surface_area_mm2` but the cold-reload reader looked for
`area_mm2`. Every design restored from the session journal therefore had
`measured.surface_area_mm2: null`, and the agent card showed `—` for area
after a worker or server restart. The volume, envelope, validity and every
check survived, so the design still worked; only the area was lost.

The fix is **reader-only**:

```python
area = _finite_or_none(value.get("surface_area_mm2"))
if area is None:
    area = _finite_or_none(value.get("area_mm2"))   # legacy fallback
```

The written payload format is untouched, so existing journals restore
correctly and a payload that carries both keys keeps the real value. The
**native** geometry service carries the same reader, so the product path
and the reference server behave identically.

The tests read the journal's own canonical `payload_json` for the
`provider.build.completed` event, assert that the stored bytes carry
`surface_area_mm2`, then open a second service on the same data directory
and assert that the area (`11889.101731 mm²`) and volume survive. There is
also an HTTP cold-restart test with two applications on one data directory,
and a written/legacy reader test. The cold-reload tests **fail on the old
reader** with `assert None == 11889.101731097317` and pass after the fix.
I re-ran the live path through the Rust service: after a restart against
the same data directory, the FreeCAD design came back `cached: true` with
`surface_area_mm2: 11889.101731`.

## Test hygiene: the user's data directory

> **USER DATA IS OFF LIMITS.** The user's real data directory
> `/home/v9i/.local/share/kala-agent/**` is the user's own work: their
> designs, their revisions, their approvals and their journal. No test, no
> differential, no probe and no manual command of this project may open it.
> Since 2026-10-20 every shell session must first run
>
> ```bash
> export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
> export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
>        KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
> ```
>
> and pass explicit temporary `--data-dir` values. The path is never typed
> into a command.

### The one incident, and its assessment

On **2026-10-20 at 05:35:30 +0530**, a worker of the `cli-r1` cell made a
shell quoting mistake and ran the native `kala-agent` binary **once with the
process HOME**. That invocation ran `init`, so the application opened and
closed the existing user data directory and **rewrote `kala.sqlite3`** — the
kernel-state snapshot write that `close()` performs.

What it did **not** do:

- no design, no revision, no approval and **no journal event** was written;
- **no token was read**;
- no other file in that directory changed its modification time;
- the worker did not attempt to repair or further modify the directory.

The assessment: the exposure is **real but bounded**. The file the write
touched is the SQLite database, and the write is the kernel-state snapshot
that `close()` performs — the policy and budget state of the application,
serialised under the reserved `kernel-state:` namespace. So the *bytes* of
the user's database may have changed even though the *engineering record*
did not. The journal is append-only and no event was appended, so the
recorded history is intact and verifiable.

Two honest caveats:

- The incident report states that **no other file** in the directory changed
  its modification time. It does **not** state whether the size, modification
  time and inode of `kala.sqlite3` **itself** were compared before and
  after. That check was added later, by the isolation work described below,
  and it could not be applied retroactively to this incident.
- **No repair was attempted**, and none should be attempted without the
  user's decision. The kernel-state snapshot is re-derived from the state
  the application itself restored, so the recorded policy and budget are the
  same facts in a freshly serialised form — but the *bytes* are not
  guaranteed to be identical to what was there, and "probably equivalent" is
  not a proof.

The response was procedural: the wave conventions gained the **USER DATA
rule** quoted above, and every test, differential and probe since then uses a
temporary `HOME` and temporary data directories.

### The isolation work

The `test-isolation` cell of the design-turns wave made the isolation a
property of the test suite instead of a habit:

| File | Change |
|---|---|
| `tests/conftest.py` | New. An autouse, function-scoped `isolate_user_data_dirs` fixture plus the `isolated_home`, `isolated_xdg_data_home`, `isolated_kala_data_dir` and `isolated_studio_data_dir` helpers. |
| `tests/unit/test_data_dir_isolation.py` | New. Pins the fixture contract and proves the default resolution lands under the temporary `HOME`. |
| `tests/unit/test_studio_server.py` | `test_token_is_never_echoed_in_any_response` no longer builds the application with `data_dir=None` (which resolved to the **real** default store); it uses the test's temporary `data_dir`. |

For every test, before the body runs, the fixture redirects `HOME`,
`XDG_DATA_HOME`, `KALA_AGENT_DATA_DIR` and `KALA_STUDIO_DATA_DIR` into the
test's `tmp_path`, and then **asserts** that `Path.home()` is the isolated
one — so a platform that ignored the override would fail loudly instead of
silently falling back to the real home. The fixture mirrors the production
layout (the studio directory inside the core data directory) and covers all
three resolution orders the suite can reach: `KALA_STUDIO_DATA_DIR`,
`KALA_AGENT_DATA_DIR`, and `Path.home() / ".local/share/kala-agent"`.

### The proof

The proof used **`stat` and `ls` only** — the user's files were never opened,
read, modified or deleted.

| File | Baseline (size, mtime epoch, inode) | Final |
|---|---|---|
| `studio/studio-journal.sqlite3` | `237568, 1792449777, 3714183` | **identical** |
| `kala.sqlite3` | `131072, 1792402275, 3457516` | **identical** |

A background watcher polled both directories every 0.5 s across the full test
run: **357 polls**, the watcher log contains exactly two distinct `stat`
lines and both are the baseline (no mtime change even momentarily), and the
hit log is empty — **0 occurrences** of any `-shm` or `-wal` name.

All suites are green: **2473 passed, 1 skipped** (the skip is the live
Solid Edge test, which needs Windows).

### What is still open

- The **normal** Studio CLI resolves its data directory from the
  environment when `--data-dir` is absent. The precedence is now the
  documented one — `--data-dir` > `KALA_STUDIO_DATA_DIR` >
  `KALA_AGENT_DATA_DIR/studio` > `$XDG_DATA_HOME/kala-agent/studio` >
  `$HOME/.local/share/kala-agent/studio` > `$TMPDIR/kala-studio` — and it
  is implemented identically in the native service and in the Python
  reference. `KALA_STUDIO_DATA_DIR` is no longer ignored: it is the studio
  directory itself and it wins over every lower row.
- The **packaging smokes** (`packaging/studio_smoke.py`,
  `packaging/native_studio_smoke.py`) and
  `studio/test/nativeBackendPackaging.test.ts` now launch the backend with
  an explicit temporary `--data-dir` **and** a temporary `HOME`, and they
  assert that the launch banner names that temporary directory and never
  `.local/share/kala-agent`.
- The adapter suites (`adapters/freecad`, `adapters/solidedge`,
  `adapters/solidedge-bridge`) have their own roots and do not load the
  repository's `conftest.py`. They do not reference the Kala data directory
  at all, and the watcher confirms they never touched it — but their
  isolation is a property of their own code, not of a shared fixture.

## Packaging trust

### The release packaging state

The packaging decision has been taken (ADR-0010 phase 2): **the bundle
ships one runtime, Rust only.** The desktop release bundle is a single
native binary plus the staged OCCT closure, the packaged UI and the FreeCAD
adapter. There is no Python geometry worker in it.

- The **Linux AppImage exists and has been smoke-tested**:
  `studio/release/kala-studio-0.1.0-x86_64.AppImage`, **unsigned**. Its
  exact size and SHA-256 are recorded in
  [packaging/linux/README.md](../packaging/linux/README.md) — they change
  with every rebuild, so they are not duplicated here, and a digest quoted
  anywhere else in the tree goes stale. The smoke ran from the **extracted**
  image under a literal `env -i`:
  the backend starts with `PATH=""` (no Python at all), the health document
  is `status: ok` with `design_available: true` and `step_available: true`,
  the OpenCascade L-bracket is **35571.416047113315 mm³**, the XZ section
  at `y = 4` is **2321.4601836602546 mm²** (exact, one outer and one
  inner loop), the STEP round trip re-imports one solid with equal bounds,
  and the **FreeCAD** provider (the host's own FreeCAD 1.1.4, not bundled)
  builds the same L-bracket at **35571.416047 mm³** with
  `measured_by: FreeCAD`.
- The old release rule (`metadata.nativeStep === true`) is **replaced** by
  the native-design readiness report. A stage that lacks the bundled
  FreeCAD adapter is refused, and publication is forced to `never`. The
  readiness report is green for the current feature set and classifies the
  built stage `release` / `releaseEligible: true`.
- **The Windows installer was never produced.**
  `packaging/windows/build-windows.ps1` is written and reviewed, but it
  has **never been executed**: this host has no Windows and no `pwsh`. The
  script is a **definition, not a Windows-runtime proof**; the user's
  Windows machine runs the build and the post-install checklist.
- The **Electron window was never rendered** on the build host
  (`xvfb-run` is absent), so the smoke verifies the packaged **backend**
  and the **served UI root** by extraction, not the Chromium shell.
- The AppImage is **unsigned and unpublished**. The release workflow
  (`.github/workflows/studio-release.yml`) is written but was not pushed or
  run, and the older `.github/workflows/studio.yml` still describes the
  pre-cutover CI matrix and the retired `nativeStep` marker.

### Signing posture

- **No code-signing certificate** is configured in this wave. The
  Windows `.exe` and `.nsis.7z` artefacts are **unsigned**. SmartScreen
  will warn; the user is expected to right-click -> Properties ->
  Unblock and accept the publisher warning.
- **No Linux signing key** is configured. The AppImage has no GPG
  signature. A user who wants to verify the build must rely on the
  commit SHA and the artefact's SHA-256 digest.
- When a CI run publishes the **unsigned artefact**, it prints the
  **SHA-256 digest** in the GitHub Actions summary. The digest is **not** a
  substitute for a signature; it only proves that the artefact
  uploaded to the run is the artefact the user downloaded. **No hosted run
  was inspected in this wave**, so today the digest of the file in this
  repository is the only provenance available.

### Provenance

- The **Linux artefact in this repository was built on the development
  host**, not on a hosted runner. `packaging/linux/build-appimage.sh` runs
  `npm run build`, the native staging
  (`node scripts/native-backend.cjs --require-freecad-vendor`) and then
  `node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never`.
  The **SHA-256 in [packaging/linux/README.md](../packaging/linux/README.md)**
  is the provenance of that file.
- The release workflow (`.github/workflows/studio-release.yml`) is
  **written but was not pushed or run**. No CI artefact is published, and
  the older `.github/workflows/studio.yml` still describes the pre-cutover
  CI matrix and the retired `nativeStep` marker.
- The CI workflow **does not** authenticate against any LLM provider, does
  not push to a registry, does not sign artefacts, and does not modify the
  host system.
- Source provenance comes from the GitHub commit SHA in the run header. A
  user who wants to rebuild must check out the same
  commit.
- The **OCCT closure is host-built**: the 23 staged `libTK*.so*` libraries
  come from the build host's OCCT 7.9.3. A distribution built elsewhere
  stages its own closure; the staging code path is the same.

### Why this matters

The studio is unsigned because code-signing requires a paid
certificate (Windows) and a GPG key with a published trust chain
(Linux). This wave did **not** budget for either. Until signing is
in place:

- A user should only run the studio on a host they trust.
- An enterprise should treat the studio as developer-mode software
  and run it inside its own sandbox.
- An installer that is asked to elevate, install a service, or write
  to `/usr/` is **not** this build; this build does none of those
  things.

## Testing procedure

The wave's contract is **not** a tested end-to-end installer. It is
a build pipeline and a backend that together produce a working
desktop. The honest testing procedure:

1. **The native service smoke test (the product path).** From a clean
   checkout:

   ```bash
   cd backend
   cargo build --release -p kala-studio --locked
   cd ..
   export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef
   backend/target/release/kala-studio \
     --host 127.0.0.1 --port 8765 \
     --data-dir /tmp/kala-studio-data \
     --static-dist studio/dist
   curl -fsS http://127.0.0.1:8765/api/studio/health
   ```

   Expect `{"status":"ok"}` with `design_available: true` and
   `step_available: true`. **No worker, no Python and no virtualenv are
   needed.** *Run in the cutover wave on port 8796 with a throwaway token:
   health reported `design_available: true` and `step_available: true`.*

2. **Local UI smoke test.** In a second terminal:

   ```bash
   cd studio
   npm ci
   npm run build
   # the service above now serves the UI on http://127.0.0.1:8765/
   ```

   Open the URL in a browser; paste the printed token; create a
   box; export STL; observe the download. *`npm ci` was not run in the
   cutover wave; `npm run build` was run and passed.*

3. **The Python reference server (the parity oracle).** It still serves
   the same public API without the Rust service in front:

   ```bash
   export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef
   PYTHONPATH=src .venv/bin/python -m kala_agent.studio \
     --host 127.0.0.1 --port 8765 --data-dir /tmp/kala-studio-data \
     --static-dist studio/dist
   curl -fsS http://127.0.0.1:8765/api/studio/health
   ```

   Expect `{"status":"ok"}`. The differential harnesses drive this server
   directly. A local `pip install -e ".[studio]"` is a network step and was
   **not** run in the cutover wave.

4. **The development worker fallback.** Only if you are deliberately
   comparing the native backend against the Python worker:

   ```bash
   export KALA_STUDIO_GEOMETRY_WORKER="[\"$PWD/.venv/bin/python\",\"-m\",\"kala_agent.studio\",\"--worker\"]"
   backend/target/release/kala-studio \
     --host 127.0.0.1 --port 8765 \
     --data-dir /tmp/kala-studio-data \
     --static-dist studio/dist
   ```

   The health document then carries a `geometry_worker` block with
   `state: "ready"`. The provider probe must answer the three providers
   with a live version or a live reason. The launch token must be
   **16–128 characters**; a shorter one refuses startup with
   *"the studio token must be between 16 and 128 characters"*. *Run in the
   cutover wave on port 8796 with a throwaway token:* health reported
   `geometry_worker.state: "ready"` and `design_available: true`, the
   probe answered all three providers, and a FreeCAD chat turn built
   `35571.416047 mm³` / `11889.101731 mm²` with `verified: true`.

5. **The AppImage smoke (run in the cutover wave).** The built artefact
   is verified from the extracted image under a literal `env -i`:

   ```bash
   env -i PATH=/usr/bin:/bin HOME=$(mktemp -d) TMPDIR=/tmp \
     XDG_DATA_HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
     KALA_STUDIO_DATA_DIR=$(mktemp -d) \
     /path/to/node packaging/linux/smoke-appimage.mjs \
       --appimage studio/release/kala-studio-0.1.0-x86_64.AppImage \
       --require-geometry --require-freecad
   ```

   Phase 1 runs the backend with `PATH=""` (no Python at all) and checks
   the health document, the served UI, the 401 for an unauthenticated
   preview, the OpenCascade L-bracket, the HLR drawing, the exact XZ
   section and the STEP export/import round trip. Phase 2 puts FreeCAD on
   the `PATH` and runs the FreeCAD provider from the extracted app with no
   repository, no virtualenv and no `PYTHONPATH`. *Result: **PASS**.*

   On a desktop host, the user-facing artefact is the AppImage itself:

   ```bash
   chmod +x kala-studio-0.1.0-x86_64.AppImage
   ./kala-studio-0.1.0-x86_64.AppImage
   ```

   **This step is not proven here**: the Electron **window** was never
   rendered on the build host (`xvfb-run` is absent), and the AppImage was
   not tried on a clean Linux desktop.

6. **The Windows build and install (not run anywhere yet).** On the user's
   Windows 10/11 machine:

   ```powershell
   powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1
   ```

   The script produces the unsigned NSIS installer
   `kala-studio-<version>-setup.exe`; right-click it, accept the publisher
   warning, and follow the ten-item post-install checklist in
   [studio-install.md](studio-install.md). The `.nsis.7z` payload is an
   internal archive, not an installer. **The script has never been
   executed on any host**, so nothing about a Windows runtime is claimed.

7. **CI artefact verification (not run).** No hosted run was inspected and
   the release workflow was not pushed or run, so there is no CI artefact
   to verify. If a run is published later, compare the printed
   **SHA-256** with the downloaded artefact. If they differ, do **not** run
   the artefact.

8. **Adversarial posture.** From a second host, attempt
   cross-origin requests to `http://127.0.0.1:8765/` and confirm the
   backend rejects them. Attempt a request with a wrong token and
   confirm a 401. Attempt a body larger than the configured cap and
   confirm a 413. Attempt a request with `provider: "openai"` and
   no `OPENAI_API_KEY` set; confirm the backend refuses. Attempt a
   request with `allow_remote: false` and `provider: "openai"`; the
   backend confirms the consent field is required. Kill the development
   worker mid-session (only reachable when `KALA_STUDIO_GEOMETRY_WORKER`
   named one) and confirm that the geometry routes answer
   `503 geometry_worker_unavailable`, that the chat and settings routes
   keep working, and that **no** measurement or mesh is served while it
   is down. *The manual walk was not re-run in the cutover wave; the
   boundary gates and the worker-down state are covered by the security
   suite and the earlier rust-e2e-qa evidence.*

### The test suites behind these claims

Run from the repository root:

| Command | What it proves | When |
|---|---|---|
| `.venv/bin/python -m pytest tests -q` | the whole Python suite (unit, security, integration), with the data-isolation fixture active — **1761 passed** | `test-isolation` |
| `.venv/bin/python -m pytest tests/unit -q` | the Python studio, the design turns, the providers and the API error handler — **1667 passed** | **2026-10-21 downloads-cancel cell** |
| `.venv/bin/python -m pytest tests/security -q` | the boundary gates (Host, Origin, bearer, body limit) — **189 passed** | earlier pass |
| `.venv/bin/python -m pytest tests/unit/studio_design/test_cad_providers_freecad.py -q` | the real FreeCAD provider against the frozen 28-case corpus — **8 passed in 19.96 s** | earlier pass |
| `cd adapters/freecad && <repo>/.venv/bin/python -m pytest tests -q` | the FreeCAD adapter — **388 passed** | `test-isolation` |
| `cd adapters/solidedge-bridge && <repo>/.venv/bin/python -m pytest tests -q` | the Solid Edge bridge — **186 passed, 1 skipped** (the live Windows test) | `test-isolation` |
| `cd adapters/solidedge && <repo>/.venv/bin/python -m pytest tests -q` | the Solid Edge adapter — **138 passed** | `test-isolation` |
| `cd studio && npm run typecheck && npm test && npm run build` | the UI: **53 test files, 769 tests**; typecheck and Vite build clean | **2026-10-21 downloads-cancel cell** |
| `cd backend && cargo test --workspace --locked --release` | the accepted root-workspace crates — **554 passed**, 0 failed, 0 ignored | **2026-10-21 design-budget cell** |
| `cd backend && cargo test -p kala-studio --no-fail-fast` | the native Studio service — **289 passed**, 0 failed (22 test binaries) | **2026-10-21 downloads-cancel cell** |
| `cd backend/crates/kala-cad && cargo test --locked --release` | the native OpenCascade adapter — **118 passed** | **cutover wave** |
| `cd backend/crates/kala-studio-journal && cargo test --locked --release` | the native session journal — **60 passed** | **cutover wave** |
| `cd backend/crates/kala-cad-providers && cargo test --locked --release` | the CAD provider connections — **40 passed** | **cutover wave** |
| `cd backend/crates/kala-tools && cargo test --locked --release` | the protocol, registry, simulator and executor — **229 passed**, including the 23-case executor differential | earlier pass |
| `cd backend/crates/kala-runtime && cargo test --locked --release` | the gateway **and** the agent runtime — **112 passed**, including the 36-case live agent differential | earlier pass |
| `cd backend/crates/kala-application && cargo test --locked --release` | the composition root — **53 passed**, including the 20 independent review probes | earlier pass |
| `cd backend/crates/kala-api && cargo test --locked --release` | the core API — **124 passed** | earlier pass |
| `cd backend/crates/kala-cli && cargo test --locked --release` | the CLI — **22 passed** | earlier pass |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | the native service against the live Python reference — **295 cases, 0 mismatches, 0 known gaps** | **2026-10-21 downloads-cancel cell** |
| `crates/kala-studio/tests/python_differential.py` | the native service against the live Python server — **268 cases, 0 mismatches** | **cutover wave** |
| `crates/kala-studio/tests/python_provider_dto_oracle.py` | the provider DTOs — **20 cases, 0 failures** | **cutover wave** |
| `crates/kala-studio/tests/replay_binary_corpus.py` | the frozen corpus against the release binary — **56 cases, 0 mismatches** (see below) | **cutover wave** |
| `.venv/bin/python backend/compat/self_check.py` | the corpus is reproducible and matches the checked-in files — **exit 0, `self-check OK`** | earlier pass |
| `cd studio && npm run check:native-readiness` | the route-surface parity report — **READY**, 16 Python route-methods covered, **18 in the native service** (7 answered directly, 11 through the geometry-backend seam), 0 missing; the built stage is classified `release` / `releaseEligible: true` | **2026-10-21** |
| `env -i … node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad` | the built AppImage, from the extracted image with no Python on `PATH` — **PASS** | **cutover wave** |

### The frozen corpus is not all contract

The compatibility corpus was regenerated and now holds **56 Studio cases**
over **15 Studio routes** (it held 48 over 9). Eight of the new cases cover
the design, journal and CAD-provider routes. **Four of them are not
reproducible byte for byte by a replay**, and the corpus says so in its own
metadata:

| Case | Why a replay cannot match it |
|---|---|
| `studio.cad_providers` | The recorded body is **this host's CAD-provider probe at generation time**. It recorded FreeCAD as unavailable with a 245-character reason (FreeCAD could not create its runtime directory in the generator's environment); on a host where FreeCAD works the same document is a different length. The corpus marks the case `body_deterministic: true`, which is **wrong** — it is host evidence. |
| `studio.design.l_bracket` | `body_deterministic: false`. The STEP export embeds the wall clock of the write, so `$.artifacts.step_sha256` and the `$.journal_head` that binds it change on every run. |
| `studio.journal.page` | `body_deterministic: false`. The page is a **session** record: it contains the events the earlier cases of the same generation run produced. |
| `studio.journal.verify` | `body_deterministic: false`. The chain head binds the STEP artifact digest. |

`crates/kala-studio/tests/replay_binary_corpus.py` therefore normalises the
corpus' **own declared volatile paths** and the native identity, and
reports **56 cases with 0 mismatches**: `exact: 48`, `fields: 4` (the
declared volatile paths, of which one — `studio.cad_providers` — is host
evidence compared against the current host's probe), `step structural: 2`
(a real ISO 10303-21 solid compared structurally, not byte for byte) and
`skipped: 2` (the two cases that need the 16 384-byte library body-limit
configuration and are covered by `corpus_fixture`). That replay ran against
corpus digest `c0003bae333f27a079adcf3e86f2618741a6606334a17ffe3fafe727fcd6c330`.
The corpus was regenerated again later on 2026-10-21 (corpus-hermetic) to
`39e0c0a90f9d03704945c79cdb9e777e77549ef208ac2177d3f7bb003dade3fa`, the value
`backend/crates/kala-studio/tests/corpus_fixture.rs` pins now;
`studio_http.json` did not change, so the replay result stands.

**What this does not mean.** A host-dependent record is still not a
contract: the `studio.cad_providers` case records the CAD-provider probe
of the machine that generated the corpus, and a different host answers
differently. The normalisation makes the harness honest about that; it does
not turn the record into a portable expectation. The analysis is in
[rust-migration-status.md §5.7](rust-migration-status.md).

### What this wave did not test

- The wave cut over the **Linux desktop** (ADR-0010 phase 2) and did
  **not** cut over the rest of the product. The agent runtime is ported
  and accepted in its own right, but it is not composed into the Studio
  service, the Python console script is still the installed entry point,
  and `kala-agent serve` still refuses with the gap message. The full
  per-module status is in [rust-migration-status.md](rust-migration-status.md).
- The wave did **not** test the installer end-to-end on a clean
  Windows host. The Windows build script exists and is written in the
  imperative, but it has **never been executed**, and no Windows runtime
  has been exercised.
- The wave did **not** build with Solid Edge on any host. The bridge,
  the protocol client and the lowering are fixture-tested only, and the
  bridge needs a **Windows host**.
- The wave did **not** compile the studio crate for Windows:
  `ring`'s build script needs `x86_64-w64-mingw32-gcc`, which is not
  installed on the development host. **MinGW for the Windows checks is a
  pending dependency**, and the `windows-check` cell has not filed a
  report, so no Windows compile or runtime claim is made for the studio
  crate. The other crates' Windows checks stand as their cell reports
  record them.
- The Linux AppImage **is** built and smoke-tested, but a generated
  AppImage is **not** proof that a desktop session works: the Electron
  **window** was never rendered on this host (`xvfb-run` is absent), and
  no desktop shell has been validated on a clean Linux host.
- The wave did **not** test the AppImage on every supported Linux
  distro (GNOME, KDE, Hyprland, Sway, etc.).
- The wave did **not** run a real FreeCAD on Python 3.11–3.13. The only
  *real* FreeCAD exercised here is the host's **1.1.4 on Python 3.14.8**;
  the `cp311`–`cp313` adapter imports were proven with matching CPython
  builds. An unsupported Python is reported as provider-**unavailable**
  with its exact version, never as a crash.
- The wave did **not** run the studio against a real analysis
  product. The integration rows remain honest placeholders, and the
  studio holds no solver automation.
- The wave did **not** run a **real subscription CLI**. Every CLI test
  drives a fake executable: the argv, the prompt-on-stdin, the sanitized
  environment, the private working directory, the budget, the process-tree
  kill and the `PATHEXT`/`.cmd` resolution are proven against fakes and
  against the pure resolution functions — never against a signed-in
  `claude`, `codex` or `opencode`.
- The wave did **not** execute the Windows-only CLI paths. The `.cmd`
  shim resolution and the `taskkill /T /F` tree kill are type-checked
  against the `x86_64-pc-windows-msvc` target and nothing more.
- The wave did **not** implement app orchestration; the studio produces a
  neutral handoff only. The subscription CLIs are used as **chat
  providers**, with every tool denied.
- The wave did **not** submit the artefact to VirusTotal, Microsoft,
  or any other reputation service. A user who needs that should
  arrange it separately.
- The wave ran **no benchmark**, so no speed, memory or readiness number
  is claimed anywhere in these documents.
- The wave did **not** execute the **Windows** FreeCAD kill. The cancelled
  build is terminated through `kala_process`'s supervisor; the Windows
  path is type-checked against the `x86_64-pc-windows-msvc` target by the
  runtime-fixes cell, and **no Windows host ran it**.
- The wave did **not** rebuild a **500-step** FreeCAD design after the
  cancellation landed. The live cancellation experiment used a real
  FreeCAD part on this host; the 500-step timing (1 009.5 s idle) is the
  design-budget cell's measurement, which is what sized the 1 800 s build
  deadline the backend now publishes.
- The wave did **not** exercise the **Python reference's** cancellation,
  because it has none: the reference's `/design` route is synchronous in
  the FastAPI thread pool and cannot be dropped by the event loop. The
  cancellation is native-only, and the native service is the shipped path.

## What changes would close these gaps

The next wave that addresses one of these items must update this
document with the new evidence and reference the test command. Do
not silently change a "no" into a "yes" in this file.
