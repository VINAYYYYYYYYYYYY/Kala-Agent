# Cell Delivery Task: Update Studio Documentation for Chat-First Mechanical Engineering Agent

## Objective

Update `README.md`, `docs/studio.md`, `docs/studio-limitations.md`, and `docs/studio-providers.md` so the documentation accurately reflects Kala Studio's design as a **Chat-First Mechanical Engineering Agent** with an integrated, default-active **OpenCascade (OCP) Geometry, Estimation & Neutral-File Handoff Workbench**.

## Core Requirements

1. **Chat-First Mechanical Engineering Agent Framing (`README.md`, `docs/studio.md`)**:
   - Describe Kala Studio first and foremost as a **conversational Mechanical Engineering Agent** where the agent chat workspace is the primary focal point for sizing parts, reasoning about geometry and tolerances, and planning CAD/CAE workflows.
   - Explain the role of the integrated **OpenCascade (OCP) & 3D Preview Workbench**:
     - It is active by default alongside the agent chat.
     - For workflows involving external CAD/CAE suites (`SOLIDWORKS`, `FreeCAD`, `Fusion`, `ANSYS Mechanical`, `Abaqus`, `COMSOL`, etc.), it acts as a fast **parametric geometry, bounding-envelope, volume/area estimator, and neutral `STEP` (`ISO-10303-21`) / `STL` handoff generator** before or alongside full feature modeling in external software.
     - For standalone builds without external CAD software, OpenCascade (`STEP` / `STL`) and the analytic 3D preview serve as the **default complete geometry builder** right out of the box.
2. **Strict Non-Discrimination Rule**:
   - Do **NOT** separate, label, or discriminate between "hobbyists" and "pros" (or amateur vs professional) anywhere in `README.md` or `docs/studio*.md`. Describe a single, unified workflow that serves both standalone OpenCascade builds and external CAD/CAE estimation & neutral-file handoffs seamlessly.
3. **Preserve Honesty & Security Documentation (`docs/studio-limitations.md`, `docs/studio-providers.md`)**:
   - Keep all existing security invariants, loopback/Bearer token notes, and honest integration status tables intact (never claim live COM/IPC automation of proprietary CAD/CAE solvers or fabricated FEA results).
   - Document the nominal analytical geometry estimates (volume $\text{mm}^3/\text{cm}^3$, surface area $\text{mm}^2/\text{cm}^2$, bounding envelope $\text{mm}$) displayed in the OpenCascade workbench.

## Files Permitted to Touch

- `README.md`
- `docs/studio.md`
- `docs/studio-limitations.md`
- `docs/studio-providers.md`
- `.mece/waves/studio-20261018/cells/delivery/REPORT-AGENT-DOCS.md`

Do NOT modify `packaging/**`, `.github/workflows/**`, `studio/**`, or `src/**`.

Write a concise completion summary to `.mece/waves/studio-20261018/cells/delivery/REPORT-AGENT-DOCS.md` when finished.
