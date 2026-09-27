# Kala Agent Verification Report

| Field | Value |
|---|---|
| Document | Verification report |
| Date | 2026-09-26 |
| Subject | The repository at `/home/v9i/Projects/Kala-Agent` as it stood on 2026-09-26, comprising the root distribution and the separate optional adapter distribution at `adapters/freecad` |
| Result | **Root: 1,227 tests pass on both CPython 3.11.16 and CPython 3.14.7. `ruff check` and `ruff format --check` are clean. `mypy` strict is clean. Branch coverage 94.76% (3.11.16) / 94.77% (3.14.7) against a configured `fail_under = 90`. Adapter: 301 tests pass with 100.00% branch coverage against a configured `fail_under = 60`; `ruff check`, `ruff format --check`, and `mypy` strict are clean over 30 files.** |
| Related documents | `docs/architecture-report.md`, `docs/threat-model.md`, `docs/adr/0001`–`0007` |

This report records what was actually run and observed on this host. It does not claim that a
passing suite proves the absence of defects, that coverage equals correctness, or that the system
performs engineering validation. Section 7 lists what remains unproven. Section 6 records the
defects found during verification and states, per defect, whether it is now fixed or still open.

This report covers **two independently configured distributions**:

| Suite | Root directory | Distribution | Sections |
|---|---|---|---|
| Root | repository root | `kala-agent` | §1.1–§1.3, §2.1–§2.6, §3, §4, §5.1, §6–§9 |
| Optional FreeCAD adapter | `adapters/freecad` | `kala-agent-freecad` | §1.4, §2.7, §3.1, §4.14, §5.2 |

The adapter is **not** part of the product composition root, so nothing in the root suite's pass/fail
figure says anything about whether it works, and nothing in the adapter's figure says the product has
a CAD capability. See `docs/architecture-report.md` §3.2 and §10.7.

Every claim in this document is reproducible from the commands in section 2. Where a figure depends
on the interpreter, both figures are given.

## 1. Environment

### 1.1 Host

| Property | Value |
|---|---|
| Kernel | `Linux 7.2.5-3-omarchy x86_64` |
| Architecture | x86_64 |
| Filesystem under test | local, `O_NOFOLLOW` and hard links available |
| User namespaces | available (the strict sandbox probe succeeded; see section 5) |
| `scratch_tmpfs` | **not** verified by the probe on this host, and correctly not reported |

### 1.2 Interpreters and toolchain

Two interpreters were used, and the whole suite plus all three quality gates were run on **both**.
The 3.11.16 environment is the declared minimum; the 3.14.7 environment is the development
interpreter.

| Component | 3.11.16 environment | 3.14.7 environment |
|---|---|---|
| CPython | **3.11.16** | **3.14.7** |
| Interpreter path | `/tmp/opencode/kala311/bin/python` | `/tmp/opencode/kala-quality-venv/bin/python` |
| Venv created by | `python3.11 -m venv` | `python3 -m venv` |
| `pytest` | 9.1.1 | 9.1.1 |
| `coverage` | 7.16.1 (C extension) | 7.16.1 (C extension) |
| `ruff` | 0.16.9 | 0.16.9 |
| `mypy` | 2.3.1 (compiled) | 2.3.1 (compiled) |
| `fastapi` | 0.141.1 | 0.141.1 |
| `pydantic` | 2.13.5 | 2.13.5 |
| `starlette` | 1.7.0 | 1.7.0 |
| `httpx` | 0.28.1 | 0.28.1 |
| `uvicorn` (optional `serve` extra) | 0.54.0 | 0.54.0 |
| SQLite library | 3.53.1 | 3.53.4 |

The two environments differ only in the interpreter and in the bundled SQLite version. That is
visible in one coverage number and nothing else.

### 1.3 Declared configuration actually in force

Read from `pyproject.toml`, unmodified:

- `requires-python = ">=3.11"`
- runtime dependencies: `fastapi>=0.115`, `pydantic>=2.10,<3` — **nothing else**
- optional extras: `serve` (`uvicorn>=0.30`), `dev` (`coverage>=7.6`, `httpx>=0.27`, `mypy>=1.14`,
  `pytest>=8.3`, `pytest-asyncio>=0.24`, `ruff>=0.9`, `uvicorn>=0.30`)
- `[project.scripts]`: `kala-agent = "kala_agent.cli.main:main"`
- `[tool.setuptools.packages.find]`: `where = ["src"]`, `include = ["kala_agent*"]`
- `[tool.pytest.ini_options]`: `minversion = "8.3"`,
  `addopts = ["-ra", "--strict-config", "--strict-markers"]`, `testpaths = ["tests"]`,
  `pythonpath = ["src"]`, `asyncio_mode = "strict"`,
  `asyncio_default_fixture_loop_scope = "function"`, `xfail_strict = true`
- `[tool.coverage.run]`: `branch = true`, `source = ["kala_agent"]`
- `[tool.coverage.report]`: `show_missing = true`, `skip_covered = true`, `precision = 2`,
  **`fail_under = 90`**
- `[tool.ruff]`: `target-version = "py311"`, `line-length = 100`, `src = ["src", "tests"]`
- `[tool.ruff.lint] select`: `E4, E7, E9, F, I, B, UP, ANN, ASYNC, RUF`
- `[tool.ruff.lint.isort] known-first-party`: `["kala_agent"]`
- `[tool.mypy]`: `strict = true`, `python_version = "3.11"`, `files = ["src", "tests"]`,
  `warn_unreachable = true`, `show_error_codes = true`, `pretty = true`,
  `plugins = ["pydantic.mypy"]`

Two of these are load-bearing for this report and are called out here because they are easy to
overlook:

- **`fail_under = 90` is configured.** A coverage drop below 90% fails `coverage report` with a
  non-zero exit status. The 94.76% measured below clears it with 4.76 points of headroom. This is a
  configured gate, not an aspiration.
- **`python_version = "3.11"` under `[tool.mypy]`.** Type checking is pinned to the declared
  minimum regardless of which interpreter runs it, so the 3.14.7 run is a check against 3.11
  semantics, not a second opinion about 3.14 semantics.

There is **no `conftest.py`** anywhere in `tests/`. Fixtures are module-local.

### 1.4 Optional FreeCAD adapter: host facts and declared configuration

`adapters/freecad` is a **separate distribution** (`kala-agent-freecad`) with its own `pyproject.toml`,
its own `src` root, its own tests, and its own quality configuration. It is not installed into either
verification venv and is not imported by the core; the suite runs against the uninstalled tree through
`pythonpath = ["../../src", "src"]`, so the root `src` and the adapter `src` are both on the path.

#### 1.4.1 The FreeCAD installation, as found on this host

| Property | Value | How it was observed |
|---|---|---|
| Package | `freecad` 1.1.3-2, `x86_64` | `pacman -Qi freecad` |
| Licence | **LGPL-2.0-only** | `pacman -Qi freecad` |
| Console interpreter | `/usr/bin/freecadcmd` | `discover()`; `freecadcmd --version` prints `FreeCAD 1.1.3 Revision: 44987 (Git)` |
| GUI executable | `/usr/bin/freecad` | `discover()` |
| Import root | `/usr/lib/freecad/lib`, which holds `FreeCAD.so` | `discover()`; `is_import_root` module-marker test |
| Version read by the probe | `1.1.3`, parts `(1, 1, 3)` | `discover().to_payload()` |
| `detail` | `FreeCAD 1.1.3 is available for import` | `discover().to_payload()` |
| **CalculiX** | **not installed** — no `ccx`, no `CalculiX`; `pacman -Qi calculix` reports "not found" | `which`, `pacman -Qi`, `discover().calculix_path == ""` |
| **Gmsh** | **not installed** — `pacman -Qi gmsh` reports "not found" | `pacman -Qi` |
| `solver_available` | `False` | `discover().solver_available` |
| `femtools.ccxtools` | not importable in the verification interpreter — `ModuleNotFoundError: No module named 'PySide6'` | direct import with the discovered root on `sys.path` |
| Python wheels used | **none.** Neither `pyproject.toml` lists FreeCAD, CalculiX, or Gmsh | both `pyproject.toml` files |

The FreeCAD package pulls Qt 6, VTK, OpenCASCADE, Coin, OpenMPI, PySide6, and more. None of that is
a Python dependency of either distribution; it is a host concern.

#### 1.4.2 The bare-import fact

A bare `import FreeCAD` fails, and needs the discovered import root:

```console
$ $PY314 -c "import FreeCAD"
ModuleNotFoundError: No module named 'FreeCAD'

$ $PY314 -I -B -c "import sys; sys.path.insert(0, '/usr/lib/freecad/lib'); import FreeCAD, Part, ObjectsFem; …"
FreeCAD 1.1.3  Part True  ObjectsFem True
ccxtools unavailable: ModuleNotFoundError No module named 'PySide6'
```

This is exactly why the seam is `ToolExecutor(import_roots=…)` and not an environment variable: the
worker is started as `python -I -B -c <bootstrap>`, and `-I` ignores the host's import settings.

#### 1.4.3 Declared adapter configuration, read from `adapters/freecad/pyproject.toml`

- `name = "kala-agent-freecad"`, `version = "0.1.0"`, `requires-python = ">=3.11"`
- runtime dependencies: **`kala-agent>=0.1`, `pydantic>=2.10,<3`** — and nothing else. A comment in the
  file states that FreeCAD is a system dependency discovered on the host, never a wheel, never listed
  there, and imported only inside a worker process
- extras: `serve` (`uvicorn>=0.30`), `dev` (`coverage>=7.6`, `mypy>=1.14`, `pytest>=8.3`, `ruff>=0.9`,
  `uvicorn>=0.30`)
- `[tool.setuptools.packages.find]`: `where = ["src"]`, `include = ["kala_agent_freecad*"]`
- `[tool.pytest.ini_options]`: `minversion = "8.3"`,
  `addopts = ["-ra", "--strict-config", "--strict-markers"]`, `testpaths = ["tests"]`,
  **`pythonpath = ["../../src", "src"]`**, `xfail_strict = true`, and one marker:
  `freecad: exercises a real FreeCAD installation and is skipped when it is absent`
- `[tool.coverage.run]`: `branch = true`, `source = ["kala_agent_freecad"]`
- `[tool.coverage.report]`: `show_missing`, `skip_covered`, `precision = 2`, **`fail_under = 60`**
- `[tool.ruff]`: `target-version = "py311"`, `line-length = 100`, `src = ["src", "tests"]`; lint select
  is the same `E4, E7, E9, F, I, B, UP, ANN, ASYNC, RUF`; `known-first-party` is
  `["kala_agent", "kala_agent_freecad"]`
- `[tool.mypy]`: `strict = true`, `python_version = "3.11"`, `files = ["src", "tests"]`,
  **`mypy_path = ["../../src", "src"]`**, `warn_unreachable = true`, the Pydantic plugin

Two of these are load-bearing for this report:

- **`fail_under = 60` is configured for the adapter and the measured 100.00% clears it by 40 points.**
  It is a low floor because a 100% measurement leaves no need for a high one, and it is still a
  configured gate, not an aspiration.
- **`pythonpath` and `mypy_path` both include `../../src`.** The adapter is type-checked and tested
  against the working tree of the kernel, not against an installed copy, so a change to a kernel type
  that the adapter depends on fails the adapter's gates immediately.


## 2. Exact commands and results

All commands were run from the repository root. `$PY311` and `$PY314` are the two verification
interpreters from section 1.2.

### 2.1 Test suite

```console
$ $PY311 -m pytest -q
1227 passed, 1 warning in 15.29s

$ $PY314 -m pytest -q
1227 passed, 1 warning in 15.61s
```

**Result: 1,227 passed, 0 failed, 0 skipped, 0 deselected, 0 xfailed, 1 warning, on both
interpreters.** The single warning originates in the `fastapi`/`starlette` test client, not in this
repository:

```text
../../../../tmp/opencode/kala311/lib/python3.11/site-packages/fastapi/testclient.py:1
  fastapi/testclient.py:1: StarletteDeprecationWarning: Using `httpx` with
  `starlette.testclient` is deprecated; install `httpx2` instead.
    from starlette.testclient import TestClient as TestClient  # noqa
```

`xfail_strict = true` is in force, so no xfail can be silently passing as an unexpected success.

The two tests added since the previous pass of this report are
`tests/unit/test_adapter_boundaries.py::test_core_modules_do_not_import_freecad_or_the_adapter` and
`::test_freecad_adapter_does_not_import_core_application_layers`. They are the only place the root
suite knows the adapter exists, and they assert the **absence** of an import in both directions.

### 2.2 Branch coverage

