# Rust backend audit session

Model: opencode-go/deepseek-v4.1-flash, variant max
Role: read-only architecture/compatibility inventory
Task ID: node:delegated-task:command%3Amcp%3Af570e8f0-6996-4910-885b-e1db03b3d1e5%3Adelegate-task%3Arust-backend-20261019-audit-r1
Child thread: thread:delegated-task:command%3Amcp%3Af570e8f0-6996-4910-885b-e1db03b3d1e5%3Adelegate-task%3Arust-backend-20261019-audit-r1
OpenCode session ID: not reported.

## Status

Complete. REPORT.md written. Read-only: no tracked file modified, no install, no commit/push, no
delegation. `Rust+buid123/` and `rust-implementation.md` untouched. Python sources untouched.

## What was run (evidence)

- `.venv/bin/python -m pytest -q` → 1460 passed, 1 warning, 21.05 s.
- `adapters/solidedge-bridge/tests` → 171 passed, 1 skipped (live Windows gate).
- `adapters/solidedge/tests` (with `PYTHONPATH=src:adapters/solidedge-bridge/src`) → 103 passed.
- `adapters/freecad/tests` → 382 collected only (FreeCAD not installed; not claimed as run).
- `/tmp/opencode/probe_baseline.py` → mesh/STL/canonical-JSON/CAS/journal/domain numbers.
- `/tmp/opencode/probe_concurrency.py` → journal throughput and O(N) read scaling, CAS concurrency.
- `/tmp/opencode/probe_http.py` → real studio process: startup, RSS, p50/p95, all negative gates.
- Frozen `dist/kala-studio` → health 200, startup 1.77 s, 339 MB RSS.
- `cargo 1.99.1`/`rustc 1.99.1`; crates.io index + static.crates.io reachable (HTTP 200).
- Read: studio server/schemas/middleware/boundary/errors/geometry/step/stl/providers/settings/static/
  cli; core api app/schemas/boundary/errors/middleware/dependencies/envelope; config; contracts
  models/construction; application; persistence database/chain/journal/artifacts/repositories; kernel
  budget/policy/workflow/memory/planning; tools protocol/executor/registry/sandbox/simulator; runtime
  agent/model_gateway; domains shared + package digests; CLI; packaging spec/entry; electron main/
  preload/desktop-pack/electron-builder; solidedge bridge protocol/server/jobs/worker/auth/workspace;
  solidedge client/oracle/paths/cli; pyproject files; CI workflow; docs/adr references.

## Key findings

1. No Rust code exists anywhere; `Rust+buid123/` is a Python tree copy.
2. Two independent HTTP services with two different error envelopes (studio flat, core nested).
3. Canonical JSON is CPython-specific: float exponent thresholds, `ensure_ascii`, bigint precision.
   `serde_json` was measured to differ on `1e-05`, `1e+16`, `1e-07`, and Unicode escaping.
4. Decimal scale is observable (`"1.00"`, `"0.30"`); fixed-point integers would change digests.
5. Domain package digests are Python **source-file** digests and cannot be reproduced from Rust.
6. Python is already fast at defaults (health 0.90 ms, preview box 0.84 ms); wins are startup/RSS,
   max-tessellation geometry/STL, O(N) journal reads, concurrency.
7. OCP STEP and Solid Edge COM are unavoidable native sidecars; Windows/COM remains unproven.
8. `KALA_STUDIO_MESH_LIMIT` is documented but not implemented (pre-existing doc drift).

## Handoff

First vertical: `kala-studio-api` behind the existing Electron `--host/--port` + `KALA_STUDIO_TOKEN`
launch contract (no SQLite, no kernel). Second: core read-only endpoints over a Python-created DB.
Contract freeze (golden corpus) must precede any port. Full phased plan, crate ownership, versions and
benchmark rules in REPORT.md.
