# Final Windows-local milestone integration and focused repairs

Status: **integration round complete on the Linux development host.** The five
bridge review defects and the five client review gaps are repaired, the QA
boundary test and CI workflow exist, and the docs reflect the implemented state.
**No Windows host, no Solid Edge installation, and no COM attach was available.
No native build, calibration, or Windows CI run is claimed.** There is no
production native geometry executor, by design, and no Rust code was written,
built, or benchmarked.

Read before working: `WAVE.md`, `cells/bridge/TASK.md`,
`cells/bridge/REVIEW-1.md`, `cells/bridge/REPORT.md`, `cells/windows/TASK.md`,
`cells/qa/TASK.md`.

No delegation, commit, push, deployment, install, or credential change was
performed. No production file outside the ownership listed in `TASK.md` was
edited: the contract modules (`protocol.py`, `units.py`, `errors.py`,
`__init__.py`, `tests/test_protocol.py`, `tests/test_units.py`), the core
portable-import source, and the conductor coordination files are untouched. The
two cell reports (`cells/bridge/REPORT.md`, `cells/windows/REPORT.md`) were
reconciled for accuracy only; `tests/unit/test_portable_imports.py` received the
narrow two-test platform-scope edit authorized for this round.

## Changed paths in this round

### Bridge (`adapters/solidedge-bridge`)

| Path | Change |
| --- | --- |
| `src/kala_solidedge_bridge/jobs.py` | Exact calibration equality at admission; `BuildNotStartedError` seam; ambiguous submit → `indeterminate`; running job on restart/shutdown → blocked `indeterminate`; queued job on restart/shutdown → `interrupted`; backend loss while running → `indeterminate`. |
| `src/kala_solidedge_bridge/probe.py` | One documented `AddFiniteExtrudedProtrusion` call; a rejection is reported, never retried with another argument form. |
| `src/kala_solidedge_bridge/server.py` | No same-thread `shutdown()`; public `serve` refuses non-Windows with exit 2 before any default data directory or token creation. |
| `tests/support.py` | Test request helper now carries `calibration_id="cal-test"` matching the fake readiness. |
| `tests/test_jobs.py` | Missing/stale calibration refusals with no job files; changed-host-calibration replay; lost-acknowledgement indeterminate/blocked/no-replay; definite pre-start refusal; crash event; queued-vs-running restart distinction. |
| `tests/test_server.py` | Typed HTTP refusal for missing (`503 calibration_required`) and stale (`409 conflict`) calibration; Windows-safe subprocess environment. |
| `tests/test_probe.py` | Fake that records the creation call then raises `E_INVALIDARG`: exactly one call, no calibration, owned document closed, no quit. |
| `tests/test_runtime_edges.py` | Same-thread-shutdown regression with a mocked `KeyboardInterrupt`; non-Windows `serve` creates nothing; platform-neutral subprocess environment; platform-neutral health assertion. |
| `tests/test_cli.py` | Non-Windows probe branch is injected so the assertion is valid on a Windows runner too. |
| `tests/test_session.py` | Real missing-pywin32 test skips when pywin32 is installed; an injected binding failure asserts the typed path on every platform. |
| `tests/test_worker.py` | Platform-neutral backend health assertion. |

### Client (`adapters/solidedge`)

| Path | Change |
| --- | --- |
| `src/kala_agent_solidedge/cli.py` | Live health preflight before journal/submission (`Windows`, supported, available, not busy, non-empty matching calibration); exact journal reuse with an actionable refusal for an unfinished legacy ambiguous entry; oracle result now determines the exit code; artifacts download into a build-specific directory. |
| `src/kala_agent_solidedge/journal.py` | Adds canonical `request_digest`, normalized `bridge_target`, and the normalized script; `find_exact`. Unfinished legacy ambiguous entries are never silently reused. |
| `src/kala_agent_solidedge/paths.py` | `safe_build_id` (rejects separators, ADS colon, reserved device names, trailing dot/space) and `build_output_dir` (`<output>/builds/<id>`). |
| `tests/test_preflight.py` (new) | Unready/stale health → no POST and no journal; preflight ordering; wrong volume/bounds → nonzero; missing result refusal; unchecked-but-successful wording; two builds preserve both artifacts; traversal rejected; changed calibration does not reuse an id; lost-submit exact retry keeps id and payload; legacy ambiguous entry refuses with actionable reconcile guidance and no POST; no token in the journal. |
| `tests/test_cli.py`, `tests/test_integration.py`, `tests/test_oracle.py`, `tests/test_boundaries.py` | Updated for the exact-identity journal, build-specific output paths, a real test-typing fix, and Windows-safe subprocess environments. |
| `pyproject.toml` | Test-only mypy override list extended with `test_preflight`/`test_integration`/`test_boundaries` so pytest's untyped decorators do not mask strict typing of the test bodies. No runtime or check weakened. |