```console
$ COVERAGE_FILE=cov311.data $PY311 -m coverage erase
$ COVERAGE_FILE=cov311.data $PY311 -m coverage run -m pytest
$ COVERAGE_FILE=cov311.data $PY311 -m coverage report

$ COVERAGE_FILE=cov314.data $PY314 -m coverage erase
$ COVERAGE_FILE=cov314.data $PY314 -m coverage run -m pytest
$ COVERAGE_FILE=cov314.data $PY314 -m coverage report
```

`coverage run` completes on both interpreters and writes its data file. The earlier defect in which
`coverage run -m pytest` died with `CPU time limit exceeded` and produced no data file is fixed; see
section 6, defect R2.

Totals:

```text
# CPython 3.11.16
TOTAL                                             11077    447   3238    285  94.76%

# CPython 3.14.7
TOTAL                                             11077    446   3238    285  94.77%
```

| Metric | 3.11.16 | 3.14.7 |
|---|---|---|
| Statements | 11 077 | 11 077 |
| Missed statements | 447 | 446 |
| Branches | 3 238 | 3 238 |
| Partial branches | 285 | 285 |
| **Total coverage (statements + branches)** | **94.76%** | **94.77%** |
| Configured gate | `fail_under = 90` — passed | `fail_under = 90` — passed |
| Files at 100% (excluded from the table by `skip_covered`) | 21 | 21 |

**On the exact figure, and its reproducibility.** The headline number for this report is **94.76%**,
the CPython 3.11.16 measurement, because 3.11 is the declared minimum. It is **one observation, not a
bit-reproducible constant.** Across the repeated runs performed for this report, the same commands on
this host produced:

| Run | Interpreter | TOTAL |
|---|---|---|
| 1 | 3.11.16 | **94.76%** |
| 2 | 3.14.7 | 94.77% |
| 3 | 3.11.16 | 94.74% |
| 4 | 3.14.7 | 94.80% |

So the honest reading is **≈94.8% ± 0.03 percentage points**, against a configured floor of 90%. Every
observed value clears that floor with more than 4.7 points of headroom, and no observed run has come
close to it. The spread is not a measurement artifact and not a disagreement between interpreters: it
comes from a handful of subprocess- and sandbox-timing-dependent tests that take slightly different
code paths depending on how the host is loaded, which moves 3–5 statements in and out of the executed
set. The two interpreters carry identical statement and branch totals (11 077 and 3 238); only the
covered subset moves.

This report quotes 94.76% because that is the figure the commands in this section produced, and it
gives the range alongside it rather than claiming a precision the measurement does not have. A reader
re-running these commands should expect 94.74–94.80% and should treat anything below 90% as a real
regression.

Per-file coverage on 3.11.16, lowest first. This is the honest list of what is **not** exercised:

| File | Cover | Why the gap exists |
|---|---|---|
| `cli/__main__.py` | 0.00% | 3 statements; the `python -m kala_agent.cli` shim is exercised by `tests/integration/test_mechanical_loop.py:325` in a subprocess, which coverage does not trace. |
| `api/errors.py` | 77.42% | Handler branches for framework-raised exceptions the suite does not provoke. |
| `kernel/budget.py` | 86.00% | Arithmetic-error paths in the numeric coercion helpers, and `is_exhausted(dimension=…)` with a string. |
| `config.py` | 87.22% | `OSError`/`RuntimeError` paths in `data_dir` resolution and a few coercion branches. |
| `persistence/journal.py` | 88.10% | `TypeError`/`ValueError` guards on `get`/`read` argument types. |
| `contracts/models.py` | 88.31% | Idna/percent-escape/port-parsing error branches in `_normalize_artifact_uri`, and cross-field validator messages. |
| `application.py` | 89.49% | 79 statements across view builders, defensive re-derivation, and event-payload assembly. This is the largest single coverage gap and it is in the composition root. |
| `persistence/database.py` | 89.78% | `TypeError`/`ValueError` guards on constructor arguments, the `chmod` failure guards, and timestamp parsing. |
| `api/dependencies.py` | 90.91% | The two `get_application` refusal lines for a missing or closed application. |
| `kernel/policy.py` | 90.16% | Constructor-validation and eviction-order branches. |
| `persistence/repositories.py` | 92.66% | `TypeError` guards on repository argument types. |
| `api/schemas.py` | 92.70% | Parameter-count and key-length rejections above the tested bounds. |
| `api/middleware.py` | 91.11% | Header-decode and `_declared_length` edge cases. |
| `tools/executor.py` | 93.91% | `OSError` branches in `Popen`, `_reap` retry, and `_send_request`. |
| `cli/main.py` | 95.83% | `_parse_parameter` errors and the `__main__` guard. |

Best-covered files, included because high coverage on security-relevant code is evidence worth
recording: `tools/sandbox.py` 99.13%, `kernel/planning.py` 99.37%, `runtime/agent.py` 99.43%,
`tools/protocol.py` 97.58%, `kernel/memory.py` 98.68%, `persistence/artifacts.py` 96.35%,
`persistence/chain.py` 96.43%, `kernel/workflow.py` 97.76%, `runtime/model_gateway.py` 97.42%.

### 2.3 Lint

```console
$ $PY311 -m ruff check src tests
All checks passed!
$ echo $?
0
$ $PY311 -m ruff format --check src tests
80 files already formatted
$ echo $?
0

$ $PY314 -m ruff check src tests
All checks passed!
$ $PY314 -m ruff format --check src tests
80 files already formatted
```

80 Python files are in scope. `target-version = "py311"` means the 3.14 run applies the same 3.11
upgrade and syntax rules as the 3.11 run; the two are expected to agree, and they do.

### 2.4 Strict type checking

```console
$ $PY311 -m mypy
Success: no issues found in 80 source files
$ echo $?
0

$ $PY314 -m mypy
Success: no issues found in 80 source files
$ echo $?
0
```

`mypy` runs in `strict = true` over **both** `src` and `tests`, with `warn_unreachable = true` and
the Pydantic plugin enabled, and with `python_version = "3.11"`. 80 files, zero errors, zero notes,
on both interpreters.

### 2.5 Manual end-to-end exercise

Run against a throwaway data directory to confirm the shipped surfaces behave as documented. The
loop below is the CPython 3.11.16 environment, driven through the installed `kala-agent` console
script rather than through `python -m`, so it also exercises the `[project.scripts]` entry point and
the `src` import root on the declared minimum interpreter.

```console
$ export KALA_AGENT_DATA_DIR=/tmp/kala-verify
$ $PY311/bin/kala-agent init
{ "status": "ready", "data": { "created": true, "schema_version": 2, "offline": true,
  "egress_configured": false, "provider": "local", "model": "local",
  "provider_endpoint_declared": false,
  "domains": ["building","mechanical","wing"],
  "tools": ["mechanical.reference-simulator"], "worker_count": 4, ... } }

$ $PY311/bin/kala-agent evaluate --domain mechanical
{ "status": "passed", "data": { "completed_step_ids": ["mechanical.geometry-check",
  "mechanical.material-check","mechanical.load-check","mechanical.constraint-evaluation"],
  "disclaimer": "This package performs arithmetic surrogates only. ...", ... } }

$ $PY311/bin/kala-agent run-mechanical
{ "status": "waiting_for_approval", "data": { "workflow_id": "workflow-mechanical-reference",
  "state": { "status": "waiting_for_approval" },
  "checkpoint": { "checkpoint_type": "pre_commit", "status": "pending",
    "state_digest": "a95257fe1fe6a94c2c9044f594d89c6c8f9aea1599ab9bd15a53fcb982a520ef",
    "action_digest": "eb907b6bdcea065513fbfdb3924a9e35b44dc7f0a0f85eadea81fe80388173fb" },
  "revision": null, ... } }

$ $PY311/bin/kala-agent approve --workflow-id workflow-mechanical-reference
{ "status": "completed", "data": { "state": { "status": "completed" },
  "revision": { "revision": { "revision_id": "rev-mechanical-reference-r0" },
    "provenance": { "provenance_id": "prov-mechanical-reference-r0",
      "domain_package_digests": ["cfa7d142..."],
      "transformation_ids": ["mechanical-reference-workflow","reference-workflow-evaluation"],
      "model_transcript_digest": null, "tool_call_ids": [], ... } },
  "checkpoint": { "status": "approved", "actor_id": "local-user" } } }

$ $PY311/bin/kala-agent events --limit 50
20 events: domain.evaluated, plan.created, workflow.started, workflow.created,
  4 x workflow.step_evaluated, workflow.status_changed, 3 x workflow.transition,
  workflow.checkpoint, policy.checkpoint_requested, workflow.approval_resolved,
  workflow.transition, workflow.checkpoint_decided, revision.prepared,
  revision.commit_decided, revision.created
```

`model_transcript_digest: null` and `tool_call_ids: []` are correct and expected. The reference path
uses `OfflinePlanner` and calls each domain evaluator in-process; see section 6, gap G5.

File modes on the created data directory, observed with `stat -c "%a %n"`:

```text
700 /tmp/kala-verify
600 /tmp/kala-verify/kala.sqlite3
700 /tmp/kala-verify/artifacts
700 /tmp/kala-verify/runs
```

This is the fix for defect R3: the data directory and the database file are created privately rather
than inheriting the process umask.

The two-phase revision path was exercised independently:

```console
$ $PY311/bin/kala-agent prepare-revision --design-id demo-1 --domain wing --summary demo
{ "status": "pending", "data": { "request_id": "commit-demo-1-…",
  "state_digest": "25014c10…", "action_digest": "bfc1615a…",
  "package_digests": ["…"], "artifact_digests": ["…"], "spec_digest": "…",
  "parameters_digest": "…", "sequence": 0, "parent_revision_id": null, ... } }

$ $PY311/bin/kala-agent commit-revision --request-id commit-demo-1-… \
    --decision approve --state-digest 25014c10… --action-digest bfc1615a…
{ "status": "approved", "data": { "revision": { "revision": {
    "revision_id": "rev-demo-1-r0", "sequence": 0 } },
  "commit": { "status": "approved", "committed": true } } }
```

### 2.6 CLI surface

The parser exposes twelve subcommands. Verified by introspecting the built parser on 3.11.16:

```text
init, domains, token, serve, evaluate, run-mechanical, workflow-status,
approve, reject, prepare-revision, commit-revision, events
```

`serve` is the twelfth. It is described in section 4.1 and its loopback-only enforcement is
`tests/unit/test_cli.py:99` and `tests/security/test_api_boundary.py:436`.

**No CAD command exists.** The parser exposes no FreeCAD availability probe, no tool dispatch, and no
adapter registration. This is a scope statement about the product, verified by introspecting the
built parser: `kala-agent init` reports `tools: ["mechanical.reference-simulator"]` and nothing else,
because `KalaApplication` does not import `kala_agent_freecad`. See
`docs/architecture-report.md` §3.2, §10.7, and §15.

### 2.7 The optional FreeCAD adapter suite

All commands were run from `adapters/freecad`. The adapter distribution is not installed into either
verification venv; `pythonpath = ["../../src", "src"]` puts the working tree of both distributions on
the path.

#### 2.7.1 Test suite

```console
$ cd adapters/freecad
$ $PY314 -m pytest -q
301 passed in 2.39s
```

**Result: 301 passed, 0 failed, 0 skipped, 0 deselected, 0 xfailed, 0 warnings.** Nothing is skipped
because the host *does* have FreeCAD 1.1.3, and 284 of the 301 tests do not need it at all — they cover
the absent-dependency paths, the argument readers, the contracts, the registration surface, the CAD
profile, the worker envelopes, and the error mapping with fakes.

The 17 tests that do need an installation are selected by the `freecad` marker and were run explicitly
to confirm they are not vacuous:

```console
$ $PY314 -m pytest -q -m freecad -v
collected 301 items / 284 deselected / 17 selected
tests/test_cad_live.py ............                                      [ 70%]
tests/test_executor_live.py .....                                        [100%]
17 passed, 284 deselected in 1.76s
```

#### 2.7.2 Branch coverage

```console
$ COVERAGE_FILE=/tmp/opencode/cov-fcadapter.data $PY314 -m coverage erase
$ COVERAGE_FILE=/tmp/opencode/cov-fcadapter.data $PY314 -m coverage run -m pytest -q
$ COVERAGE_FILE=/tmp/opencode/cov-fcadapter.data $PY314 -m coverage report
Name    Stmts   Miss Branch BrPart    Cover   Missing
-----------------------------------------------------
TOTAL    1152      0    306      0  100.00%

13 files skipped due to complete coverage.
```

