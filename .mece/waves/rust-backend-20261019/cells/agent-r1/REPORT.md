# agent-r1 — the agent runtime in `kala-runtime`

Cell: `agent-r1` of the `rust-backend-20261019` wave. Owner: the Rust worker
`agent-r1`. Status: **complete** — the module is ported, the live differential
against the repository's Python agent is green with **zero unexplained
mismatches** over 36 cases, and every Rust gate passes except the Windows
cross-check, which cannot run in this environment (evidence in §7).

No commit, no push, no delegation, no credential read, and no edit outside
`backend/crates/kala-runtime/` and this file.

---

## 1. What was built

`backend/crates/kala-runtime` — the **accepted** crate of the `gateway-r1`
cell — is extended with a new module `agent`, a complete port of
`src/kala_agent/runtime/agent.py` (1,104 lines) plus the `runtime/__init__.py`
exports it contributes.

The module is the deterministic substrate of ADR-0003:

* a plan is **data**: the model may only propose steps that resolve to the
  step catalog, with declared parameters, and the plan is validated before
  anything runs (D3);
* a tool call goes **only** through the tool executor, and a tool that needs a
  decision gets one from the policy engine (D5);
* the budget is **reserved and settled** by the executor, and the coordinator
  refuses a layer the budget does not admit (ADR-0006);
* the offline planner is deterministic, and the degrading planner only degrades
  when the degradation was approved in advance (ADR-0001, D8);
* the task graph is a bounded, cycle-free DAG, and a task whose dependency
  did not succeed is **cancelled**, never dispatched.

### Changed paths (the only ones I touched)

| Path | What |
|---|---|
| `backend/crates/kala-runtime/src/agent/mod.rs` | the `agent` module and its public surface |
| `backend/crates/kala-runtime/src/agent/error.rs` | the error hierarchy (`AgentError` and its nine families) |
| `backend/crates/kala-runtime/src/agent/value.rs` | the bounded validators, the JSON freeze and the code-point truncation |
| `backend/crates/kala-runtime/src/agent/task.rs` | `AgentTask`, `CatalogStep`, `StepCatalog` and the task graph |
| `backend/crates/kala-runtime/src/agent/outcome.rs` | `AgentStepOutcome` and `AgentRunResult` |
| `backend/crates/kala-runtime/src/agent/planner.rs` | the `Planner` trait, `OfflinePlanner`, `ModelPlanner`, `DegradingPlanner` |
| `backend/crates/kala-runtime/src/agent/coordinator.rs` | `AgentCoordinator` |
| `backend/crates/kala-runtime/src/lib.rs` | the crate root: the new module and the re-exports |
| `backend/crates/kala-runtime/README.md` | the crate documentation |
| `backend/crates/kala-runtime/tests/agent_oracle.py` | the test-only Python oracle |
| `backend/crates/kala-runtime/tests/agent_support/mod.rs` | the shared test helpers |
| `backend/crates/kala-runtime/tests/agent_differential.rs` | the live differential |
| `backend/crates/kala-runtime/tests/agent_semantics.rs` | the semantics tests |
| `backend/crates/kala-runtime/tests/agent_security.rs` | the security tests |

Nothing else in the repository was modified: the root `backend/Cargo.toml`,
the root `backend/Cargo.lock`, every accepted crate (including
`kala-kernel`, `kala-memory`, `kala-tools`, `kala-budget`, `kala-contracts`,
`kala-json`, `kala-persistence` and `kala-repositories`), the Python sources,
the Python tests, the shared fixtures, `adapters/` and `studio/` are
untouched. The crate's own `Cargo.lock` is unchanged by this cell (the new
module adds no dependency).

---

## 2. Public API

The Python module's public surface is reproduced name for name. The Rust
module is `kala_runtime::agent`; the crate root re-exports the types.

### Types

