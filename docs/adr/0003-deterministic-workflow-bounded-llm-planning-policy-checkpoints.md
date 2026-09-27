# ADR-0003: Deterministic Workflow Engine with Bounded LLM Planning and Policy Checkpoints

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-25 |
| Last reviewed | 2026-09-26 |
| Scope | Control flow, planning, human-in-the-loop |
| Requirements addressed | FR-06 policy checkpoints, FR-15 deterministic workflow with bounded LLM planning |
| Related ADRs | ADR-0001, ADR-0002, ADR-0004, ADR-0005, ADR-0006 |
| Implementation status | **Largely implemented, with three named gaps.** `kernel/workflow.py` executes a bounded state machine over a versioned `WorkflowDefinition` with per-step retry bounds, replan bounds, and digest-bound checkpoints. `kernel/planning.py` validates plans with 13 rejection codes. `kernel/policy.py` evaluates capability requests and produces 6 checkpoint kinds. `kernel/budget.py` bounds model calls, tool calls, tokens, artifact bytes, duration, cost, and replanning. Gaps: **(1)** the shipped mechanical workflow reaches only `PRE_COMMIT` — no `PLAN_APPROVAL` checkpoint is raised on that path; **(2)** the per-checkpoint policy of `required` / `recommended` / `auto-within-scope`, resolved from a domain overlay, is **not** implemented — there is no overlay and no strictest-wins resolution; **(3)** the replay guarantee in D8 is **not** delivered: there is no replay driver and no read-model projection, so a run is *reproducible* but not demonstrably *replayable*. **Hardening since the last review:** `TOOL_CLASS_CHANGE` is genuinely enforced rather than merely declared — `PolicyRequest.tool_class_change` is `True` only for that kind (`kernel/policy.py:345`) and `ToolExecutor._policy_request` raises it on every non-egress call (`tools/executor.py:1123`) — and durable kernel state now lives under a **reserved `kernel-state:` namespace**, so a user-supplied `workflow_id` can neither name nor collide with the aggregate holding the policy or budget snapshot (`_is_reserved_identifier`; `RESERVED_WORKFLOW_IDS`). Restart restore is stricter: missing kernel state in an **already-used** directory stops the launch rather than resetting to defaults, while a **fresh** directory restores defaults with no record at all. |
| Verification | 1,227 tests pass on CPython 3.11.16 and CPython 3.14.7; branch coverage 94.76% / 94.77% against a configured `fail_under = 90`; `kernel/workflow.py` 97.76%, `kernel/planning.py` 99.37%, `kernel/policy.py` 90.16%, `kernel/budget.py` 86.00%. Checkpoint durability and stale-decision invalidation are covered by 43 tests across `tests/security/test_policy.py` (20) and `tests/security/test_revision_commit.py` (23). See `docs/verification-report.md` §2. |

## Context

An engineering design agent must do two things that pull in opposite directions: it must be
genuinely adaptive, because the right sequence of analysis steps depends on the design and on what
the user asked for; and it must be reproducible, because an engineering result that cannot be
re-derived is not trustworthy and cannot be audited.

The dominant current practice is to hand the whole loop to a language model: the model decides what
to do, calls tools, interprets results, and continues. This is flexible and easy to demo, and it is
also non-deterministic in its control flow, opaque at the point of consequential action, impossible
to replay exactly, and impossible to budget. For work that ends in a dimensioned, committed design
revision, that trade-off is wrong.

The counter-extreme — a fully fixed pipeline with no model involvement — is deterministic and
auditable, but it cannot respond to "what if I thicken this flange?", cannot interpret a surprising
analysis result, and cannot help the user formulate the problem. It is a calculator, not an agent.

Kala Agent also needs an explicit human control point. Some transitions must never happen without
explicit approval: agreeing a plan before work begins, introducing a tool class the user has not
used before, sending anything off the device, and committing an immutable revision. Automatic
progress past those points is how an agent silently produces a design the user did not sanction.

The planning surface must also be bounded. Whatever the model produces, the system must be able to
say exactly what is permitted to run, check it before execution, and refuse it. "Bounded" has to be
structural — a closed vocabulary and a strict schema — not a matter of prompt wording.

## Decision

