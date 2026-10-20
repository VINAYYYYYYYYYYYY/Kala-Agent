# Kernel cell report — native policy, workflow and planning

Date: 2026-10-19
Cell: `kernel` (wave `rust-backend-20261019`)
Crate: `backend/crates/kala-kernel`
Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**
Status: implemented and validated in isolation; ready for independent review.
No delegation, provider/model switch, commit, push, publish, deploy, source
deletion, unrelated install, root-manifest edit or packaging cutover.

## Scope and ownership

Owned and changed only:

- `backend/crates/kala-kernel/**` (new standalone workspace, local
  `Cargo.lock`, ignored `target/`)
- this cell's `REPORT.md` and `SESSION.md`

Not touched: `backend/Cargo.toml`/root `Cargo.lock`, any accepted crate
source/manifest, `kala-domains`, `kala-repositories`, Python product/tests,
adapters, frontend/desktop, packaging, shared fixtures/generators, other-wave
files, `Rust+buid123/` or `rust-implementation.md`. The preceding untracked
changes shown by `git status` belong to other authorized waves and were left
alone.

## Delivered

A complete native port of:

- `src/kala_agent/kernel/policy.py` — grants/scopes/expiry, capability sets,
  approval-requiring effects/kinds, exact action/state digest and request-ID
  binding, request-ID reuse, stale/conflicting invalidation, pending/approved/
  rejected/aborted/invalidated transitions, terminal refusal finality,
  tool-class cache, tiered pruning/eviction, thread-safe decisions, strict
  dynamic state import/export and cross-record validation.
- `src/kala_agent/kernel/workflow.py` — immutable approval/attempt/
  transition/state/event records, start/run/resume/restore/snapshot/serialize/
  digest APIs, exact transition ordering, attempt limits, cancellation,
  replanning, checkpoint approve/reject/abort/resolve/decide, stale-approval
  invalidation, canonical CPython state/action digests, strict restore
  consistency, event sinks and callbacks.
- `src/kala_agent/kernel/planning.py` — normalized plans, all validation
  codes/details, capability definition/intersection, duplicate/missing/
  unknown steps and dependencies, deterministic cycle discovery/path,
  topological ordering, attempt bounds and first-error precedence.

The crate uses accepted `kala-contracts` typed models and `kala-json`
canonical bytes read-only. It does not depend on `kala-domains`,
`kala-repositories`, persistence, memory, tools or the runtime.

### Changed paths

```
backend/crates/kala-kernel/Cargo.toml
backend/crates/kala-kernel/Cargo.lock
backend/crates/kala-kernel/.gitignore
backend/crates/kala-kernel/README.md
backend/crates/kala-kernel/src/lib.rs
backend/crates/kala-kernel/src/json_input.rs
backend/crates/kala-kernel/src/planning.rs
backend/crates/kala-kernel/src/policy.rs
backend/crates/kala-kernel/src/time.rs
backend/crates/kala-kernel/src/value.rs
backend/crates/kala-kernel/src/workflow.rs
backend/crates/kala-kernel/tests/support/mod.rs
backend/crates/kala-kernel/tests/oracle.py
backend/crates/kala-kernel/tests/policy_semantics.rs
backend/crates/kala-kernel/tests/workflow_semantics.rs
backend/crates/kala-kernel/tests/planning_semantics.rs
backend/crates/kala-kernel/tests/python_differential.rs
backend/crates/kala-kernel/tests/export_import.rs
backend/crates/kala-kernel/tests/workflow_restore.rs
.mece/waves/rust-backend-20261019/cells/kernel/REPORT.md
.mece/waves/rust-backend-20261019/cells/kernel/SESSION.md
```

The public API is listed exhaustively in
`backend/crates/kala-kernel/README.md`. Root re-exports include the accepted
contract types (`ApprovalKind`, `ApprovalStatus`, `PlanStep`,
`SideEffectLevel`, `Text`, `WorkflowDefinition`, `WorkflowStatus`,
`WorkflowTransition`) and lossless JSON (`CanonicalObject`, `CanonicalValue`,
`Sha256Digest`).

