# r2-studio-proxy: Rust kala-studio front door + supervised geometry worker (ADR-0010, ADR-0011)
Follow `../CONVENTIONS.md`, except for ownership (below). Read ADR-0010, ADR-0011, `../r1-worker-mode/REPORT.md` (the internal worker route contract; binding), `../studio/REVIEW-4.md` and the studio cell REPORTs (the accepted kala-studio and its 268-case live differential harness), `../process/REVIEW-2.md` (the kala-process API), and `../../SYNTHESIS.md` ("30 mismatches; /journal/verify Python 200 vs native 404").

Ownership: you own `backend/crates/kala-studio/**` and, because delivery is paused, the root `backend/Cargo.toml` / `Cargo.lock` for adding kala-process as a dependency of kala-studio ONLY. You may read `src/kala_agent/studio/**` but must not edit it. B2 (another worker) is changing the Python provider routing concurrently; your proxy forwards JSON bodies unchanged, so its new fields pass through.

## Build
1. A `GeometryBackend` trait (one async method per worker route: preview, export, handoff, design, drawing, section, import, intent_parse, intent_validate, intent_schema, journal_page, journal_verify, journal_append, providers_cad) and its implementation `PythonWorkerBackend`:
   - Spawn via kala-process. Command from `KALA_STUDIO_GEOMETRY_WORKER` (argv array; whitespace-split is NOT allowed: accept a JSON array or a single executable path plus the fixed args), with dev default `<repo>/.venv/bin/python -m kala_agent.studio --worker`.
   - Fresh 32-byte random token in the env `KALA_WORKER_TOKEN` (never argv, never logged), plus `KALA_WORKER_STDIN_LIFELINE=1`.
   - Parse the readiness line with a bounded start timeout. Restart with capped backoff, and report state in `/health` as `geometry_worker {state, version, design_available}`.
2. Routing per ADR-0010/0011:
   - Rust-native: health, integrations, providers, settings, chat.
   - Proxied (same public paths and payloads, Rust-enforced body, response and timeout limits; STEP import gets its large dedicated limit, matching the worker's): preview, export, handoff, `/design*`, journal, journal/verify, `/providers/cad`, and `/import` (public `POST /api/studio/import` for provider STEP). Worker down → 503 with a clear message while the native routes keep working.
   - Move the native `kala-geometry`/`kala-step` usage OFF the default path (ADR-0010: parked) behind a non-default cargo feature `native-primitives`. Delete nothing.
3. Chat-with-design:
   - Local provider: `intent/parse` then `design`.
   - Remote providers: schema from `intent/schema`, fenced JSON extraction, `intent/validate` with one repair, then `design`.
   - Pass the request's `cad_provider` through. If it is missing, ask the user to choose; NEVER default to opencascade.
   - Append `chat.turn` via `journal/append`.

## Tests
- A live differential: the existing corpus PLUS the ADR-0009/0011 design routes, run through the Rust front door + the real Python worker vs the Python public server, with ZERO unexplained mismatches (the 30 known mismatches must close).
- Worker crash/kill → 503 on the geometry routes and recovery after restart; the token is absent from argv (/proc/<pid>/cmdline) and from logs; a dead Rust parent → the worker exits via the lifeline.
- The full Rust gates.
Write REPORT.md.
