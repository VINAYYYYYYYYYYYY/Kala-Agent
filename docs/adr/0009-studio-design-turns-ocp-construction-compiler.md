# ADR-0009: Studio Design Turns: ConstructionScript Compiled and Measured by OpenCascade

| Field | Value |
|---|---|
| Status | **Proposed**. Implementation authorized by the user ("continue the work") on 2026-10-19; the decision stays Proposed until a stakeholder accepts it. |
| Date | 2026-10-19 |
| Scope | Kala Studio (`src/kala_agent/studio/**`, `studio/**`): turn a chat request into a real, measured, journaled B-Rep part |
| Related ADRs | ADR-0001 (local-first), ADR-0003 (bounded LLM planning, deterministic workflow), ADR-0004 (journal, content-addressed artifacts), ADR-0007 (FreeCAD adapter), ADR-0008 (Solid Edge bridge) |
| Implementation status | Not implemented. Wave `.mece/waves/design-turns-20261019/`. |

## Context

The Studio UI (the Vellum drafting board) is designed around a mechanical part that the agent drafts, measures, and hands off: an L-bracket with a bore and four holes, the measured volume and area, "B-Rep Verified", a hash-chained journal, a Section A–A, and an orthographic 3-view. The Studio backend cannot honestly supply any of that today:

- `POST /api/studio/chat` returns text only (`ChatResponse{text, provider, model, offline}`).
- `POST /api/studio/preview` and `/export` know three analytic primitives (box, cylinder, sphere).
- The Studio server deliberately never instantiates `KalaApplication` (`studio/server.py` docstring), so the core journal, revisions, and artifact store are unreachable from the UI.

Meanwhile the parts needed already exist:

- `kala_agent.contracts.ConstructionScript`: the neutral, bounded construction language (sketch rectangle/circle on XY/XZ/YZ, extrude, revolve, pocket, place, fuse) that the FreeCAD adapter and the Solid Edge compiler both accept, with semantics pinned by `docs/construction-semantics.md` and an expectation oracle (`kala_agent_solidedge.oracle`).
- OCP (`cadquery-ocp`) is installed and already used for STEP export. `BRepGProp`, `Bnd_Box`, `BRepCheck_Analyzer`, `HLRBRep_Algo`, and `BRepAlgoAPI_Section` are importable.
- `kala_agent.persistence.chain` (length-prefixed SHA-256 event chain) and `ContentAddressedArtifactStore` are reusable libraries.

## Decision

Add **design turns** to Studio: the LLM (or the offline deterministic helper) proposes, the geometry kernel disposes.

```
chat prompt ──► DesignIntent (template call or raw ConstructionScript + declared tolerances + material)
                  │  validated by pydantic; invalid ⇒ explicit error turn, never a silent fallback
                  ▼
             OCP compiler  (ConstructionScript ⇒ TopoDS_Shape; supported subset, refuses the rest)
                  ▼
             measure  (volume, area, centroid, exact bounds, BRepCheck validity, solid count)
                  ▼
             checks   (declared envelope vs measured, validity) ⇒ "B-Rep Verified" only if all pass
                  ▼
             drawing  (HLR visible/hidden edges for front/top/right/iso; section loops at a plane)
                  ▼
             studio session journal (own SQLite, chain digests) + artifact store (STEP, script JSON)
                  ▼
             UI: inline result table, digest chips, real Section A–A, HLR pencil lines, HASH-CHAIN OK
```

### 1. OCP construction compiler (`src/kala_agent/studio/design/compile_ocp.py`)
- Input: a `ConstructionScript`. Output: one `TopoDS_Shape` per component plus the placed or fused assembly shape.
- v1 subset: `sketch` (rectangle or circle, plane XY/XZ/YZ, offset x/y), `extrude` (length, `reversed`), `pocket` (finite depth or through-all, same-plane circle/rectangle), `revolve` (rectangle about the plane axis, as specified in construction-semantics), `place` (translation), `fuse`.
- Refuse `helix`, `fillet`, `move`, `resize`, `compound`, and all analysis ops with `ConstructionError("unsupported by the Studio OCP compiler v1: <op>")`. Never approximate (no thread-as-cylinder, no skipped fillet).
- Semantics must follow `docs/construction-semantics.md` exactly (plane normals, offset directions, reversed). Where the doc and intuition disagree, the doc wins.
- Budget: the existing contract limits (MAX_STEPS, MAX_COMPONENTS, MAX_DIMENSION_MM) plus a wall-clock budget per compile (default 5 s). Exceeding it returns an error and leaves no partial artifact.

