# Cell UI Task: Pivot Kala Studio to a Chat-First Mechanical Engineering Agent

## Objective

Refactor the Studio frontend (`studio/src/**` and `studio/test/**`) so that **Kala Studio is a Chat-First Mechanical Engineering Agent** with an integrated, default-active **OpenCascade (OCP) Geometry, Estimation & Neutral-File Handoff Workbench** alongside it, rather than a traditional WebGL/OpenCascade-centric CAD studio where the assistant is buried in a third right-rail tab.

## Core Product & Architectural Requirements

1. **Primary Workspace = Mechanical Engineering Agent Chat (`AppShell.tsx`, `ChatPanel.tsx`, `Header.tsx`, `Sidebar.tsx`)**:
   - In `AppShell.tsx`, make the **Mechanical Engineering Agent (`ChatPanel`)** the primary focal point inside `.app-shell-viewport` (`grid-area: viewport`).
   - Add a clean workspace view bar or mode switcher inside `.app-shell-main` (where `Toolbar` lives) or inside `.app-shell-viewport` that lets the user view:
     - **Agent + OpenCascade Workbench** (default: primary Agent Chat in `.app-shell-viewport`, while the OpenCascade 3D Preview, Parametric Sizing & Geometry Estimate Inspector, and STEP/STL CAD Handoff live in the right rail `.right-rail`), AND
     - **3D OpenCascade Canvas Focus** (optional toggle in the top toolbar so users can also expand `SceneCanvas` in `.app-shell-viewport` when inspecting 3D meshes closely, or keep a live 3D preview docked in the right rail).
   - Enhance `ChatPanel.tsx`:
     - Frame it as the **Mechanical Engineering Agent** (e.g. header/subtitle explaining conversational mechanical sizing, parametric geometry estimation, and STEP/STL workflow planning).
     - Add quick-action **Engineering Starter Prompts** (buttons that populate or send common engineering prompts such as sizing a bracket/shaft/housing, estimating envelope & volume before external CAD handoff, or planning a direct OpenCascade STEP/STL build) AND a compact **Active Geometry Context** summary banner showing the currently active OpenCascade primitive (`box`, `cylinder`, `sphere`), its key dimensions in `mm`, and its nominal volume/envelope estimate so the conversation and the OpenCascade workbench feel tightly integrated.
     - **Preserve all existing `ChatPanel.dom.test.tsx` selectors and behaviors**:
       - `select` for provider selection (`#chat-provider-select`)
       - `input[type="text"]` for optional model override (`#chat-model-input`)
       - `input[type="checkbox"]` for `allowRemote`
       - `textarea.chat-panel-prompt` (`placeholder` containing `"Ask a short question about the current primitive..."` or similar)
       - `button.chat-panel-send` (`Send`), `button.chat-panel-cancel` (`Cancel`), `button.chat-panel-clear` (`Clear`)
       - `.chat-panel-message-user`, `.chat-panel-message-assistant`, `.chat-panel-error`, `.chat-panel-empty`

