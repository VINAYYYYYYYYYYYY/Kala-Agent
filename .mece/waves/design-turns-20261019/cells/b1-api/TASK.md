# B1 api: templates, intent, design routes, chat integration (ADR-0009 §4, §6)

Read first: `../../WAVE.md`, ADR-0009 (full), and the REPORTs of `../a1-compiler`, `../a2-drawing`, and `../a3-journal`. Those modules are DONE and reviewed. Import them and do not edit them. (A2 may still be finishing a small repair that adds `area_exact` to section loops. If `drawing.py` changes under you, re-read it before wiring.)

## You own
- `src/kala_agent/studio/design/templates.py`, `intent.py`, `mesh.py` (OCP tessellation), and `service.py` (orchestration).
- `src/kala_agent/studio/server.py`, `schemas.py`, `providers/local.py`, `providers/dispatch.py` (only the design-intent hook).
- Tests: `tests/unit/studio_design/test_templates.py`, `test_intent.py`, `test_service.py`, `test_design_api.py`. Also update existing `tests/unit/test_studio_*` files only where a contract legitimately grew (an optional field), never by weakening an assertion.

Do NOT edit `src/kala_agent/__init__.py`, `src/kala_agent/api/**`, the A1/A2/A3 modules, `adapters/**`, or `studio/**` (the frontend is C1's).

## Build
1. **templates.py**: `l_bracket`, `stepped_shaft`, `housing_envelope`, exactly as specified in ADR §4. Each returns `(ConstructionScript, declared_envelope, declared_tolerances, label)`. A1 found that pockets must be same-plane, so the L-bracket upright is a separate XZ component placed so it occupies y∈[0,T], fused with the base. Reuse A1's working hand-built test construction (see `test_compile_ocp.py` / `test_measure.py`). Validate parameter ranges and geometric feasibility (for example `2e < W`, `B < H − T`, `h < T`, hole edge distance) with clear errors.
2. **intent.py**: pydantic `DesignIntent` (template call or raw script, plus `material`, `tolerances[]`, `label`), and its JSON Schema export for LLM prompts.
3. **mesh.py**: `BRepMesh_IncrementalMesh` tessellation into positions/normals/indices, bounded by `studio/geometry/limits.py` (MAX_MESH_VERTICES/TRIANGLES). Refuse rather than truncate.
4. **service.py**: `DesignService(data_dir)` with a `SessionJournal` from A3 in the Studio data dir (`resolve_studio_data_dir` / `KALA_STUDIO_DATA_DIR`, never the core's DB).
   - `design_id` = the sha256 of the canonical script JSON, which is also the script artifact digest.
   - Bounded LRU (32) of compiled shapes. On a miss, recompile from the stored script artifact.
   - ONE process-wide geometry lock serialises every OCP operation, because OCP is not safe for concurrent mutation in the FastAPI threadpool.
   - Journal events: `design.intent`, `design.compiled`, `design.measured`, `design.checked`, `artifact.stored`, `export.completed`, `chat.turn`. Payloads hold no secrets.
   - On a compile error or refusal: write `design.rejected` with the reason, store no artifact, and write no `design.compiled` (criterion 3).
5. **Routes** (behind the existing bearer/Host/Origin/body-limit gates; reuse the existing error envelope):
   - `POST /design`, `GET /design/{id}/drawing?views=`, `GET /design/{id}/section?plane=&offset=`, `GET /journal?after=&limit=`, `GET /journal/verify`.
   - `/export` additionally accepts `design_id` (STEP or STL from the compiled shape, digest returned, stored as an artifact).
   - `/chat` response gains optional `design`.
   - If OCP is not importable, the design routes return 503 with the existing STEP-unavailable style message. `/health` reports `design_available`.
6. **Chat**:
   - **Local helper**: a strict parser for the four starter phrasings and similar ones (family keyword plus explicit mm values: "L-bracket … 40 mm … 8 mm wall … 10 mm bore … 4x M5"; M5 → 5.5 clearance per ISO 273 medium, stated in the reply). It runs the design and replies with a short text summary of MEASURED values plus `design`. If required values are missing, reply listing exactly which ones are needed and return no design (criterion 6). Keep the existing non-design local replies working.
   - **Remote/LLM providers**: the system prompt includes the DesignIntent JSON Schema and an instruction to answer with a fenced json block when the user asks for a part. Parse and validate; on failure, allow ONE repair round-trip containing the validation error, then fail honestly. `allow_remote` consent semantics stay exactly as they are.

## Acceptance (tests are the spec)
- Criterion 2 end to end: `POST /design` with the `l_bracket` template at W=60, D=45, H=40, T=8, B=10, h=5.5, 4 holes returns volume 35571.416047 (rel 1e-6), bounds [-30,30]×[0,45]×[0,40], `verified: true`, and checks including `brep_valid`, `single_solid`, and `envelope_x|y|z`, all passed.
- Criterion 4 via `/section?plane=XZ&offset=4` returns 1 outer and 1 inner loop with net area 2400−25π (use A2's exact area).
- Criterion 5 via `/journal/verify` after a design: ok. After tampering a payload byte through sqlite on the data dir: ok false with `first_bad_sequence`.
- Criterion 6: the local helper with "make me a bracket" (no numbers) returns no `design` and names the missing values. The local helper with the Stitch L-bracket sentence returns a design with the criterion-2 volume.
- Criterion 7: every new route rejects a missing or wrong bearer, a bad Host, and a bad Origin, plus oversize bodies. `views` and `plane` are validated against allow-lists, and `offset` is finite and bounded.
- The same script posted twice gives the same `design_id`, and the journal shows the second as a cache hit, not a second compile.
- Concurrency: 8 parallel `POST /design` calls all succeed and the journal verifies ok.

## Gates
WAVE.md Python gates for your files, plus full `pytest tests/unit -q`. The boundary and portable-import tests must stay green.

## REPORT
Write `REPORT.md` in this folder. Include the exact JSON response shapes for `DesignResult`, drawing, section, journal page, and verify, plus a sample response from each, because C1 builds the UI against them. Also report gate counts and gaps.
