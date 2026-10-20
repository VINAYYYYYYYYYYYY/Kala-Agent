# kernel-review: independent adversarial review of `kala-kernel`

Date: 2026-10-20
Reviewer: `kernel-review` (independent sub-agent)
Cell: `.mece/waves/rust-backend-20261019/cells/kernel-review`
Crate under review: `backend/crates/kala-kernel`
Python reference: `/home/v9i/Projects/Kala-Agent/src/kala_agent/kernel/{policy,workflow,planning}.py`
Python runtime used: `.venv/bin/python` (3.14.8), `-I`, `PYTHONPATH=src`
Toolchain: `cargo 1.99.1` / `rustc 1.99.1`

**Recommendation: REPAIR.** One real defect (D1) blocks acceptance: policy
`decide()` drops the accountable actor from non-approved decisions. The replay,
restore, re-entrancy, concurrency and resource-bound properties otherwise held
under every probe. D1 is a one-line fix and is not a privilege-widening bug.

## Scope and method

Read `../CONVENTIONS.md`, `../kernel/TASK.md`, `../kernel/REPORT.md`, the three
Python modules, `tests/security/test_policy.py`, the kernel unit tests, the
implementer's Rust tests, and `src/{lib,policy,workflow,planning,json_input,time,value}.rs`.

I changed **only** these new files; `src/**` was not edited, and there was no
commit, push, delegation, credential read, or source deletion:

```
backend/crates/kala-kernel/tests/review_policy_import.rs      (7 tests)
backend/crates/kala-kernel/tests/review_policy_bounds.rs      (6 tests)
backend/crates/kala-kernel/tests/review_workflow_restore.rs   (4 tests)
backend/crates/kala-kernel/tests/review_workflow_events.rs    (4 tests)
backend/crates/kala-kernel/tests/review_planning_bounds.rs    (2 tests)
```

Each probe is a live differential against the repository Python where the
boundary is representable, plus native adversarial assertions where Python's
API cannot express the attack (typed sinks, threads, 12k-step plans). The
differential compares exact canonical bytes, including error class and message;
no oracle normalization is used. Two temporary fuzz drivers were run and
removed after recording their results (they were `review_scratch*.rs` and are
not part of the delivered suite):

- 260 randomized policy-import mutations (fields, whole records, dropped keys,
  junk types): **0 mismatches**, including every error message.
- 120 randomized workflow-restore mutations: 15 accepted by both with
  **byte-identical** re-serialization, 6 native-only rejections (documented
  strictness), 99 rejected by both with identical previous-state bytes, and
  **0 cases where native accepted a document Python rejected**.

## Per-probe verdicts

### Probe 1 — policy import / `from_state` shapes: PASS

Tests: `review_policy_import::{import_boundary_matches_python_and_never_mutates_on_failure,
failed_imports_leave_the_engine_byte_identical, accepted_imports_restore_normalized_values_and_limits,
rejected_type_confusion_never_widens_privileges}`.

- 57 rejected shapes (forged/default/missing/extra/duplicated/type-confused,
  wildcard and unsafe grants, naive/invalid timestamps, orphan/duplicate
  approvals, duplicate normalized request IDs, duplicate tool classes, bool
  where an integer/string/boolean is required) match Python's accept/reject
  result **and** error class/message byte-for-byte.
- After **every** failed import the full `PolicyState`, its canonical JSON,
  grants, tracked IDs and approvals are identical to the pre-import baseline;
  the engine still decides correctly afterwards. `from_state` and
  `from_state_json` reject the same corpus.
- 8 accepted shapes (minimal `{"version":1}`, missing optional collections,
  missing limit, normalized `" capability "` / `" request-id "` values,
  duplicate exact grants) restore with Python-identical normalization. The
  state's own `max_tracked_requests` wins over the factory fallback, matching
  Python's `from_state` factory.