- `kala_kernel::policy`: constants `POLICY_STATE_VERSION`,
  `DEFAULT_MAX_TRACKED_REQUESTS`, `MAX_NOTE_LENGTH`; types `ApprovalKey`,
  `ToolClassKey`, `PolicyDecisionStatus`, `PolicyReason`, `PolicyError`
  (`Base`/`Transition`/`Request`/`State`/`Key`), `PolicyConfigError`,
  `PolicySetupError`, `CapabilityGrant`, `PolicyRequest`, `PolicyDecision`,
  `ApprovalRecord`, `PolicyToolClass`, `PolicyState`, `GrantSpec`,
  `DecisionContext`, `PolicyEngine`; aliases `PolicyTransitionError`,
  `PolicyRequestError`, `PolicyStateError`, `PolicyApprovalRequest`,
  `PolicyApproval`, `ApprovalRequest`, `PolicyRequestModel`,
  `DecisionStatus`, `PolicyStatus`, `ApprovalDecision`.
- `kala_kernel::workflow`: types `WorkflowError`
  (`Base`/`Transition`/`LimitExceeded`/`Approval`/`Callback`),
  `WorkflowApproval`, `StepAttempt`, `WorkflowTransitionRecord`,
  `WorkflowState`, `WorkflowEvent`, `WorkflowEventSink`,
  `EventCallbackSink`, `PayloadCallbackSink`, `JournalCallbackSink`,
  `CollectingSink`, `CheckpointContext`, `WorkflowEngine`; aliases
  `WorkflowTransitionError`, `WorkflowLimitExceeded`,
  `WorkflowApprovalError`, `WorkflowCheckpoint`,
  `WorkflowCheckpointStatus`, `WorkflowStateStore`, `EventJournalCallback`,
  `WorkflowEventCallback`.
- `kala_kernel::planning`: `CapabilitySet`, `PlanValidationCode`,
  `PlanValidationError`, `PlanRejectedError`, `NormalizedPlan` (aliases
  `Plan`, `ValidatedPlan`), `PlanValidator`, `PlanConfigError`,
  `capability_set`, `combine_capability_sets`, `find_cycle`,
  `topological_order`.
- Shared: `KernelTimestamp`, `IsoformatError`, `parse_model_json`,
  `ModelJsonError`, `canonical_state_json`, `canonical_state_digest`.

The Python exception aliases preserve class names through
`python_class_name()`; event aliases preserve arity adapters and collecting
sinks.

## Compatibility and security evidence

- **Exact CPython bytes/digests.** Workflow `to_json()`, state digests and
  action digests are compared directly against the live repository Python
  implementation. A fixed gate asserts
  `fb1cc7d28b1f133e058cc8debc9f91c16a99a83d1a6afdcd032b6e9b85502252` for a
  sample state and
  `4dd3b1f3859ad11aa8e216e1c5a6797d71e814c4c2c1806117c24bed77caab9d` for its
  transition. The differential harness compares the entire oracle stdout
  byte-for-byte with native canonical JSON, not a normalized value.
- **Live Python differential (13 tests).** Targeted policy/workflow/planning
  scenarios, 220 random policy operations, 180 random workflow operations,
  160 random checkpoint-resolution operations, exact event payload/order
  comparison, Unicode/lone-surrogate/C0/offset timestamp cases, and the full
  plan validation matrix. No normalization is applied to hash, error or
  array ordering.
- **Legacy state cross-readers.** Live Python-exported policy and workflow
  states are imported natively; native exports are imported by Python; valid
  states re-serialize byte-identically both ways.
- **Rejection matrices.** Policy import rejects/accepts all tested
  malformed/default/duplicate/orphan/version/type/grant/request/approval/
  tool-class shapes with the Python class/message where the source uses
  `PolicyStateError` (and `KeyError` for indexed missing request/approval
  fields). Workflow restore rejects definition/version/state mismatches,
  malformed records, naive/invalid times and unsafe identifiers; native-only
  tests additionally reject forged history paths, unknown steps/transitions,
  over-bound replanning and inconsistent pending checkpoints that Python's
  unvalidated restore accepts (documented strictness).