### QA and docs

| Path | Change |
| --- | --- |
| `tests/unit/test_solidedge_boundaries.py` (new) | AST boundaries: core never imports the adapters/CAD; bridge never imports `kala_agent`; client imports core contracts only; no FreeCAD coupling; `pywin32` lazily imported only inside `session.py`; protocol import loads no Windows or core module. |
| `tests/unit/test_portable_imports.py` (narrow edit) | Two identity tests that resolve the real Linux core executor objects are marked Linux-only with an explicit "not ported to Windows" reason; every portable/no-Linux-layer test in the file still runs on Windows. |
| `.github/workflows/solidedge.yml` (new) | Bridge-only job on Ubuntu+Windows; Windows-local job for root+bridge+client, portable imports, boundary tests, and both package suites; Linux development job with both suites and ruff/mypy. |
| `docs/solidedge.md` | Honest current status, actual flags/defaults, offline dry-run, meaningful probe command, build-specific artifact layout, calibration equality, restart semantics. |
| `docs/construction-semantics.md` | Status line updated: the compiler exists, native reproduction is still unverified. |
| `docs/adr/0008-solid-edge-windows-bridge.md` | Implementation status/verification refreshed; D4 restart and ambiguity semantics; D7 exact calibration; D8 Windows-only `serve`. |
| `cells/bridge/REPORT.md` | Repair round recorded with the new checks. |

## Review items resolved

### Bridge REVIEW-1

1. **Calibration binding.** A new job is created only when the request's
   non-empty `calibration_id` exactly equals the host's current calibration.
   Missing → `CalibrationRequiredError`, stale → `ConflictError`, both before
   any job directory, queue entry, or `backend.submit`. An identical
   request-id+digest replay still returns the existing job after a re-probe and
   never re-executes it.
2. **Ambiguous submission.** Any submit exception other than the explicit
   `BuildNotStartedError` marks the job `indeterminate`, persists the admission
   block, and aborts only the child. A scripted backend that records the start
   then raises proves indeterminate + blocked + no replay after restart.
3. **Recovery distinction.** A `running` job recovered after a crash is
   `indeterminate` and blocks admission; a `queued` job that was never submitted
   is `interrupted` and does not block. Both are persisted and never replayed.
4. **No mutating fallback.** The probe calls
   `AddFiniteExtrudedProtrusion` once. A rejection records the failure and the
   reason `not retried`; it never sends a second creation call. The owned
   document is still closed in `finally`.
5. **Serve cleanup and Windows gate.** `serve` closes the server and manager
   after the loop without calling `shutdown()` on the serving thread. The
   public command refuses non-Windows with exit 2 before creating a default
   data directory or token. Regression tests cover both.

### Client review

1. **Health preflight.** `build` fetches `/v1/health` after the offline compile
   and Windows/confirmation checks, and refuses before the journal or POST when
   the platform is not Windows, `supported` is false, `available` is false,
   the bridge is busy, the calibration is empty, or it differs from
   `--calibration-id`.
2. **Journal identity.** The journal stores the normalized script, the
   canonical request digest, and a normalized bridge target. A request id is
   reused only for the same script digest, request digest, target, and an
   unfinished status. When no exact replay exists and an unfinished pre-upgrade
   entry for the same script lacks that identity, the client **refuses** with an
   actionable message naming the existing request/build id and how to reconcile
   it; it never silently submits a fresh id that could duplicate a job already
   at the bridge. A known-different complete identity (for example a changed
   calibration) is a new semantic request and does submit a fresh id.
3. **Oracle exit code.** A checked mismatch or a missing result returns
   nonzero. An unchecked script may still succeed, but the message says
   `unchecked` and `not matched`; it never says the oracle matched.
