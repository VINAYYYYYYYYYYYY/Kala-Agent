# Kala Studio

> Status: **preview**. Built and packaged in this wave as an unsigned
> standalone desktop application. This document states the contract,
> the real capabilities, and the limits a user must understand
> before trusting anything the desktop shows.

## What Kala Studio is

Kala Studio is a **chat-first Mechanical Engineering Agent**: a
standalone workspace for a single user, shipped as an unsigned
desktop bundle for Windows and Linux (AppImage). The conversational
agent workspace is the primary focal point — the user describes a
part, a requirement, or a CAD/CAE workflow, and the agent helps size
it, reason about geometry and tolerances, and plan the next step. An
integrated **OpenCascade (OCP) Geometry, Estimation & Neutral-File
Handoff Workbench** is **active by default** alongside the chat, so
the conversation can be turned into real parametric geometry, a
nominal size estimate, and a neutral `STEP` (`ISO-10303-21`) / `STL`
file without leaving the app.

The whole product — chat plus workbench — is a single, unified
workflow. The studio does not separate, label, or discriminate
between user profiles, and it does not require a user to pick between
a standalone geometry build and an external CAD/CAE handoff: the same
interface covers both.

It bundles three layers:

1. A **Python backend** (`src/kala_agent/studio/`) that exposes a
   strict loopback HTTP API on `127.0.0.1`. The hardened core API
   on port 8765 is unchanged.
2. A **React + TypeScript + Vite + Three.js** UI under `studio/`
   that the backend serves same-origin from `studio/dist` when
   present. The chat workspace is the primary surface, with the
   geometry workbench (toolbar, 3D preview, inspector, export)
   active beside it.
3. An **Electron shell** under `studio/electron/` that launches
   the backend as a child process, generates a random bearer
   token in the parent, and exposes it to the renderer only
   through a narrow IPC channel.

## The agent chat workspace

The chat workspace is where engineering intent is captured. It is a
**text surface**: a prompt goes to the selected provider and the
reply comes back as text. The chat request schema carries only
`{provider, model, prompt, allow_remote}` — no geometry, no preview
context, no estimate, no entitlement or integration status is attached
to it. The agent never evaluates prompt text as code.

A prompt is a starting point for the workbench, not a replacement for
it: the user reads the agent's suggestion, agrees dimensions, and the
workbench builds the geometry and neutral file. When a prompt asks for
a material strength, a factor of safety, a fatigue life, a
displacement, or a stress, the studio returns an honest refusal naming
the limitation rather than inventing an engineering fact.

## The OpenCascade (OCP) geometry, estimation & neutral-file handoff workbench

The workbench is active by default next to the chat. It builds
parametric geometry, reports nominal estimates, renders the analytic
3D preview, and exports neutral files. It serves two complementary
ends through one interface:

1. **Estimation and neutral-file handoff for external CAD/CAE
   suites.** For workflows that finish in `SOLIDWORKS`, `FreeCAD`,
   `Fusion`, `ANSYS Mechanical`, `Abaqus`, `COMSOL`, and similar
   suites, the workbench acts as a fast **parametric geometry,
   bounding-envelope, and volume/area estimator**, and a neutral
   `STEP` (`ISO-10303-21`) / `STL` handoff generator. It runs *before
   or alongside* full feature modeling in the external software. The
   studio's role ends at the neutral file; the external suite owns
   feature modeling, meshing, materials, loads, boundary conditions,
   and the solve.
2. **Default complete geometry builder for standalone builds.** With
   no external CAD software installed, OpenCascade (`STEP` / `STL`)
   plus the analytic 3D preview is the complete geometry builder out
   of the box for the supported primitives. No external CAD
   dependency is required to size a part and export a neutral file.

### Nominal geometry estimates

For the active primitive the workbench presents **nominal analytical
estimates** derived from the same validated parameters the preview
mesh uses:

