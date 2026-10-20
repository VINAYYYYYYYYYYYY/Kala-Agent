# Windows bridge and probe milestone — implementation report

Status: **foundation complete and checked on a Linux development host, plus
the REVIEW-1 repair round. No Windows host, no Solid Edge installation, and no
COM attach was available, so nothing here is a native compatibility,
calibration, or build claim.** The production native build executor is
deliberately absent: the worker acknowledges a build and then fails it with
`unavailable`, and health never reports a native capability it cannot prove.

Read before working: `WAVE.md`, `cells/contract/REPORT.md`,
`cells/api/REPORT.md`, `protocol.py`.

No delegation, commit, push, deployment, or unrelated install was performed.
The user's correction was applied: everything is local to Windows; Linux is only
a development/test host; the token file is `bridge.token` under
`%LOCALAPPDATA%/Kala/SolidEdgeBridge` so a Windows-local client resolves the
same file.

## Changed paths

New runtime modules (all under
`adapters/solidedge-bridge/src/kala_solidedge_bridge/`):

| File | Role |
| --- | --- |
| `version.py` | `BRIDGE_VERSION = "0.1.0"` runtime metadata. |
| `session.py` | **Only lazy `pywin32` importer.** One STA thread, `CoInitializeEx(COINIT_APARTMENTTHREADED=2)` / matched `CoUninitialize` / idle `PumpWaitingMessages`; attach (`GetActiveObject`) or `--launch` (`Dispatch` + `Visible=True`); HRESULT normalization; retry of only `0x80010001`/`0x8001010A` with a 5 s deadline; VARIANT by-ref helpers; live type-library member/parameter extraction; enum resolution by name with no literal substitution. |
| `workspace.py` | Data-directory layout, atomic JSON writes, private-file helpers, Windows `icacls` ACL tightening, `ArtifactStore` (manifest-only resolution, per-read size+digest verify), append-only `DocumentLedger`. |
| `auth.py` | `bridge.token` create/read, exclusive create, POSIX 0600, constant-time compare, no token logging. |
| `calibration.py` | Signed probe records keyed to SE version + template + binding + probe version; HMAC with a private per-data-dir key; user-edited JSON fails verification; `native_build_verified` is always false. |
| `probe.py` | Diagnostic JSON plus guarded private 10 mm cube probe: platform, interactive hints, attach, `Application.Version`, type-library metadata, resolved enum values; cube attempted only when required enum names and runtime members are confirmed; saves `.par` + STEP, hashes them, closes only its own document in `finally`; records calibration only on a measured, in-tolerance cube. |
| `jobs.py` | Typed `BuildBackend` seam, durable `JobManager`, idempotency, one active build, bounded trace, cooperative cancel, supervisor deadline → `indeterminate` and admission block, restart distinction (`queued` → `interrupted`, `running` → `indeterminate`/blocked), artifact publication; production `ChildProcessBackend`. |
| `worker.py` | Spawned child process: newline-delimited JSON over stdin/stdout, owns the STA; no HTTP. Production build returns `unavailable` honestly. |
| `server.py` | `ThreadingHTTPServer` on `127.0.0.1`; bearer auth on every route; `Origin` rejection; exact loopback Host authority; body bounds before read; routes `GET /v1/health`, `POST /v1/builds`, `GET /v1/builds/{id}`, `POST /v1/builds/{id}/cancel`, `GET /v1/builds/{id}/artifacts/{name}`; typed errors; no COM on HTTP threads. |
| `__main__.py` | `main(argv) -> int`; `token --data-dir`, `probe --launch --output --data-dir`, `serve --port --data-dir`; `--help` and module entry point. |

New tests (`adapters/solidedge-bridge/tests/`): `support.py`,
`test_auth.py`, `test_calibration.py`, `test_session.py`, `test_probe.py`,
`test_jobs.py`, `test_server.py`, `test_worker.py`, `test_cli.py`,
`test_runtime_edges.py`, `test_live_solidedge.py`.

`pyproject.toml`: added the `solidedge` pytest marker and strict-mypy
`disallow_untyped_decorators = false` overrides for the new test modules only.
No contract setting, dependency, script, or check was weakened; the contract
worker's files (`protocol.py`, `units.py`, `errors.py`, `__init__.py`,
`tests/test_protocol.py`, `tests/test_units.py`) were not touched.