### 2. Measure and checks (`design/measure.py`)
- `BRepGProp.VolumeProperties_s` / `SurfaceProperties_s` give volume (mm³), area (mm²), and centroid. Bounds come from `BRepBndLib.AddOptimal_s` (not the loose box). `BRepCheck_Analyzer(shape).IsValid()` gives validity, and the solid count comes from a topology explorer.
- `checks[]`: `{id, label, expected, measured, tolerance, unit, passed}`. Always present: `brep_valid`, `single_solid` (when one part is expected), and `envelope_x|y|z` (declared vs measured, tolerance 0.01 mm). `verified = all(passed)`.
- Every number shown in the UI comes from here. Templates may declare an expected envelope; they never declare volume.

### 3. Drawing (`design/drawing.py`)
- `HLRBRep_Algo` + `HLRBRep_HLRToShape` for views `front` (−Y), `top` (+Z), `right` (+X) in third-angle arrangement, plus `iso`. Each view yields `visible` and `hidden` polylines in view-plane mm, discretized with a chord tolerance (default 0.05 mm) and capped at 20k points per view.
- `section(plane, offset_mm)`: `BRepAlgoAPI_Section` of the shape with the plane, giving closed loops (outer and inner) in plane coordinates. The UI applies 45° hatching between outer and inner loops. Default plane: the mid-plane of the measured bounds normal to Y.

### 4. Templates and intent (`design/templates.py`, `design/intent.py`)
- `DesignIntent = {template: TemplateCall} | {script: ConstructionScript}`, plus `material` (string key, declared), `tolerances[]` (declared strings such as `"Ø10 H7"` and `"ISO 2768-m"`), and an optional `label`.
- Templates are deterministic functions that produce a `ConstructionScript` and an expected envelope:
  - `l_bracket(width W, depth D, height H, wall T, bore_d B?, hole_d h?, hole_count 0|2|4, edge e = max(1.5·h, 6))`. Base plate: x∈[−W/2,W/2], y∈[0,D], z∈[0,T]. Upright: x∈[−W/2,W/2], y∈[0,T], z∈[0,H]. Bore along Y through the upright at (x=0, z=T+(H−T)/2). Base holes along Z at x=±(W/2−e), y∈{T+e, D−e} (4) or y=(T+D)/2 (2).
  - `stepped_shaft(segments: [(diameter, length)…])`: coaxial circle extrudes along +Z, fused.
  - `housing_envelope(W, D, H, wall)`: box with a rectangular pocket from +Z leaving the floor and walls `wall` thick.
- Local deterministic helper: a strict parser maps the four starter prompts and similar phrasings (part family plus explicit mm values) to template calls. Anything it cannot parse returns an explicit "need: …" clarification, never a guess.
- LLM providers: the system prompt carries the JSON Schema of `DesignIntent`. The reply is parsed and validated. If invalid, return one repair round-trip with the validation error, then fail the turn honestly.

### 5. Studio session journal and artifacts (`studio/session_journal.py`)
- The Studio keeps its **own** SQLite journal and artifact store under the Studio data directory. This preserves the boundary that Studio shares no state with the core `KalaApplication`; it reuses only the `chain` and `artifacts` libraries.
- Events: `design.intent`, `design.compiled`, `design.measured`, `design.checked`, `artifact.stored` (STEP and script JSON, with sha256), `export.completed`, `chat.turn`. The payload is canonical JSON and the digest is `chain_digest`.
- `verify()` recomputes the chain and returns `{ok, event_count, head_digest, first_bad_sequence?}`.

### 6. API (all under `/api/studio`, behind the existing bearer/Host/Origin gates and body limits)
| Route | Purpose |
|---|---|
| `POST /design` | `DesignIntent` ⇒ `DesignResult{design_id, label, script, measured, checks, verified, mesh, artifacts{script_sha256, step_sha256?}, journal_head}` |
| `GET /design/{id}/drawing?views=front,top,right,iso` | HLR polylines |
| `GET /design/{id}/section?plane=XZ&offset=…` | section loops |
| `POST /chat` | unchanged request; response gains optional `design: DesignResult` when the turn produced one (backward compatible) |
| `GET /journal?after=<seq>&limit=<n>` | bounded page of session events |
| `GET /journal/verify` | chain verification |
| `POST /export` | additionally accepts `design_id`; STEP is written from the compiled shape and the digest is returned |

