# B2 providers: CAD provider routing (ADR-0011), Python reference backend

Start after r1 (worker mode + `design/importer.py`) has landed. Read ADR-0011 (binding), ADR-0009, ADR-0007 (FreeCAD adapter), ADR-0008 (Solid Edge bridge), `../b1-api/REPORT.md`, `/home/v9i/Projects/Kala-Agent/.mece/waves/rust-backend-20261019/cells/r1-worker-mode/REPORT.md`, and `adapters/freecad/**` (how the adapter is invoked and what it returns).

Own: NEW `src/kala_agent/studio/design/providers/{__init__,base,opencascade,freecad,solidedge}.py`, plus `design/service.py`, `server.py`, `schemas.py`, `providers/dispatch.py` (only the provider pass-through), and tests `tests/unit/studio_design/test_cad_providers*.py` and `test_design_api.py` (update). Core must NEVER import `adapters/**` (the boundary tests must stay green): reach FreeCAD through a subprocess (`FreeCADCmd` + the adapter's documented entry point) and Solid Edge through its loopback bridge protocol. Do not touch `src/kala_agent/__init__.py` or `src/kala_agent/api/**`, or `studio/**` (the UI is a later cell).

## Build
- `CadProvider` interface: `probe() -> Availability{available, version?, reason?}` and `build(script, intent) -> ProviderBuild{native_artifact?, step_bytes, measured{volume_mm3?, area_mm2?, bounds?, mass_g?}, provider_version, warnings}`.
  - `opencascade`: wraps the existing compile/measure/STEP code. Opt-in only.
  - `freecad`: subprocess with a bounded wall time and no shell. Builds the model, exports STEP and FCStd, and returns FreeCAD's own measured properties. If the adapter cannot build an op (for example fuse), it returns an honest refusal naming the op.
  - `solidedge`: probes the bridge per ADR-0008 (token and port discovery). On a non-Windows or bridge-less host it reports `available: false` with a reason. Build calls are implemented only to the extent the bridge contract allows, without importing adapters; otherwise refuse with "Solid Edge build requires the Windows bridge (not reachable)".
- `DesignService`: `provider` is REQUIRED (no default, never an automatic opencascade fallback). Build via the provider, store the STEP and native artifacts, then IMPORT the STEP with r1's importer for the view mesh (for every provider, opencascade included, so the view always shows what the provider exported). The checks compare provider-measured values with the declared envelope; a missing provider value gives check status `not_reported`, never a substituted number. Journal events per ADR-0011. `design_id` = sha256 of (canonical script + provider).
- API: `GET /providers/cad`; `POST /design` takes `provider`; `ChatRequest` gains an optional `cad_provider`. If a chat turn produces an intent with no provider selected, reply asking the user to choose one (listing the available ones) and return no design.

## Acceptance
- opencascade: the L-bracket is still 35571.416047 mm³ (rel 1e-6), and the imported view bounds are [-30,30]×[0,45]×[0,40].
- freecad (REAL, on this host with FreeCAD 1.1.4): every corpus case that FreeCAD supports builds through the provider, its volume matches `tests/fixtures/construction_parity.json` within 1e-6 relative, and the imported view bounds match within 0.001 mm. The L-bracket either builds (report FreeCAD's volume) or is refused honestly naming the unsupported op. Mark these tests `freecad` and skip with a CLEAR reason only if FreeCAD is absent; on this host they must run.
- solidedge: on this Linux host, `available: false` with a reason; a build request returns a 503-style refusal; no fake success.
- There is no default provider, and a missing provider gives a 422 naming the allowed values.
- The full unit suite and the boundary tests are green; ruff, format, and mypy are clean on touched files.
Write REPORT.md with the updated JSON shapes (for the UI cell C2 and the Rust r2 mirror).

## Added by Opus after reviewing r1 (binding)
- r1 landed: `design/importer.py` plus `POST /internal/v1/import` (see `/home/v9i/Projects/Kala-Agent/.mece/waves/rust-backend-20261019/cells/r1-worker-mode/REPORT.md`). You may edit `internal_api.py` and `importer.py` for the item below.
- The STEP import is capped at the generic Studio body limit of 262,144 bytes, which is far too small for real provider exports (multi-MB STEP from Solid Edge or FreeCAD). Give STEP import (and provider-returned STEP inside the service) its OWN limit: default 64 MiB, configurable via settings or env. Read the body in bounded chunks (never more than the limit in memory, and refuse as soon as it is exceeded). Keep the 200k-vertex / 400k-triangle tessellation limits, and when they are hit, retry ONCE with a coarser deflection before refusing with `step_too_complex`. Every other route keeps the generic limit. Test it with a generated multi-MB STEP (for example many FreeCAD solids, or a large valid STEP from the opencascade path) and an over-limit body.
