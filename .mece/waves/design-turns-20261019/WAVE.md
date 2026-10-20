# Wave: Studio design turns (ADR-0009), 2026-10-19

Spec: `/home/v9i/Projects/Kala-Agent/docs/adr/0009-studio-design-turns-ocp-construction-compiler.md`. Its "Acceptance criteria" section is binding and the tests are the spec. Read it fully before coding.

## Routing
Opus conducts, reviews, and verifies. All workers are OpenCode via T3: `opencode-go/deepseek-v4.1-flash` (max) and `opencode-go/minimax-m3.1-flash-free` (max). At most 2 concurrent workers per model. No commit, push, deploy, credential reads, or further delegation.

## Invariants
- Respect the user's uncommitted work: do NOT edit `src/kala_agent/__init__.py`, `src/kala_agent/api/**`, `adapters/**`, `backend/**`, `Rust+buid123/**`, `rust-implementation.md`, or anything not owned by your cell.
- Core never imports adapters (`tests/unit/test_solidedge_boundaries.py`, `tests/unit/test_portable_imports.py` must stay green).
- Studio never instantiates `KalaApplication`. It reuses only the libraries `kala_agent.persistence.chain` and `kala_agent.persistence.artifacts`, with its own data dir.
- No fabricated numbers. Every value is measured by OCP or declared by the user/template and labelled as declared.
- Python gates for touched files: `.venv/bin/python -m pytest <your tests> -q`, `.venv/bin/ruff check <paths>`, `.venv/bin/ruff format --check <paths>`, `.venv/bin/mypy <paths>` (strict, from pyproject). Also run the full `pytest tests/unit -q` once at the end and report the counts. Report any pre-existing failures, but don't fix files you don't own.
- Frontend gates (C1 and D1): `npm run typecheck && npm test && npm run build` in `studio/`.

## Phases and ownership
- **A1 compiler** (DeepSeek): `src/kala_agent/studio/design/__init__.py`, `compile_ocp.py`, `measure.py`; `tests/fixtures/construction_parity.json` + `tests/fixtures/generate_construction_parity.py`; `tests/unit/studio_design/test_compile_ocp.py`, `test_measure.py`. Criteria 1, 3, and the measurement half of 2 (build the L-bracket ConstructionScript by hand in the test).
- **A2 drawing** (MiniMax): `src/kala_agent/studio/design/drawing.py`; `tests/unit/studio_design/test_drawing.py`. Build its own test shapes directly with OCP (BRepPrimAPI box + cylinder cut) so it doesn't depend on A1. Criterion 4 is exercised on an equivalent hand-built L-bracket shape.
- **A3 journal** (DeepSeek): `src/kala_agent/studio/session_journal.py`; `tests/unit/studio_design/test_session_journal.py`. Criterion 5 at library level (append, page, verify, tamper detection) plus the artifact-store wrapper.
  A2 and A3 must not create `design/__init__.py` contents beyond what A1 owns. If A2 needs the package to exist before A1 creates it, create an empty `__init__.py` only if missing, and A1 then owns it.
- **B1 api** (after A; DeepSeek): `design/templates.py`, `design/intent.py`, `src/kala_agent/studio/server.py`, `schemas.py`, `providers/local.py` (strict parser), and the matching tests. Criteria 2 (end to end), 4 (via the API), 6, and 7.
- **C1 ui** (after B1): `studio/src/**` and `studio/test/**`, covering ADR §7 plus the top audit gaps.
- **D1 qa** (after C1): end-to-end screenshots, all gates, and an honest gap report.

Each cell writes `cells/<cell>/REPORT.md`: files, public interfaces, gate output counts, and gaps.
