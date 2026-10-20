# Python compatibility corpus

Model: opencode-go/deepseek-v4.1-flash, max
Task ID: node:delegated-task:command%3Amcp%3Af570e8f0-6996-4910-885b-e1db03b3d1e5%3Adelegate-task%3Arust-backend-20261019-corpus-r1
Child thread: thread:delegated-task:command%3Amcp%3Af570e8f0-6996-4910-885b-e1db03b3d1e5%3Adelegate-task%3Arust-backend-20261019-corpus-r1

## Status

Corpus generator and versioned fixtures are implemented, generated, and
verified. No Rust product code, Cargo workspace, packaging cutover, Python
product edit, install, commit, push, or delegation was performed.

## Deliverables

- `backend/compat/**` — reproducible Python generator and self-check.
- `backend/fixtures/**` — `corpus/v1` JSON fixtures, real schema-v2 SQLite
  database, and byte-exact default STL files.
- This cell `REPORT.md` / `SESSION.md`.

## Commands

```bash
.venv/bin/python backend/compat/generate_corpus.py
.venv/bin/python backend/compat/generate_corpus.py --check
.venv/bin/python backend/compat/self_check.py
```

All three pass on the generated tree. Fixtures total about 1.0 MiB.

## Scope notes

- Existing Python/core/Windows workers were read-only and untouched.
- HTTP used in-process Starlette `TestClient` with synthetic tokens and temp
  data directories; no live socket, external LLM, user DB, or user credential.
- Journal timestamps are wall-clock in production; the fixture harness injects
  a fixed clock into the journal module so event digests are reproducible.
- STEP is structural only; OCP 8.0.1.1.0 is installed on this host.
- The live core route table has 17 routes (16 under `/api/v1` plus root
  `/health`), not 18 as stated in the audit/task text. All 17 are covered.