## Backend factories and seams

* Production: `ChildProcessBackend(data_dir)` spawns
  `python -m kala_solidedge_bridge.worker --data-dir <dir>`; the parent never
  calls COM. `production_manager(data_dir)` wires it with `JobManager`.
* `JobManager(data_dir, backend_factory, readiness_provider=...)` is the single
  injection point. Tests use a scripted in-process backend and a test-only
  readiness provider. The fake backend is never labelled native, and the
  production worker can never report `supported=True`.
* Readiness = platform + backend health (attach state, `Application.Version`) +
  a matching signed calibration record; `supported` additionally requires a
  backend that advertises a live-verified executor, which none does.

## Probe command and output structure

```text
kala-se-bridge probe [--launch] [--output PATH] [--data-dir PATH]
kala-se-bridge token [--data-dir PATH]
kala-se-bridge serve [--port 8797] [--data-dir PATH]
```

Exit codes for `probe`: `0` only if a measured 10 mm cube produced `.par` +
header-valid STEP and a signed calibration record; `2` on a non-Windows host
(diagnostic JSON still written); `1` otherwise.

Report JSON top level: `schema`, `probe_version`, `binding`, `template`,
`generated_at`, `platform` (sys.platform, Python version/bits/machine), `host`
(`SESSIONNAME`, non-proof interactive hint), `attach` (`attached` /
`not_running` / `unsupported` / `failed` with HRESULT), `application_version`,
`typelib` (per-object library/version/member metadata incl. parameter count and
VT flags for the app, PartDocument, Models, Lines2d, Model, Body), `constants`
(`resolved` + `missing`), `geometry` (attempted/valid, ref-plane normal and root
point, attempts, measured `volume_mm3`/`surface_area_mm2`/`min`/`max` in mm,
physical-properties status, reason), `artifacts` (name/size/sha256),
`document` (created/tagged/closed, never `ActiveDocument`), `probe_run` private
run dir, `session.stopped_cleanly`, `calibration_required`,
`calibration_id`, `native_build_capability` (always `false`), and
`missing_probe_work`.

Calibration records live in `<data-dir>/calibration/*.json` and are HMAC-signed
with `<data-dir>/calibration.key` (0600). Editing the record invalidates the
fingerprint and the signature, and readiness is never read from a flag.

## Behavior summary

* Admission order: idempotent `request_id`+digest replay → `busy` for an active
  or unreconciled build → `supported` → `available` → exact calibration
  equality (`request.calibration_id` must be non-empty and equal to the
  host's current calibration; missing → `calibration_required`, stale →
  `conflict`) → job creation. Errors map to 400/401/404/409/413/503 (`busy`
  409, unready/calibration 503, stale calibration 409, internal 503).
* Cancellation is cooperative between operations. A supervisor-deadline overrun
  marks `indeterminate`, stops only the child process, and blocks new admission
  until `JobManager.reconcile`. A lost submission acknowledgement, a worker
  crash while `running`, a backend poll failure while `running`, and a backend
  that vanishes while `running` are likewise `indeterminate`: the child may
  already have started a native operation. Only a `BuildNotStartedError` from a
  backend that positively proves it never started the build is a definite
  `failed`.
* On restart, a `queued` job that was never submitted becomes `interrupted`
  and frees admission; a `running` job becomes `indeterminate`, blocks
  admission, and carries a typed `timeout_indeterminate` result until
  reconciled. Nothing is ever replayed.
* Artifacts are named only through a completed manifest and re-verified on every
  read; partial artifacts from a failed build are preserved and reported.

## REVIEW-1 repairs (all five landed)

1. **Exact calibration binding.** `JobManager.submit` compares the request's
   non-empty `calibration_id` for exact equality with the readiness calibration
   before creating a job file, queue entry, or backend submission. Missing →
   `CalibrationRequiredError`; mismatched → `ConflictError`. The same
   request_id+digest replay still returns the existing job even after the host
   re-probes, and never re-executes. Tests cover missing/stale/refusal without
   job files, plus HTTP typed refusals (409 `conflict`, 503
   `calibration_required`).