- Type-confusion import into an empty engine never leaves a grant, approval or
  tool-class cache behind; egress and read stay denied. A default-shaped
  document is accepted and intentionally clears privileges (Python parity).
- Oversized shapes: a 1,000,000-character request ID, a 1,200-record imported
  state, and 600/5,000/50,000-deep JSON are refused or pruned without panic or
  stack overflow; failures leave the engine unchanged.

No privilege widening was found on any import failure path.

### Probe 2 — approval replay, expiry, identity and finality: FAIL (D1)

Tests: `review_policy_import::{approval_replay_matrix_matches_python,
replayed_approval_never_authorizes_a_changed_binding,
denied_decisions_preserve_the_acting_approver}`.

The security properties themselves pass:

- Reusing a `request_id` with a changed state **or** action digest returns
  `invalidated` and never `allowed`; the old record becomes `INVALIDATED`.
- A new binding for the same request ID is allowed only after its own explicit
  approval; the old approved record cannot be replayed.
- Rejections/aborts are terminal for that exact `(request_id, state_digest,
  action_digest)` binding: repeat approve/invalidate returns the refusal, and a
  different action binding needs its own approval (Python parity).
- Capability expiry denies even after an earlier approval; an imported
  `expired` approval returns `invalidated`; an imported `approved` approval
  with mismatched digests returns `invalidated`; an exact imported binding
  allows. Digest-override approval returns `invalidated` and never widens.
- Workflow identity is covered under probe 3.

**Defect D1 — decision payload loses the acting approver on refusals.**

`PolicyEngine::decide_locked` (`backend/crates/kala-kernel/src/policy.rs`,
the non-approved branch around lines 1700–1711) constructs the decision with
`actor_id = None`:

```rust
let (status, reason) = unapproved_outcome(approval.status());
self.prune_locked(inner);
return Ok(PolicyDecision::new(
    request, status, reason, false, Vec::new(),
    None,   // <-- Python passes approval.actor_id here
    None,
));
```

Python (`src/kala_agent/kernel/policy.py`, lines 716–725) passes
`actor_id=approval.actor_id` for `rejected`, `aborted`, `invalidated` and
`expired` records (and only omits `decided_at`, which native correctly leaves
`None`). Minimal repro:

```
cargo test --locked --test review_policy_import denied_decisions_preserve_the_acting_approver
# left: None, right: Some("reviewer")
```

The broad differential shows exactly three mismatches, all `actor_id`:
`results[5]` (rejected decision), `results[8]` (repeat read of a rejected
decision), `results[19]` (imported `expired` approval → `invalidated`). Python
reports the deciding actor; native reports `null`.

Impact: no privilege widening and no change to status/reason/digests/state
persistence, but denied decisions lose accountable attribution in an
audit/security API, and the implementer's REPORT claim of exact decision
payloads is false. Severity **medium**; blocking because the kernel's decisions
are the approval/audit record consumed by later cells.

Fix: pass `approval.actor_id().cloned()` as the sixth argument in that branch;
leave `decided_at` as `None`. After that, both failing tests pass.

### Probe 3 — workflow restore / advance / stale approvals: PASS

Tests: `review_workflow_restore::{python_lifecycle_states_restore_and_reserialize_byte_identically,
forged_history_and_unknown_steps_are_rejected_transactionally,
changed_native_transition_identity_never_replays_an_approval,
stale_restored_approval_is_invalidated_not_replayed}`.

- Live Python-exported states at created, running, replanned, waiting,
  approved, resumed/completed, rejected, aborted and cancelled lifecycle points
  restore natively and re-serialize **byte-identically**; restored
  state/action digests also match Python exactly.
- Native restore rejects (and leaves the previous engine/JSON unchanged):
  created-with-history; unknown status/state; completion without history;
  history sequence gaps; unknown/replayed/later transitions; from/to/step
  mismatches; unknown attempt steps; extra success without history; attempt
  sequence mismatch; completed/attempt projection mismatch; over-replanning;
  changed definition ID/version; malformed pending approval. Unknown state keys
  are ignored, as the Python loader does.