**D1 — The workflow is a versioned, declarative state machine executed by the kernel.** A workflow
instance is a state machine whose transitions are defined in a versioned workflow definition
contributed by a domain package (ADR-0002). The kernel executes transitions. The model does not
control flow. The definition names the legal states, the legal transitions, the step catalog
available in each state, and the checkpoint required at each boundary.

**D2 — Steps are pure with respect to kernel state; all effects are journaled.** A step receives
typed inputs, a snapshot of the design graph, and a capability set; it returns a typed result. The
kernel applies the result, and the step's invocation and result are journaled. Two executions of the
same step against the same state, with the same tool versions and seed, produce the same result.
Time, randomness, and process identity are injected by the kernel, never read ambiently.

**D3 — Bounded LLM planning: a plan is data, validated before execution.** The model may produce
only a *plan object* conforming to a strict schema:

- an ordered list of steps, each drawn from the domain package's step catalog by identifier;
- typed parameters for each step, within per-step declared bounds;
- declared intent, expected outputs, and a budget request;
- an explicit list of capabilities the plan requires.

A plan is rejected unless it validates against the schema, resolves entirely to catalog steps,
stays within bounds, and requests no capability outside the session's grant. Rejection reasons are
journaled and surfaced. The model cannot emit code, a shell string, a path, a tool not in the
catalog, or a step outside the current state. A plan is advisory input to the state machine: it may
propose a path the machine is willing to take, and the machine decides.

**D4 — Planning is iterative, budgeted, and never unbounded.** A state may admit at most a
configured number of replanning attempts. Each attempt consumes from the session budget (ADR-0006).
Exhausting the replanning allowance is not a failure; it is a checkpoint that asks the user to
choose. There is no unbounded retry loop anywhere in the kernel, and a step that fails retries only
under a fixed, journaled policy.

**D5 — Policy checkpoints are first-class states, not prompts.** Five checkpoint types are defined
in the MVP: `PLAN_APPROVAL` (before execution begins), `TOOL_CLASS_CHANGE` (a step would use a tool
class the session has not been granted), `EGRESS` (anything would leave the device),
`PRE_COMMIT` (before an immutable revision is written), and `EXPORT` (before a design binary is
released externally). `PROVIDER_DEGRADED` is added by ADR-0001 D8. Each has a policy of `required`,
`recommended`, or `auto-within-scope`, resolved from the domain package overlay and the user
preference, with the kernel enforcing the strictest applicable setting.

**D6 — A checkpoint is a persisted decision, not a console prompt.** Resolving a checkpoint writes
an immutable record to the journal: the checkpoint type, the state and revision it applies to, the
plan or action digest, the options presented, the decision, any user-supplied modification, the
timestamp, and the acting identity. Pending checkpoints survive process exit and crash. On resume,
the kernel re-presents the checkpoint and validates that the decision still applies to the current
state digest; a stale decision is invalidated and must be retaken.

**D7 — The permitted modification surface at a checkpoint is typed.** A user may approve, reject,
abort, or supply a *typed* parameter modification that the validator re-checks. A checkpoint cannot
be used to inject an unvalidated instruction into the next step. This keeps the human in control
without reopening the arbitrary-code hole that a free-text override would create.

**D8 — Determinism is scoped and stated, not overclaimed.** The kernel and every step are
deterministic given `(design state, plan, tool versions, package versions, seed)`. Model output is
treated as recorded input, not as a deterministic function: replanning is an explicit new event with
a new transcript, not a transparent re-execution. The system can therefore guarantee *replay* of a
recorded run exactly, and *reproducibility* of a run only when the same recorded plan is reused.
Documentation and the API must use these two words in this sense and must not claim that a fresh
run reproduces an old one.

## Consequences

### Positive

- Every consequential action is preceded by a persisted, auditable human decision, which is the
  core safety property for work that produces a real design.
- Control flow is inspectable: the legal transitions are in a versioned definition, not in a prompt.
- A plan is checkable before it runs, so a model cannot widen its own authority.
- Recorded transcripts plus a journal make any run exactly replayable, which is what engineering
  review and dispute resolution need.
- Budgets and replanning limits make runaway behaviour structurally impossible rather than
  merely discouraged.
- The kernel stays domain-independent: the step catalog is data from the domain package, so
  ADR-0002's boundary holds without exceptions for planning.