| Python | Rust | Notes |
|---|---|---|
| `AgentTask` | `AgentTask` | `from_parts`, `new`, `with_arguments` / `with_tool_version` / `with_required_capabilities` / `with_depends_on` / `with_intent` / `with_timeout_seconds`, `arguments_json`, `arguments_digest`, `call_id` |
| `CatalogStep` | `CatalogStep` | `from_parts`, `new`, `to_task` |
| `StepCatalog` | `StepCatalog` | `new`, `with_max_steps`, `len`, `is_empty`, `contains`, `step_ids`, `require`, `to_tasks` |
| `AgentStepOutcome` | `AgentStepOutcome` | `from_parts`, `from_tool_result`, `succeeded`, `denied`, `to_payload` |
| `AgentRunResult` | `AgentRunResult` | `from_parts`, `succeeded`, `step_ids`, `artifacts`, `outcome_for`, `to_payload` |
| `Planner` (Protocol) | `Planner` (trait) | `plan` (async) and `degradations` |
| `OfflinePlanner` | `OfflinePlanner` | `new`, `catalog`, `fallback_step_id` |
| `ModelPlanner` | `ModelPlanner` | `new`, `catalog`, `gateway`, `max_tokens`, `response_schema`, `plan_steps` |
| `DegradingPlanner` | `DegradingPlanner` | `new`, `allow_degraded`, `degradations` |
| `AgentCoordinator` | `AgentCoordinator` | `new`, `with_policy` / `with_planner` / `with_ledger` / `with_gateway` / `with_catalog` / `with_max_parallel_tasks` / `with_clock`, the accessors, `plan`, `run`, `run_plan`, `run_objective`, `requires_approval` |
| `AgentError` and its four subclasses | `AgentError` (one enum) | `base`, `plan`, `policy`, `merge`, `planner_failure`; `python_class_name()` returns the Python class name |
| `AgentPlanError`, `AgentPolicyError`, `AgentMergeError`, `PlannerFailure` | documented type aliases of `AgentError` | the Python names stay available |

### Functions and constants

The Python-private helpers the repository's own tests import are public in
Rust under their unprefixed names: `_bounded_identifier` →
`bounded_identifier`, `_bounded_text` → `bounded_text`, `_bounded_seconds` →
`bounded_seconds`, `_identifier_tuple` → `identifier_tuple`, `_bounded_reasons`
→ `bounded_reasons`, `_freeze_value` → `freeze_value`, `_freeze_object` →
`freeze_object`, `_thaw` → `thaw`, `_thaw_object` → `thaw_object`,
`_plan_parameters` → `plan_parameters`, `_validate_task_graph` →
`validate_task_graph`, `_dependency_order` → `dependency_order`,
`_dependency_layers` → `dependency_layers`.

`MAX_AGENT_TASKS` (64), `MAX_PARALLEL_TASKS` (8), `MAX_MERGED_RESULT_BYTES`
(4,194,304), `MAX_BOUNDED_TEXT` (512), `MAX_ERROR_TEXT` (1,024),
`MAX_ARGUMENT_DEPTH` (16), `APPROVAL_REQUIRED_EFFECTS` and
`NEVER_DEGRADED_ERRORS` (the four Python class names).

### Accepted crates consumed read-only

`kala-kernel` (the policy engine — including the **D1 `actor_id` fix**, which
the port inherits because it calls the accepted `PolicyEngine::decide` and
`PolicyEngine::approve`; the fix is the `approval.actor_id().cloned()` that
the allowed branch passes to `PolicyDecision::new` at
`backend/crates/kala-kernel/src/policy.rs:1736`, with the sibling
invalidation branch at `:1724`),
`kala-tools` (the protocol, the registry, the executor and the simulator),
`kala-budget` (the ledger), `kala-contracts` (the plan step, the tool call,
the tool result, the run status and the side-effect level), `kala-json` (the
canonical bytes and the digests).

**`kala-memory`, `kala-persistence` and `kala-repositories` are *not*
consumed, because `kala_agent.runtime.agent` does not use them.** The import
list of the Python module is the evidence:

```
$ sed -n '1,40p' src/kala_agent/runtime/agent.py
from kala_agent.contracts import PlanStep, SideEffectLevel, ToolCall, ToolResult, ToolRunStatus
from kala_agent.kernel.budget import BudgetExhaustedError, BudgetLedger
from kala_agent.kernel.policy import PolicyEngine
from kala_agent.runtime.model_gateway import (...)
from kala_agent.tools.executor import ToolExecutor
from kala_agent.tools.protocol import (...)
from kala_agent.tools.registry import RegisteredTool, ToolRegistry, ToolRegistryError
```

There is no memory, persistence or repository import, and no memory write, no
journal write and no database access anywhere in the module. The wave brief's
"memory state" comparison therefore has **no** counterpart in this cell: the
Python module has no memory state to compare. This is reported rather than
worked around, and the accepted `kala-memory` crate was not patched.

---

## 3. The live differential

