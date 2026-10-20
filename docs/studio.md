# Kala Studio

> Status: **preview**. This document states what the studio actually
> builds, measures and shows today, and the limits a user must understand
> before trusting anything on the screen.
>
> **Current state (2026-10-20).** The studio is **Rust-only** (ADR-0010
> phase 2). The shipped app runs the native `kala-studio` service and
> **no Python process at all**; the geometry comes from the **real
> OpenCascade 7.9.3** through the `kala-cad` adapter. The Python geometry
> worker survives as a **development-only opt-in** and as the differential
> oracle. The **Linux AppImage is built and smoke-tested but unsigned**;
> the **Windows installer is a script that has not been executed yet**.
>
> Read with: [studio-providers.md](studio-providers.md) (language-model
> providers), [cad-providers.md](cad-providers.md) (CAD providers),
> [studio-limitations.md](studio-limitations.md) (the honesty sheet) and
> [studio-install.md](studio-install.md) (how to run it).

## What Kala Studio is

Kala Studio is a **chat-first Mechanical Engineering Agent** with a
**Vellum drafting board** beside it. The user describes a part in chat; the
studio turns that description into a `ConstructionScript`, hands it to a
**CAD provider** that builds and measures the model, and shows the result on
a real drafting sheet with a section cut, hidden-line views and a
hash-chained session ledger.

The board is not a decoration. Every number on it was measured by the
provider that built the model, and every stamp on it is backed by a real
backend answer.

The studio is a **local-first, single-user desktop workspace**. It runs on
loopback only, talks to no host unless the operator explicitly picks a
remote language-model provider, and stores nothing outside its own data
directory.

## How it is put together

Two layers, and the boundary between them is deliberate:

1. **The native Studio service** (`backend/crates/kala-studio/`) that owns
   the HTTP boundary (Host / Origin / bearer / body-limit gates), the
   settings and token stores, the static UI, the chat providers, the
   CAD-provider routing, the static file server — **and the geometry**.
2. **A React + TypeScript + Vite + Three.js UI** under `studio/` that the
   back door serves same-origin from `studio/dist`.

The service computes its own geometry, through the **real OpenCascade
7.9.3**. `kala-cad` owns one narrow C++ adapter (`src/ffi/adapter.cpp`)
that **calls** the OCCT SDK and never rewrites it; `kala-cad-providers`
owns the CAD provider **connections** (`FreeCADCmd` out of process, the
Solid Edge loopback bridge); `kala-design-intent` owns the intent contract
and the strict prompt parser; and `kala-studio-journal` owns the
hash-chained session journal and the content-addressed artifact store. The
geometry is never re-implemented in Rust.

The **Python geometry worker** (`python -m kala_agent.studio --worker`)
still exists in the tree, but it is **not a product layer any more**: it is
a **development-only fallback** that the service starts **only** when
`KALA_STUDIO_GEOMETRY_WORKER` names one. The packaged app starts no Python
process at all.