- Typed checkpoint modifications give the user meaningful control while keeping validation intact.

### Negative

- Latency increases at every checkpoint, and the user is interrupted more often than in a fully
  autonomous agent.
- The step catalog constrains what can be planned, so a request outside the catalog cannot be
  served by improvisation; the user gets a refusal rather than an attempt. This is a real loss of
  flexibility, accepted in exchange for checkability.
- Persisting every checkpoint adds latency and schema surface, and stale-decision invalidation is a
  subtle correctness area that needs careful tests.
- Strict determinism requires discipline everywhere: injected time and randomness, no reliance on
  dict ordering or filesystem traversal order, and pinned tool versions. Any violation is a
  reproducibility bug, not a cosmetic one.
- The distinction between replay and reproducibility (D8) is easy to overstate in conversation and
  in docs, and users may initially expect stronger guarantees than are claimed.
- Bounded replanning means some genuinely hard requests terminate in a checkpoint asking the user
  what to do, which is slower than a model that keeps trying.
- The plan schema is a stability surface; changing it requires care because plans are persisted.

### Neutral

- Plans are persisted artifacts and are themselves subject to retention and digest rules.
- Checkpoint policy is configuration, so a user can reduce interruptions for low-risk work at the
  cost of weaker oversight; the kernel still enforces the strictest applicable policy.
- Deterministic steps still may call tools, so "deterministic" describes the engine's control flow,
  not the hermeticity of external solvers.

## Alternatives Considered

**Model-driven control loop (the model decides the next action, as in most current agent
frameworks).**
- Rejected: non-deterministic control flow, no pre-execution check on actions, no reliable place to
  put a mandatory approval gate, and poor replay. This is the central decision being made here.

**Fully fixed pipelines with no model involvement, exposed as selectable workflows.**
- Rejected as the whole answer: it cannot adapt to the request, interpret results, or help formulate
  the problem. Retained as the deterministic substrate inside D1, and used as the fallback path
  when a provider is unavailable (ADR-0001 D8).

**Bounded planning, but letting the model emit a tool call sequence directly under a tool-use
protocol.**
- Rejected: it re-enters the model-driven loop with the same auditability problem. Our plan object
  resolves to catalog steps before execution, which is what makes pre-execution validation possible.

**Human approval on every step rather than at defined checkpoints.**
- Rejected: it makes the system unusable for multi-step work while adding little safety, since the
  consequential transitions are few and identifiable.

**A sandboxed "plan sandbox" where the model may run arbitrary steps and only final results are
checked.**
- Rejected: it defers validation to the point where damage is done, and it makes rollback and
  provenance much harder. Side effects must be capability-checked at the point of execution, not
  filtered afterwards.

**Free-text user instructions injected into subsequent steps as overrides.**
- Rejected: reintroduces arbitrary instruction-following into the load-bearing path. Typed,
  re-validated modifications (D7) preserve control without that.

**Adopt an existing workflow/state-machine engine (e.g. Temporal, or a Python state-machine
library) as the engine.**
- Deferred: a server-oriented engine is a poor fit for a local single-user process, and a library
  would still leave the state machine definition and checkpoint policy to us. Revisit if workflow
  durability requirements outgrow the local journal; the definition format is kept portable so
  this remains possible.

**Event sourcing alone, with the model reacting to events.**
- Rejected: events are our record of what happened (ADR-0004), but the ordering and legality of
  transitions still need an explicit state machine; deriving control flow from the event stream
  would make legality implicit and uncheckable.

## Trade-offs

- We accept more user interruption and higher latency in exchange for guaranteed human control over
  consequential actions.
- We accept reduced flexibility — refusals instead of improvisation outside the step catalog — in
  exchange for a plan that can be validated before anything runs.
- We accept the ongoing discipline of strict determinism, and the risk of reproducibility bugs, in
  exchange for exact replay of any recorded run.
- We accept a weaker guarantee than "same input, same output" across fresh runs, and state it
  precisely rather than overclaiming it.
- We accept the extra schema surface of persisted plans and checkpoints in exchange for crash-safe
  resumability of human decisions.

## Revisit Triggers