### 3.1 The scripted mock-model differential (`tests/agent_differential.rs`)

The Python agent and the Rust agent are driven with **the same scenario**: the
same step catalog, the same tasks, the same scripted model provider (a
deterministic in-process double that answers with a fixed structured payload),
the same tool registry (the shipped simulator tools: the pure reference
evaluator and the side-effecting simulator), the same budget and the same
policy engine.

The oracle (`tests/agent_oracle.py`) imports the repository's own
`kala_agent.runtime.agent` and runs with the repository's
`.venv/bin/python`. It is a **test artefact only**: the crate never invokes
Python at runtime.

**36 cases**, covering:

| Group | Cases |
|---|---|
| dispatch | the pure evaluator succeeds, two independent tasks run in one layer, a chained dependency runs after its predecessor, a failed dependency cancels the dependent step, an unknown tool is denied, an undeclared capability is denied, a task with a valid timeout runs, a task with an invalid timeout is refused |
| budget | the budget denies the whole layer, the budget denies a single step, the executor reserves and settles the budget of a call |
| policy | a side-effecting tool without a policy engine is a hard failure, a side-effecting tool with a policy engine is denied and never dispatched |
| planning | the offline planner scores the catalog, the offline planner uses its fallback, the offline planner reports a total miss, a dotted keyword scores the same tokens, an empty objective is refused |
| the model planner | a valid answer becomes tasks, a two-step answer becomes two tasks, an empty intent becomes "planned step", an unknown catalog step is refused by the schema, a malformed answer is refused by the schema, a plan with too many steps is refused by the bound |
| degradation | an unapproved degradation is refused, a provider failure degrades to the deterministic path, a planner failure degrades, and the four authoritative refusals (policy, egress, capability, budget) **never** degrade |
| plans | a declared plan runs, an undeclared parameter is refused, an undeclared dependency is refused, an unsafe run identifier is refused, a duplicate task identifier is refused |

Compared, byte for byte over canonical JSON:

* the **decisions** — the outcome of every case, the Python exception class
  name and the exact message of every failure;
* the **events** — the whole run result payload: every outcome (status,
  dispatch flag, error code, error message, policy reason, result and
  artifact identifiers), the merged results, the digest of every merged
  result, the degradation reasons and the budget refusals;
* the **tool calls made** — the execution reports of the executor (call
  identifier, tool identifier and version, status, error code, granted
  capabilities, artifact bytes);
* the **budget ledger** — the whole exported ledger state (budget, usage,
  reserved, reservations, next reservation);
* the **final results** — the payload of the run.

**Result: zero mismatches.** The two documents are identical.

Two fields are normalized away on *both* sides, and only because they are
measurements and not decisions:

1. the measured `duration_ms` of a tool call, and the measured
   `duration_seconds` of the ledger usage;
2. the *order* of the execution reports — the reports are appended when a
   call finishes, so their order is a race between the workers of a layer on
   both sides. The reports are sorted by their call identifier, so the *set*
   of calls is compared.

### 3.2 The Python oracle

`tests/agent_oracle.py` is the live oracle: it imports the repository's own
`kala_agent.runtime.agent` with the repository's `.venv/bin/python`, exactly
like the accepted cells do. Python is a **test oracle only**.

### 3.3 The Python tests that were read in full

`tests/unit/test_agent_branches.py` (all 1,319 lines), `tests/unit/test_runtime.py`,
`tests/unit/test_model_gateway_branches.py`,
`tests/integration/test_mechanical_loop.py` and
`tests/security/test_model_residency.py`.

The repository's own Python suite is unchanged and green:

```
$ .venv/bin/python -m pytest tests/unit/test_runtime.py tests/unit/test_agent_branches.py \
      tests/unit/test_model_gateway_branches.py tests/integration/test_mechanical_loop.py \
      tests/security/test_model_residency.py -q
218 passed, 1 warning in 2.90s
```

---

## 4. Test counts and the gates

```
cargo fmt --check                                          → clean
cargo clippy --locked --all-targets --all-features -- -D warnings → clean
cargo test  --locked            → 112 passed, 0 failed
cargo test  --locked --release  → 112 passed, 0 failed
cargo build --locked --release  → ok
cargo check --target x86_64-pc-windows-gnu            → FAILS (environment, see §7)
```

The **112** tests are:

| Binary | Tests | What |
|---|---|---|
| `tests/agent_semantics.rs` | 44 | every public function of the agent module: the bounded validators, the JSON freeze, the task, the catalog step, the step catalog, the plan parameters, the task graph, the dependency order and layers, the three planners, the step outcome, the run result and the coordinator |
| `tests/agent_security.rs` | 11 | the agent controls: the bounded and validated plan, the closed vocabulary of the response schema, the undeclared parameter, the unknown catalog step, the policy checkpoint, the tool that needs an approval without a policy engine, the pure read that needs none, the budget reservation and settlement, the budget denial of a layer, the absence of an approval replay and the bound of the merged results |
| `tests/agent_differential.rs` | 1 | the 36-case live differential against the repository's Python agent |
| `tests/gateway_semantics.rs` | 37 | the gateway (unchanged by this cell) |
| `tests/gateway_security.rs` | 14 | the gateway residency controls (unchanged) |
| `tests/python_differential.rs` | 2 | the gateway mock-provider differential (unchanged) |
| `tests/corpus_differential.rs` | 1 | the gateway randomized corpus (unchanged) |
| the crate doc tests | 2 | the crate example and the new `agent` module example compile and run |

The gateway cell's **55** tests stay green; this cell adds **56** tests and
one doc test.

---

## 5. Compatibility and security evidence

**Fidelity.** The validation messages, the error precedence, the canonical
argument bytes, the digests, the merged results, the degradation reasons, the
budget ledger bytes and the tool calls are identical to the Python
implementation — that is what the 36-case differential compares, and it is
green with zero mismatches.

**The error precedence is preserved.** The Python dataclasses validate their
fields in a fixed order, and the Rust constructors reproduce it exactly:

* `AgentTask`: `step_id` → `tool_id` → `tool_version` → `arguments` → the
  self-dependency check → `required_capabilities` → `depends_on` → `intent` →
  `timeout_seconds`;
* `CatalogStep`: `catalog_step_id` → `tool_id` → `tool_version` →
  `default_arguments` → `required_capabilities` → `keywords` → `intent`;
* `AgentStepOutcome`: `step_id` → `call_id` → `tool_id` → the status → the
  dispatch flag → the result → `artifact_ids` → `error_message` →
  `policy_reason`;
* `AgentRunResult`: `run_id` → the outcomes → `merged` → the digests → the
  degradation reasons → the budget refusals;
* the coordinator: the graph → the budget admission → the blocked dependency
  → the tool resolution → the capability check → the policy decision → the
  dispatch → the merge.

**The security properties of the wave brief**, and where each one is proven:

| Property | Evidence |
|---|---|
| *an LLM plan is bounded and validated before execution* | `tests/agent_security.rs::a_plan_that_names_an_unknown_catalog_step_never_runs` (the response schema is the closed vocabulary of the catalog and publishes the bound of 64 steps), `::a_plan_with_an_undeclared_parameter_is_refused_before_the_run` and `::a_task_that_is_not_in_the_catalog_is_refused` (both assert that the executor recorded **no** call), and the differential cases `model-planner-unknown-step`, `model-planner-malformed`, `model-planner-too-many-steps`, `run-plan-undeclared-parameter`, `run-plan-undeclared-dependency` |
| *tool calls go only through the executor with policy approval* | `tests/agent_security.rs::a_side_effecting_tool_without_a_policy_engine_never_runs` (the call is a hard `AgentPolicyError` and the executor recorded no report) and `::a_side_effecting_tool_pauses_for_an_approval` (the call is denied, the reason carries the policy decision, and the executor recorded no report). The coordinator reaches a tool only through `ToolExecutor::execute`; there is no other dispatch path in the module |
| *the budget is reserved and settled* | `tests/agent_security.rs::the_budget_reserves_and_settles_the_call` (the ledger records one tool call, nothing stays reserved, no reservation is left open) and `::an_exhausted_budget_denies_the_whole_layer` (the layer is denied, `budget_denied` names the step, the executor recorded no report). The differential compares the whole exported ledger state byte for byte in three cases |
| *checkpoints pause for approval* | `tests/agent_security.rs::a_side_effecting_tool_pauses_for_an_approval` — the outcome is `denied`, `dispatched` is false, and the `policy_reason` names the decision and the request. The decision is the *pause*: the tool is not dispatched |
| *no approval is replayed* | `tests/agent_security.rs::an_approval_does_not_cover_another_action` — the policy engine is given two requests with different action digests, the first is approved, and the second is still pending. The agent's approval path is the kernel's, and the action digest binds the arguments, the workspace, the capabilities and the tool |
| *memory writes are bounded* | **not a property of this module** — `kala_agent.runtime.agent` has no memory dependency and performs no memory write (the import list is in §2). Nothing to bound and nothing to compare; the accepted `kala-memory` crate was not consumed and not patched |