4. **Artifact directories.** Downloads go to
   `<output-dir>/builds/<build-id>/`; the build id is validated for Windows
   directory safety (separators, ADS colon, reserved names, trailing dot/space).
   Two builds with the same artifact filename preserve both files.
5. **CLI checks.** `--help`, both examples with `--dry-run`, `doctor`, `status`,
   and the non-Windows refusals were run directly through the module entry
   point, not only through tests.

## Deviations and decisions

* The QA scope said no existing test/config edits should be needed. Three small
  edits were necessary for an honest Windows CI definition, not to make a
  failing check green: subprocess-based import tests now copy the inherited
  environment and prepend `PYTHONPATH` (a stripped environment can break Python
  startup on Windows), two platform-asserting tests were made explicit
  (`sys.platform` health, injected non-Windows probe branch), and two identity
  tests in `tests/unit/test_portable_imports.py` were marked Linux-only. Those
  two tests resolve `kala_agent.application` and the core API stack whose
  executor dependency is not ported to Windows; on Windows they skip with an
  explicit reason, while every portable/no-Linux-layer test in the same file
  still runs. The client `pyproject.toml` gained test modules in the existing
  mypy override list so pytest's untyped decorators do not mask strict typing
  of the test bodies. No runtime setting, lint rule, or assertion was relaxed.
* The bridge probe still has one documented argument form rather than a
  runtime-selected form. The review allowed either "determine one form before
  invocation" or "use one documented form and report failure"; the latter was
  chosen because the installed type information cannot be queried before the
  call without a live host.
* A running job recovered on restart becomes blocked `indeterminate`, not
  `interrupted`. This is a deliberate refinement of the earlier recovery rule
  and is reflected in the code, tests, and docs.
* The rate-limited bridge worker was resumed externally and landed its final
  bridge state (186 passed, 1 skipped) after this round's first bridge edits.
  That newer state is preserved as-is; this report and the docs were reconciled
  to its counts, and the five review repairs were re-verified present in it.

## Public contracts

### Bridge CLI (`kala-se-bridge`)

```text
kala-se-bridge token  [--data-dir PATH]
kala-se-bridge probe  [--launch] [--output PATH] [--data-dir PATH]
kala-se-bridge serve  [--port 8797] [--data-dir PATH]
```

* `token` is the only command that prints the token; the file is
  `<data-dir>/bridge.token` (Windows default
  `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token`).
* `probe` writes diagnostic JSON, exits `0` only for a measured in-tolerance
  cube with a signed calibration, `2` on a non-Windows host, `1` otherwise.
  `--launch` dispatches Solid Edge; without it the probe attaches only.
* `serve` binds `127.0.0.1` and requires Windows; off Windows it exits `2`
  before creating anything. Health and builds remain honest (`supported=false`,
  `available=false` until a real executor exists).

### Client CLI (`kala-solidedge`)

```text
kala-solidedge doctor  [connection flags]
kala-solidedge build --script FILE.json [--dry-run] [--yes]
                     [--calibration-id ID] [--timeout SECONDS]
                     [connection flags] [--output-dir PATH]
kala-solidedge status BUILD_ID [connection flags]
kala-solidedge cancel BUILD_ID [connection flags]
```

* Connection flags: `--bridge-url` (default `$KALA_SOLIDEDGE_BRIDGE_URL` or
  `http://127.0.0.1:8797`), `--token-file` (default
  `$KALA_SOLIDEDGE_TOKEN_FILE` or the Windows `bridge.token` path),
  `--output-dir` (default `%LOCALAPPDATA%\Kala\SolidEdge\runs` on Windows).
* `--dry-run` compiles and prints the review summary with no token and no
  socket. A real build requires Windows, `--calibration-id`, `--yes` on a
  non-TTY, and a live ready health.
* Exit codes: `0` success (including a successful unchecked build), `1` typed
  client/oracle failure, `2` usage or non-Windows refusal.

### Factory and test seams

* `kala_solidedge_bridge.jobs.JobManager(data_dir, backend_factory,
  readiness_provider=..., start_supervisor=...)` is the single injection point;
  `BuildNotStartedError` is the only definite pre-start refusal.
* `kala_solidedge_bridge.server.create_server(port, manager, token)` and
  `production_manager(data_dir)` are the server seams; `serve` is the public
  Windows-only loop.
