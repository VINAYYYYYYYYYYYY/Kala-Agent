# Kala Agent

Local-first engineering design agent with a hardened Python core, an
optional FreeCAD tool adapter, and a standalone desktop workspace
called **Kala Studio**.

## UI Note

> **Basic AI-generated UI for test:** The Kala Studio frontend (`studio/`) is a basic AI-generated UI created for testing and evaluating the chat-first Mechanical Engineering Agent workflow, OpenCascade (`STEP` / `STL`) estimation and neutral-file handoff, and local/remote provider configuration.

**Kala Studio is a chat-first Mechanical Engineering Agent.** The
conversational agent workspace is the primary focal point: the user
describes a part, a requirement, or a CAD/CAE workflow in natural
language, and the agent helps size the part, reason about geometry and
tolerances, and plan the downstream work. An integrated **OpenCascade
(OCP) Geometry, Estimation & Neutral-File Handoff Workbench** is
active by default alongside the chat, so a conversation can be turned
into real parametric geometry, a nominal size estimate, and a neutral
`STEP` (`ISO-10303-21`) or `STL` file without leaving the application.

Runtime dependencies: `fastapi` and `pydantic` (pinned in
`pyproject.toml`). The studio runtime additionally needs `uvicorn`;
exact STEP export additionally needs `cadquery-ocp`. None of these are
vendored; they are installed by `pip`.

## Chat-first agent with an integrated geometry workbench

Kala Studio is a single workspace with two cooperating surfaces.
There is no separate product, no separate mode, and no separate
workflow for different kinds of user — the same chat-plus-workbench
flow covers standalone geometry builds and external CAD/CAE
estimation and handoff.

1. **Agent chat (primary focal point).** The conversational workspace
   is where engineering intent is captured and reasoned about: part
   sizing, geometry and tolerance discussion, workflow planning, and
   the choice of an external CAD/CAE suite to hand off to. Chat is a
   text surface. It never evaluates prompt text as code, and it never
   fabricates analysis results (see the honesty disclaimer below).
2. **OpenCascade (OCP) Geometry, Estimation & Neutral-File Handoff
   Workbench (active by default).** Alongside the chat, the workbench
   builds parametric geometry from the agreed dimensions, reports a
   nominal bounding envelope and nominal analytical volume and surface
   area estimates, renders an analytic 3D preview, and exports neutral
   files.

The workbench serves two complementary ends through the same
interface:

- **External CAD/CAE estimation and neutral-file handoff.** For
  workflows that finish in `SOLIDWORKS`, `FreeCAD`, `Fusion`,
  `ANSYS Mechanical`, `Abaqus`, `COMSOL`, and similar suites, the
  workbench acts as a fast **parametric geometry, bounding-envelope,
  and volume/area estimator** and a neutral `STEP` (`ISO-10303-21`) /
  `STL` handoff generator. It runs *before or alongside* full feature
  modeling in the external software. The external suite owns feature
  modeling, meshing, materials, loads, boundary conditions, and the
  solve.
- **Standalone geometry build.** For a build with no external CAD
  software installed, OpenCascade (`STEP` / `STL`) plus the analytic
  3D preview is the **default complete geometry builder** — it works
  out of the box for the supported primitives, with no external CAD
  dependency.

### Nominal geometry estimates

For the active primitive the workbench presents nominal analytical
estimates derived from the same validated parameters the preview mesh
uses:

| Estimate | Units |
|---|---|
| Volume | `mm³` and `cm³` |
| Surface area | `mm²` and `cm²` |
| Bounding envelope (width × height × depth) | `mm` |
| Envelope volume | `mm³` |

These are closed-form, ideal-primitive values — a sizing aid, not an
analysis result. The bounding envelope is the axis-aligned extent of
the mesh, and the server re-derives it from the mesh it returns
(`GET`/`POST` preview responses carry `bounds`, the mesh, and the
engine name). Exact B-rep mass properties (`volume` / `area`) are
confirmed through OpenCascade's `BRepGProp` when the `[geometry]`
extra is installed. The estimates never replace a solver: they are
not stresses, displacements, factors of safety, or certification.

See [docs/studio.md](docs/studio.md) for the workbench and estimation
details, and
[docs/studio-limitations.md](docs/studio-limitations.md) for the
precise limits of every number the studio shows.

## Repository layout

| Path | What lives there |
|---|---|
| `src/kala_agent/` | Core agent (kernel, domains, persistence, API, CLI) |
| `src/kala_agent/studio/` | Kala Studio backend (separate loopback service) |
| `adapters/freecad/` | Optional FreeCAD adapter (separate distribution) |
| `studio/` | Kala Studio desktop UI (React + TypeScript + Vite + Three.js + Electron) |
| `tests/` | Core test suite |
| `docs/` | Product documentation |
| `packaging/` | PyInstaller spec and entry script for the studio desktop backend |
| `.github/workflows/` | CI matrices (studio.yml only, no API/credentials) |