**The bounds.** The task count (64), the parallel task count (8), the
argument nesting depth (16), the bounded text (512 code points), the error
text (1,024 code points), the merged result size (4 MiB) and the timeout
(3,600 s) are the Python bounds and are enforced in the same places.

**No stack overflow on hostile input.** The freeze of a task argument walks
the document recursively, but the depth is checked **before** the recursion
and the bound is 16, so the recursion is at most 18 frames deep. A document
that nests deeper is refused with `task arguments exceed the supported
nesting depth` (proved by
`tests/agent_semantics.rs::the_freeze_refuses_an_empty_key_and_a_deep_document`).

**No unbounded work.** The coordinator dispatches at most
`MAX_PARALLEL_TASKS` tasks at a time, the task graph is bounded, and the
model planner's plan is bounded by the published schema and by the catalog.

---

## 6. Intentional differences

Every difference below is deliberate and was checked against the Python
behaviour.

1. **The planner is asynchronous.** The Python `Planner.plan` is synchronous;
   the accepted Rust `ModelGateway::complete` is asynchronous (the HTTP stack
   is `reqwest`). The `Planner` trait is therefore `async fn plan`, and
   `AgentCoordinator::plan` and `AgentCoordinator::run_objective` are
   `async`. `AgentCoordinator::run` stays synchronous, because the tool
   executor is synchronous. Every message, precedence and byte is unchanged.
2. **One error enum instead of an exception hierarchy.** The five Python
   classes of the module (`AgentError`, `AgentPlanError`, `AgentPolicyError`,
   `AgentMergeError`, `PlannerFailure`) and the four foreign families (the
   model, budget, policy, registry, executor and protocol errors) are the
   variants of one `AgentError` enum. `AgentError::python_class_name()`
   returns the exact Python class name, so a differential oracle can compare
   the class name as well as the message, and the differential does.
3. **The type checks are compile-time.** The Python validators accept any
   object and reject a non-string with `"{field} must be a string"`, a
   non-number with `"{field} must be a number"`, a non-collection with
   `"{field} must be a collection of identifiers"`, and a non-`CatalogStep`
   with `TypeError("catalog steps must be CatalogStep values")`. The Rust
   validators take the typed value, so those five messages are unreachable.
   The *value* checks — `must be a safe identifier`, `exceeds the bounded
   length`, `must be positive and bounded`, `must be a JSON object`,
   `must contain finite numbers`, `must be nonempty strings`, `exceeds the
   supported nesting depth`, `the plan exceeds the bounded step count`, `a run
   cannot exceed 64 tasks` and the rest — are reproduced exactly, including
   the message text and the precedence.
4. **A task is built from typed fields.** The Python `AgentTask` is a
   dataclass with keyword arguments; the Rust `AgentTask::from_parts` takes
   the same fields in the same validation order, and the `with_*` methods
   mirror `dataclasses.replace` (they re-run the whole validation).
5. **The catalog keeps its registration order.** The Python catalog is a
   dictionary and publishes `tuple(selected)`; the Rust catalog is a vector
   and publishes `step_ids()` in the same order. The order is observable: the
   offline planner scores the entries in that order and the response schema
   publishes the entry identifiers as its enum.
6. **The merged results are a sorted object.** The Python merged mapping
   keeps the insertion order of the outcomes; the canonical object of
   `kala-json` is sorted by code point. The canonical JSON encoding of both
   is identical, and the differential compares the canonical bytes.
7. **The freeze is a check, not a copy.** The Python `_freeze_value` returns
   a read-only proxy and a tuple; the canonical value model of `kala-json` is
   already immutable, so the Rust freeze only performs the three checks
   (depth, finite numbers, nonempty keys).
8. **The parallelism uses scoped threads.** The Python dispatch uses a
   `ThreadPoolExecutor` with `max_workers=min(max_parallel, len(tasks))` and
   `pool.map`; the Rust dispatch uses `std::thread::scope` with the same
   worker count and the same "every task of the layer runs, the first error
   in task order is reported" behaviour. The *order* of the execution
   reports is a race on both sides, so the differential sorts them.
