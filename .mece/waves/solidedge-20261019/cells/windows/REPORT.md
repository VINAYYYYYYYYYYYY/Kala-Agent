# Windows-local Solid Edge client/compiler — implementation report

Round: windows cell, first delivery. Owned paths only: `adapters/solidedge/**`
and this report. No commits, pushes, deployment, installs, or delegation. The
former Linux worker left no files; nothing here resumes it.

Read before implementing: `WAVE.md`, `cells/contract/REPORT.md`, the actual
`adapters/solidedge-bridge/src/kala_solidedge_bridge/protocol.py`, the bridge's
`auth.py`/`server.py` (concurrent worker's files, read-only), and
`docs/construction-semantics.md`.

## Delivered paths

Distribution `kala-agent-solidedge`, package `kala_agent_solidedge`,
`requires-python = ">=3.11"`, dependencies `kala-agent>=0.1`,
`kala-solidedge-bridge>=0.1`, `pydantic>=2.10,<3`.

- `adapters/solidedge/pyproject.toml` — pytest `pythonpath` is repo `src`,
  bridge `src`, own `src`; ruff and strict mypy mirror the FreeCAD adapter.
- `src/kala_agent_solidedge/compile.py` — ConstructionScript -> typed
  `BridgeOp` tuple and the `BuildRequest` factory.
- `src/kala_agent_solidedge/oracle.py` — closed-form corpus and comparison.
- `src/kala_agent_solidedge/client.py` — stdlib `urllib` client.
- `src/kala_agent_solidedge/journal.py` — durable JSONL journal.
- `src/kala_agent_solidedge/cli.py` + `__main__.py` — `kala-solidedge` and
  `python -m kala_agent_solidedge`; entry point
  `kala-solidedge = "kala_agent_solidedge.cli:main"`.
- `src/kala_agent_solidedge/paths.py`, `errors.py`, `__init__.py`.
- `examples/plate_with_hole.json`, `examples/positioned_parts.json`.
- `tests/` — `test_compile.py`, `test_oracle.py`, `test_client.py`,
  `test_cli.py`, `test_paths.py`, `test_integration.py`, `test_preflight.py`,
  `test_boundaries.py`.

## Behavior actually implemented

- Compiler refuses helix, fuse, compound, analysis, fillets, duplicate place,
  and rectangle-revolve on `XZ`, with a typed `CompileError` before any socket.
  Planes/offsets pass through in mm; `reversed` is preserved; a pocket with no
  length is through-all; several additive features extend one part; a circle
  revolve lowers to a pad of its length, else the diameter.
- Every part emits `part.new`, features, `part.measure`, `part.save`,
  `part.export_step`. An assembly is emitted for several components or any
  nonzero translation, with every component placed once (zero default) under a
  name that cannot collide with a component.
- Oracle predicts only: a rectangle/circle pad, and a pad with one same-plane
  contained circular hole. Plane mappings `XY(u,v,0)/+Z`, `XZ(u,0,v)/-Y`,
  `YZ(0,u,v)/+X`; reversal flips the normal extent; volume tolerance 0.1 %,
  bounds 0.001 mm; invalid/zero volume fails; assembly bounds are the translated
  union and no union volume is claimed; everything else is explicitly unchecked.
- Client refuses non-loopback URLs, userinfo/query/fragment/base path, and
  redirects; requires bearer auth on every call; caps JSON at 1 MiB and
  artifacts at 256 MiB; streams and verifies actual size plus SHA-256
  (Content-Length is never trusted); has socket and overall timeouts; downloads
  atomically under a sanitized name into a caller-owned directory; never logs
  the token; reports a lost submission as ambiguous with guidance to keep the
  request id.
- CLI: `doctor`, `build --script f.json [--dry-run] [--yes]`,
  `status id`, `cancel id`, with `--bridge-url`, `--token-file`, `--output-dir`
  and `KALA_SOLIDEDGE_BRIDGE_URL`/`KALA_SOLIDEDGE_TOKEN_FILE`.
  Windows defaults are `%LOCALAPPDATA%/Kala/SolidEdgeBridge/bridge.token` and
  `%LOCALAPPDATA%/Kala/SolidEdge/runs`; no Linux runtime default exists and no
  `~/.local` path is chosen. Token filename coordinated from the bridge's own
  `auth.TOKEN_FILENAME == "bridge.token"`; the bridge was not edited.
- `doctor` distinguishes unreachable, auth, unsupported, uncalibrated, busy,
  and version. A real build requires the live health to report a supported,
  available, calibrated host whose id matches `--calibration-id`; a stale id,
  busy bridge, or unsupported host never creates a journal entry or job.
- Non-Windows: `--dry-run` and `--help` work offline without a token or socket;
  a real `build`, `status`, and `cancel` exit `2` with a Windows-only message.
- Journal records the normalized script, script digest, canonical request
  digest, bridge target, request id, build id, and status — no credentials. A
  pending entry is reused only on an exact script+request+bridge match, so a
  lost submission retries the same id instead of creating a second job. An
  unfinished pre-upgrade entry that lacks that identity makes the CLI refuse
  with the existing request/build id and reconcile guidance, instead of
  silently minting a fresh id that could duplicate a job already at the bridge.
- Cancel is reported as cooperative: it never claims an in-flight COM call
  stopped. Indeterminate/timeout keep the build id and tell the operator not to
  resubmit.

## Checks and outcomes (actual)

Run from `adapters/solidedge` with `/home/v9i/Projects/Kala-Agent/.venv`:

- `python -m pytest -q` — **138 passed in 24.67s**.
- `ruff check .` — **All checks passed**.
- `ruff format --check .` — **17 files already formatted**.
- `mypy` (strict, `src` + `tests`, pydantic plugin) — **Success: no issues
  found in 17 source files**.
- `coverage run -m pytest -q && coverage report` — **84.80 %** total, above the
  configured `fail_under = 80`.

Test evidence highlights: compiler negatives (helix, fuse, compound, analysis,
fillets, duplicate place, XZ rectangle revolve), max core IR (8×12 steps) fits
the 600-op and 1 MiB caps, dimension parity (5000/0.001/±5000), plane/reversed/
circle-revolve, oracle corner errors and unchecked cases, real loopback HTTP for
auth/redirect/oversize/hash/size/network-ambiguity, Windows path selection,
CLI gating and offline dry run, health-preflight refusals, journal identity and
token-free content, artifact directory isolation, and an isolated-import test
proving no `FreeCAD`, studio, kernel, tools, or COM module is loaded.

## Entry points

- `kala-solidedge doctor|build|status|cancel` (console script).
- `python -m kala_agent_solidedge` (module entry point).
- `kala_agent_solidedge.compile.compile_operations` / `build_request`,
  `kala_agent_solidedge.oracle.expectation` / `compare`,
  `kala_agent_solidedge.client.BridgeClient`.

## Platform limits and open items

- No Windows host, Solid Edge installation, COM attach, calibration, or live
  feature build was available. Nothing here claims native Windows evidence;
  the integration tests use a test-only loopback stub, not Solid Edge.
- The bridge currently reports `supported=False` without a live-verified native
  executor, so a real `build` will honestly refuse at preflight on any host
  until that work lands. That is the intended gate, not a defect in this cell.
- Linux checks ran on the repository venv's Python 3.14 interpreter while mypy
  targets 3.11. No wheel build or installation was attempted.
- `adapters/solidedge` is untracked and was not committed, by instruction.

## Session

Task/child-thread IDs are in `cells/windows/SESSION.md`; the OpenCode session id
was not reported by the transport.