- Same definition ID/version but a changed native transition target: restore
  accepts the self-consistent pending record, `approve_checkpoint` succeeds
  (the source's digest check is self-referential), but `resume`,
  `record_step_success` and repeated `run` **never apply** the old approval:
  the record becomes `INVALIDATED`, state stays at `start`, history stays
  empty. A fresh checkpoint/approval is required and then completes on the new
  definition. Changed checkpoint type/step/missing transition/identity are
  rejected at restore.
- Stale restored approvals: unapproved stale records block; an approved record
  with a mutated state/action digest is invalidated on resume; an imported
  `expired` record stays expired. No replay in any case.
- 120-case restore fuzz (above): no unsafe acceptance; both-accepted cases were
  byte-identical.

### Probe 4 — sink failure, re-entrancy, concurrency: PASS

Tests: `review_workflow_events::{panicking_sink_preserves_source_mutation_order_and_leaves_engine_usable,
reentrant_sink_can_read_and_mutate_without_deadlock,
concurrent_snapshots_restore_and_are_never_torn,
failing_sink_after_store_matches_python_mutation_order}` and
`review_policy_bounds::concurrent_decisions_and_imports_never_tear_state`.

- A sink that **panics** on `workflow.created` leaves the created state stored
  (source stores before emitting); a panic on `workflow.transition` or
  `workflow.approval_resolved` leaves the prior state and no approval/history
  mutation (source stores after emitting). The engine is not poisoned and
  continues to complete on a subsequent call.
- A re-entrant sink that reads `state()`, `serialize()` and `state_digest()`
  during emit and then calls `cancel()` is executed on a worker with a 10 s
  receive timeout: no deadlock, outer `start` returns `Created`, final state is
  `Cancelled`, events are exactly `created` then `cancelled`. This would hang
  on a non-reentrant lock held across the callback.
- A sink returning `Err` after `start` leaves the created state (`start`
  stores first), while a `workflow.transition` error leaves `Created` and no
  history (`run` stores last) — the Python mutation order.
- 6 reader threads snapshot a 24-step workflow while it advances; every
  snapshot parses, equals itself, restores into a fresh engine and re-serializes
  identically. No torn state.
- Policy: 32 threads × 200 identical read decisions are all `allowed`;
  a writer alternates two valid imports (600 imports) while 4 readers export;
  every export validates and re-serializes. 48 threads approve one request:
  exactly one succeeds, the rest report the resolved record; the final record is
  `approved`. No deadlock or accidental widening.

### Probe 5 — resource bounds: PASS (with observations)

Tests: `review_planning_bounds::{huge_acyclic_chain_and_cycle_do_not_overflow_the_stack,
attempt_and_step_bounds_use_arbitrary_precision_and_exact_limits}` and
`review_policy_bounds::{tracking_and_tool_class_bounds_hold_under_churn,
oversized_imported_state_is_pruned_and_arbitrary_limits_survive,
imported_tool_class_cache_matches_python,
deep_json_and_oversized_values_are_refused_without_panicking}`.

- A 12,000-step acyclic plan validates without recursion overflow;
  `find_cycle` returns `None` and `topological_order` returns a 12,000-node
  order. A 12,000-node cycle is found deterministically (closed path length
  12,001, Python's doubled start node) and rejected with `dependency_cycle` and
  the exact detail string; no recursion overflow.
- Attempt bounds use `BigInt`: a 10^100 step value is rejected against a 10^99
  bound with the exact `requested` detail; a 10^101 bound accepts and preserves
  the value; aggregate 2 × 10^50 sums are compared without overflow; zero
  attempts is `invalid_attempts`. Exactly 256 steps pass, 257 fail with
  `count=257`, `maximum=256`.
- Policy tracking: 240-operation churn keeps requests, approvals and the
  tool-class cache at or below the configured limit after every operation; a
  1,200-record import with limit 5 prunes requests/approvals to ≤ 5; a
  10^40 limit preserves 40 requests; a 1 MB ID is refused.
- Deep JSON: 480 levels parse; 600/5,000/50,000 levels are refused cleanly
  (the parser's 512-depth bound), with the engine unchanged.

Observations (source parity, not repair blockers):

- **O1:** `import_state` prunes requests/approvals but not `tool_classes`; an
  imported cache of 1,200 entries survives until the next tool-class remember
  trims it. Python does the same (verified with an exact-oracle 64-entry
  import). If persisted state is not fully trusted, both implementations
  should bound this at import.
- **O2:** Pruning ranks every request against every approval
  (`_eviction_tier_locked` scans all approvals), so import pruning is quadratic
  in the input state size. Python is identical; a huge trusted state is a CPU
  spike, not a privilege issue.
- **O3:** With a saturated limit, the just-approved request can be evicted
  immediately by pruning (pending rows outrank approved rows), so `approve`
  returns `pending` after storing/evicting the record. Verified as Python
  parity; surprising but not a widening path (the caller must re-approve).
- **O4:** Native workflow restore is stricter than Python in the documented
  places (unknown pending transition ID, current-state/history mismatch,
  string `completed_step_ids`, non-string `last_error`, etc.). The fuzz found
  six native-only rejections and no unsafe acceptance. Valid Python exports
  still round-trip byte-identically.
- **O5:** `apply_transition`, `record_failure` and `fail_state` are public and
  accept a caller-supplied `WorkflowTransition`. They validate the current
  origin and declared step but not membership in the definition (and do not
  store). No privilege widening was found, but later composition should treat
  these as low-level helpers or add a definition-membership check.
- **O6:** The 512-depth `parse_model_json` bound is not listed in the REPORT's
  intentional differences; add it to the limits section.
- **O7:** For non-approved decisions Python omits `decided_at`; native matches
  this. The D1 fix must change only `actor_id`.

## Commands and results

All from `backend/crates/kala-kernel/`:

```
cargo fmt --check                                                -> clean
cargo clippy --all-targets --locked -- -D warnings               -> clean
cargo test --locked --no-fail-fast                               -> 152 tests, 150 passed, 2 failed (D1 only)
cargo test --locked --release --no-fail-fast                     -> 152 tests, 150 passed, 2 failed (D1 only)
cargo build --locked --release                                   -> success
cargo check --locked --target x86_64-pc-windows-gnu --all-targets -> success
cargo check --locked --target x86_64-pc-windows-msvc --all-targets -> success
```

The 129 implementer tests all pass in debug and release. The 23 review tests
contribute 21 passes and the two D1 failures. Both failing tests were re-run in
release with the same result. `cargo test` without `--no-fail-fast` stops at the
`review_policy_import` binary; `--no-fail-fast` was used to obtain the full
count. No Windows runtime or performance claim is made.

## Recommendation

**REPAIR** (one defect):

1. `policy.rs`, `decide_locked`, non-approved branch: pass
   `approval.actor_id().cloned()` instead of `None` for the decision actor.
   Keep `decided_at` as `None` (Python parity).

After that, re-run the two named tests and the full debug/release suites. The
import, replay, restore, event-sink, concurrency and resource-bound probes were
otherwise adversarial and clean; I found no privilege-widening path in the
Rust port. The remaining items (O1–O7) are documentation/design notes that
should be carried into the integration cell, not blockers for this crate.

## Resolution (Opus, conductor)
D1 fixed by Opus with a one-line change in `src/policy.rs` (`decide_locked`, non-approved branch): pass `approval.actor_id().cloned()`, keep `decided_at` None (Python parity). Re-ran: fmt and clippy clean; debug and release 152/152 including both review tests. kala-kernel ACCEPTED.
