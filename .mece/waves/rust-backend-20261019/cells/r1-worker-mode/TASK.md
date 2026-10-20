# r1-worker-mode: Python Studio geometry worker (ADR-0010 "Geometry worker (v1)")

Read `docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md` fully, then ADR-0009, then `src/kala_agent/studio/{cli.py,server.py,schemas.py,boundary.py,middleware.py}`, `design/service.py`, `providers/local.py`, `design/intent.py`, and `/home/v9i/Projects/Kala-Agent/.mece/waves/design-turns-20261019/cells/b1-api/REPORT.md`.

Own: `src/kala_agent/studio/worker.py` (new), the minimal `--worker` wiring in `src/kala_agent/studio/cli.py` / `__main__.py`, an internal router module `src/kala_agent/studio/internal_api.py` (new), `tests/unit/studio_worker/**` (new). Do NOT change the public `/api/studio/*` behaviour or any ADR-0009 module's public behaviour. If `service.py` needs a journal-append method, add it additively (no signature changes). Do not touch the user's uncommitted `src/kala_agent/__init__.py` or `src/kala_agent/api/**`, Rust, adapters, or `studio/**` (the frontend is being edited by another wave).

## Build
- `python -m kala_agent.studio --worker`: binds 127.0.0.1:0, data dir as normal Studio, reads `KALA_WORKER_TOKEN` from the env (refuse to start if missing, or shorter than 32 chars). Prints EXACTLY one stdout line `KALA_WORKER_READY {json}` after the socket is listening, then logs only to stderr. Exits cleanly on SIGTERM, and on stdin EOF if `KALA_WORKER_STDIN_LIFELINE=1` (so a dead parent never leaves an orphan).
- Gate: bearer == token (constant-time), Host must be 127.0.0.1:port, any Origin header → 403, body limits as Studio.
- Routes `/internal/v1/`: `preview`, `export`, `handoff`, `design`, `design/{id}/drawing`, `design/{id}/section`, `intent/parse`, `intent/validate`, `intent/schema` (GET; the DesignIntent JSON Schema text), `journal`, `journal/verify`, `journal/append` (event_type from an allow-list `chat.turn|settings.changed|provider.call|export.completed`, bounded payload). Reuse the existing handlers and services; no duplicated logic.
- Worker mode exposes NO `/api/studio/*`, settings, or provider routes, and makes no provider network calls.

## Tests
Readiness-line contract (spawn a real subprocess, parse the line, call `/internal/v1/design` with the L-bracket and get volume 35571.416047); token missing → refuse to start; wrong or missing bearer 401; bad Host 421; any Origin 403; oversize 413; journal append allow-list; SIGTERM and stdin-EOF shutdown; `intent/parse` on the Stitch sentence → intent, and on "make me a bracket" → missing values; `intent/validate` good/bad. Python gates per the design-turns WAVE.md plus the full `pytest tests/unit -q`.

Write REPORT.md here: the exact route contract (request/response shapes and status codes) for the Rust proxy worker, plus gate counts.