- A domain requires workflow topologies the shared state machine cannot express.
- Users find checkpoint frequency materially obstructive in practice, suggesting the checkpoint set
  or the `required`/`recommended` defaults need revision.
- A reproducibility defect traced to ambient state in the kernel indicates D2 needs strengthening
  rather than relaxing.
- A demand arises for genuinely autonomous unattended operation; this would require a different
  approval model and should be a new ADR, not an amendment.

## Verification

Status as of 2026-09-26:

1. A replay test executes a recorded run twice and asserts byte-identical event sequences and
   artifact digests. **Not met.** There is no replay driver and no projection layer. The
   *preconditions* are in place and tested: canonical JSON everywhere, a verified hash chain, a
   deterministic offline planner, pure evaluators, `WorkflowState.to_json()` / `from_json()`
   round-tripping, and digest-verified artifacts. See `docs/architecture-report.md` §3.8.
2. A test asserts that an out-of-catalog step, an out-of-bounds parameter, and an unrequested
   capability are each rejected at plan validation with a specific reason, and that nothing was
   executed. **Met.** `PlanValidator` returns a `PlanValidationError` carrying one of 13 codes —
   including `unknown_catalog_step`, `invalid_attempts`, `too_many_attempts`, and
   `capability_not_granted` — and `validate_or_raise` raises `PlanRejectedError`. Covered in
   `tests/unit/test_kernel_memory_planning.py` (96 tests) and
   `tests/unit/test_kernel_workflow_branches.py` (141 tests).
3. A test kills the process while a checkpoint is pending, restarts, and asserts the checkpoint is
   still pending and still valid. **Partially met.** Pending checkpoints are durable: workflow state
   is persisted as canonical JSON with a verified digest in `workflow_snapshots`, and checkpoint
   decisions are stored in `approval_requests`. Restart restore is verified for policy approvals and
   budget usage (`tests/security/test_revision_commit.py:515,638`), and the restore is **fail-closed**:
   a tampered record, a record that does not validate, and missing state in an already-used directory
   each **stop the launch** rather than resetting to defaults
   (`test_revision_commit.py:581,608,794`), while a fresh directory restores defaults with no record
   (`test_revision_commit.py:842`). There is **no** process-kill fault-injection test.
4. A test mutates design state after a checkpoint decision and asserts the stale decision is
   invalidated rather than applied. **Met, in two places.** In the workflow engine, a checkpoint whose
   recomputed `state_digest` or `action_digest` no longer matches is marked `INVALIDATED` with the
   reason `state or action_digest_changed`. In the revision commit, both digests are recomputed from
   the state that exists at commit time, and a mismatch writes nothing.
   (`tests/security/test_revision_commit.py:183,444`;
   `tests/unit/test_kernel_workflow_branches.py`.)
5. A test asserts that no code path in `kernel` or `runtime` calls a model provider outside the
   planning interface. **Partially met.** `tests/unit/test_kernel.py:268` proves `kernel` imports
   neither `runtime` nor `tools`, so the kernel cannot reach a provider at all. The converse — that
   `runtime` reaches a provider only through `ModelGateway` — is enforced by construction
   (`httpx` is imported lazily inside `OpenAICompatibleClient.complete`) but is not asserted by a
   dedicated test.
6. A test asserts that a user-supplied identifier cannot name the reserved aggregate that holds
   durable kernel state. **Met.** `kernel_state_aggregate` prefixes every stored aggregate with
   `kernel-state:`, and `_is_reserved_identifier` refuses both the bare legacy aggregate names
   (`kernel-policy-state`, `kernel-budget-state`) and anything beginning with that prefix in a
   user-supplied `workflow_id`, raising `ReservedIdentifierError`. The bare names stay reserved so
   records written by the earlier version still read back. Refused in-process and over HTTP.
   (`tests/security/test_revision_commit.py:676,709,735`, `tests/unit/test_api.py:780`.)
7. A test asserts that a completed workflow cannot authorize a commit it did not prepare, and that a
   completed workflow with no stored approval authorizes nothing. **Met.** The commit path re-derives
   the stored decision rather than trusting the workflow's status.
   (`tests/security/test_revision_commit.py:475,487`.)