| Metric | Value |
|---|---|
| Files measured | 13 (all of `src/kala_agent_freecad`) |
| Statements | 1 152 |
| **Missed statements** | **0** |
| Branches | 306 |
| **Partial branches** | **0** |
| **Total coverage (statements + branches)** | **100.00%** |
| Configured gate | `fail_under = 60` — passed |
| Files at 100% (excluded from the table by `skip_covered`) | 13 — i.e. every file |

**On this figure, honestly.** 100.00% is a real, reproducible measurement of *line and branch
execution*, and unlike the root figure it did not move between runs. It is **not** a claim of
correctness. The `pragma: no cover` markers the root suite already uses are present in the adapter too,
and they mark defensive branches rather than tested ones. There is no mutation testing, so nothing
confirms that deleting a control would fail a test. The claim this report makes is narrow: **every
statement and every branch in the adapter executes in this suite**, and the suite includes 17 tests
that drive a real FreeCAD installation out of process through the kernel's own executor.

#### 2.7.3 Lint

```console
$ $PY314 -m ruff check src tests
All checks passed!
$ $PY314 -m ruff format --check src tests
30 files already formatted
```

#### 2.7.4 Strict type checking

```console
$ $PY314 -m mypy
Success: no issues found in 30 source files
```

`mypy` runs `strict = true` over both `src` and `tests` with `mypy_path = ["../../src", "src"]` and
`python_version = "3.11"`, so the adapter is checked against the kernel's working tree and against 3.11
semantics. 30 files, zero errors.

#### 2.7.5 What the adapter suite does **not** cover

- The `completed`, `prerequisites_unmet`, and `failed` solver branches are exercised only against
  fakes (`test_cad_fem_fakes.py`), because no CalculiX exists on this host. The live path proves
  `solver_unavailable` and `not_requested`; it does not prove a real solve.
- `mesh.generated is True` and a non-null `results` block are therefore **never observed live**.
- The **strict** CAD profile is verified as *enforced or refused*, not as *enforced and successful*,
  because the executor's own probe is unavailable on this host. See §5.2.
- The adapter suite was run on CPython 3.14.7 only. The root suite was run on both interpreters; the
  adapter was not repeated on 3.11.16, and this report does not claim otherwise.
- No FreeCAD version matrix was exercised. Only 1.1.3 was present, and nothing pins a version.
- No Windows or macOS host was available, so `FreeCAD.dylib` discovery and the non-Linux path are
  covered by fakes only.

## 3. Test suite composition

1,227 tests across 28 files. No test is skipped, deselected, or xfailed.

| Area | Tests | Files |
|---|---|---|
| `tests/unit/` | 1 057 | 18 |
| `tests/security/` | 158 | 9 |
| `tests/integration/` | 12 | 1 |

Largest files, which show where the effort went:

| File | Tests |
|---|---|
| `tests/unit/test_kernel_workflow_branches.py` | 141 |
| `tests/unit/test_protocol_branches.py` | 139 |
| `tests/unit/test_executor_branches.py` | 119 |
| `tests/unit/test_sandbox_branches.py` | 104 |
| `tests/unit/test_kernel_memory_planning.py` | 96 |
| `tests/unit/test_agent_branches.py` | 78 |
| `tests/unit/test_model_gateway_branches.py` | 76 |
| `tests/unit/test_artifacts_branches.py` | 52 |
| `tests/unit/test_simulator_branches.py` | 51 |
| `tests/unit/test_registry_branches.py` | 49 |
| `tests/security/test_tool_executor.py` | 44 |
| `tests/unit/test_runtime.py` | 36 |
| `tests/unit/test_api.py` | 29 |
| `tests/unit/test_contracts.py` | 23 |
| `tests/security/test_revision_commit.py` | 23 |
| `tests/security/test_policy.py` | 20 |
| `tests/unit/test_cli.py` | 19 |
| `tests/security/test_journal_integrity.py` | 19 |
| `tests/unit/test_persistence.py` | 16 |
| `tests/unit/test_domains.py` | 16 |
| `tests/security/test_api_boundary.py` | 15 |
| `tests/security/test_model_residency.py` | 14 |
| `tests/integration/test_mechanical_loop.py` | 12 |
| `tests/unit/test_kernel.py` | 11 |
| `tests/security/test_schema_redos.py` | 9 |
| `tests/security/test_domain_integrity.py` | 7 |
| `tests/security/test_artifact_concurrency.py` | 7 |

The two security files added since the previous verification pass,
`tests/security/test_schema_redos.py` and `tests/security/test_domain_integrity.py`, are the direct
test evidence for the strict-regex-admission and evaluator-digest remediations described in
section 4. The one file added in this pass, `tests/unit/test_adapter_boundaries.py` (2 tests), is
different in kind: it asserts the **absence** of imports, in both directions, between the core and the
optional adapter.

### 3.1 Optional FreeCAD adapter suite composition

301 tests across 15 files, in the separate `adapters/freecad` distribution. Nothing is skipped,
deselected, or xfailed on this host. 17 of the 301 are marked `freecad` and need an installation;
284 do not.

| File | Tests | What it covers |
|---|---|---|
| `tests/test_contracts.py` | 44 | Tool ids, versions, capabilities, side effects, timeouts, schema digests, descriptor validity, the export suffix and media-type tables, `SOLVER_STATUSES` |
| `tests/test_cad_units.py` | 37 | Every scalar argument reader's bounds, defaults, and refusals |
| `tests/test_worker.py` | 33 | Worker envelope construction, argument re-validation, workspace absoluteness, result validation, error-code mapping |
| `tests/test_cad_fem_fakes.py` | 32 | Analysis construction, the three solver-gate branches, prerequisite refusal, result extraction with fakes, export and republication |
| `tests/test_availability.py` | 31 | Executable lookup, candidate import roots, module-marker test, the isolated version probe, `is_safe_calculix`, `resolve_calculix`, `discover` |
| `tests/test_availability_records.py` | 28 | `FreeCADAvailability` field bounds, `parse_version`, `parse_version_payload`, `unavailable`, `to_payload` |
| `tests/test_registration.py` | 17 | `register`, `register_if_available`, `freecad_import_roots`, `worker_import_roots`, `tool_identifiers`, type refusals |
| `tests/test_cad_runtime_loader.py` | 16 | `freecad_session` root handling, the fixed import order with fakes, the optional solver bridge, fd-1 redirection, document cleanup |
| `tests/test_cad_geometry_fakes.py` | 18 | Shape construction, measurement, export, refusal paths with fakes |
| `tests/test_worker_envelopes.py` | 12 | Envelope identity, status, and error paths |
| **`tests/test_cad_live.py`** | **12** | **A real FreeCAD 1.1.3**: measurement values, cylinder volume, workspace confinement, artifact budget, FEM document construction, `solver_unavailable` honesty, `not_requested`, document cleanup, the observed import order in a real subprocess |
| `tests/test_policy.py` | 9 | `cad_profile` derivation, budgets, no granted authority, strict-by-default, non-mutation of the shipped profile, `cad_report`, the fail-closed judgement |
| **`tests/test_executor_live.py`** | **5** | **A real FreeCAD 1.1.3 through `ToolExecutor`**: both tools register and dispatch out of process, artifacts re-verify, invalid arguments refused, ungranted capability denied, the CAD profile budget reaches the executor, and the strict profile is enforced or refused |
| `tests/test_cad_artifacts_limits.py` | 5 | Workspace resolution, symlinked directory and destination refusal, budget ceiling, digest and re-measure |
| `tests/test_registration_roots.py` | 2 | A non-directory root is dropped; a duplicate root is collapsed |

The 17 `freecad`-marked tests are the 12 in `test_cad_live.py` and the 5 in `test_executor_live.py`.
They are the only evidence in either suite that this adapter talks to a real native engineering kernel.

## 4. Security controls and where they are verified

Every row names the test that would fail if the control regressed. Line numbers are from this
checkout.

### 4.1 HTTP request boundary

| Control | Implementation | Test evidence |
|---|---|---|
| Serve only loopback `Host`; refuse a rebound name before any body, route, or application access | `api/boundary.py` `HostGuardMiddleware`, `is_permitted_authority` | `tests/security/test_api_boundary.py:93` |
| Refuse more than one `Host` header | `header_values(scope, b"host")`, `len(hosts) != 1` | `test_api_boundary.py:128` |
| Refuse a request with no `Host` header | same | `test_api_boundary.py:139` |
| Refuse every wildcard authority, explicitly rather than by accident | `WILDCARD_AUTHORITIES` | `test_api_boundary.py:283` |
| Bearer token required on `POST`/`PUT`/`PATCH`/`DELETE`; constant-time compare | `BearerTokenMiddleware`, `secrets.compare_digest` | `test_api_boundary.py:147,224` |
| A valid token authorizes every mutating endpoint, including routes added after launch | middleware sits above routing | `test_api_boundary.py:194,211` |
| Every mutating method is judged, not a hand-listed subset | `MUTATING_METHODS` | `test_api_boundary.py:224` |
| Refusal carries no hint about the expected token | `TOKEN_REQUIRED_MESSAGE`, `AUTH_CHALLENGE` | `test_api_boundary.py:147` |
| Body-size limit answers **every** method, including those with no payload | `MaxBodySizeMiddleware`, declared length then buffered replay | `test_api_boundary.py:243` |
| A chunked request with no declared `content-length` cannot bypass the limit | buffered replay, no dependency on the header | `tests/unit/test_api.py:629` |
| Any `Origin` header is refused; no CORS grant exists to anyone | `CrossOriginGuardMiddleware` | `test_api_boundary.py:310` |
| Token stored `0o600` with `O_NOFOLLOW`, rotated per launch, never returned | `issue_api_token`, `_store_api_token` | `test_api_boundary.py:340` |
| A missing, unreadable, over-long, or malformed token store reads as absent | `read_api_token` | `test_api_boundary.py:366` |
| Token and its store are never logged | `test_api_boundary.py:396`, `LOGGER.warning` with type only | `test_api_boundary.py:396` |
| An unstorable token still guards the process, in memory only | `issue_api_token` catches `OSError` | `test_api_boundary.py:420` |
| **A real `serve` launch keeps the same `Host` and token boundary** | `cli/main.py` `build_serve_target`, `build_serve_plan`; the app is the unmodified `create_app` app | `test_api_boundary.py:436`, `tests/unit/test_cli.py:138` |
| **`serve` binds loopback only; a wildcard or routable address is refused** | `is_loopback_host`, `SERVE_HOST = "127.0.0.1"` | `tests/unit/test_cli.py:99` |
| **`serve` never prints the launch token to stdout** | the token is written to `<data_dir>/api-token` and read back with the `token` command | `tests/unit/test_cli.py:210` |
| Security headers on every response, including transport refusals; `server` and `x-powered-by` stripped | `api/middleware.py` `SecurityHeadersMiddleware`, `send_refusal` | `tests/unit/test_api.py:76,800` |

### 4.2 Errors never leak

| Control | Implementation | Test evidence |
|---|---|---|
| No traceback, no exception chain, no submitted value, no credential text in any error | `api/errors.py` `error_response`, bounded message and details | `tests/unit/test_api.py:652`, `tests/integration/test_mechanical_loop.py:388` |
| Only the exception **type** is logged for an unexpected failure | `handle_unexpected_error` | `tests/unit/test_api.py:815` |
| An unexpected exception fails closed rather than returning a partial success | `handle_unexpected_error` | `tests/unit/test_api.py:815` |
| The CLI prints a configuration error as field:validation-type only | `_safe_configuration_message` | `tests/unit/test_cli.py` |
| A reserved workflow identifier is refused over HTTP as well as in-process | `_require_workflow_identifier`, `ReservedIdentifierError` | `tests/unit/test_api.py:780` |

### 4.3 Health and egress reporting truth

`/health` and `kala-agent init` report facts about the client the process actually holds, never
about a configuration value or an intent.

| Control | Implementation | Test evidence |
|---|---|---|
| `egress_configured` is `False` for the default local composition root | `KalaApplication.egress_configured` | `tests/unit/test_api.py:86` |
| A supplied gateway bound to a hosted client reports `egress_configured: true` | same property, client-derived | `tests/unit/test_api.py:113,125,133` |
| The local client never reports a provider path that is not wired | same property | `tests/unit/test_api.py:97,142` |
| `provider_endpoint_declared` is reported, so an operator can see that no endpoint was declared | `init` payload, `/health` payload | `tests/unit/test_api.py:76`, `tests/unit/test_cli.py:640` |
| The served launch reports the same truth over a real ASGI app | `build_serve_plan` → `create_app` | `tests/unit/test_cli.py:175` |
| No endpoint, key, or model URL is ever returned | `/health` shape | `tests/unit/test_api.py:76` |

### 4.4 Journal integrity