`design_id` is the sha256 of the canonical script JSON, so the same script always gives the same id (idempotent).

### 7. UI
- Agent card: an inline table of `measured` plus `checks` (✓/✗ per row), digest chips from `artifacts`, and "B-Rep Verified" only if `verified`.
- Center sheet: when a design is active, the mesh comes from `DesignResult.mesh`. Pencil 2D mode draws the HLR `iso` polylines (visible solid, hidden dashed) over the canvas instead of relying only on three.js edge heuristics. The title block uses `label` and `design_id[:8]`.
- AuxView: Section uses `/section` loops, Ortho uses `/drawing` front/top/right, and Tolerance lists the declared `tolerances[]`, labelled "declared — not analysed".
- Journal: polls `/journal?after=` (1 s while a turn is running, 5 s idle, paused when the document is hidden) and merges with UI-local entries. The `HASH-CHAIN OK` stamp shows only if `/journal/verify` reports `ok: true`.

## Acceptance criteria (the tests are the spec)
1. **Parity:** every `docs/construction-semantics.md` corpus case compiled by the OCP compiler matches the oracle's expected volume and bounds within 1e-6 relative / 0.001 mm. Core must never import the adapters (`tests/unit/test_solidedge_boundaries.py`). So the expectations are frozen once into `tests/fixtures/construction_parity.json` by a generator script run with `PYTHONPATH=adapters/solidedge/src:src`, which calls `kala_agent_solidedge.oracle.expectation`. The fixture records the generator command and the oracle source digest. The core test reads only the JSON and fails, never skips, if the fixture is missing. The fixture must also agree with every measured number printed in construction-semantics.md.
2. **L-bracket oracle:** `l_bracket(W=60, D=45, H=40, T=8, B=10, h=5.5, hole_count=4)` gives volume **35 571.416 047 mm³** (= W·D·T + W·T·(H−T) − π(B/2)²·T − 4·π(h/2)²·T) within 1e-6 relative, bounds exactly [−30,30]×[0,45]×[0,40], valid single solid, and `verified=true`.
3. **Refusals:** each unsupported op raises `ConstructionError` naming the op. No partial artifact is stored and no `design.compiled` event is written.
4. **Section:** the L-bracket from criterion 2 cut by the XZ plane at y = T/2 = 4 passes through the upright and the base (both span y = 4) and misses the base holes (at y = 16.25 and 36.75, since e = 8.25). It has exactly one outer loop, the 60 × 40 rectangle x∈[−30,30], z∈[0,40], and one inner circular loop of Ø10 centred at (x=0, z=24), within ±0.05 mm. Enclosed area (outer − inner) = 2400 − 25π = **2321.460 mm²** within 0.1 %.
5. **Journal:** after a design turn, `/journal/verify` is ok. Mutating any stored payload byte makes verify return `ok: false` with `first_bad_sequence`.
6. **Honesty:** with the local helper and an unparseable prompt, the response has no `design` and asks for the missing values. No endpoint returns a number that was not measured.
7. **Security:** all new routes reject a missing or wrong bearer, a wrong Host, and a wrong Origin, and enforce body limits. Mesh and polyline outputs are bounded.

## Consequences
- The Studio becomes a real design tool for a narrow, honest subset: prismatic and turned parts without fillets or threads.
- The same `ConstructionScript` drives Studio's OCP preview, the FreeCAD adapter, and the Solid Edge compiler, so a design drafted in Studio can be handed off unchanged.
- The Rust backend migration (`.mece/waves/rust-backend-20261019/`) gains new Python surface that becomes part of its parity corpus. OCP has no first-class Rust binding, so the Rust plan must either keep a geometry worker process or adopt an OCCT binding. That wave must record the choice; this ADR does not prejudge it.

## Not claimed
No FEA, tolerance-stack analysis, fits computation, or certification. "Verified" means OCP validity plus measured-vs-declared envelope. It says nothing about strength or manufacturability.