8. A test asserts that first use of a tool class requires an explicit approval and that a repeat
   inside the same class does not. **Met.** With a `PolicyEngine` attached, every non-egress call
   becomes a `TOOL_CLASS_CHANGE` policy request keyed by `(action_digest, scope)`, so a changed
   capability set produces a different `action_digest` and is a new class that re-approves.
   (`tests/security/test_tool_executor.py:504,618`, `tests/security/test_policy.py:373,406,456`,
   `kernel/policy.py:345`, `tools/executor.py:1123`.)

## Implementation notes (2026-09-26)

The reference mechanical run, observed end to end, produced exactly this event sequence:

```text
 1 domain.evaluated             11 workflow.step_evaluated
 2 plan.created                 12 workflow.step_evaluated
 3 workflow.started             13 workflow.checkpoint
 4 workflow.created             14 policy.checkpoint_requested
 5 workflow.step_evaluated      15 workflow.approval_resolved
 6 workflow.status_changed      16 workflow.transition
 7 workflow.transition          17 workflow.checkpoint_decided
 8 workflow.step_evaluated      18 revision.prepared
 9 workflow.transition          19 revision.commit_decided
10 workflow.step_evaluated      20 revision.created
```

| Decision | Implementation | Reference |
|---|---|---|
| D1 versioned declarative state machine | `WorkflowEngine` over `WorkflowDefinition`; the model does not control flow | `kernel/workflow.py:467` |
| D2 pure steps, journaled effects | `EvaluationResult` from pure evaluators; every transition, checkpoint, and evaluation journalled | `application.py` `_on_workflow_event` |
| D3 a plan is data, validated before execution | `PlanStep` contract, `PlanValidator`, 13 `PlanValidationCode` values | `kernel/planning.py:105` |
| D4 iterative, budgeted, never unbounded | Per-step retry bounds; `WorkflowDefinition.max_replanning_attempts`; `RunBudget.replanning_attempts`; `WorkflowLimitExceeded` | `kernel/workflow.py:881,1313` |
| D5 checkpoints as first-class states | 6 `ApprovalKind` values in the contract, the policy engine, and the tool plane. **Gap:** the `required`/`recommended`/`auto-within-scope` policy and its strictest-wins resolution are not implemented. | `contracts/models.py:310` |
| D6 a checkpoint is a persisted decision | `WorkflowApproval` in canonical JSON, `workflow_snapshots` with a verified digest, `approval_requests` rows; stale digests invalidate | `kernel/workflow.py:106`, `persistence/database.py:480` |
| D7 typed modification surface | A decision is an `ApprovalStatus` plus an option from a declared set (`("approve", "reject")` in the product); there is no free-text override path | `application.py`, `kernel/policy.py` |
| D8 determinism scoped and stated | The offline planner and the evaluators are deterministic; model output is recorded input. **Gap:** no replay driver. | `runtime/agent.py`, `domains/__init__.py:461` |

Two honest notes on D5 and D6. The checkpoint **types** all exist and the tool plane raises
`TOOL_CLASS_CHANGE` and `EGRESS` automatically, so the machinery is exercised — it is the shipped
*mechanical workflow* that only declares `PRE_COMMIT`. And the checkpoint **policy** layer, which
would let a domain package raise a checkpoint to `required` and forbid itself from lowering it, was
not built; there is no overlay mechanism at all.

## References

- `docs/architecture-report.md` — the kernel (§11), the two-phase commit and its digests (§9.6), the
  exact API and CLI surfaces (§14, §15), and the replay and overlay gaps (§3.8, §3.10).
- `docs/verification-report.md` — per-file coverage for the kernel, the checkpoint and durability
  tests, and residual risks RR-9 and RR-10.
- `docs/threat-model.md` — TB-5 rows T5.4, T5.8, and T5.9 on the revision write and checkpoint
  durability.
- ADR-0001 — Local-First, No-Egress-By-Default Operation with a Provider-Neutral LLM Gateway.
- ADR-0002 — Field-Independent Kernel with Declarative Domain Packages.
- ADR-0004 — Single-Writer Immutable Revisions over an Event Journal and Content-Addressed Artifact Store.
- ADR-0005 — Capability-Based Typed Tool Contracts Executed in Isolated Processes.
- ADR-0006 — Package Trust, Sandboxing, Provenance, Rollback, and Agent Budgets.
