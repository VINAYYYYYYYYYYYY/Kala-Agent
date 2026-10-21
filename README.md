# Kala Agent

Local-first engineering design agent with a hardened Python core, an
optional FreeCAD tool adapter, and a standalone desktop workspace
called **Kala Studio**.

## Architecture

The **shipped** Kala Studio is a **native Rust application**: an Electron
shell launches the `kala-studio` binary, which owns the HTTP boundary, the
settings and token stores, the static UI, the chat providers **and the
geometry**. The geometry comes from the **real OpenCascade 7.9.3**, called
through one narrow C++ adapter (`backend/crates/kala-cad/src/ffi/adapter.cpp`)
that never rewrites the kernel. The Linux deliverable is a single
**AppImage** that contains the Rust binary, the staged OCCT shared-library
closure, the compiled UI and the FreeCAD adapter — and **no Python
interpreter, no PyInstaller payload and no `cadquery-ocp` binding**. The
Windows deliverable is an NSIS installer built from the same native
runtime.

The Python tree (`src/kala_agent/`, including the Python Studio reference
server and the `cadquery-ocp` bindings) is the **development-only
reference**: it is the parity oracle the differential harnesses compare
the Rust product against, and the optional fallback geometry worker. It is
**not** what the shipped app runs, and the packaged app starts no Python
process at all.

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

Runtime dependencies of the **Python** core and reference server:
`fastapi` and `pydantic` (pinned in `pyproject.toml`), plus `uvicorn` for
the studio server; the Python reference's exact STEP export additionally
needs `cadquery-ocp`. None of these are vendored; they are installed by
`pip`. **The shipped desktop app needs none of them**: it runs the native
Rust runtime and the real OpenCascade.

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
| `src/kala_agent/` | Core agent (kernel, domains, persistence, API, CLI) and the **development-only** Python Studio reference |
| `backend/crates/` | The **shipped** native Rust runtime: `kala-studio` (the Studio service), `kala-cad` (the OpenCascade adapter), `kala-cad-providers`, `kala-design-intent`, `kala-studio-journal` |
| `adapters/freecad/` | Optional FreeCAD adapter (separate distribution) |
| `studio/` | Kala Studio desktop UI (React + TypeScript + Vite + Three.js + Electron) |
| `tests/` | Core test suite |
| `docs/` | Product documentation |
| `packaging/` | The Linux AppImage build and the Windows NSIS build script |
| `.github/workflows/` | CI matrices (studio.yml only, no API/credentials) |

## Quick start

### Core agent

```bash
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev,serve]"
uvicorn app:app --factory --app-dir src/kala_agent/api --host 127.0.0.1 --port 8765
```

### Kala Studio backend (developer mode) — the native Rust service

This is the shipped runtime: a native Rust binary that owns the HTTP
boundary **and** the geometry. No Python worker, no virtualenv and no
`pip install` in the default path.

```bash
cd backend
cargo build --release -p kala-studio --locked
cd ..

cd studio && npm ci && npm run build && cd ..

export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef     # 16–128 characters
backend/target/release/kala-studio \
  --host 127.0.0.1 --port 8765 \
  --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

### The Python Studio reference server (development only)

The Python reference server still serves the same public API without the
Rust service in front. It is the **parity oracle** the differential
harnesses compare against — it is not what the shipped app runs.

```bash
pip install -e ".[studio,geometry]"

KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
  kala-studio --host 127.0.0.1 --port 8765
# equivalent:
KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
  python -m kala_agent.studio --host 127.0.0.1 --port 8765
```

### Kala Studio desktop (unsigned preview)

The Linux AppImage is built from the native Rust runtime by
`packaging/linux/build-appimage.sh`; the Windows installer is built by
`packaging/windows/build-windows.ps1` (which has **never been executed** —
there is no Windows host in this repository).

```bash
export RUSTUP_HOME=/home/v9i/.rustup
export CARGO_HOME=/home/v9i/.cargo
(cd studio && npm ci)
packaging/linux/build-appimage.sh
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

Kala Studio is a preview build. The native Rust Studio service, the
desktop shell, the AppImage/NSIS packaging and the GitHub Actions matrices
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
- The chat surface publishes **seven** provider ids: `local`, `openai`,
  `openrouter`, `claude_code`, `codex_cli`, `opencode_cli` and the
  historical alias `codex`. The two API-key routes call the provider's
  fixed endpoint; the three subscription routes run the operator's own
  Claude Code / Codex / OpenCode CLI as a sandboxed subprocess with every
  tool denied, a sanitized environment and a private working directory.
  Every route that leaves the machine needs the operator's
  `allow_remote` consent. There is no arbitrary-provider escape hatch.
- A subscription CLI does **not** report which model it picked when the
  request names none, so the Studio says so honestly
  (`Codex CLI default model (chosen by Codex)`) instead of inventing a
  model name.
- Native vendor connectors (SOLIDWORKS, Solid Edge, AutoCAD, Fusion)
  are not implemented. The studio reports each tool's status
  honestly and produces neutral-file handoffs (STL / STEP) where
  supported. There is no **commercial** CAD automation and no solver
  automation; the only geometry the studio builds itself is the
  native OpenCascade path, and the Solid Edge bridge has never
  executed a build on any host.
- Analysis solvers (ANSYS Mechanical, Abaqus, COMSOL, Simcenter
  Nastran, Altair OptiStruct) are listed as representative target
  integrations, not as ranked top-five or evidence-backed market
  leaders.
- The preview mesh generators are covered by regression tests,
  including an independent signed-volume check this wave that
  confirmed outward triangle winding for a box, cylinder, and
  sphere. The earlier winding-defect warning has been removed.
- The OpenCascade.js WASM kernel is a possible future path. Exact
  STEP export in the **shipped app** uses the **native OpenCascade
  7.9.3** through the Rust `kala-cad` adapter; the `cadquery-ocp`
  binding belongs to the **development-only Python reference**. The
  browser viewport renders the server's mesh.
- The native backend binary was built in this wave and validated by
  `packaging/studio_smoke.py` (health, UI assets, preview mesh, and a
  real STEP export against the existing snapshot), but this wave did
  not run a tested end-to-end installer on a clean host.

See [docs/studio-limitations.md](docs/studio-limitations.md) for the
full list and engineering safety guidance.

## License

The core is offered under the project's existing licence. The
FreeCAD adapter inherits FreeCAD's LGPL-2.0-only terms through its
system dependency. See the repository `LICENSE` if present.