| Control | Implementation | Test evidence |
|---|---|---|
| `UPDATE` and `DELETE` on `events` abort at the database, even when the triggers are dropped the way a file-level attacker would | `persistence/database.py` `events_prevent_update` / `events_prevent_delete` triggers | `tests/security/test_journal_integrity.py` — written from the position of an attacker who already has direct write access to the database file |
| `INSERT` without a 64-char lowercase chain digest aborts | `events_require_event_chain` trigger | `test_journal_integrity.py` |
| Every event commits to its predecessor | `persistence/chain.py` `chain_digest` | `test_journal_integrity.py:130` |
| Every covered field is bound by the chain | `EventChainFields` | `test_journal_integrity.py:159` |
| An omitted event is detected | `verify_chain` sequence continuity | `test_journal_integrity.py:177` |
| A reordered history is detected | predecessor-linkage check | `test_journal_integrity.py:187` |
| Appending to a tampered journal fails closed | `_verified_tail` before insert | `test_journal_integrity.py:223` |
| Startup refuses a journal tampered while closed | `EventJournal.__init__` calls `verify_chain` | `test_journal_integrity.py:241` |
| A wholesale rewrite yields a **different** head digest | `ChainVerification.head_event_digest` | `test_journal_integrity.py:256` |
| Migration refuses to chain a corrupt or gapped journal | `backfill_event_chain` | `test_journal_integrity.py:312` |
| An empty journal verifies without a head rather than raising | `verify_chain` on genesis | `test_journal_integrity.py:146` |

The chain is tamper-**evident**. There is no external anchor for the head digest; see gap G4.

### 4.5 Artifact store and private on-disk modes

| Control | Implementation | Test evidence |
|---|---|---|
| Real multi-threaded stress publishes every object exactly once | temp file + fsync + `os.link` | `tests/security/test_artifact_concurrency.py:177` |
| Real multi-process stress publishes every object exactly once | same, with no inter-process coordination | `test_artifact_concurrency.py:207` |
| Bucket creation survives a lost creation race | `mkdir` tolerating `FileExistsError` | `test_artifact_concurrency.py:260` |
| A reader never observes a partially published object | publication is a hard link of a fully written file | `test_artifact_concurrency.py:287` |
| A symlinked bucket never redirects a concurrent write | `O_NOFOLLOW` plus `(st_dev, st_ino)` identity re-check | `test_artifact_concurrency.py:341` |
| A symlinked object is never read as a stored artifact | `O_NOFOLLOW` on the object open | `test_artifact_concurrency.py:418` |
| Artifact publication never blocks on the database writer lock | store and database take different locks | `test_artifact_concurrency.py:482` |
| **Data directory is created `0o700` and the database file `0o600`, regardless of umask** | `Database.__init__` `mkdir(..., mode=0o700)` then explicit `os.chmod` on the directory and the connection target | `tests/unit/test_persistence.py:190` |
| Artifact store directories `0o700`, objects `0o600` | `_DIRECTORY_MODE`, `_OBJECT_MODE` | `tests/unit/test_artifacts_branches.py:298` |
| Tool run directory is `0o700` | executor run-directory creation | `tests/unit/test_executor_branches.py:1895` |
| Every read verifies size and SHA-256 | `_read_object` | `test_artifact_concurrency.py`, `tests/security/test_tool_executor.py:1094` |
| A failed publication leaves no partial object | temp file is not linked into place | `tests/unit/test_persistence.py:413` |

The `chmod` calls are explicit rather than relying on `mkdir(mode=…)` alone, because `mkdir`'s mode is
masked by the process umask; the `os.chmod` after creation is what makes `0o700`/`0o600` hold on a
host with a permissive umask. This is the fix for defect R3.

### 4.6 Revision commit

| Control | Implementation | Test evidence |
|---|---|---|
| A prepared commit is not a revision and needs an approval | `application.py` `prepare_design_revision` writes no revision row | `tests/security/test_revision_commit.py:102` |
| The state digest covers head, parent, spec, package digests, artifact digests | `_derive_commit` | `test_revision_commit.py:138` |
| A stale approval writes nothing | digest recomputation at commit time | `test_revision_commit.py:183` |
| A rejection writes nothing and cannot be reused | `decide_revision_commit` | `test_revision_commit.py:219` |
| A revoked capability prevents the write even with matching digests | `PolicyEngine.revoke` | `test_revision_commit.py:254` |
| A commit cannot fork design history | `parent_revision_id` must be the head | `test_revision_commit.py:268` |
| The head CAS is transactional and leaves no orphan row | `commit_revision` inside `BEGIN IMMEDIATE` | `test_revision_commit.py:296` |
| A lost race reports a head-moved conflict and writes no row | `HeadMovedError` | `test_revision_commit.py:325` |
| Two concurrent commits produce exactly one revision | real threads | `test_revision_commit.py:371` |
| A workflow revision needs its own approved checkpoint | `_require_stored_decision` | `test_revision_commit.py:410` |
| A rejected workflow leaves no revision and no authorization | `reject_workflow_checkpoint` | `test_revision_commit.py:455` |
| **A completed workflow cannot authorize another design's commit** | request-to-workflow binding | `test_revision_commit.py:475` |
| **A completed workflow with no stored approval authorizes nothing** | `_require_stored_decision` | `test_revision_commit.py:487` |

### 4.7 Restart restore and reserved kernel-state identifiers

| Control | Implementation | Test evidence |
|---|---|---|
| Policy approvals and budget usage survive a restart | `_restore_kernel_state` | `test_revision_commit.py:515` |
| A revoked capability stays revoked across a restart | policy state round-trip | `test_revision_commit.py:563` |
| A tampered kernel-state record stops the launch | `CorruptKernelStateError` | `test_revision_commit.py:581` |
| A kernel-state record that does not validate stops the launch | strict `import_state` | `test_revision_commit.py:608` |
| A clean restart restores usage, grants, and approvals | full round-trip | `test_revision_commit.py:638` |
| **A user-supplied `workflow_id` may not name a reserved kernel-state aggregate** | `_require_workflow_identifier`, `RESERVED_WORKFLOW_IDS`, `KERNEL_STATE_NAMESPACE` | `test_revision_commit.py:676` |
| **Kernel state is stored under the reserved `kernel-state:` namespace** | `kernel_state_aggregate` | `test_revision_commit.py:709` |
| **A legacy bare-aggregate record from an earlier version is still restored** | `RESERVED_WORKFLOW_IDS` keeps the bare names reserved | `test_revision_commit.py:735` |
| **Missing kernel state in an already-used directory stops the launch rather than resetting** | `_restore_kernel_state` | `test_revision_commit.py:794` |
| **A fresh directory restores default kernel state with no record at all** | first-launch path | `test_revision_commit.py:842` |

The reserved-namespace control closes a real collision: without it a user could name a workflow
`kernel-state:kernel-policy-state` and write into the aggregate that stores the policy snapshot.
`kernel_state_aggregate` now prefixes every aggregate, `_is_reserved_identifier` refuses both the
bare legacy names and the whole `kernel-state:` prefix, and the bare names stay reserved so records
written by the earlier version still read back.

### 4.8 Capability policy

| Control | Test evidence (`tests/security/test_policy.py`) |
|---|---|
| Egress and network are denied without grants | `:74` |
| Egress with no capabilities is never approvable | `:100` |
| Egress requires explicit approval even with a valid grant | `:116` |
| Read-only requests are allowed inside the active grant | `:142` |
| An expired grant denies even after approval | `:154,524` |
| A stale state digest invalidates an existing approval | `:186` |
| An approval with mismatched digests is invalidated | `:209` |
| Explicit invalidation requires matching digests | `:239` |
| Rejection and abort are terminal and block a later approval | `:267` |
| A revoked grant denies a previously allowed request | `:294` |
| A scoped grant denies a request outside its scope | `:307` |
| A scoped side-effect grant still needs approval within scope | `:333` |
| Wildcard grants and naive timestamps are rejected | `:357` |
| Every checkpoint type requires approval without a declared side-effect level | `:373` |
| **First use of a tool class requires approval; repeats stay inside the class** | `:406` |
| An approval cannot be replayed onto a different action | `:456` |
| Policy state round-trips grants and decisions | `:490` |
| State import rejects malformed and inconsistent state | `:557` |
| Policy maps are bounded and evict the oldest resolved entries first | `:638` |

`TOOL_CLASS_CHANGE` is enforced, not merely declared. `PolicyRequest.tool_class_change` is `True`
only for `ApprovalKind.TOOL_CLASS_CHANGE` (`kernel/policy.py:345`), and `ToolExecutor._policy_request`
raises the checkpoint for every non-egress tool call (`tools/executor.py:1123`). A tool class is
therefore remembered per `(action_digest, scope)`: the first call in a class needs an explicit
approval, and a repeat inside the same class with the same capability set does not. A change to the
requested capability set for that class produces a different `action_digest` and is a new class,
which re-approves. This is verified end to end by `tests/security/test_tool_executor.py:504,618`
together with `tests/security/test_policy.py:406,456`.

### 4.9 Tool execution and isolation

| Control | Test evidence (`tests/security/test_tool_executor.py`) |
|---|---|
| A registered tool runs successfully in a real subprocess | `:210` |
| `AUTO` never runs a tool inside the kernel process | `:243` |
| `TRUSTED_IN_PROCESS_EVALUATOR` serves only a registered pure evaluator | `:276` |
| Untrusted handlers cannot reach the trusted in-process mode | `:310` |
| Only a first-party pure evaluator may be registered as trusted | `:362` |
| Missing capabilities are denied before dispatch | `:393` |
| Registration refuses a tool that claims ungranted authority | `:404` |
| Network and egress are denied by default | `:434` |
| Unknown tools and versions are denied | `:477` |
| A side-effect mismatch is denied | `:488` |
| Undeclared capability requests are denied | `:496` |
| Policy approval is required and recorded | `:504,618` |
| An approval cannot be replayed with changed arguments | `:516` |
| An approval is bound to the resolved workspace and tool identity | `:552` |
| Sandbox controls are reported only when probed | `:568` |
| A timeout terminates the worker | `:627` |
| A timed-out tool leaves no running descendant | `:724` |
| The worker cap bounds concurrent tool processes | `:757` |
| A saturated pool denies instead of queueing forever | `:790` |
| A crashing worker is a structured failure, not a result | `:833` |
| Output flooding is bounded | `:871` |
| Oversized arguments are rejected | `:905` |
| Cancellation is observed during dispatch | `:940` |
| A pre-cancelled call never dispatches | `:982` |
| Workspace paths reject traversal and symlinks | `:992` |
| Artifacts outside the workspace are rejected | `:1024` |
| Symlinked artifacts are rejected | `:1061` |
| Artifact digest and size are verified | `:1094` |
| **Unknown artifact formats classify as `design_binary`, fail-closed** | `:1124` |
| **Schema patterns are admitted against a strict subset before they are executed** | `:1209`, plus all 9 tests in `tests/security/test_schema_redos.py` |
| An unrecognised tool error code is normalized | `:1316` |
| Child stderr is surfaced as a sanitized diagnostic | `:1332` |
| Child environments are minimal and workspace-scoped | `:1374` |
| Executor-owned environment keys cannot be overridden | `:1409` |
| The registry is immutable and version-aware | `:1440` |
| The registry rejects mutated or mismatched schemas | `:1467` |
| Schema validation rejects unsafe documents | `:1557` |
| Isolation reports expose no secrets | `:1583` |
| Ledger accounting bounds tool calls | `:1612` |
| The `tools` package does not import the network | `:1647` |
| `ToolResult` and the response envelope stay consistent | `:1655` |

#### Fail-closed classification

`_derive_classification` (`tools/executor.py:441`) is monotone: it can only move content toward
`design_binary`, never away from it. A recognised binary signature, a payload that is not decodable
text, an unrecognised extension, or the tool's own `design_binary` claim each produce
`design_binary`; a declared `TEXT` cannot survive any of them. An unknown format is therefore
classified `design_binary` **by default** rather than by falling through to a permissive case. That
is the fail-closed direction, and `tests/security/test_tool_executor.py:1124` is written to require
it.

#### Strict regex admission

A tool descriptor's schema `pattern` is a denial-of-service vector aimed at the validator, so
admission is decided from the pattern's own text before anything is compiled and before the pattern
is ever run against a value (`tools/protocol.py:459`, `_bounded_pattern`):

- the schema must carry a `maxLength`, bounded to `1..MAX_PATTERN_INPUT_LENGTH` — a pattern with no
  input bound is refused outright;
- the pattern must be within `MAX_PATTERN_LENGTH`;
- a fixed table of unsafe constructs is searched: backreferences, lookaround assertions, conditional
  groups, inline flags, atomic and possessive groups, and wildcard repetition;