2. **OpenCascade (OCP) as Default Geometry & Estimation Workbench (`RightRail.tsx`, `Inspector.tsx`, `ExportPanel.tsx`, `Toolbar.tsx`, `localPrimitives.ts`)**:
   - OpenCascade + analytic 3D preview must be **enabled and active by default** so:
     - Users who want a complete standalone build have OpenCascade (`STEP` / `STL`) + 3D preview + parametric sizing ready by default.
     - Users who work with external CAD/CAE packages (`SOLIDWORKS`, `FreeCAD`, `Fusion`, `ANSYS Mechanical`, `Abaqus`, etc.) can use the OpenCascade workbench to quickly get **nominal geometry & bounding-envelope estimates** (volume, surface area, bounding box dimensions) and export a neutral `STEP` (`ISO-10303-21`) or `STL` file for downstream CAD/CAE.
   - **STRICT NON-DISCRIMINATION RULE**:
     - Do **NOT** separate, label, toggle, or discriminate between "hobbyists" and "pros" (or "amateur" vs "professional") anywhere in UI labels, state, tooltips, comments, or tests. Present a single unified engineering workflow where OpenCascade is active by default for both complete standalone builds and quick geometry estimation / neutral-file handoff to external CAD/CAE suites.
   - In `studio/src/viewer/localPrimitives.ts`:
     - Export a pure helper `computeGeometryEstimate(kind: PrimitiveKind, parameters: StudioParameters): GeometryEstimate` (and interface `GeometryEstimate`) that computes exact nominal analytical values in `mm` / `mm²` / `mm³` / `cm³`:
       - `envelopeMm: [number, number, number]` (bounding box $\Delta X, \Delta Y, \Delta Z$ in $\text{mm}$)
       - `envelopeVolumeMm3: number` ($\Delta X \cdot \Delta Y \cdot \Delta Z$)
       - `volumeMm3: number` (Box: $X \cdot Y \cdot Z$; Cylinder: $\pi r^2 h$; Sphere: $\frac{4}{3}\pi r^3$)
       - `volumeCm3: number` (`volumeMm3 / 1000`)
       - `surfaceAreaMm2: number` (Box: $2(XY + YZ + ZX)$; Cylinder: $2\pi r(r + h)$; Sphere: $4\pi r^2$)
       - `surfaceAreaCm2: number` (`surfaceAreaMm2 / 100`)
   - In `studio/src/components/inspector/Inspector.tsx`:
     - Alongside the parameter inputs and `BoundsReadout`, render a clear **OpenCascade Geometry Estimate** section (`.inspector-estimates`) showing:
       - Nominal Volume ($\text{mm}^3$ and $\text{cm}^3$)
       - Nominal Surface Area ($\text{mm}^2$ and $\text{cm}^2$)
       - Bounding Envelope ($\Delta X \times \Delta Y \times \Delta Z\text{ mm}$)
       - A concise note that these are nominal analytical geometry estimates for sizing and CAD/CAE handoff (no fabricated FEA/stress numbers).
   - In `studio/src/components/shell/RightRail.tsx`:
     - Since `ChatPanel` is now the primary focal point in `.app-shell-viewport`, repurpose `RightRail.tsx` as the **OpenCascade Geometry & CAD Workbench** with tabs:
       - `Workbench` (or `Inspector` — default active tab: shows a docked live 3D `SceneCanvas` preview + `Inspector` parametric sizing & geometry estimates!)
       - `Export` (`ExportPanel` for direct OpenCascade `STEP` / `STL` build or neutral-file handoff to external CAD/CAE)
       - `Assistant` (optional secondary/quick view or keep tab support so any code dispatching `Inspector` / `Export` / `Assistant` still works seamlessly).
     - Note: If `SceneCanvas` is rendered inside the right rail when the main viewport is in Agent Chat mode, and rendered in the main viewport when the user toggles "Expand 3D Canvas", only mount one `SceneCanvas` at a time so WebGL context count stays 1 and happy-dom tests stay fast and clean!

3. **Preserve ALL Existing CSS & DOM Test Invariants**:
   - Check `studio/test/layoutCss.test.ts`! Every single regex and assertion in `layoutCss.test.ts` MUST continue to pass:
     - `html, body, #root` has `overflow: hidden`
     - `.app-shell-body` has `grid-template-rows: minmax(0, 1fr)`, `overflow: hidden`, `min-height: 0`
     - `.app-sidebar` has `min-height: 0` and `overflow-y: auto`
     - `.app-shell-main` has `min-height: 0` and `grid-template-rows: auto minmax(0, 1fr)`
     - `@media (max-width: 1100px)` has `grid-template-rows: auto minmax(0, 1.4fr) minmax(0, 1fr)`, `"viewport"`, `"rail"`, and NO `footer-area`
     - `@media (max-width: 720px)` has `grid-template-rows: auto minmax(0, 1fr)` and `max-height: 42vh`
     - `.boot-gate` has `overflow: auto`
   - Check `studio/test/AppBoot.dom.test.tsx`, `ManualConnect.dom.test.tsx`, `ChatPanel.dom.test.tsx`, `ExportPanel.dom.test.tsx`, `localPrimitives.test.ts`, `store.test.ts` — all 13 existing test files must pass without regressions, and add new unit/DOM tests covering:
     - `computeGeometryEstimate` in `localPrimitives.test.ts`
     - The Agent-first workspace layout (ChatPanel rendered in the primary `.app-shell-viewport` by default upon boot, OpenCascade geometry estimates visible in Inspector, and 3D canvas toggle working cleanly).

## Verification Commands

Run from `/home/v9i/Projects/Kala-Agent/studio`:
1. `npm test` (all Vitest suites must pass)
2. `npm run typecheck` (0 TypeScript errors)
3. `npm run build` (clean Vite production bundle in `studio/dist`)

Write a concise completion summary to `.mece/waves/studio-20261018/cells/ui/REPORT-AGENT-UI.md` when finished.
