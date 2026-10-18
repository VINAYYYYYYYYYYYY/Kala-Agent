# How to read this codebase

Read this repository as two products that share a vocabulary, not as one file from top to bottom. The core is a local engineering-design agent under `src/kala_agent/`. The optional FreeCAD adapter under `adapters/freecad/` is a separate package that the core never imports.

The module docstrings are the map. Almost every file starts by stating what it owns, what it refuses to do, and which other layers it may touch. Read that paragraph before any function.

## How to read one file

1. Read the module docstring. It is the contract.
2. Read the public types and the public class methods. Names that start with `_` are helpers for those methods. Skip them until a public method calls one you do not understand.
3. Open the matching test and read one test that calls the method you just read. The test shows the legal inputs and the failure the code is written to catch.
4. When a line imports another module, stop and read that module's docstring, then come back. Do not follow every import to the bottom.

`application.py` is about 3,000 lines and is the worst place to start. It only wires pieces that already exist.

## Read the core in this order

Follow one mechanical design from a typed parameter set to a committed revision. That path touches every layer that matters.

| Order | Where | What you are learning |
|---|---|---|
| 1 | `docs/architecture-report.md`, sections 1 and 2 only | What exists, and the three limits: domain math is a surrogate, evaluators are trusted Python, the default sandbox does not confine the host filesystem. |
| 2 | `src/kala_agent/contracts/models.py` | The nouns: design spec, revision, workflow, approval, artifact, tool call. Later files only pass these records around. |
| 3 | `src/kala_agent/config.py` | How a data directory and settings are validated. |
| 4 | `src/kala_agent/domains/__init__.py`, then `domains/mechanical/parameters.py`, `evaluation.py`, `workflow.py` | A domain declares parameters, catalog steps, and a workflow. It computes bounded arithmetic. It must not import `kernel`, `tools`, `persistence`, `runtime`, `api`, or `cli`. |
| 5 | `src/kala_agent/persistence/database.py`, then `journal.py`, `artifacts.py`, `repositories.py` | SQLite is the record. Events have one order. Revisions are immutable. Artifact files are named by their SHA-256 digest. |
| 6 | `src/kala_agent/kernel/policy.py`, `budget.py`, `workflow.py`, `planning.py` | The kernel decides checkpoints, budgets, and workflow steps. It must not import `domains`, `runtime`, or `tools`. |
| 7 | `src/kala_agent/tools/protocol.py`, `registry.py`, `executor.py` | A tool is a typed contract that runs in a child process. The reference workflow does not dispatch one. |
| 8 | `src/kala_agent/runtime/model_gateway.py`, then `runtime/agent.py` | The model may propose a bounded plan. It does not own control flow or tool authority. The default client is offline and deterministic. |
| 9 | `src/kala_agent/application.py`, class `KalaApplication` only | The only place that wires the layers. Start at `run_mechanical_reference_workflow`, `prepare_design_revision`, and `commit_design_revision`. |
| 10 | `src/kala_agent/cli/main.py` and `src/kala_agent/api/app.py` | Thin adapters over `KalaApplication`. Read them last. |

`docs/adr/0001` through `0007` explain why those boundaries exist. Read an ADR when a boundary surprises you, not before the code.

## The FreeCAD adapter slice

Read this slice on its own, after the core vocabulary:

1. `src/kala_agent/contracts/construction.py` — the script the adapter is allowed to accept.
2. `adapters/freecad/src/kala_agent_freecad/cad/models.py` — the adapter's records.
3. `procedure.py`, `design.py`, `compose.py`, `accept.py` — turn a script into a design and decide whether to accept it.
4. The test next to each file (`adapters/freecad/tests/test_procedure.py`, `test_design.py`, `test_compose.py`, `test_accept.py`, and `tests/unit/test_construction.py`).

`docs/adr/0007-optional-freecad-cad-cae-fem-adapter.md` states that this adapter is not part of the product composition root.

## What to ignore

Skip `__pycache__`, `.mypy_cache`, `.ruff_cache`, and `.pytest_cache`. They are tool output. Skip `docs/architecture-report.md` after section 2 until a specific claim in the code disagrees with what you just read. The rest of that report is evidence, not an introduction.