- nested repetition is rejected (`_reject_nested_repetition`), because `(a+)+` makes matching cost
  grow exponentially in the subject length;
- **ambiguous repetition is rejected** (`_reject_ambiguous_pattern`), so a quantifier whose split
  point is not unique cannot be registered even when it is not literally nested;
- a repetition count beyond the backtracking budget is refused.

A refused pattern is **never compiled and never executed**, so a tool author cannot buy an unbounded
regex by registering a hostile schema. `tests/security/test_schema_redos.py` proves this from four
directions: ambiguous repetition is refused (`:112`), constructs outside the admitted subset are
refused (`:118`), over-budget repetition is refused (`:124`), registration refuses an ambiguous
schema (`:130`), a refused pattern is provably never compiled (`:145`, by monkeypatching the
compiler), the admitted subset still matches and still binds (`:162`), refusing a hostile pattern is
itself bounded work (`:171`), a pattern needs a bounded `maxLength` (`:181`), and the registry refuses
a tool whose schema pattern is ambiguous (`:193`).

### 4.10 Model residency and egress

| Control | Test evidence (`tests/security/test_model_residency.py`) |
|---|---|
| Egress without a policy engine fails closed | `:114` |
| A constructor boolean cannot bypass egress | `:142` |
| Egress requires a **per-request** approval | `:158` |
| A stale egress approval is refused | `:185` |
| The egress action digest pins every egress facet | `:208` |
| The egress manifest is recorded before dispatch | `:263` |
| A missing or failing egress manifest denies the call | `:300` |
| Transcript truncation is byte-safe | `:340` |
| Artifact preview and metadata text is redacted | `:378` |
| AWS-style secrets are redacted | `:445` |
| Hosted HTTPS endpoints require allowlist **and** consent | `:472` |
| Policy failures propagate without degradation | `:526` |
| `DegradingPlanner` never downgrades a denial | `:585` |
| Model requests reject oversized content | `:625` |

**Per-request policy and manifest.** Egress authorization is evaluated on every call, not cached
against a client-level grant. `_authorize_egress` (`runtime/model_gateway.py:1339`) builds a fresh
`PolicyRequest` with `requested_capabilities = ("model.egress",)`, `side_effect_level = egress`,
`egress=True`, and `checkpoint_type = EGRESS`. Its `state_digest` covers destination, provider,
model, request id, and a digest of the redactions; its `action_digest` additionally covers the
request digest and the capability list. Both are recomputed at call time, so a changed request
invalidates the approval. Independently, an `ModelEgressManifest` must be recorded **before**
dispatch: with no manifest callback attached the call raises rather than proceeding unrecorded, and
a callback that raises also fails the call.

**Empty allow-list denies.** `OpenAICompatibleClient._authorize_destination`
(`runtime/model_gateway.py:980`) normalizes each entry of `allowed_endpoints` and requires the
request's own normalized destination to be a member. The default for `allowed_endpoints` is the
empty tuple, so **the default denies**. Confirmed directly on 3.11.16:

```console
$ $PY311 -c "…OpenAICompatibleClient(api_key='k', endpoint='https://api.example.com/v1/chat/completions',
                model_id='m', consent=True)…"
EgressDeniedError: the hosted provider endpoint is not in the egress allowlist
```

This holds with `consent=True`, so consent alone is never sufficient. A non-loopback plaintext
endpoint is refused independently.

**Byte-safe redaction and truncation.** Redaction and transcript bounding both operate on encoded
bytes, not on character counts. `truncate` encodes to UTF-8, slices at `maximum_bytes`, and decodes
with `errors="ignore"` (`runtime/model_gateway.py:236`), so a multi-byte character straddling the
cut is dropped whole rather than producing a replacement character or an invalid sequence. Size
checks are `len(text.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES` on both the request and response
bodies (`:564`, `:566`, `:1541`, `:1550`), and outbound payloads are built with
`canonical_json_dumps(payload, max_bytes=MAX_REQUEST_BYTES).encode("utf-8")` (`:1017`). Verified by
`test_model_residency.py:340`.

### 4.11 Domain evaluator implementation digests and capability enforcement

A domain package's declared identity is no longer only its declarations. Every evaluator bound to a
catalog step is identified and digested.

| Control | Implementation | Test evidence |
|---|---|---|
| Every evaluator has an identified, digested implementation bound to its step | `evaluator_implementation`, `EvaluatorImplementation`, `IMPLEMENTATION_DIGEST_VERSION = "evaluator-implementation-digest-v1"` | `tests/unit/test_domains.py:834`, `tests/security/test_domain_integrity.py:261` |
| The digest commits to qualified name, module source digest, step id, and declared capabilities | `evaluator_implementation` | `test_domain_integrity.py:261` |
| **Substituting an evaluator changes the package digest even when the numbers are identical** | `DomainPackage.digest()` over `package_payload` | `test_domain_integrity.py:261` |
| **A substituted evaluator cannot carry an undeclared capability** | step/evaluator capability equality check in `_identify_evaluators` | `test_domain_integrity.py:302` |
| Digests are stable within a process and across processes | `_MODULE_DIGEST_CACHE` keyed by file identity | `test_domain_integrity.py:320` |
| **Rewriting an evaluator module on disk changes its digest** | `module_source_digest` digests the bytes on disk, cached per `(path, dev, ino, size, mtime_ns)` | `test_domain_integrity.py:333` |
| An evaluator with no identifiable source implementation is refused, not digested blind | `module_source_digest` raises `DomainPackageError` | `test_domain_integrity.py:366` |
| **Declared capabilities are denied before any evaluation runs** | `enforce_capability_request` | `test_domain_integrity.py:407`, `tests/unit/test_domains.py:872` |
| **A checkpoint-kind mismatch rejects the package at construction** | `_validate_checkpoints` | `test_domain_integrity.py:475` |
| Evaluator implementation digests are published in the manifest and pinned on every revision | `manifest_schema_digests` = `schema_digests ∪ evaluator_implementation_digests` | `tests/unit/test_domains.py:834` |
| An unimported module, a module with no `__file__`, and an unreadable source file are all refused | `module_source_digest` guards | `tests/security/test_domain_integrity.py:366` |

`module_source_digest` digests **the bytes on disk that would be imported**, not the loaded module
object, so it identifies the implementation rather than the declaration that references it. A
module that is not imported, has no source file, or whose file cannot be read has no identifiable
implementation and is refused. That refusal is the honest failure mode: the alternative is to digest
blind and report a number that means nothing.

This closes the *substitution* half of the package-trust problem. It does **not** close the
*authority* half: the evaluators are still trusted first-party Python running in the kernel process.
See gap G1.

### 4.12 Architectural boundary enforcement

| Control | Test evidence |
|---|---|
| `kernel/*.py` never imports `domains`, `runtime`, or `tools` | `tests/unit/test_kernel.py:268` (AST walk over every file in the package) |
| `domains/**/*.py` imports only `kala_agent.contracts` and `kala_agent.domains`, and the module count is asserted | `tests/unit/test_domains.py:975` |
| `tools` imports no network module | `tests/security/test_tool_executor.py:1647` |
| No provider SDK is a dependency at all | `pyproject.toml` declares only `fastapi` and `pydantic` |
| All three domain packages run through one evaluator interface | `tests/unit/test_domains.py:729` |
| A domain package with inconsistent declarations is refused | `tests/unit/test_domains.py:754` |

### 4.13 End-to-end properties

From `tests/integration/test_mechanical_loop.py` (12 tests), which drives the same loop through the
HTTP API, through the CLI adapter, and once through a real CLI subprocess:

| Property | Test |
|---|---|
| Artifact digest matches the bytes in the store | `:106` |
| Revisions are immutable, sequenced, chained, and listed | `:127` |
| Full loop through the API, including the approval | `:154` |
| Full loop through the CLI | `:271` |
| A real CLI subprocess runs the same loop | `:325` |
| The API and the CLI share one data directory | `:355` |
| Failure paths are reported, not swallowed | `:388` |
| A rejected checkpoint leaves no revision | `:409` |
| A revoked capability blocks and the checkpoint recovers | `:427` |
| Stale checkpoint digests are refused | `:444` |
| A closed application refuses further work | `:452` |
| Every component can be injected and injected resources survive `close()` | `:464` |

### 4.14 Optional FreeCAD adapter controls

Every row names the test that would fail if the control regressed. Test evidence in this section is in
`adapters/freecad/tests/` unless the row says otherwise.

#### 4.14.1 The boundary between the core and the adapter

| Control | Implementation | Test evidence |
|---|---|---|
| **No core module imports FreeCAD, `femtools`, or the adapter** | AST walk over every file under `src/kala_agent/` | `tests/unit/test_adapter_boundaries.py:32` (root suite) |
| **No adapter module imports a core application layer** | AST walk over every file under `adapters/freecad/src/kala_agent_freecad/`, rejecting `kala_agent.{application,api,cli,domains,kernel,persistence,runtime}` | `tests/unit/test_adapter_boundaries.py:38` (root suite) |
| The root distribution gains no dependency and no extra for the adapter | `pyproject.toml` declares only `fastapi` and `pydantic` | Read directly; the two `pyproject.toml` files |
| **No FreeCAD PyPI wheel is used anywhere** | The adapter's dependencies are `kala-agent` and `pydantic`; a comment states FreeCAD is a discovered system dependency | `adapters/freecad/pyproject.toml` |
| FreeCAD is never imported at module scope | every import is inside `freecad_session` at call time | 284 of the 301 adapter tests run with no FreeCAD needed; `test_cad_runtime_loader.py` (16) |
| A host without FreeCAD is unaffected | `register_if_available` returns `()`; `discover` reports `available: false` with a bounded detail | `test_registration.py` (17), `test_availability.py` (31) |

#### 4.14.2 Discovery and the solver

| Control | Implementation | Test evidence |
|---|---|---|
| Availability is resolved from the filesystem, never by importing FreeCAD | `discover`, `candidate_lib_roots`, `is_import_root` | `test_availability.py` (31) |
| A candidate import root is kept only if it holds `FreeCAD.so`, `FreeCAD.dylib`, or `FreeCAD.py` | `MODULE_MARKERS`, `_holds_module_marker` | `test_availability.py` |
| The version is read in an isolated probe with a constructed environment and a 30 s timeout, and roots travel in `KALA_FREECAD_PROBE_ROOTS` because `-I` ignores `PYTHONPATH` | `read_version`, `_run_probe`, `VERSION_PROBE_TIMEOUT_SECONDS` | `test_availability.py`, `test_availability_records.py` (28) |
| The availability record bounds every field and rejects a non-`bool` or unbounded value in `__post_init__` | `FreeCADAvailability` | `test_availability_records.py` (28) |
| **A caller cannot supply the solver executable** | no FEM input schema field carries a path; `invoke_fem` calls `resolve_calculix()` itself | `test_contracts.py` (44), `test_cad_fem_fakes.py` (32) |
| A discovered solver must match a bounded name pattern with no separator, be a regular non-empty file ≤ 64 MiB, and carry an execute bit | `is_safe_calculix`, `_CALCULIX_NAME_PATTERN`, `MAX_SOLVER_BYTES` | `test_availability.py` |
| **Without a safe solver the call reports `solver_unavailable` and no result is invented** | `_run_solver`, `result_summary`, `SOLVER_STATUSES` | `test_cad_live.py:177` (live, on this host) |
| `run_solver: false` is a clean skip, not a failure | `_run_solver` first branch | `test_cad_live.py:201` |
| Displacement and stress are read from a real result object, and non-numeric or non-finite entries are rejected | `result_summary`, `_extreme`, `_is_number`, `RESULT_TYPE_ID` | `test_cad_fem_fakes.py` |
| The `completed` / `prerequisites_unmet` / `failed` solver branches are exercised | fakes only — **no CalculiX on this host** | `test_cad_fem_fakes.py`. Not observed live; see §2.7.5 |

#### 4.14.3 The lazy import and the native boundary

| Control | Implementation | Test evidence |
|---|---|---|
| **The import order is `FreeCAD` → `Part` → `ObjectsFem` → optional `femtools.ccxtools`, and it is observed in a real subprocess** | `FREECAD_IMPORT_ORDER`, `REQUIRED_MODULE_ORDER`, `freecad_session` | `test_cad_live.py:242` — a `python -I -B` subprocess writes the observed order and the test asserts `order[0] == "FreeCAD"` and `order[:3] == FREECAD_IMPORT_ORDER[:3]` |
| The optional solver bridge failing is reported, not raised | `_import_solver_bridge` with `require=False` | `test_cad_live.py:286`, `test_cad_runtime_loader.py` |
| **Native fd-1 output cannot corrupt the response envelope** | `native_output_to_stderr` around every FreeCAD call | `test_cad_runtime_loader.py`; every live test runs real FreeCAD, which writes a native banner |
| Every document a call opened is closed, on success and on failure | `close_documents`, `freecad_session` finally, per-tool `finally: _close` | `test_cad_live.py:224` — `FreeCAD.listDocuments() == []` after a geometry and an FEM call in one process |
| Only the import roots the call introduced are removed from `sys.path` on exit | `_extend_loader_path`, `_restore_loader_path` | `test_cad_runtime_loader.py` |
| No FreeCAD import root reachable means a typed refusal, not an attempt | `freecad_session` raises `FreeCADDependencyError` → `sandbox_unavailable` | `test_cad_runtime_loader.py` |