9. **The degradation reasons are read through the trait.** The Python
   coordinator reads them with an `isinstance(planner, DegradingPlanner)`
   check; the Rust `Planner` trait has a `degradations()` method that only
   `DegradingPlanner` overrides, so the behaviour is identical (a nested
   degrading planner reports its own reasons, exactly like the Python
   `isinstance` check).
10. **The digest of a merged result is a pair.** The Python `digests` is a
    mapping; the Rust `digests()` is a sorted list of `(step_id, digest)`
    pairs, because the canonical object is sorted and a mapping would lose
    the order. `to_payload()` renders the same mapping shape on both sides.
11. **The dynamic class name of the Python test is unreachable.** The Python
    test `test_degrading_planner_truncates_long_reasons` builds a refusal
    whose *class name* is 2,000 characters long, to make the degradation
    reason long. Rust has no dynamic class name, so the Rust test makes the
    reason long through the provider identifier of a degraded provider, which
    the reason quotes. The truncation (`reason[:MAX_ERROR_TEXT]`, counted in
    code points) and the bound are the same.
12. **The Python monkey-patch of `strict_json_loads` is unreachable.**
    `test_agent_step_outcome_rejects_a_non_object_payload` replaces the
    module-level `strict_json_loads` with a function that returns a list, to
    reach the `AgentMergeError("a tool result payload must be a JSON object")`
    branch. The Rust contract already carries a parsed JSON object, so the
    decode cannot fail and the branch is unreachable. The refusal is kept in
    the code for the shapes the contract admits.
13. **The merged-result size refusal is a protocol error.** The Python
    `canonical_json_dumps(..., max_bytes=MAX_MERGED_RESULT_BYTES)` raises a
    `JsonEnvelopeError` (not an `AgentError`) when the merged document is too
    large, so the `AgentMergeError("the merged results exceed the bounded
    size")` branch below it is dead code in Python. The Rust port reproduces
    the same precedence: the protocol error is reported first, and the merge
    error remains as the last line of defence.
14. **The Windows cross-check cannot run here.** The target
    `x86_64-pc-windows-gnu` **is** installed, but the mingw-w64 C toolchain
    is not, and the `ring` crate (a rustls dependency) needs it:
    ```
    $ cargo check --target x86_64-pc-windows-gnu
       Compiling ring v0.17.14
       error occurred in cc-rs: failed to find tool "x86_64-w64-mingw32-gcc"
    ```
    The **accepted `kala-studio` crate fails identically** in this
    environment, so this is a missing system toolchain and not a defect of
    this crate. No Windows runtime was exercised and no Windows claim is
    made.
15. **No runtime cutover.** The Python runtime is still the product. The Rust
    crate is a library; the composition root (`application.py`) and the API
    layer still build the Python agent. No speed, readiness or deployment
    claim is made.

### Remaining gaps

* The Rust `AgentCoordinator` is a library: no composition root, no CLI and
  no HTTP route uses it yet. The `application.py` composition root and the
  API layer still build the Python agent.
* The Python `AgentCoordinator` exposes private methods (`_merge`,
  `_preflight`, `_dispatch`) that the repository's own tests call
  (`test_coordinator_rejects_a_merge_schema_violation`,
  `test_coordinator_rejects_a_duplicate_merged_result`,
  `test_coordinator_dispatch_requires_no_approval_for_pure_reads`). The Rust
  port keeps `merge`, `preflight` and `dispatch` **private**, so those three
  cases are covered by the differential and by the public behaviour rather
  than by a private call. `requires_approval` is public, because it is a
  property of the tool and not of a private method.
* `kala_agent.runtime.agent` writes no journal and no database record, so
  the Rust port writes none either. The journal seam belongs to the
  composition root, exactly as it does in Python.
* The model planner publishes the response schema but the *provider* is the
  only thing that can answer it; the `LocalModelClient` of the gateway is a
  deterministic test double, not a language model.
* The `AgentCoordinator` does not journal the plan itself. The Python one
  does not either: the plan is data that the caller hands to `run`.

---

## 7. How to reproduce

```bash
cd backend/crates/kala-runtime
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
```

The differential test needs the repository virtual environment
(`<repo>/.venv/bin/python`); `KALA_PYTHON` overrides it. The test opens no
outbound connection: the provider double is in-process and the tools are the
shipped simulator tools, which the pure reference evaluator runs inside the
kernel process.