The **Python reference server** (`python -m kala_agent.studio`) still exists
and is still the parity oracle: it serves the same public API without the
Rust service in front. It is what the differential harnesses compare
against. See [The native Studio service and the worker
fallback](#the-native-studio-service-and-the-worker-fallback).

The **Electron shell** (`studio/electron/`) launches the back door as a child
process, generates a random bearer token in the parent, and exposes it to
the renderer only through one narrow IPC channel. In the packaged app it
resolves `resources/backend/kala-studio` **only** — never a PATH Python, a
repository virtualenv or a worker command.

## The Vellum drafting board

The board is a four-zone drafting workspace. There is no top header bar and
no application footer; the connection information lives in the journal
footer and in the Options drawer.

| Zone | Width (1440×900) | Contents |
|---|---:|---|
| **Left — agent chat** | 340 px | the header stamp, the engineering starter prompts, the thread, the composer with the egress consent box and the **CAD-target selector** |
| **Centre — the drafting sheet** | flexible (780 px) | the sheet itself: the A–F / 1–8 coordinate ticks, the 50 px drafted grid, the model, redline dimensions, the title block, the small tool palette, the sheet tab strip; the sheet touches the **top edge** |
| **Right — top** | 320 px (AuxView exactly 250 px) | the **AuxView**: `Section A–A` / `Ortho` / `Tolerance` tabs |
| **Right — rest** | 320 px | the **Agent & Kernel Journal**: the mono ledger with chain state |
| **Bottom bar** (centre column only) | 48 px | render-mode switch, Grid/Axes/Dims/Section toggles, Reset View, the live readout, Export STEP, the FreeCAD registry pill, the ⚙ Options trigger |

At 1280×800 the columns become 280 / 740 / 260. There is exactly **one**
WebGL canvas in the whole app and it survives mode switches, theme switches
and drawer opens.

### Render modes

* **2D Pencil** — a vector drafting view. With a design active it draws the
  kernel's hidden-line result (visible edges solid, hidden edges dashed) as
  SVG over the sheet rather than relying on browser edge heuristics.
* **3D Clay** — a warm clay solid in the live WebGL viewport, rendering the
  **imported** provider STEP (see
  [CAD providers](cad-providers.md#who-measures-what)).

Switching between them re-frames the camera, because the field of view
changes. Changing the theme, the material, or the grid/axes/dimension
toggles does **not** re-frame and does not rebuild the GPU buffers.

### The title block

Real values only: sheet, part id, title, `SCALE`, units, projection, the
declared tolerance list, revision and author. `SCALE` reads **`FIT`**, not
`1:1` — the viewport frames the part to fit the sheet, so there is no fixed
model-to-paper ratio and claiming one would be false. With a design active
the block gains a **`MEASURED BY`** cell naming the provider.

### The Options drawer

A modal opened from the ⚙ trigger in the bottom bar. `Esc` closes it, focus
is trapped while it is open and returns to the trigger, and it carries
`role="dialog"` and `aria-modal="true"`. It holds four drafted cards:

| Card | Title | Contents |
|---|---|---|
| **01** | Parametric Inspector & Primitive Controls | the primitive selector, the parameter sliders, the surface-material control |
| **02** | Neutral-File Export & CAD/CAE Handoff | the export panel, the design export block with its digests, the integration registry |
| **03** | Model Gateway & API Providers | provider status and API-key configuration |
| **04** | Loopback Boot Gate & Recovery | the live origin, mode, health, version, and a reconnect/retry action |

Cards 02 and 04 read the live store: the port is `NO LOOPBACK` before a
connection exists, and the export note follows the backend's
`step_available` flag (`STEP —` / `ISO 10303 READY` / `STEP UNAVAILABLE`).
No token is displayed or persisted there.

### Themes

Two themes ship: **`vellum`** (day) and **`slate`** (night). Only the theme
**name** is persisted, in `localStorage['kala-studio.theme']`. No origin, no
token, no chat text and no journal content is written there. The canvas
background follows the theme (paper by day, slate at night) so the viewport
surface and the sheet behind it can never disagree about the paper they are
drawn on.

### The measure tool

The fifth tool in the sheet palette. It is **disabled while no mesh
exists**.

* **Picking** is a real raycast against the face mesh. The picked point is
  in model millimetres, so the number is a real length. A pointer that
  moved more than 4 px between press and release was an orbit drag and is
  ignored; a click that misses the mesh changes nothing.
* The **leader** is a redline line with jaws at both ends and a mono label
  (`20.10 mm`), projected from the same camera as the dimension labels, so
  it follows the orbit.
* The **sheet note** says what the tool is waiting for
  (`MEASURE · PICK POINT 1 ON THE MESH`) or the measured distance. It never
  invents a value while the measurement is incomplete.
* **Escape** or a second click on the tool button clears the measurement; a
  third pick starts a new one.
* It measures the drawn solid only. There is no "distance to nothing".

## The CAD-target selector

The composer carries a **CAD-target selector**. It is the only way to pick
a provider, and it starts with **no choice**.

* The rows come from `GET /api/studio/providers/cad` — a live probe of the
  host, never a constant.
* An unavailable provider is **disabled** and shows the backend's reason.
  OpenCascade is always labelled **opt-in**.
* The choice lives in the in-memory session store only; it is never written
  to `localStorage`.
* With no choice, the chat request carries `cad_provider: null` and the
  backend answers with its provider-choice prompt and builds nothing.

Full detail in [cad-providers.md](cad-providers.md).

## Design turns

A chat turn can now produce a real, measured, journaled part.

```
chat prompt ──► DesignIntent (template call, or a raw ConstructionScript
                  plus DECLARED tolerances and material)
                  │  validated; an invalid intent is an explicit error turn
                  ▼
              CAD provider builds the model and reports ITS OWN properties
                  ▼
              checks: declared envelope vs the provider's measurement
                  ▼
              the provider's STEP is stored (sha256) and imported by
              OpenCascade for DISPLAY ONLY (mesh, HLR views, section)
                  ▼
              the session journal records the turn, the build, the
              artefacts and the measurement, with chain digests
                  ▼
              the UI: the agent card with the measured table and the check
              rows, the sheet with the drawing, the section and the ledger
```

The viewer is a separate statement from the measurer. The viewport says
`view: OpenCascade import (view-only)` for every provider, and
`view: OpenCascade (opt-in) build` only when the selected provider **is**
`opencascade` — the native `kala-cad` adapter, which is then both the
builder and the viewer.

The local helper is a **strict parser**: it reads the prompt into a
declared intent, and when a value is missing it asks for exactly that value
("`Local helper. I can draft a L-bracket, but I need these declared
values: …`"). It never guesses a dimension. With a remote provider the
DesignIntent JSON Schema is added to the prompt, the reply is validated,
and **one** repair round-trip is allowed before the turn fails honestly.

The studio also refuses to build a design from an *unparseable* prompt: the
answer asks for the missing values and no design is compiled.

## The journal, the ledger, and the two stamps

The studio keeps its **own** SQLite journal and artifact store under the
studio data directory. It shares no state with the core `KalaApplication`;
it reuses only the chain and artifact libraries. The journal is the single
writer of its own file, so there are never two writers on one chain. The
journal is **native** now (`kala-studio-journal`): the same hash-chained
SQLite file, the same rows and the same chain digests as the Python
journal, which can still open and extend it.

The **ledger** on the right is a real table:

```
SEQ | UTC | TYPE | MESSAGE | DIGEST | CHAIN
```

* `SEQ` is the backend sequence; a local UI row has none and shows `—`.
* `TYPE` is the backend event type (`design.intent`, `provider.build.*`,
  `artifact.stored`, `view.imported`, `chat.turn`, …) or the local tag
  (`[PREVIEW]`, `[CHAT]`, `[EXPORT]`, `[TOOL]`, `[BRIDGE]`, `[SYSTEM]`).
* `DIGEST` is cut to eight characters with a copy button that copies the
  **full** digest.
* `CHAIN` is `local` for a UI row, `ok` for an event the chain covers,
  `broken` for one it does not, and `—` while nothing has been verified.

The poller reads the journal in bounded pages: **1 s** while a turn is
running, **5 s** while idle, paused entirely while the document is hidden,
with an exponential backoff (1 s → 30 s) when the backend fails.

### When the two stamps appear

| Stamp | Appears when | Never when |
|---|---|---|
| **`B-Rep Verified`** | the design's `verified` is `true` — every check passed, i.e. the provider's measurement agrees with the declared envelope and the model is a valid solid of the expected count | a check failed, a check is `not_reported`, or there is no design |
| **`HASH-CHAIN OK`** | the backend's `/journal/verify` answered `ok: true` | the chain has not been verified, the chain is broken, or the poll failed |
| `CHAIN BROKEN @ n` | verification failed and the backend named the first bad sequence | — |
| `SESSION LOG` | nothing has been verified yet | — |

Neither stamp is derived from local state. A local row, a page, or a poll
failure never lights `HASH-CHAIN OK`; a failed check never lights
`B-Rep Verified`.

## The AuxView

| Tab | With a design | Without a design |
|---|---|---|
| **Section A–A** | the kernel's section of the imported model: the exact area (`KERNEL AREA`), the offset, the outer/inner loop counts, a 45° ANSI hatch | the analytic profile of the active primitive |
| **Ortho** | the kernel's hidden-line front/top/right views in third-angle order, labelled `SOURCE: OCP HLR · VIEW: IMPORT` | the AABB silhouettes of the live mesh |
| **Tolerance** | the **declared** tolerance list, labelled `declared — not analysed` | an explicit empty state |

The tolerance tab never computes a fit, a limit or a GD&T call. It states
plainly: *"The backend reports declared limits only. Kala Studio computes no
fits, GD&T, or stack-up."*

> The `SOURCE: OCP HLR` stamp is the **literal on-screen label** the UI
> prints. The drawing behind it is now produced by the native `kala-cad`
> adapter over the real OpenCascade 7.9.3; the label text was not changed
> by the cutover.

## The API contract

All routes live under `/api/studio`. The **Python reference surface is 15
route-methods**; the native service answers **16** (the same 15 plus the
ADR-0011 view-only import).

**All 16 are answered natively, in-process.** There is no proxied route and
no Python process in the default path:

| Route | Owner |
|---|---|
| `GET /health` | the HTTP boundary and the capability probe |
| `GET /integrations` | the integration registry |
| `GET /providers` | the chat provider rows |
| `GET /settings`, `POST /settings` | the settings and token stores |
| `POST /chat` | the chat providers and the design turn |
| `POST /preview`, `POST /export`, `POST /handoff` | the analytic primitives (`kala-geometry`, `kala-step`) |
| `POST /design` | the native geometry backend |
| `GET /design/{id}/drawing`, `GET /design/{id}/section` | the native geometry backend (the OCCT hidden-line drawing and the exact-area section) |
| `GET /journal`, `GET /journal/verify` | the native journal |
| `GET /providers/cad` | the CAD provider connections |
| `POST /import` | the native view-only STEP import |

`POST /import` is the one **native-only** route: the ADR-0011 view-only
STEP import exists on the service and on the Python reference server, but
the Python public server does not mount it. The readiness scanner reports it
as "native-only (allowed, not parity)" and does not count it as a
difference.

`npm run check:native-readiness` in `studio/` parses the real Python and
Rust sources and reports **READY**: **15** Python route-methods covered,
**0** missing, **0** missing DTO capabilities, and the built stage
(`studio/release/native-readiness.json`) classifies the artefact `release`
with `releaseEligible: true`. That report still *labels* ten route-methods
"proxied" — they travel through the geometry-backend seam — but the default
build answers all sixteen from the native backend, and the Python worker is
opt-in.

Highlights:

- `GET /health` returns `{"status":"ok"}` plus the service, version,
  provider, integration and capability fields, the ADR-0009
  `design_available` flag, and the ADR-0010 `geometry_worker`
  `{state, version, design_available}` block. In the native build that
  block reports the **native** geometry backend (`state: "ready"`,
  `version: "7.9.3"`); it reports the Python worker only when the
  development fallback is in use.
- `POST /chat` accepts `{provider, model, prompt, allow_remote}` plus the
  optional `cad_provider`, and returns `{text, provider, model, offline}`
  plus an optional `design` (the `DesignResult`) when the turn produced one.
- `POST /design` takes a `DesignIntent` **plus a required `provider`**.
  There is no default.
- `GET /providers/cad` reports each provider's live availability, version
  and reason.

### Security posture

Every mutating route requires `Authorization: Bearer <token>`. The read-only
design, journal and CAD-provider routes carry their own bearer gate too. The
middleware order is the policy: security headers → loopback `Host` guard
(421) → body-size cap (413) → same-origin guard (403) → bearer (401) →
static → router. The STEP import route has its own, larger body cap (see
`KALA_STEP_IMPORT_MAX_BYTES` below). The back door never raises the
geometry backend's ceiling; a refusal is forwarded unchanged.

## The native Studio service and the worker fallback

### What the native service owns

| Concern | Owner |
|---|---|
| The HTTP boundary, the Host/Origin/bearer/body gates | **Rust** (`kala-studio`) |
| `/health`, `/integrations`, `/providers`, `/settings`, `/chat` | **Rust** |
| CAD provider routing and the "never default to opencascade" rule | **Rust** |
| The settings and token stores, the static UI server | **Rust** |
| The construction compiler, the measurement, the hidden-line drawing, the exact-area section, the view-only STEP import | **Rust** — `kala-cad`, one narrow C++ adapter over the real OCCT 7.9.3 |
| The strict intent parser and the part templates | **Rust** — `kala-design-intent` |
| The session journal and the artifact store | **Rust** — `kala-studio-journal` |
| The CAD provider **connections** (`FreeCADCmd`, the Solid Edge bridge) | **Rust** — `kala-cad-providers` |
| The analytic primitive meshes and the analytic STEP writer | **Rust** — `kala-geometry`, `kala-step` (on the default path) |
| The **Python geometry worker** | **a development-only fallback** — it is not started unless `KALA_STUDIO_GEOMETRY_WORKER` names one |

ADR-0010 keeps the geometry in the CAD kernel on purpose, and phase 2
reached it without rewriting the kernel: the measured numbers stay the ones
the tested OCCT code produces; Rust never fabricates a mesh, a STEP file, a
measurement, a design or a provider availability fact.

The two **declared** differences against the Python oracle are the identity
of the kernel binding (the native build links the **system OCCT 7.9.3**,
the Python oracle imports the `cadquery-ocp` binding **8.0.1.1**) and the
**display-mesh vertex order** (the same geometry, assembled in a different
order; the differential compares it by an order-independent fingerprint).
The geometry, the measurements, the checks, the drawing and the section are
identical. The full list is in
[rust-migration-status.md §4.8](rust-migration-status.md).

### Running it in development

Build the service and serve the built UI:

```bash
cd backend
cargo build --release -p kala-studio --locked

cd ..
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef      # 16–128 characters
backend/target/release/kala-studio \
  --host 127.0.0.1 --port 8765 \
  --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

```bash
curl -fsS http://127.0.0.1:8765/api/studio/health
curl -fsS -H "Authorization: Bearer devtoken-0123456789abcdef" \
  http://127.0.0.1:8765/api/studio/providers/cad
```

**No worker, no Python and no virtualenv** are needed: the health document
must carry `design_available: true` and `step_available: true`, and the
provider probe must answer the three CAD providers with a live version or a
live reason.

The Python reference server alone (no Rust in the path) is still a
supported way to run the studio:

```bash
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef
PYTHONPATH=src .venv/bin/python -m kala_agent.studio \
  --host 127.0.0.1 --port 8765 --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

### The worker fallback

`KALA_STUDIO_GEOMETRY_WORKER` names the development fallback. It is
**either** a JSON argv array:

```json
["/abs/path/python","-m","kala_agent.studio","--worker"]
```

**or** a single executable path, in which case the fixed arguments
`-m kala_agent.studio --worker` are appended. **Whitespace splitting is not
performed**: a value that names an executable *and* arguments is refused
with a clear message rather than silently re-parsed.

**The service starts no worker at all unless this variable names one**,
and there is **no development default** any more. An unset variable means
the native backend answers everything.

### `KALA_STEP_IMPORT_MAX_BYTES`

The one STEP import ceiling, shared by the service and the worker.

| Value | Result |
|---|---|
| absent or blank | the 64 MiB default |
| an integer in `1 … 536 870 912` (1 B … 512 MiB) | that value |
| anything else | a startup refusal (exit 2) with the worker's own message |

The service resolves it **once, before anything is started**, enforces
it on the public import route, and hands the same number to the worker
process when the fallback is in use. Every other route keeps the shared
256 KiB Studio limit.

### The worker's token and the lifeline

These apply to the **development fallback** only.

* The worker token is **32 fresh random bytes** rendered as 64 hexadecimal
  characters, minted per launch.
* It is passed **only** through the `KALA_WORKER_TOKEN` environment
  variable. It is never in `argv`, never in a URL, and never logged. The
  lifecycle test reads `/proc/<pid>/cmdline` and asserts that the token and
  the variable name are absent from it, and that the token is absent from
  the worker's stdout and stderr.
* `KALA_WORKER_STDIN_LIFELINE=1` is set, so the worker stops when the front
  door closes its stdin: **a dead front door can never orphan a worker.**
* The worker binds `127.0.0.1:0`, listens before it announces itself, and
  prints exactly one readiness line:

  ```
  KALA_WORKER_READY {"design_available":true,"port":54321,"version":"0.1.0"}
  ```

  Keys are sorted and the JSON is compact. `design_available` is true only
  when OCP is importable. The token is never printed.
* The worker enforces the token plus `Host: 127.0.0.1:<port>` and **refuses
  any `Origin` header** — it is never browser-facing.
* A worker that cannot start leaves the native routes working and the
  geometry routes answering `503 geometry_worker_unavailable`. A worker
  that dies is restarted with a capped exponential backoff (250 ms → 5 s).
  **No mesh, no STEP file and no measurement is ever fabricated while the
  worker is down.**

### The worker's internal route set

Internal, under `/internal/v1`, behind the worker's own gate stack
(Host → body size → Origin → bearer). This set belongs to the **fallback**
and to the Python reference server; the native service answers the public
routes itself:

`POST /preview`, `POST /export`, `POST /handoff`, `POST /design`,
`GET /design/{id}/drawing`, `GET /design/{id}/section`, `POST /import`,
`GET /journal`, `GET /journal/verify`, `POST /journal/append`,
`POST /intent/parse`, `POST /intent/validate`, `GET /intent/schema`,
`GET /providers/cad`.

The worker mounts **no** `/api/studio/*` route, no settings route and no
credential route. It makes no language-model provider call; the only network
call it can make is the loopback Solid Edge bridge health probe, and only on
Windows.

## Honest capabilities

### What the studio really does

- **Chat-first engineering assistance** over a bounded text prompt, with a
  strict local parser for design intent and an explicit "I need these
  values" answer when something is missing.
- **Real, provider-measured geometry.** The volume, area, bounds and solid
  count on screen were measured by the CAD provider that built the model.
- **A real drafting view.** The hidden-line drawing, the section cut and the
  imported 3D solid are kernel results, not browser heuristics.
- **A hash-chained session journal** with artefact digests and a verifiable
  chain.
- **Neutral-file export** — STEP from the provider, STL for the primitives.

### What the studio does not claim

- **No FEA, no CFD, no solver.** See
  [studio-limitations.md](studio-limitations.md).
- **Tolerances are declared, not analysed.** A tolerance string is what the
  operator said. Kala computes no fit, no limit, no stack-up and no GD&T.
- **"B-Rep Verified" means exactly three things**: the provider's
  measurement agrees with the declared envelope, the model is a valid solid,
  and it has the expected solid count. It says nothing about strength,
  fatigue, or manufacturability.
- **No live Solid Edge build.** The bridge, the protocol client and the
  lowering are fixture-tested, and the bridge needs a **Windows host** and
  a licensed Solid Edge installation. See
  [cad-providers.md](cad-providers.md#solidedge-on-windows-the-bridge-live-proof-pending).
- **The clay viewport is a perspective camera**, not an orthographic one —
  see [studio-limitations.md](studio-limitations.md).
- **The release artefact is unsigned.** The Linux AppImage is built and
  smoke-tested, but it carries no signature and is not published; the
  Windows installer is a script that has never been executed. The MinGW
  cross-check is still pending on a missing C toolchain. See
  [studio-limitations.md](studio-limitations.md#packaging-trust).
- **No arbitrary LLM code execution.** Prompts are returned as text. The
  studio never evaluates prompt text as Python, CSG, or any other
  instruction language.
- **The `codex` route is not implemented** in this build; it fails closed
  with a sanitised `503` and sends no request.
- **No app or OpenCode orchestration.** The studio does not drive other
  applications, editors or agent runtimes.

## How the desktop shell runs

The Electron shell (`studio/electron/main.cjs`) launches the back door as a
child process with a random bearer token. It:

- sets `nodeIntegration: false`, `contextIsolation: true`, `sandbox: true`,
  and restricts navigation and window opening;
- exposes only one IPC channel, `getConnection`, returning the loopback
  base URL and the bearer token for the trusted current window;
- loads the UI from the back door over HTTP (there is no separate static
  asset origin);
- in the **packaged** app resolves `resources/backend/kala-studio[.exe]`
  **only** — never `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND`, a PATH
  Python or a repository virtualenv — and strips `PYTHONPATH`,
  `PYTHONHOME`, `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and
  `KALA_STUDIO_GEOMETRY_WORKER` from the child environment;
- shuts the back door down on `before-quit` and on startup failure, and does
  not terminate unrelated processes.

The browser-fallback path requires manual token entry. There is no
auto-login and no third-party auth flow.

## Primary sources

The integration and provider notes in this document are anchored to the
primary sources checked on October 18, 2026, plus the ADRs listed below.
The studio does not assert terms-of-service or eligibility claims
beyond what those sources state.

- ADR-0007 (FreeCAD adapter), ADR-0008 (Solid Edge bridge), ADR-0009
  (Studio design turns), ADR-0010 (Rust control plane / geometry worker)
  and ADR-0011 (CAD provider model) — [adr/](adr/).
- `docs/construction-semantics.md` — the construction semantics the
  provider corpus is measured against.
- OpenCascade.js (WASM, not used):
  <https://github.com/donalffors/opencascade.js>.
- Solid Edge automation (older SDK):
  <https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html>.
- AutoCAD .NET / ObjectARX:
  <https://help.autodesk.com/cloudhelp/2024/ENU/OARX-DevGuide-Managed/files/GUID-390A47DB-77AF-433A-994C-2AFBBE9996AE.htm>.
- Ansys PyMechanical:
  <https://mechanical.docs.pyansys.com/version/stable/architecture.html>.
- COMSOL API:
  <https://doc.comsol.com/6.3/doc.com.comsol.help.comsol/comsol_api_intro.46.01.html>.
- OpenRouter chat completions:
  <https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request>.
- Codex CLI authentication:
  <https://learn.chatgpt.com/docs/auth> (redirect target of
  <https://developers.openai.com/codex/auth>).

## What this wave does not ship

- A signed installer, or a published artefact.
- A tested end-to-end installer run on a clean host.
- A validated runtime on a clean Windows host — the Windows build script
  has **never been executed**.
- A proof that the AppImage launches a desktop on every supported distro:
  the **Electron window was never rendered** on the build host, so the
  smoke verifies the packaged **backend** and the served UI root by
  extraction.
- A bundled FreeCAD, CalculiX, Gmsh, or any other solver binary. FreeCAD
  is **detected on the system, never bundled**; the app carries the adapter
  and its vendored dependencies for **cp311–cp314**.
- A live native bridge to any commercial CAD or analysis tool.
- A **Solid Edge** build: it needs a Windows host with the ADR-0008 bridge,
  and no Solid Edge build has ever been executed.
- A performance, memory or readiness number. No benchmark was run.
- App/OpenCode orchestration; the studio only produces a neutral handoff.

The **Linux AppImage is built and smoke-tested**:
`studio/release/kala-studio-0.1.0-x86_64.AppImage`, 163 642 516 bytes,
sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`,
**unsigned**. The release workflow is written but was not pushed or run. See
[studio-install.md](studio-install.md) and
[studio-limitations.md](studio-limitations.md) for the complete honesty
notes.