#### 4.14.4 Artifacts

| Control | Implementation | Test evidence |
|---|---|---|
| **Every write is confined to the worker workspace** | `resolve_target` via the kernel's `resolve_workspace_path`; `prepare_target` refuses a symlinked directory or destination | `test_cad_live.py:118` — a sibling path outside the workspace does not exist, and the only files written are the three declared exports |
| An export over the caller's budget is refused and nothing is published | `finalize`, `digest_artifact` chunk loop | `test_cad_live.py:137`, `test_cad_artifacts_limits.py` |
| Every published artifact is digest-verified from the bytes on disk | `digest_artifact`, and a re-measure that must equal the stat | `test_cad_live.py:65,151` — every artifact is re-hashed by the test |
| Every artifact is `design_binary`, and the kernel re-derives that independently | `PublishedArtifact.artifact`; `tools/executor.py:_derive_classification` | `test_cad_live.py`, `test_executor_live.py::test_both_tools_register_and_dispatch`; kernel side `tests/security/test_tool_executor.py:1124` |
| A rejected export is unlinked rather than left partial | `discard` on the export error path | `test_cad_artifacts_limits.py` |

#### 4.14.5 The CAD profile

| Control | Implementation | Test evidence |
|---|---|---|
| The CAD profile is derived from the shipped defaults and the shipped profile is never mutated | `dataclasses.replace`; two calls produce equal-but-distinct values and the shipped profile is unchanged | `test_policy.py::test_profile_does_not_mutate_the_shipped_profile` |
| The profile grants **no** network, egress, or privileged authority and keeps child limits on | `cad_profile` sets all three flags `False` and `enforce_child_limits=True` | `test_policy.py::test_profile_never_grants_authority` |
| The only authority-shaped change is `isolation_profile`, which defaults to `STRICT_LINUX` | `cad_profile(isolation=IsolationProfile.STRICT_LINUX)` | `test_policy.py::test_profile_is_isolated_where_the_host_can_enforce_it` |
| The budgets are larger than the shipped ones and stay inside the kernel's own bounds | `CAD_MAX_ARTIFACT_BYTES = 268_435_456`, `CAD_MAX_WORKERS = 2`, `CAD_MAX_QUEUE_SECONDS = 30.0`, `CAD_DEFAULT_TIMEOUT_SECONDS = 900.0` | `test_policy.py::test_profile_budgets_exceed_the_shipped_ones` |
| A non-`IsolationProfile` or a non-`SandboxProbe` is refused | `cad_profile` type checks | `test_policy.py::test_profile_accepts_a_probe_and_nothing_else` |
| `cad_report` describes what is enforced, not what is hoped for | it renders `SandboxProfile.enforced_controls` | `test_policy.py::test_report_describes_what_is_enforced` |
| **Strict means strict: an unavailable probe refuses side effects** | `judge_sandbox(IsolationProfile.STRICT_LINUX, None).allowed is False` | `test_policy.py:117` |
| **The strict CAD profile is enforced or refused, end to end** | the executor denies a `local_write` tool when the probe is unavailable | `test_executor_live.py::test_strict_profile_is_enforced_or_refused` — on this host the probe is unavailable and the call is `DENIED` with `sandbox_unavailable`. See §5.2 |

#### 4.14.6 The seam, exercised through the kernel's own executor

| Control | Test evidence (`adapters/freecad/tests/test_executor_live.py`) |
|---|---|
| Both tools register through `ToolRegistry` and dispatch out of process | `test_both_tools_register_and_dispatch` — `registry.tool_ids()` is exactly the two CAD ids, and both calls are `SUCCEEDED` |
| The kernel re-verifies what FreeCAD wrote | the test re-hashes every artifact and asserts `classification is DESIGN_BINARY` |
| A real FEM document is built and named | `result["objects"]["analysis"] == "KalaAnalysis"`, and both `fcstd` and `step` artifacts are published |
| An argument the schema forbids is refused | `test_dispatch_refuses_arguments_the_schema_forbids` — `FAILED` / `invalid_arguments`, `result_json is None` |
| An ungranted capability denies before dispatch | `test_dispatch_refuses_an_ungranted_capability` — `DENIED` / `capability_denied` |
| The CAD budget and profile name actually reach the executor | `test_cad_profile_budget_reaches_the_executor` — `executor.profile` matches, the report carries `sandbox_profile == "freecad-cad"`, and `process_isolation` is in the enforced controls |
| The `TOOL_CLASS_CHANGE` checkpoint applies to both tools, unchanged | the adapter supplies no policy code; the kernel's own tests are the evidence — `tests/security/test_tool_executor.py:504,516,552,618` and `tests/security/test_policy.py:373,406,456` |
| The `import_roots` seam is real: the executor refuses a root that is not a directory | `tools/executor.py:_normalize_import_roots`; adapter side `test_registration_roots.py` (2) |

## 5. Static sandbox probe result

### 5.1 The shipped profile, probed directly

Run directly against this host, outside the test suite, on CPython 3.11.16:

```console
$ $PY311 -c "from pathlib import Path; from kala_agent.tools.sandbox import probe_sandbox, judge_sandbox, IsolationProfile; \
             p = probe_sandbox(import_roots=(Path('src').resolve(),)); print(p); \
             print(judge_sandbox(IsolationProfile.PORTABLE, p)); print(judge_sandbox(IsolationProfile.STRICT_LINUX, p))"
SandboxProbe(available=True, platform='linux-x86_64',
  controls=('chroot_workspace_only', 'device_nodes_minimal', 'ipc_namespace', 'mount_namespace',
            'network_isolation', 'pid_namespace', 'procfs', 'readonly_system_bindings',
            'user_namespace', 'uts_namespace', 'workspace_read_write'),
  detail='', elapsed_ms=201.7)
SandboxVerdict(allowed=True, reason='the portable profile confines a process, not the filesystem', controls=())
SandboxVerdict(allowed=True, reason='', controls=('chroot_workspace_only', 'device_nodes_minimal',
  'ipc_namespace', 'mount_namespace', 'network_isolation', 'pid_namespace', 'procfs',
  'readonly_system_bindings', 'user_namespace', 'uts_namespace', 'workspace_read_write'))
```

All six **required** controls verified. `scratch_tmpfs` was **not** verified on this host, and the
tool correctly does not report it.

The two verdicts are the whole point of the probe, and they are deliberately different in kind:

- The **portable** verdict is allowed with a non-empty `reason` and an **empty** `controls` tuple.
  That is the tool saying, in one line, that the default profile confines a *process* and not the
  filesystem. It claims no filesystem control because it has none.
- The **strict** verdict is allowed with an empty `reason` and the full observed control set, every
  one of which was verified **from inside** a real child process.

The shipped application still selects the portable profile, so a side-effecting tool in the product
runs without filesystem confinement. That is gap G2, and this report does not claim it is fixed.

### 5.2 The optional FreeCAD adapter's CAD profile probe

`cad_profile()` defaults to `IsolationProfile.STRICT_LINUX`, so the adapter inherits the same fail-closed
behaviour. Two different observations are recorded here, and the difference between them matters.

**Observation A — the executor's own probe is unavailable on this host.** Constructing a
`ToolExecutor` with the CAD profile, the registered tools, and `worker_import_roots(availability)`,
then reading `executor.sandbox_probe`:

```text
available  False
platform   linux-x86_64
controls   ()
detail     the strict profile is unavailable on this host
```

Because both CAD tools are `local_write`, this means a live call under the profile's own default is
**denied** with `sandbox_unavailable`. That is the fail-closed path working, and it is what
`adapters/freecad/tests/test_executor_live.py::test_strict_profile_is_enforced_or_refused` asserts on
this host. No degradation to portable occurs.

**Observation B — the host itself can build the jail.** Calling `probe_sandbox` directly, outside the
executor, with the FreeCAD import root and the executor's own child import roots:

```console
$ $PY314 -c "from kala_agent.tools.sandbox import probe_sandbox; \
             p = probe_sandbox(import_roots=('/usr/lib/freecad/lib',), scratch_bytes=16*1048576); print(p)"
SandboxProbe(available=True, platform='linux-x86_64',
  controls=('chroot_workspace_only', 'device_nodes_minimal', 'ipc_namespace', 'mount_namespace',
            'network_isolation', 'pid_namespace', 'procfs', 'readonly_system_bindings',
            'user_namespace', 'uts_namespace', 'workspace_read_write'),
  detail='', elapsed_ms=217.3)
```

All six required controls verified, five optional ones besides, and `scratch_tmpfs` correctly absent.

**The cause of the difference, isolated.** `ToolExecutor._probe_sandbox` passes
`python_executable=os.path.realpath(sys.executable)` when the caller supplies none. In a venv, that
resolves the launcher symlink to the **system** interpreter (`/usr/bin/python3.14`), which cannot
import `kala_agent` — and the probe entry point is
`from kala_agent.tools.sandbox import run_probe`, executed *before* the child isolates itself. The
child therefore dies with `ModuleNotFoundError: No module named 'kala_agent'`, the non-zero exit makes
the probe `available=False`, and the executor refuses. Reproduced directly:

```text
rc 1
Traceback (most recent call last):
  File "<string>", line 1, in <module>
    import json,sys;from kala_agent.tools.sandbox import run_probe;sys.exit(run_probe(sys.argv[1], sys.argv[2]))
ModuleNotFoundError: No module named 'kala_agent'
```

Passing the venv interpreter instead of the resolved system interpreter makes the same probe succeed.

**What this means for the claims in this document set, stated precisely:**

- The claim "the CAD profile defaults to strict Linux and fails closed when the probe is unavailable"
  is **verified**, and the fail-closed branch is the one actually taken on this host.
- The claim "the strict CAD profile confines the filesystem" is **not verified on this host through the
  executor**, because the executor's probe never got as far as building a jail. Observation B shows the
  host can build one; it does not show the executor doing it.
- The claim "the adapter's functional tests run a FreeCAD worker with no filesystem confinement" is
  **verified and true**: `test_executor_live.py` and the CAD live tests construct the executor with
  `cad_profile(isolation=IsolationProfile.PORTABLE)` or the default `SandboxProfile`, so a FreeCAD
  worker in those tests can read the host filesystem. That is **RR-1** and **RR-19**, and it is not
  presented here as mitigated.

This is a kernel-side issue in the probe's interpreter selection, not an adapter defect, and it is
recorded as residual risk **RR-19**, as open decision 17 in `docs/architecture-report.md` §21, and as an
ADR-0007 Revisit Trigger. It is **not** claimed as fixed.

## 6. Defects found during verification

Defects R1–R3 from the previous pass of this report are **fixed**. Defect R4 was accepted and
remains accepted. Nothing in this section is a claim about work not done.

### R1 — The package did not import on the declared minimum interpreter — **FIXED**

**Previously: high severity, open.** `pyproject.toml` declares `requires-python = ">=3.11"`. On
CPython 3.11.16 the package previously failed at import time with 11 collection errors under
`pytest`, all from one root cause: `runtime/model_gateway.py` declared
`metadata: ArtifactMetadata = MappingProxyType({})` as a field of a
`@dataclass(frozen=True, slots=True)`. CPython 3.11's `dataclasses` rejects a field default whose
class has `__hash__ is None`, and `types.MappingProxyType.__hash__` is `None` on every version —
3.11 rejects it, 3.14 accepts it.

**The fix.** The field now uses a factory:

```python
# runtime/model_gateway.py:306
metadata: ArtifactMetadata = field(default_factory=lambda: MappingProxyType({}))
```

This removes the version sensitivity entirely rather than papering over it, and it is the
conventional fix: a mutable default in a dataclass field should be produced by a factory.

**Verification of the fix, on a real 3.11 environment with the `src` import root.** This is not a
bare `import` check. A CPython 3.11.16 virtual environment was created, the package was installed
into it so the `src` layout import root and the `[project.scripts]` entry point both resolve, and
then:

