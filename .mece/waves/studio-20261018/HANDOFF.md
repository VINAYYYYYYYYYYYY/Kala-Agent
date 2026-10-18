# Handoff — Kala Studio Wave (`studio-20261018`)

- **Date**: 2026-10-18
- **Wave root**: `.mece/waves/studio-20261018/`
- **Current status**: **COMPLETE & VERIFIED** (including the Chat-First Mechanical Engineering Agent + Default OpenCascade Geometry, Estimation & Neutral-File Handoff Workbench pivot)
- **Last commit checkpoint**: `19582e3` (`feat(studio): ship standalone desktop CAD/CAE workspace and OCP STEP pipeline`) — uncommitted working-tree changes contain the verified post-handoff hardening and Chat-First Mechanical Engineering Agent pivot.

---

## 1. Completed Orchestration Summary

### A. Chat-First Mechanical Engineering Agent Pivot (OpenCode Non-Go Free Workers)
Executed across three parallel OpenCode worker instances (`opencode/space-bunny-free` for UI, `opencode/deepseek-v4.1-flash-free` for Backend and Delivery/Docs):

1. **UI Cell (`studio/**` — Report: `cells/ui/REPORT-AGENT-UI.md`)**:
   - **Primary Viewport = Mechanical Engineering Agent (`AppShell.tsx`, `ChatPanel.tsx`)**: `ChatPanel` now owns `.app-shell-viewport` (`grid-area: viewport`) by default on boot, featuring an **Active OpenCascade Geometry Context** banner (`.chat-panel-context`) and **Engineering Starter Prompts** (`.chat-panel-starter`).
   - **Workspace View Switch (`Toolbar.tsx`, `workspaceView.ts`)**: Added a `role="radiogroup"` switch (`Agent + Workbench` default vs `3D Canvas Focus`) inside the toolbar while keeping `.app-shell-main`'s `grid-template-rows: auto minmax(0, 1fr)` layout invariant intact.
   - **OpenCascade (OCP) Geometry, Estimation & CAD Workbench (`RightRail.tsx`, `Inspector.tsx`, `localPrimitives.ts`)**:
     - Active by default alongside the agent chat with tabs `Workbench` (default) · `Export` · `Assistant`, plus a docked live 3D `SceneCanvas` preview (`.right-rail-preview`) when the viewport is in Agent mode.
     - Enforces a single WebGL `SceneCanvas` instance across the app at all times (`docked = viewMode === 'agent'`).
     - Added pure `computeGeometryEstimate(kind, parameters)` in `studio/src/viewer/localPrimitives.ts` and `.inspector-estimates` in `studio/src/components/inspector/Inspector.tsx` reporting closed-form nominal Volume (`mm³` & `cm³`), Surface Area (`mm²` & `cm²`), Bounding Envelope (`ΔX × ΔY × ΔZ mm`), and Envelope Volume (`mm³`).
   - **Strict Non-Discrimination**: Zero separation, labeling, or discrimination between "hobbyists" and "pros" anywhere in UI, state, tooltips, or tests—one unified workflow serves both quick geometry/envelope estimation + neutral `STEP`/`STL` handoff to external CAD/CAE packages and complete standalone OpenCascade builds.

2. **Backend Cell (`src/kala_agent/studio/**`, `tests/unit/test_studio_providers.py` — Report: `cells/backend/REPORT-AGENT-BACKEND.md`)**:
   - Enhanced `local_chat` in `src/kala_agent/studio/providers/local.py` and `list_provider_records` in `src/kala_agent/studio/providers/dispatch.py` so the offline deterministic helper extracts dimensions (`mm`, `cm`, `m`, triples, or labeled `radius`/`height`/`size_x`/`y`/`z`), computes closed-form nominal analytical volume (`mm³`/`cm³`) and surface area (`mm²`), and explains the unified OpenCascade (`STEP`/`STL`) estimation, external CAD/CAE handoff, and standalone build workflow without fabricating solver numbers or separating user tiers.

3. **Delivery / Docs Cell (`README.md`, `studio/README.md`, `docs/studio*.md` — Report: `cells/delivery/REPORT-AGENT-DOCS.md`)**:
   - Updated `README.md`, `studio/README.md`, `docs/studio.md`, `docs/studio-limitations.md`, and `docs/studio-providers.md` to document the chat-first Mechanical Engineering Agent architecture, the default-active OpenCascade (OCP) geometry & estimation workbench, nominal geometry estimate formulas/units, and honest boundaries.

---

## 2. Final Verification Results

| Check | Command | Result |
|---|---|---|
| Frontend Unit & DOM Tests | `npm --prefix studio test` | **14 test files, 156/156 passed** (+31 new tests in `AgentWorkspace.dom.test.tsx`, `localPrimitives.test.ts`, `layoutCss.test.ts`) |
| Frontend Typecheck & Production Bundle | `npm --prefix studio run typecheck && npm --prefix studio run build` | **0 errors**, clean Vite bundle in `studio/dist/` |
| Frontend Dependency Audit | `npm --prefix studio audit --omit=dev` | **0 vulnerabilities** |
| Python Lint, Format & Typecheck | `.venv/bin/ruff check src tests packaging && .venv/bin/ruff format --check src tests packaging && .venv/bin/mypy src/kala_agent` | **All checks passed**, 116 files formatted, **0 mypy issues in 76 source files** |
| Full Python Test Suite | `.venv/bin/pytest -q` | **1,448 passed** (0 failures) |
| Frozen Backend Binary Build | `.venv/bin/pyinstaller packaging/studio-backend.spec --noconfirm --clean` | Built `dist/kala-studio` (`168 MB` ELF binary) |
| Frozen Backend End-to-End Smoke | `.venv/bin/python packaging/studio_smoke.py --backend dist/kala-studio --timeout 90` | **PASS** (health `ok`, `step_available: true`, UI assets `200`, preview mesh `ok`, `STEP export: 15518 bytes, ISO-10303-21 magic ok`) |
| Linux Electron AppImage Bundle | `npm --prefix studio run desktop:pack -- --linux AppImage` | Built `studio/release/kala-studio-0.1.0-x86_64.AppImage` (`289 MB`) |
| Live Loopback QA Server | `http://127.0.0.1:8765` (PID `1466789`, token `kala-studio-browser-qa-20261018`) | Serving new chat-first UI bundle and updated `local` mechanical engineering agent provider |