- **Approvals.** Tests cover exact action+state binding, request-ID reuse,
  replay onto a changed action, stale/conflicting invalidation, terminal
  rejected/aborted finality, rejection survival across invalidation, expiry
  boundaries, scope mismatch, revocation, wildcard refusal, all checkpoint
  kinds, tool-class exact-capability cache and forgotten caches.
- **Concurrency/re-entrancy.** Policy has 32-thread atomic approval and
  16-thread single-approval replay tests (exactly one allows; no privilege
  widening). Workflow has concurrent state/serialize/digest access, a
  re-entrant sink that reads engine state during `start`, and sink-failure
  tests proving each mutation point matches the Python order (start stores
  before emit; run/approval/cancel/replan store only after successful emit).
- **Planning robustness.** All codes and precedence rules are tested,
  deterministic 2-/3-node cycle paths match Python (including the source's
  doubled final node in the detail string), and a 5,000-step acyclic plan
  validates without recursion overflow. Topological ordering is iterative.

## Commands and results actually run

From `backend/crates/kala-kernel/` with `cargo 1.99.1` / `rustc 1.99.1`:

```
cargo fmt --check                                   -> clean
cargo test --locked                                 -> 129 passed, 0 failed
  (18 lib + 5 export/import + 17 planning + 25 policy + 13 differential
   + 2 restore + 49 workflow + 0 doc)
cargo test --locked --release                       -> 129 passed, 0 failed
cargo clippy --all-targets --locked -- -D warnings  -> clean
cargo build --locked --release                      -> success
cargo check --locked --target x86_64-pc-windows-msvc --all-targets
                                                    -> success
cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
                                                    -> clean
```

The live tests used `/home/v9i/Projects/Kala-Agent/.venv/bin/python`
(Python 3.14.8) with `PYTHONPATH=src`. A missing oracle fails the test; the
override is `KALA_KERNEL_PYTHON`.

`cargo test` and release totals are identical. No ignored test, benchmark,
Windows runtime execution or performance result is claimed.

## Remaining limits (explicit)

1. **Typed API.** Python's dynamic kwargs/aliases, duck-typed values,
   multiple simultaneous grant collections and arbitrary callback exceptions
   cannot be expressed. Event sinks use the typed `WorkflowEventSink`.
2. **Stricter workflow restore.** Native restore rejects forged history,
   unknown history/attempt/completed steps, over-definition replanning and
   inconsistent pending approvals. Python accepts these documents; valid
   Python exports are accepted. Non-string `last_error`/`reason` are rejected
   at the dynamic boundary although Python's dataclass leaves them unvalidated.
3. **`record_step_failure(None)`.** Rust `None` is mapped to Python's
   `step_failed` default; an explicit Python `None` is not distinguishable.
4. **Non-finite model JSON.** Refused by the permissive parser; Python may
   ignore a non-finite value in an unknown field and only fail when a known
   field is validated or serialized.
5. **Sink locking.** Sinks run outside the engine mutex to avoid deadlocks.
   Mutation order matches Python, but a concurrent observer can see an
   intermediate state that Python's RLock would serialize.
6. **State document shape.** Dynamic imports use `kala_json::CanonicalValue`
   rather than Python `Mapping`; non-string object keys are therefore
   unrepresentable, and object insertion order is not retained in exports
   (canonical JSON is sorted, as required for byte parity).
7. **Platform.** Windows is compile/clippy only; no runtime or COM proof.
8. **Integration.** No persistence/memory/tool/runtime/HTTP composition, no
   packaging cutover, no deletion of the Python reference and no full
   migration, optimization or readiness claim.

## Review requests

1. Re-run the listed commands and the live differential from the crate
   directory.
2. Probe dynamic policy import/from_state with additional forged/default
   shapes and confirm the engine is unchanged after each failure.
3. Probe workflow restore/advance with forged history, changed definition
   identity and stale approvals; confirm no approval replay or privilege
   widening.
4. Probe event-sink failure/re-entrancy and concurrent policy decisions.
