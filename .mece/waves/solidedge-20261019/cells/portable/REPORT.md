# Portable imports — implementation report

Scope: make `kala_agent`, `kala_agent.contracts`, `kala_agent.studio` and
`kala_agent.api.boundary` importable on a Windows-only target without loading
the Linux execution layer (application → tools/executor → tools/sandbox, whose
module-level `_load_libc()` calls `ctypes.CDLL` and fails on Windows).

No delegation, commit, push, publish, deploy, or package installation was
performed. No Windows execution is claimed; all verification below ran on
Linux, and the Windows CI smoke test is defined elsewhere and not run here.

## Changed paths

Originally owned:

- `src/kala_agent/__init__.py` — PEP 562 lazy `KalaApplication`.
- `src/kala_agent/api/__init__.py` — PEP 562 lazy re-exports for the whole
  existing `__all__`.
- `tests/unit/test_portable_imports.py` (new) — 9 regression tests.
- `.mece/waves/solidedge-20261019/cells/portable/REPORT.md` (this file).

Granted additionally by the conductor/user (narrow extraction; see "Extra
paths" below):

- `src/kala_agent/api/envelope.py` (new) — portable error envelope.
- `src/kala_agent/api/schemas.py` — re-export only; application-view models and
  imports unchanged.
- `src/kala_agent/api/middleware.py` — one import line now points at
  `api.envelope`.

No other path was edited. `api/boundary.py`, `api/app.py`,
`api/dependencies.py`, `api/errors.py`, `studio/**`, `config.py`,
`contracts/**`, `tools/**` and `application.py` are untouched.

## Extra paths and why they were required

`kala_agent.studio` → `studio.boundary` → `kala_agent.api.boundary` →
`kala_agent.api.middleware` → `kala_agent.api.schemas` →
`kala_agent.application` (plus `kala_agent.tools.protocol`). The two owned
`__init__` files cannot break that chain: `api.boundary` legitimately needs
`api.middleware`, and middleware only needed `error_response` from schemas.

The fix is an extraction, not a deferral:

- `ErrorDetail`, `ErrorResponse`, `error_response`, `MAX_ERROR_DETAILS` and
  `MAX_ERROR_MESSAGE_LENGTH` moved verbatim into `api/envelope.py` with the
  same `ConfigDict(extra="forbid", frozen=True)` base, the same bounding
  behaviour and the same JSON shape.
- `api/schemas.py` re-exports them, so `kala_agent.api.schemas.error_response`
  is the *same object* as `kala_agent.api.envelope.error_response`, and
  `schemas.__all__` still lists `ErrorDetail`, `ErrorResponse`,
  `error_response`.
- `api/middleware.py` imports `error_response` from `api.envelope`.
- `kala_agent.api.__init__` routes the lazy `error_response` export to
  `api.envelope`.
- `api/errors.py` still imports the envelope names through `api.schemas` and is
  unchanged.

No response-model build was deferred and no application import was moved under
`TYPE_CHECKING`, so `DomainListResponse`, `EvaluationResponse`,
`RevisionListResponse`, `CheckpointListResponse` and `RunResponse` are still
resolved eagerly by Pydantic exactly as before; normal API validation cannot
see an unresolved forward reference. Importing `kala_agent.api` eagerly is
still not possible — `create_app` and every transport class remain reachable
only through explicit access, which is the point of the lazy `__init__`.

## Import behaviour after the change

- `import kala_agent` loads only `kala_agent` and `kala_agent.config`.
- `import kala_agent.api` loads only `kala_agent`, `kala_agent.api` and
  `kala_agent.config`; all 43 `__all__` names resolve on demand.
- `import kala_agent.contracts`, `import kala_agent.api.boundary` and
  `import kala_agent.studio` load none of `kala_agent.application`,
  `kala_agent.tools*`, `kala_agent.runtime*`, and never call `ctypes.CDLL`.
- `from kala_agent import KalaApplication` still imports
  `kala_agent.application` (and therefore the tool plane) on first access, so
  Linux behaviour and identities are unchanged; `Settings` remains eager and
  identical to `kala_agent.config.Settings`.
- `from kala_agent.api import create_app, HostGuardMiddleware, error_response,
  HealthResponse, ...` resolves to the same objects as before;
  `api.__all__` is byte-identical to `HEAD` (checked programmatically).
- Unknown attributes raise `AttributeError` from both packages; `__dir__`
  advertises `__all__` plus resolved names.
- No sandbox policy was weakened or bypassed: the strict profile, the executor
  and `_load_libc()` are unchanged; they are simply not imported until a caller
  asks for the composition root.

## Regression tests

`tests/unit/test_portable_imports.py` (9 tests). Each import test runs in a
real `subprocess` with:

- `PYTHONPATH` pointing at `src`, `PYTHONDONTWRITEBYTECODE=1`;
- a `sys.meta_path` finder that raises `ImportError` for
  `kala_agent.application`, `kala_agent.tools`, `kala_agent.tools.*`,
  `kala_agent.runtime`, `kala_agent.runtime.*`;
- a patched `ctypes.CDLL` that raises `OSError` for any `libc*`/`libSystem*`
  name and records the attempt.

Tests:

1. `test_root_import_stays_portable` — root import, `Settings` module, no
   blocked module, zero libc attempts.
2. `test_contracts_and_studio_import_without_the_linux_execution_layer` —
   imports `kala_agent.contracts`, `kala_agent.api.boundary`,
   `kala_agent.studio`; builds `create_studio_app(data_dir=tmp, api_token=…)`;
   asserts health is 200/`ok`/`0.1.0`, a rejected Host is 421, an unauthenticated
   mutating POST is 401 (so the portable error paths, not just app creation,
   are exercised), the data dir is created, and no application/tools/runtime
   module or libc load occurred.
3. `test_api_boundary_import_does_not_reach_the_application` — boundary
   predicates work and stay portable.
4. `test_api_package_import_is_lazy_but_complete` — bare `import
   kala_agent.api` loads exactly the three expected modules.
5. `test_root_lazy_exports_resolve_to_the_real_objects` — identity vs
   `importlib.import_module`.
6. `test_root_dir_and_unknown_attribute_behaviour`.
7. `test_api_lazy_exports_match_their_defining_modules` — every `__all__` name
   checked against its defining module, including
   `error_response is envelope.error_response is schemas.error_response`.
8. `test_api_dir_and_unknown_attribute_behaviour`.
9. `test_settings_public_import_compatibility`.

The tests were mutation-checked, not just observed green:

- restoring the old eager `src/kala_agent/__init__.py` fails tests 1, 2, 3 and
  4;
- restoring `middleware.py`'s `from kala_agent.api.schemas import
  error_response` fails tests 2 and 3.

Both mutations were reverted; the suite is green again.

## Commands and actual results

All commands run from the repository root with the existing `.venv`
(Python 3.14.8, no installs).

| Command | Result |
| --- | --- |
| `.venv/bin/python -m pytest tests/unit/test_portable_imports.py -q` | `9 passed in 1.43s` |
| `.venv/bin/python -m pytest tests/unit/test_portable_imports.py tests/unit/test_api.py tests/unit/test_studio_server.py tests/unit/test_studio_errors.py tests/unit/test_contracts.py -q` | `108 passed, 1 warning in 2.95s` |
| `.venv/bin/python -m pytest tests -q` | `1460 passed, 1 warning in 20.89s` |
| `.venv/bin/ruff check src tests` | `All checks passed!` (exit 0) |
| `.venv/bin/ruff format --check src tests` | `117 files already formatted` (exit 0) |
| `.venv/bin/mypy src` | `Success: no issues found in 78 source files` |
| `.venv/bin/mypy --python-version 3.12 src tests/unit/test_portable_imports.py` | `Success: no issues found in 79 source files` |

Notes on the mypy results, reported honestly:

- The project-configured `mypy` (default `files = ["src", "tests"]`, strict)
  aborts in this environment before checking our files:
  `.venv/lib/python3.14/site-packages/numpy/__init__.pyi:737: error: Type
  statement is only supported in Python 3.12 and greater [syntax]`. That is a
  pre-existing environment/dependency mismatch (numpy's stubs vs the configured
  `python_version = "3.11"`), not caused by this change; it reproduces with
  `tests/unit/test_portable_imports.py` moved out of the tree.
- Under `--python-version 3.12` the whole `src` tree plus the new test file is
  clean. Full-repo strict mypy still reports 14 pre-existing errors confined to
  `tests/unit/test_studio_errors.py`, `tests/unit/test_studio_providers*.py`,
  `tests/unit/test_studio_server.py` and `tests/security/test_studio_security.py`
  (generator return types, `memoryview` items, unused ignores) — none in the
  files changed here.
- `pytest` emits one pre-existing Starlette deprecation warning about
  `httpx`/`httpx2` in `fastapi.testclient`.

## Remaining Windows import risks discovered (not fixed; outside ownership)

1. `src/kala_agent/api/boundary.py:175` calls `os.fchmod(handle.fileno(), …)`
   in `_store_api_token`. `os.fchmod` does not exist on Windows. Importing
   `kala_agent.api.boundary` is fine (the call is inside a function), but
   `issue_api_token()` — used by the core CLI/API launch path — would raise
   `AttributeError` on Windows. The Studio has its own portable token store and
   is unaffected. A port would need a Windows branch (e.g. `os.chmod` after
   close, or skip mode tightening), and that file is not owned by this cell.
2. `src/kala_agent/tools/sandbox.py` remains Linux-only by design
   (`os.uname`, `/proc`, `os.fork`, `libc` symbols). It is now only reachable
   through an explicit `KalaApplication` access, which is correct: the strict
   sandbox is unported and must not be silently replaced by a weaker profile.
3. `src/kala_agent/api/schemas.py` still imports `kala_agent.application` and
   `kala_agent.tools.protocol` at module import. That is intentional and
   unchanged: the transport schemas mirror application views, and `api.app` /
   `api.errors` / `api.dependencies` are not part of the portable Windows
   surface. The portable Studio path no longer touches this module.
4. Studio portability was spot-checked by reading: `studio/boundary.py` guards
   `os.chmod` behind `IS_WINDOWS`, writes tokens through
   `tempfile` + `os.replace`, and `studio/geometry/step.py` probes OCP by
   `__import__` inside a `try/except`, so a missing `cadquery-ocp` yields a 503
   rather than an import failure. No Linux-only `ctypes`/`/proc`/`fork` use
   exists under `studio/`.
5. Not verified: an actual Windows interpreter run, Windows path semantics for
   the data directory, and the Windows CI workflow itself. No claim is made
   that Windows execution has passed.

## Session

Transport: T3 OpenCode delegation. Model:
`opencode/deepseek-v4.1-flash-free`. No OpenCode provider session ID was
reported; `cells/portable/SESSION.md` holds the delegated task/child-thread
references.