```console
$ $PY311 -c "import kala_agent, sys; print(kala_agent.__file__, sys.version.split()[0])"
/home/v9i/Projects/Kala-Agent/src/kala_agent/__init__.py 3.11.16

$ $PY311 -m pytest -q
1227 passed, 1 warning in 15.29s

$ $PY311/bin/kala-agent init          # console script, not `python -m`
{ "status": "ready", "data": { "schema_version": 2, ... } }
```

Both the `python -m kala_agent.cli` path and the installed `kala-agent` console script work on
3.11.16, the whole suite (1,227 tests) passes on 3.11.16, and both quality gates are clean on 3.11.16
(section 2). **The declared `requires-python = ">=3.11"` floor is now true.** The workaround of
treating 3.12 as
the effective minimum is no longer needed and is not claimed anywhere in this document set.

### R2 — A test lowered `RLIMIT_CPU` in the pytest process itself — **FIXED**

**Previously: low severity, open, and it blocked coverage measurement.** The test
`test_child_limits_sets_bounded_resource_limits` called `tools.executor._child_limits(...)()` inside
the pytest process. That helper sets `RLIMIT_CPU = (6, 7)`, `RLIMIT_FSIZE`, `RLIMIT_NOFILE = 256`,
and `RLIMIT_CORE = 0` on the caller, and the limits were never restored. By the time the test
executed under instrumentation the process had burned more than 7 s of CPU, so the kernel delivered
`SIGXCPU` and `coverage run -m pytest` died with no data file written. The previous pass of this
report therefore had to quote a coverage figure obtained with a measurement shim, and explicitly
could not report `fail_under` as a configured gate.

**The fix.** The test now intercepts the syscall instead of making it:

```python
# tests/unit/test_executor_branches.py:436
def test_child_limits_sets_bounded_resource_limits(monkeypatch: pytest.MonkeyPatch) -> None:
    import resource

    limits = _child_limits(SandboxProfile(), 5.0)
    assert limits is not None
    applied: dict[str, tuple[int, int]] = {}

    def tracking(constant: int, values: tuple[int, int]) -> None:
        applied[str(constant)] = values

    monkeypatch.setattr(resource, "setrlimit", tracking)
    limits()
    assert applied
    assert all(soft >= 0 and hard >= soft for soft, hard in applied.values())
```

The test still asserts the real thing — that the limits the child would receive are well formed and
ordered — without touching the limits of the process running the test.

**Verification of the fix.** `coverage run -m pytest` now completes on both interpreters and writes
its data file, and `coverage report` exits 0 against `fail_under = 90` (section 2.2). Every coverage
figure in this report is a direct, unshimmed measurement. The split-run workaround described in the
previous pass is no longer required and is not used here.

### R3 — Data directory and SQLite files inherited the process umask — **FIXED**

**Previously: medium on a multi-user host, open.** `KalaApplication` and `Database` created the data
directory and the database with the process umask. `Database.__init__` called
`candidate.parent.mkdir(parents=True, exist_ok=True)` with no mode, and `sqlite3.connect` creates the
file with `0666 & ~umask`. Observed on the verification host before the fix:

```text
drwxr-xr-x  kala-data
-rw-r--r--  kala-data/kala.sqlite3
drwx------  kala-data/artifacts
drwx------  kala-data/runs
-rw-------  kala-data/api-token
```

The artifact store, the run directory, and the API token were already owner-only; the data directory
and the database were not. On a host with a permissive umask, another local account could read the
journal, the design specifications, and every committed revision.

**The fix.** `persistence/database.py:312` creates the directory with `mode=0o700` and then sets it
explicitly, and `:329` sets the connection target to `0o600`:

```python
candidate.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
os.chmod(candidate.parent, 0o700)
…
os.chmod(connection_target, 0o600)
```

The `os.chmod` calls are what make the modes hold: `mkdir`'s `mode` argument is masked by the process
umask on POSIX, so `mode=0o700` alone would still yield `0o755` under umask `022`. The explicit
`chmod` after creation is the part that closes the gap.

**Verification of the fix.** Observed on the verification host after the fix, for both a
`python -m` invocation and an installed `kala-agent` console-script launch on 3.11.16:

```text
700 /tmp/kala-verify
600 /tmp/kala-verify/kala.sqlite3
700 /tmp/kala-verify/artifacts
700 /tmp/kala-verify/runs
```

`tests/unit/test_persistence.py:190` (`test_database_uses_private_directory_and_file_modes`) asserts
`st_mode & 0o777 == 0o700` for the directory and `== 0o600` for the database file, so a regression to
umask-dependent modes fails the suite rather than passing quietly on a well-configured host.

### R4 — The local data directory is never encrypted — **ACCEPTED, NOT FIXED**

**Severity: informational, accepted.** The journal, artifacts, revisions, and the API token are
plaintext on disk. There is no encryption, at rest or in transit, anywhere in the repository. R3
narrows *who on the host* can read the files; it does not protect their contents from anyone who can
read them. ADR-0001 lists a user-controlled encrypted local backup as an assumption, and
confidentiality here still rests on OS account protection and filesystem permissions. This is gap
G5's neighbour and is not claimed as fixed.

### Gaps that remain open

These are stated so that no reader of this document infers a fix that does not exist. Each is
tracked in `docs/threat-model.md` §7 and in `docs/architecture-report.md` §3.

| ID | Gap | Why it is not closed |
|---|---|---|
| G1 | Domain evaluators are **executable trusted Python**, not the data-only bounded expression packages ADR-0002 specified. | `evaluator_implementation` digests and identifies the implementation and detects substitution, and declared capabilities are enforced before evaluation, but the evaluator is still a Python class imported into the kernel process. Identification is not inertness. A first-party package is trusted to be sensible. No signature verification, no quarantine store, no installer. |
| G2 | The **default portable profile cannot confine the host filesystem or network.** | The strict Linux profile exists, is probed rather than assumed, verifies its controls from inside a real child process, and fails closed when it cannot — but it is not the default, so the shipped reference path runs portable. A side-effecting tool in the product can currently read the host filesystem as the operator. `KalaApplication` does not select the strict profile and no probe-first preflight is wired into startup. |
| G3 | **No certified engineering, and no solved FEM result anywhere.** | The `kala-agent-freecad` distribution now integrates a real native CAD/CAE kernel (FreeCAD 1.1.3) and builds real geometry, measurements, STEP/IGES/FCStd exports, and a real Fem analysis document — see `docs/architecture-report.md` §3.2 and §10.7. **What is still absent is the part that would make it engineering:** no solver and no mesher are installed, bundled, pinned, or verified, so the FEM tool reports `solver_unavailable` and produces a document rather than an analysis; there is no BIM model, no building-code implementation, and no CFD or thermal solver. The kernel's own tool is still closed-form rectangular-section arithmetic, and every domain evaluation carries `NON_AUTHORITATIVE_FINDING`; hard constraints in the domain packages are declared bounds on surrogate arithmetic, not engineering acceptance criteria. |
| G4 | **No external anchor for the journal head digest.** | The chain is tamper-**evident**. A wholesale rewrite produces an internally consistent chain with a different head digest, and `ChainVerification.head_event_digest` exists so a caller *could* anchor it — but nothing does. No anchor, no backup mechanism, no signature. |
| G5 | **No encryption at rest.** | See R4. |
| G6 | **No retention policy, no garbage collection, no projection or replay product layer.** | Artifact objects are immutable and never deleted; journal events are never pruned; `VACUUM` is never run. There is a bounded policy working set and bounded budget reservations, but nothing bounds the growth of the durable store. The journal is verified and queryable, but there is no read-model projection, no projection rebuild, and no golden-replay harness. `provenance` is recorded per revision with no graph traversal or audit query builder. |
| G7 | **The shipped reference path does not dispatch domain evaluators through the tool executor.** | `run_mechanical_reference_workflow` advances the workflow by calling each domain package's evaluator in-process. `ToolExecutor` is constructed, configured, and exposed as `KalaApplication.tool_executor`; `mechanical.reference-simulator` is registered; `AgentCoordinator` dispatches through the executor — but no HTTP route, CLI command, or workflow step routes a call through it. The out-of-process tool plane is real and independently tested (44 tests) and is simply not on the reference loop. This is why `provenance.tool_call_ids` is `[]` on every reference revision. |
| G8 | **No vector memory, no multi-user, no cloud sync.** | `MemoryManager` is implemented, tested, and exported, but is **not constructed by `KalaApplication`**, so no product path stores or retrieves a memory. There is no embedding and no similarity index. There is one operator, one process, one data directory: no identity system, no per-user authorization beyond the per-launch loopback bearer token, no cross-device synchronization, and no hosted deployment mode. |
| G9 | **The optional FreeCAD adapter is implemented and measured, and is on no product path.** | `adapters/freecad` is a real, working, 100%-branch-covered integration with a real FreeCAD 1.1.3, and it is **not reachable from the product**: `KalaApplication` does not import it, does not call `discover()`, does not register `cad.freecad-geometry` or `cad.freecad-fem`, and does not dispatch them; no HTTP route and no CLI subcommand reaches either tool; no catalog step or workflow step maps to them. Four further limits stand alongside it: **(a)** no mesher or solver is installed or verified, so no solved result is demonstrated; **(b)** the configuration the functional tests exercise is the **portable** profile, which does not confine the host filesystem — the CAD profile's own strict default fails closed here, so the two branches are "confined or refused", and the refused one is the one taken (§5.2); **(c)** the discovered `ccx` is validated structurally, never by signature, and no solver binary is pinned; **(d)** FreeCAD is an LGPL-2.0-only host system dependency outside the dependency graph, so nothing in this repository's quality gates or an SBOM audit inspects it, and no dependency or supply-chain audit was run at all (§7 item 5). |

## 7. Known limitations of the verification itself

1. **Coverage is not correctness.** 94.76% branch coverage means 447 of 11 077 statements were not
   executed on 3.11.16. The uncovered set is listed in section 2.2 rather than hidden. The
   reproducibility caveat in item 2 applies to that count as well: across runs it moved between 443
   and 450 missed statements.
2. **The coverage figure is not bit-reproducible.** Repeated identical runs of the commands in
   section 2.2 on this host produced 94.74%, 94.76%, 94.77%, and 94.80% — a spread of about 0.06
   percentage points, or 3–5 statements, from subprocess- and sandbox-timing-dependent tests. Both
   interpreters report identical statement and branch totals, so this is not an interpreter
   disagreement. Every observed value clears the configured `fail_under = 90` by more than 4.7 points.
   A reader should expect 94.74–94.80% and should treat anything below 90% as a genuine regression.
   Quoting a single number without the range would overstate the measurement's precision.
3. **No mutation testing.** Nothing in the suite checks that the tests would actually fail if a
   control were removed. A test can pass while asserting nothing about the control it is named for.
   This matters most for the controls added in this pass: the regex admission, the evaluator digests,
   and the reserved-identifier rules are verified by construction and by named tests, but nothing
   deletes the control to confirm the test bites.
4. **No property-based or fuzz testing.** The protocol bounds, canonical JSON parser, schema
   validator, and the new regex admission have hand-written boundary cases, not generated ones.
   `tools/protocol.py` at 97.58% is reassuring but not a substitute, and
   `tests/security/test_schema_redos.py` is 9 tests over a deliberately enumerated construct table —
   an unlisted ReDoS shape would not be caught by construction.
5. **No dependency or supply-chain audit.** `pip-audit`, `safety`, and SBOM generation were not run.
   The dependency surface is two runtime packages and seven dev/optional packages, which limits
   exposure, but nothing is pinned to a hash.
6. **No performance measurement.** `NFR-08` (kernel p95 below 200 ms at 10 000 events) is unmeasured.
   There is no benchmark in the repository and none was written for this report.
7. **No air-gapped run.** Offline operation is demonstrated structurally — the composition root
   opens no socket and the `tools` package imports no network module — but the workflow was not
   executed inside a disabled network namespace.
8. **No crash-injection run.** Durability at transaction boundaries is argued from `BEGIN IMMEDIATE`
   and the head CAS and verified by concurrency tests, but no `SIGKILL`-at-a-boundary test exists.
9. **The sandbox probe result is host-specific.** Section 5 records what this kernel enforces. A host
   with user namespaces disabled, or a container without `CAP_SYS_ADMIN`, will get
   `available=False`, and the correct fail-closed consequence is that side-effecting tools are denied.
   The probe was not exercised in a failing configuration on this host, so the fail-closed path is
   verified by test rather than by observation.
10. **No test runs the API under a real listening socket.** Every API test uses an in-process ASGI
    client. The `serve` command's loopback refusal is verified as a decision (`build_serve_target`
    refuses before any socket is created) and the boundary it serves is verified in-process, but the
    Host guard has not been verified against a browser resolving a rebound DNS name over a real
    socket.