## Quick start

### Core agent

```bash
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev,serve]"
uvicorn app:app --factory --app-dir src/kala_agent/api --host 127.0.0.1 --port 8765
```

### Kala Studio backend (developer mode)

The studio backend reuses the project's `.venv`. The Electron shell
and dev workflows point Python at the project interpreter through
`KALA_STUDIO_PYTHON`.

```bash
pip install -e ".[studio]"
# Optional but recommended for real STEP export. Without it,
# /api/studio/export for STEP returns 503 with an actionable error.
pip install -e ".[geometry]"

KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
  kala-studio --host 127.0.0.1 --port 8765
# equivalent:
KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
  python -m kala_agent.studio --host 127.0.0.1 --port 8765
```

### Kala Studio desktop (unsigned preview)

The desktop bundle is built by `.github/workflows/studio.yml`. A
local build needs the PyInstaller backend binary first, then the
Electron bundle:

```bash
pip install -e ".[studio,geometry]"
pip install pyinstaller
pyinstaller packaging/studio-backend.spec --noconfirm --clean
mkdir -p studio/backend && cp dist/kala-studio studio/backend/kala-studio

cd studio
npm ci
npm run build
npm run desktop:pack
```

The bundle is **unsigned** on Linux (AppImage) and Windows (NSIS).
End-to-end install on a clean host was not executed in this wave;
see [docs/studio-limitations.md](docs/studio-limitations.md).

## Documentation

- [docs/studio.md](docs/studio.md) — Kala Studio as a chat-first agent plus the OpenCascade geometry, estimation, and handoff workbench, with honest limits.
- [docs/studio-install.md](docs/studio-install.md) — install, dev workflow, desktop launch.
- [docs/studio-providers.md](docs/studio-providers.md) — provider story.
- [docs/studio-limitations.md](docs/studio-limitations.md) — limitations, engineering safety, nominal-estimate limits, packaging trust, testing.
- [docs/how-to-read.md](docs/how-to-read.md) — how to read the core code.
- [docs/architecture-report.md](docs/architecture-report.md) — full architecture report.
- [docs/adr/](docs/adr/) — architecture decision records.
- [docs/threat-model.md](docs/threat-model.md) — security threat model.
- [docs/verification-report.md](docs/verification-report.md) — verification claims and residual risks.

## Honesty disclaimer

Kala Studio is a preview build. The studio backend, the desktop
shell, the PyInstaller pipeline, and the GitHub Actions matrices
were produced by this wave. They are honest about what they do and
what they do not do:

- The agent chat is a text surface, not a code interpreter: prompts
  sent to any provider are returned as text and are never evaluated
  as Python, CSG, or any other instruction language.
- The workbench's volume, surface-area, and bounding-envelope figures
  are **nominal analytical estimates** of the ideal primitive, not
  solver output. They are not stresses, displacements, factors of
  safety, or certification, and no FEA/CFD result is ever fabricated.
- The desktop bundle is unsigned on both Ubuntu and Windows. The
  Windows build path is configured in CI, but no Windows runtime
  has been exercised for this wave, and a Linux AppImage
  configuration is not proof that a desktop session works.
- The ChatGPT subscription route (`codex`) is **not implemented** in
  this build: it fails closed with a sanitised `503` and sends no
  request. A `codex` binary merely present on `PATH` does not make
  the provider usable. The only real API providers are `openai` and
  `openrouter`, and there is no arbitrary-provider escape hatch.
- Native vendor connectors (SOLIDWORKS, Solid Edge, AutoCAD, Fusion)
  are not implemented. The studio reports each tool's status
  honestly and produces neutral-file handoffs (STL / STEP) where
  supported. There is no native CAD or solver automation and no
  app/OpenCode orchestration.
- Analysis solvers (ANSYS Mechanical, Abaqus, COMSOL, Simcenter
  Nastran, Altair OptiStruct) are listed as representative target
  integrations, not as ranked top-five or evidence-backed market
  leaders.
- The preview mesh generators are covered by regression tests,
  including an independent signed-volume check this wave that
  confirmed outward triangle winding for a box, cylinder, and
  sphere. The earlier winding-defect warning has been removed.
- The OpenCascade.js WASM kernel is a possible future path; exact
  STEP export today uses the native `cadquery-ocp` bindings, and the
  browser viewport renders the server's analytic WebGL mesh.
- The frozen backend binary was built in this wave and validated by
  `packaging/studio_smoke.py` (health, UI assets, preview mesh, and a
  real STEP export against the existing snapshot), but this wave did
  not run a tested end-to-end installer on a clean host.

See [docs/studio-limitations.md](docs/studio-limitations.md) for the
full list and engineering safety guidance.

## License

The core is offered under the project's existing licence. The
FreeCAD adapter inherits FreeCAD's LGPL-2.0-only terms through its
system dependency. See the repository `LICENSE` if present.