* `kala_solidedge_bridge.probe.run_probe(..., active_platform=...,
  session_factory=...)` is the probe seam.
* `kala_agent_solidedge.cli.run(argv, stdout=..., stderr=..., stdin=...,
  platform=...)` is the CLI seam; `BridgeClient` takes an explicit loopback URL
  and token.

## Checks and outcomes

All commands were run on the Linux development host with
`/home/v9i/Projects/Kala-Agent/.venv` (Python 3.14; mypy targets 3.11).

| Command | Result |
| --- | --- |
| `cd adapters/solidedge-bridge && python -m pytest -q` | **186 passed, 1 skipped** (gated live Windows test; final state from the resumed bridge worker, preserved) |
| `cd adapters/solidedge-bridge && ruff check .` | All checks passed |
| `cd adapters/solidedge-bridge && ruff format --check .` | 27 files already formatted |
| `cd adapters/solidedge-bridge && mypy` | Success: no issues found in 27 source files (strict) |
| `cd adapters/solidedge && python -m pytest -q` | **138 passed** |
| `cd adapters/solidedge && ruff check .` | All checks passed |
| `cd adapters/solidedge && ruff format --check .` | 17 files already formatted |
| `cd adapters/solidedge && mypy` | Success: no issues found in 17 source files (strict) |
| `cd adapters/solidedge && coverage run -m pytest -q && coverage report` | **138 passed**, **84.80%** total (above `fail_under=80`) |
| `python -m pytest tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py -q` | **16 passed** (the two core-identity tests run here on Linux and skip on non-Linux) |
| `python -m ruff check tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py` | All checks passed |
| `python -m ruff format --check tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py` | Already formatted |
| `python -m mypy tests/unit/test_solidedge_boundaries.py` | Success: no issues found |
| `python -c "import yaml; yaml.safe_load(...)"` on the workflow | Parsed; jobs `bridge-only`, `windows-local`, `linux-development` |
| `python -m kala_agent_solidedge --help` | Exit 0; commands doctor/build/status/cancel and honest epilog |
| `python -m kala_agent_solidedge build --script examples/plate_with_hole.json --dry-run` | Exit 0; 8 operations, no assembly, no network |
| `python -m kala_agent_solidedge build --script examples/positioned_parts.json --dry-run` | Exit 0; 17 operations, assembly true, no network |
| `python -m kala_agent_solidedge doctor` (Linux) | Exit 1 with `condition: unsupported` |
| `python -m kala_agent_solidedge status build-1` (Linux) | Exit 2 with a Windows-only message |
| `python -m kala_solidedge_bridge --help` | Exit 0; token/probe/serve |
| `python -m kala_solidedge_bridge serve --port 8797` (Linux) | Exit 2; no data directory created |
| `python -m kala_solidedge_bridge probe` (Linux, earlier round) | Exit 2 with `attach.status: unsupported` |

The earlier full-root evidence (1460 tests) from the portable worker is still
valid: this round changed no core source. The new boundary file adds tests, and
the only existing-root edit is the authorized two-test platform-scope change in
`tests/unit/test_portable_imports.py`; no other root test or configuration was
altered.

## Unresolved and explicitly not claimed

* **No Windows native proof.** No Windows host, no Solid Edge install, no COM
  attach, no type-library read, no `Application.Version`, no interactive-session
  probe, no calibration record, and no native part/assembly/`.par`/`.asm`/STEP.
* **No production native executor.** The worker acknowledges and then fails a
  build with `unavailable`. The probe is diagnostic, not a build executor.
* **No Windows CI execution.** The workflow is defined and parsed locally; it
  has not been run remotely. Live Solid Edge tests stay skipped unless
  `KALA_SOLIDEDGE_LIVE=1` on a real desktop.
* **SDK/type-library details remain unverified.** The probe's
  `ComputePhysicalProperties` array shapes, RefPlanes index mapping, STEP
  translator behaviour, and Student Edition automation access are unresolved
  and listed in the probe's `missing_probe_work`.
* **Prompt/Studio wiring deferred.** No Studio launcher, no shared-CAD tool
  extraction, and no UI implying native readiness.
* **Strict sandbox unported.** The portable import fix does not port the Linux
  sandbox; the bridge runs outside it with the compensating controls recorded
  in ADR-0008.
* **No Rust implementation or benchmark result** was produced in this round.