| Estimate | Units | Closed-form value |
|---|---|---|
| Volume | `mm³`, `cm³` | box `xyz`; cylinder `πr²h`; sphere `(4/3)πr³` |
| Surface area | `mm²`, `cm²` | box `2(xy + yz + zx)`; cylinder `2πr(r + h)`; sphere `4πr²` |
| Bounding envelope (width × height × depth) | `mm` | axis-aligned extent of the mesh |
| Envelope volume | `mm³` | product of the envelope extents |

Unit conversions shown by the workbench are nominal: `1 cm³ = 1000
mm³` and `1 cm² = 100 mm²`.

The workbench surfaces these in the inspector ("OpenCascade estimate
(nominal)") and shows the active envelope and nominal volume beside
the chat input as a display hint. The chat request body itself still
carries only the prompt text; no estimate is attached to a provider
call.

These estimates are a **sizing aid, not an analysis result**. They are
closed-form values for the ideal primitive computed from validated
parameters, not measured from the tessellation and not derived from a
solver. The bounding envelope is re-derived server-side from the
triangle mesh the preview route returns, so a local approximation and
the server result can be compared directly (the inspector reports any
drift). When the optional `[geometry]` extra is installed, the exact
B-rep mass properties of the exported shape are confirmed through
OpenCascade's `BRepGProp` volume and surface properties; the exported
STEP round-trips to within `1e-7` relative of the closed-form volume.

The estimates are **not** stresses, displacements, factors of safety,
fatigue life, or any certification. The studio never fabricates an
FEA/CFD result. See
[studio-limitations.md](studio-limitations.md) for the exact scope of
every number.

### Geometry and export capability

- **Parametric primitives.** Box, cylinder, and sphere meshes are
  generated by `kala_agent.studio.geometry`. Every public builder
  emits a watertight, outward-wound triangle mesh in millimetres; the
  engine reports itself as `analytic`, so a caller never mistakes the
  preview for a CAD-grade NURBS or B-rep surface. Primitive counts are
  bounded and validated before dispatch.
- **STL export.** Binary STL export is implemented in
  `kala_agent.studio.geometry.stl` and is always available.
- **Exact STEP export when native OCP is installed.** When the host
  has the optional `cadquery-ocp` package installed, the backend
  calls the **native** OpenCascade Python bindings
  (`BRepPrimAPI_MakeBox` / `MakeCylinder` / `MakeSphere` plus
  `STEPControl_Writer`) to write an exact B-rep `ISO-10303-21` file.
  When the dependency is absent, the export route returns a `503`
  with an actionable message. The studio **never** ships a fake STEP
  file.
- **OpenCascade.js (WASM)** is a possible future browser-side B-rep
  path; it is **not** used in this wave. The browser viewport renders
  the server's analytic triangle mesh.

## Contract with the rest of the product

The studio backend is **separate from the hardened core** and never
imports `kala_agent.application`, `kala_agent.api`, or any module
that reads user credentials. The full route contract is fixed by
the wave `WAVE.md` and is enforced by the studio backend:

- `GET  /api/studio/health` returns `{"status":"ok"}` plus service,
  version, provider, integration, and `step_available` fields.
- `GET  /api/studio/integrations` returns the registry of CAD and
  analysis integrations. Each row carries `id`, `name`,
  `category`, `platforms`, `status`, `capabilities`, and `notes`.
  Native connectors are listed as `native_api_unavailable` or
  `not_configured`; no fake connector claims a working bridge.
- `GET  /api/studio/providers` returns the configured status of
  each provider. The boolean is derived from environment
  variables; the value of the environment variable is never
  returned.
- `POST /api/studio/chat` accepts `{provider, model, prompt,
  allow_remote}` and returns `{text}`. The chat request schema is
  exactly that — no geometry, no preview context, no estimate, no
  entitlement status, no integration status is attached. `allow_remote`
  defaults to `false` and must be `true` for any provider other
  than `local`.
- `POST /api/studio/preview` accepts `{kind, parameters}` where
  `kind` is one of `box | cylinder | sphere`. The backend
  returns a real triangle mesh with positions, normals, indices,
  a finite positive bounding box, an engine name, and any
  non-fatal warnings. Dimensions are millimetres.
- `POST /api/studio/export` accepts the preview body plus
  `{format: "stl" | "step"}` and returns the binary file. STEP
  requires the `[geometry]` extra.
- `POST /api/studio/handoff` accepts `{integration_id, kind,
  parameters, format}` and either returns a downloadable neutral
  artifact (STL or STEP) or an honest unsupported error. It never
  claims to launch or drive a native vendor product.

All mutating routes require `Authorization: Bearer <token>`. The
token is supplied by the desktop shell on loopback; in browser
fallback the user pastes it from a launcher file with restricted
mode. The backend refuses requests whose `Origin` / `Host` /
`Referer` are not loopback, refuses bodies larger than the
configured cap, and never returns the token over HTTP.

## Honest capabilities

### What the studio really does

- **Chat-first engineering assistance.** The chat workspace reasons
  with the selected provider over a bounded text prompt only. The
  local helper is deterministic and never invents engineering facts.
- **Parametric geometry with nominal estimates.** Box, cylinder, and
  sphere geometry is generated analytically. The bounding envelope is
  server-verified against the returned mesh, and nominal volume and
  surface-area estimates are reported from the validated parameters.
- **Neutral-file handoff.** STL is always available; exact B-rep STEP
  is available when native OCP is installed. The studio never writes
  an STL or STEP it did not produce.

### What the studio does not claim

- **No native vendor bridges.** SOLIDWORKS, Solid Edge, AutoCAD, and
  Fusion are listed in the integration registry, but their status is
  `native_api_unavailable`. The studio backend does not import vendor
  SDKs, does not drive COM, and does not implement a Fusion add-in.
  Vendor tools require their own licences and host SDK installation.
- **No solver execution.** ANSYS Mechanical, Abaqus, COMSOL,
  Simcenter Nastran, and Altair OptiStruct are listed as
  representative target integrations, **not** as a ranked
  top-five and **not** as evidence-backed market leaders. The
  studio does not launch a solver process, does not read
  solver result files, and does not fabricate displacements or
  stresses.
- **No analysis result from the nominal estimates.** The volume,
  surface-area, and bounding-envelope figures are ideal-primitive
  sizing estimates, not a mesh, a load case, a material assignment,
  or a boundary condition.
- **OpenCascade (OCP) is not OpenSCAD.** OCP is the native
  Python bindings to OpenCascade used by the `[geometry]` extra
  for STEP export. OpenSCAD is a separate CSG-based tool with a
  different evaluation model; the studio reports OpenSCAD
  discovery as `discovered` or `not_found` and does not embed
  an OpenSCAD interpreter. OpenCascade.js is a WASM port of
  OpenCascade that is **not** used in this wave; the preview
  path is analytic triangulation, not a browser-side B-rep
  kernel.
- **No arbitrary LLM code execution.** Prompts sent to any
  provider are returned as text. The studio never evaluates
  prompt text as Python, CSG, or any other instruction language.
- **No Codex subscription execution.** The `codex` (ChatGPT
  subscription) route is **not implemented** in this build; it
  fails closed with a sanitised `503` and sends no request. A
  `codex` binary merely present on `PATH` is not treated as a
  working provider. The only providers that make a real API call
  are `openai` and `openrouter`, and the studio exposes no
  arbitrary-provider escape hatch.
- **No app or OpenCode orchestration.** The studio does not drive
  other applications, editors, or agent runtimes. Its output is a
  neutral handoff — a mesh or a STEP file — that a human or
  another tool picks up explicitly.
- **No WebGL B-rep.** The Three.js viewport renders the
  server's triangle mesh. It does not render an exact B-rep
  and it does not round-trip STEP geometry to a parametric
  kernel.
- **No ready-to-solve analysis from STEP.** STEP geometry is a
  neutral boundary representation. It is **not** a mesh, a load
  case, a material assignment, or a boundary condition. An
  engineer must still mesh the body, assign materials, apply
  loads and constraints, and configure the solver. The studio's
  role ends at exporting a file the next tool can read.

## How the desktop shell runs

The Electron shell (`studio/electron/main.cjs`, owned by the UI
worker) launches the Python backend with a random bearer token.
The backend exits when the desktop window closes or when startup
health checks fail. The desktop shell:

- Sets `nodeIntegration: false`, `contextIsolation: true`,
  `sandbox: true`, and restricts navigation and window opening.
- Exposes only one IPC channel: `getConnection`, returning the
  loopback base URL and bearer token for the trusted current
  window.
- Loads UI from the backend over HTTP. There is no separate
  static asset origin.
- Shuts the backend down on `before-quit` and on startup
  failure; it does not terminate unrelated processes.

The browser-fallback path requires manual token entry. There is
no auto-login and no third-party auth flow.

## Primary sources

The integration and provider notes in this document are anchored
to the following primary sources checked on October 18, 2026. The
studio does not assert terms-of-service or eligibility claims
beyond what these sources state.

- OpenCascade.js (WASM, not used in this wave):
  <https://github.com/donalffons/opencascade.js>.
- Autodesk Fusion SDK:
  <https://help.autodesk.com/cloudhelp/ENU/Fusion-360-API/files/WelcomeToSDK_UM.htm>.
- SOLIDWORKS API:
  <https://help.solidworks.com/2026/english/api/sldworksapiprogguide/GettingStarted/SolidWorks_API_Getting_Started_Overview.htm>.
- Solid Edge automation (older SDK):
  <https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html>.
- AutoCAD .NET / ObjectARX:
  <https://help.autodesk.com/cloudhelp/2024/ENU/OARX-DevGuide-Managed/files/GUID-390A47DB-77AF-433A-994C-2AFBBE9996AE.htm>.
- Ansys PyMechanical:
  <https://mechanical.docs.pyansys.com/version/stable/architecture.html>.
- COMSOL API:
  <https://doc.comsol.com/6.3/doc/com.comsol.help.comsol/comsol_api_intro.46.01.html>.
- OptiStruct run options:
  <https://help.altair.com/hwsolvers/os/topics/solvers/os/run_options_os_r.htm>.
- Simcenter Femap/Nastran:
  <https://www.siemens.com/en-us/products/simcenter/mechanical-simulation/femap/femap-nastran/>.
- OpenRouter chat completions:
  <https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request>.
- Codex CLI authentication:
  <https://learn.chatgpt.com/docs/auth> (redirect target of
  https://developers.openai.com/codex/auth).
- Codex CLI noninteractive:
  <https://developers.openai.com/codex/noninteractive>.

## What this wave does not ship

- A signed installer.
- A tested end-to-end installer run on a clean host.
- A validated runtime on a clean Windows host. The Windows build
  path is configured in CI, but no Windows runtime has been
  exercised for this wave.
- A generated AppImage proven to launch a desktop on every
  supported distro. The AppImage configuration exists; a generated
  file is not proof that a desktop session works.
- A bundled FreeCAD, CalculiX, Gmsh, or any other solver binary.
- A live native bridge to any commercial CAD or analysis tool, and
  no native solver automation.
- App/OpenCode orchestration; the studio only produces a neutral
  handoff.
- A documented CI matrix that opens an actual desktop window.

The CI pipeline builds both the backend binary and the desktop
bundle and uploads them as unsigned artefacts. See
[studio-install.md](studio-install.md) and
[studio-limitations.md](studio-limitations.md) for the complete
honesty notes.