11. **The suite touches the network zero times, by design.** The hosted-client tests inject an
    `httpx` transport. The real `httpx` code path, DNS resolution, TLS, and proxy behaviour are
    untested.
12. **Two interpreters were used; the range in between was not.** Everything in this report is
    CPython 3.11.16 or CPython 3.14.7. Behaviour on 3.12 and 3.13 is unverified. The declared floor
    and the development interpreter are both covered, which is what the declaration and the
    toolchain actually use, but it is not the same as covering every 3.x.
13. **`mypy` type-checks against 3.11 semantics on both runs.** `python_version = "3.11"` under
    `[tool.mypy]` means the 3.14.7 mypy run is a second execution of the same 3.11 check, not an
    independent check of 3.14 typing behaviour.
14. **Coverage was measured with the repository's own configuration.** `branch = true`,
    `source = ["kala_agent"]`, and no `omit` or `exclude_lines` beyond the eight narrow
    `pragma: no cover` markers in `src/`. There is no second, independent coverage configuration
    that might disagree. The adapter's 100.00% was measured the same way, with
    `source = ["kala_agent_freecad"]` and `fail_under = 60`.
15. **The adapter suite was run on one interpreter only.** The root suite ran on CPython 3.11.16 and
    3.14.7; `adapters/freecad` ran on 3.14.7 only. The adapter declares the same
    `requires-python = ">=3.11"` and its `mypy` is pinned to `python_version = "3.11"`, but nothing
    here demonstrates the adapter suite importing and passing on 3.11.16.
16. **100% branch coverage is execution, not correctness.** 1 152 statements and 306 branches all
    executed, and the adapter uses the same narrow `pragma: no cover` markers the root suite uses on
    defensive branches. No mutation testing was run, so nothing confirms that deleting a control would
    fail a test. The narrow claim is "every statement and branch in the adapter executes in this
    suite", not "the adapter is correct".
17. **A real FreeCAD, but only one version, on only one host.** Only FreeCAD 1.1.3 on
    `linux-x86_64` was exercised. `FreeCAD.dylib` discovery, the non-Linux discovery path, and any
    other FreeCAD version are covered by fakes only. Nothing pins a FreeCAD version, and no
    compatibility matrix exists.
18. **No solver, so three solver branches were never observed live.** `completed`,
    `prerequisites_unmet`, and `failed` are exercised against fakes in `test_cad_fem_fakes.py`.
    Because neither CalculiX nor Gmsh is installed on this host, the live path proves
    `solver_unavailable` and `not_requested` and nothing else. `mesh.generated is True` and a non-null
    `results` block have never been observed. This is the single largest unverified area in the
    adapter.
19. **The strict CAD profile was verified as refused, not as enforced.** See §5.2. The executor's
    probe is unavailable on this host, so a live CAD call under the profile's own default is denied.
    The host *can* build the jail (Observation B), but the executor never got there, so "the strict CAD
    profile confines the filesystem" is **not** verified through the executor. The configuration the
    functional tests actually exercise is portable, which does not confine the filesystem.
20. **No FreeCAD-specific dependency or supply-chain audit.** No `pip-audit`, no `safety`, no SBOM,
    and no inspection of the FreeCAD package's contents. FreeCAD is an LGPL-2.0-only **system**
    dependency outside the Python dependency graph, so no gate in this repository covers it. The
    Python dependency surface is unchanged (two runtime packages in each distribution), which limits
    the exposure this report can speak to and says nothing about FreeCAD itself.
21. **The boundary test is a source-tree gate, not a runtime guard.**
    `tests/unit/test_adapter_boundaries.py` parses the files in the repository. It does not constrain
    installed bytecode, and it does not prevent an embedder from importing `kala_agent_freecad` and
    calling `register_if_available` — which is the intended API, but it means "the core never imports
    the adapter" is a property of this source tree and not of any deployment.

## 8. Residual risks

Ordered by the product consequence if the risk materializes. Gaps G1–G9 in section 6 map onto these.

| ID | Risk | Why it is not closed | Suggested mitigation |
|---|---|---|---|
| RR-1 | A side-effecting tool reads or writes the host filesystem | G2. The shipped application uses the portable profile, which confines a process, not a filesystem, and `judge_sandbox` says so in as many words. | Select `IsolationProfile.STRICT_LINUX` as the default once a probe-first preflight is wired into startup, and refuse startup when the probe fails. |
| RR-2 | The SQLite journal is rewritten wholesale and stays internally consistent | G4. The hash chain is tamper-**evident**; a full rewrite produces a different head digest, but nothing anchors that digest outside the database. | Anchor `head_event_digest` in a separate, append-only location and verify it on startup. See ADR-0004 open question 3. |
| RR-3 | Another local account reads confidential design data from the files themselves | R3 is fixed, so the data directory and database are `0o700`/`0o600`. What remains is R4/G5: the contents are plaintext for anyone who can read them. | Document an encrypted-backup requirement, or add encryption at rest. |
| RR-4 | A tool declares fewer side effects than it performs | A descriptor is a declaration. A lying tool is trusted until review. The `misbehaving` entry points in `tools/simulator.py` are labelled and reviewed; a third-party tool would not be. | Treat tool admission as a review event. Signature verification and a quarantine store are unimplemented (ADR-0006). |
| RR-5 | A domain package is trusted first-party Python and can do more than arithmetic | G1. Evaluators are imported and run in the kernel process. The import boundary and the new implementation digests constrain and detect substitution, but do not render them inert. | Keep the first-party-only posture explicit, or move to the data-only expression model ADR-0002 specified. |
| RR-6 | Local disk fills and committed state is lost or unwritable | G6. No retention policy, no quota check, no `STORAGE_EXHAUSTED` path. Artifacts are never deleted. | Add a retention policy that walks journal references, plus a pre-dispatch free-space check. |
| RR-7 | The API is bound to a routable address | Mitigated but not eliminated. The `serve` command now refuses any non-loopback bind before a socket is created, which is the shipped path. A caller who imports `create_app` and hands it to their own ASGI server bypasses that refusal. | Keep the `serve`-only posture documented; consider binding a Unix domain socket. |
| RR-8 | A proxy-header middleware is added in front of the app | It would rewrite `X-Forwarded-Host` into the `Host` header the guard judges, defeating the guard. The `serve` command serves the unmodified app, so this is a risk for a caller who wraps it themselves. | Keep the module docstring warning; add a startup assertion that no proxy-header middleware is installed. |
| RR-9 | The journal grows without bound and never compacts, and no read model exists | G6. No pruning, no `VACUUM`, no projection tier to rebuild from. | Retention policy; a rebuildable read model. |
| RR-10 | `MemoryManager` is believed to be protecting the system when it is not wired | G8. It is exported from `kernel/__init__.py` and thoroughly tested, which can read as "memory is handled". | Wire it or remove it from the public export. Documented as §3.4 of the architecture report. |
| RR-11 | Engineering users mistake surrogate output for validation | G3. Every evaluation carries a disclaimer, but a JSON report can be screenshotted out of context. | Keep `NON_AUTHORITATIVE_FINDING` in every report; consider a machine-readable `authoritative: false` flag. |
| RR-12 | The declared `requires-python` floor is wrong | **Closed.** R1 is fixed and verified on a real 3.11.16 environment. | None. |
| RR-13 | Coverage regression goes unnoticed | Mitigated. `fail_under = 90` is configured in `[tool.coverage.report]` and the measured 94.76% clears it, but CI is not described in this repository. | Run `coverage report` in CI and treat a non-zero exit as a build failure. |
| RR-14 | The `RLIMIT` leak silently truncates future coverage runs | **Closed.** R2 is fixed; `coverage run -m pytest` completes and writes its data file on both interpreters. | None. |
| RR-15 | Replay and provenance audit are claimed but absent | G6. `docs/architecture-report.md` §3.8 records this; the risk is that a reader of the ADRs alone believes ADR-0003's replay verification exists. | Keep the verification sections of the ADRs and their Implementation status rows in sync with reality. |
| RR-16 | Evaluator substitution is mistaken for evaluator confinement | G1, and the newest risk in this register. Implementation digests and capability enforcement now detect a swapped evaluator and an undeclared capability, which is a real improvement — but a digest identifies code, it does not sandbox it. | Do not let the digest controls be read as closing RR-5. The data-only expression model is the only structural fix. |
| RR-17 | The reference path is read as evidence that the tool plane is on the critical path | G7. The reference workflow calls evaluators in-process, so the tool executor is fully implemented and fully tested but never exercised by the reference loop. | Wire the reference workflow through the executor, or state the boundary in the CLI and API output. |
| RR-18 | The optional FreeCAD adapter is read as a product CAD capability | G9. It is a separate distribution with 301 tests at 100% branch coverage driving a real FreeCAD 1.1.3, and `KalaApplication` does not import it, does not register either tool, and exposes neither over HTTP or the CLI. An installed adapter plus a passing suite can read as "CAD works". | Keep the scope note in `docs/architecture-report.md` §3.2, §10.7, §14.1, and §15, and state the boundary in `init` output if the adapter is ever loaded. |
| RR-19 | A FreeCAD worker runs without filesystem confinement, or is refused on a host that could confine it | G9(b) and §5.2. Both are true at once: the CAD profile's own strict default is **refused** on this host because the executor resolves `sys.executable` to an interpreter that cannot import `kala_agent` before isolating, while the configuration the functional tests exercise is **portable**, which confines a process and not a filesystem. | Fix the probe's interpreter selection (pass the venv interpreter, or make the probe entry point importable without the kernel on the path), then make strict the default for `local_write` tools. Closes the second half of RR-1 for CAD. |
| RR-20 | A discovered CalculiX executable is trusted on structure alone | G9(c). `is_safe_calculix` checks the name pattern, the absence of a path separator, a regular non-empty file ≤ 64 MiB, and an execute bit. That prevents argument injection and path traversal into an invocation. It is **not** a signature, a hash, or a provenance check, and nothing pins which build was discovered. | Pin and verify a solver, or state the structural check as the accepted interim control. Any distribution of a solver raises ADR-0006's D3 quarantine and D1 signature questions first. |
| RR-21 | An LGPL-2.0-only native dependency escapes every quality gate in the repository | G9(d). FreeCAD is installed by the host package manager, not by pip. It is not in either `pyproject.toml`, so `ruff`, `mypy`, `pip-audit`, and any SBOM this repository might generate would not see it, and no supply-chain audit was run. | Document the FreeCAD version and package provenance in the run report, and add an operator-level check on the host package. Redistribution inside the adapter is a different question and is out of scope (ADR-0007 Revisit Trigger). |
| RR-22 | A `solver_unavailable` status is read as an analysis result | G3, G9(a). The FEM tool returns a real FreeCAD analysis document, a material card, a constraint, a mesh object, a solver object, and a status string. A screenshot of that JSON can easily be read as a solved model. `results.available` is `false` and the extrema are `null`, which is the honest answer and is easy to lose out of context. | Keep the status string and the null extrema in every consumer, and keep a non-authoritative marker on the document itself, as the domain packages already carry `NON_AUTHORITATIVE_FINDING`. |

## 9. What "verified" means in this report

- **Verified** means: a named test in this repository exercises the behaviour, the test passes on
  this host on the interpreter named for that suite, and the implementation reference is given so the
  claim can be re-checked. For the root suite that is both 3.11.16 and 3.14.7; for the adapter suite it
  is 3.14.7 only (§7 item 15).
- **Fixed** means: the defect was reproduced, the change is in the source, and the same
  reproduction now shows the corrected behaviour. Sections 6, R1–R3.
- **Not verified** means: no test exercises it, no measurement was taken, or the feature does not
  exist.
- **Not applicable** means: the risk sits outside the application, in the operating system or the
  local account.

Nothing in this report asserts that the system performs, validates, or certifies engineering work.
The domain packages compute bounded, deterministic, non-authoritative arithmetic surrogates, and the
evaluators that compute them are trusted first-party Python rather than the data-only expression
packages ADR-0002 specified. The reference workflow exists to prove the kernel, the journal, the tool
boundary, and the approval machinery — not to produce a design anyone should build.

**And nothing here asserts that the optional FreeCAD adapter performs engineering.** It was verified
to build real geometry, real measurements, real STEP/IGES/FCStd exports, and a real FreeCAD Fem
analysis document through the kernel's own executor on a host with FreeCAD 1.1.3. No solver and no
mesher is installed, bundled, or verified on that host, so the FEM path was verified to report
`solver_unavailable` with `executed: false` and null extrema — not to solve anything. The adapter is
not loaded by the composition root, no HTTP route or CLI command reaches it, and its functional tests
run under the portable profile, which does not confine the host filesystem. It is an integration, not a
capability, and it is not a certification of any design.