2. **Ambiguous submission is indeterminate.** `run_once` marks an unknown
   `backend.submit` failure `indeterminate`, persists the admission block, and
   aborts only the child. A scripted backend records that it started and then
   raises; the test asserts indeterminate + blocked + no resubmission + blocked
   after restart. A poll failure while `running` is treated the same way. The
   new `BuildNotStartedError` seam is the only definite pre-start refusal.
3. **Restart distinction.** `_recover` (and `close`) marks a `running` job
   `indeterminate` with a typed error and blocks admission, while a `queued`
   job that was never submitted becomes `interrupted` and does not block.
4. **No mutating retry in the probe.**
   `Models.AddFiniteExtrudedProtrusion` is called exactly once with the
   documented tuple form; a rejection is recorded and reported as
   `not retried` rather than re-sent as a byref VARIANT. A fake that records
   the creation and raises `E_INVALIDARG` proves one call, no calibration, and
   owned-document cleanup.
5. **Serve cleanup and Windows gate.** `serve` no longer calls
   `server.shutdown()` from the serving thread; it closes the socket and
   manager after the loop. The public CLI refuses non-Windows with exit 2 in
   `main` before resolving the default data directory, and `serve` repeats the
   check for direct callers, before any directory or token is created. A mocked
   `KeyboardInterrupt` regression test proves cleanup returns and no
   same-thread shutdown occurs; separate tests prove the non-Windows refusal
   creates nothing, including when no explicit data directory is supplied.

## Checks and outcomes

Run from `adapters/solidedge-bridge` with the repository virtualenv
(`/home/v9i/Projects/Kala-Agent/.venv`, Python 3.14; mypy targets 3.11):

* `python -m pytest -q` — **186 passed, 1 skipped** (the skipped test is the
  gated live Windows attach test). The contract's 87 tests are included and
  still pass unchanged.
* `ruff check .` — **All checks passed**.
* `ruff format --check .` — **27 files already formatted**.
* `mypy` — **Success: no issues found in 27 source files** (strict).
* `coverage run -m pytest && coverage report` — **84.58%**, above the
  configured `fail_under=80`. Uncovered statements are almost entirely Windows
  COM paths (`session.py` gencache/typeinfo/VARIANT calls, `workspace`
  `icacls`), the long-running `serve` loop, and the `--launch` branch.

Offline smoke on this Linux host: `probe` wrote the diagnostic JSON and exited
2; `serve` refused the non-Windows host with exit 2 before creating a data
directory; a production manager with an injected test backend answered an
authenticated `GET /v1/health` with `supported=false`, `available=false`,
rejected an unauthenticated GET with 401, and rejected a valid
`POST /v1/builds` with 503 `unavailable`.

## Unimplemented pieces and honest gaps

* **No production native geometry executor.** The worker never creates a
  feature; builds fail with `unavailable` on every platform. The cube probe is
  diagnostic evidence only and is not a build executor.
* The cube probe is **unverified against a real Solid Edge/typelib**. Its
  cross-checks (type-library member confirmation, `igSymmetric`/`igProfileClosed`
  resolution, 13-parameter `ComputePhysicalProperties`, ISO-10303-21 header,
  10 mm volume/extent tolerances) all fail closed, but the
  `ComputePhysicalProperties` array lengths for global moments / principal axes
  are not recoverable from type information; the probe uses the SDK example
  sizes and reports the real error instead of assuming success.
* RefPlanes index→normal mapping, ordered-vs-synchronous template behavior,
  STEP translator flavour (`STEP.INI`), density/accuracy units, student-edition
  automation access, and `SaveCopyAs` STEP behaviour remain open; they are listed
  in the probe report's `missing_probe_work`.
* Windows-specific ACL tightening and the live COM session have no execution
  evidence on this host; neither test nor report claims them.
* The ledger records probe-owned documents with `KalaBuildId`-style probe/run
  identity and a closed event. Build-owned document tracking will be recorded
  by the future native executor, which does not exist yet.
* `main`/`serve` on Windows was not exercised; the Linux path was.

## Session

Transport task/child thread and IDs are recorded in `cells/bridge/SESSION.md`.
The OpenCode provider session ID was not reported by the transport.
