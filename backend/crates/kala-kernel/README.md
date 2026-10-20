# kala-kernel

Native policy, workflow and planning state machinery for the Kala Agent
kernel. This crate is a standalone workspace port of
`kala_agent.kernel.policy`, `kala_agent.kernel.workflow` and
`kala_agent.kernel.planning`; it consumes the accepted `kala-contracts` and
`kala-json` crates read-only and keeps its own `Cargo.lock`.

It does **not** compose persistence, memory, tools, the runtime, HTTP or the
CLI. It never invokes Python in the product path; the live Python oracle is
test-only.

## Public API

### `kala_kernel::policy`

- `POLICY_STATE_VERSION` (`1`), `DEFAULT_MAX_TRACKED_REQUESTS` (`512`),
  `MAX_NOTE_LENGTH` (`512`), `ApprovalKey`, `ToolClassKey`.
- `PolicyDecisionStatus` (`allowed`, `denied`, `pending`, `invalidated`,
  `rejected`, `aborted`) and `PolicyReason` (`allowed`,
  `approval_required`, `capability_not_granted`, `capability_expired`,
  `scope_mismatch`, `stale_approval`, `rejected`, `aborted`,
  `invalid_request`), both with `ALL`, `as_str`, `parse`, `Display`.
- `PolicyError` variants `Base`, `Transition`, `Request`, `State`, `Key` with
  `python_class_name()`/`message()`; `PolicyConfigError` for `TypeError` /
  `ValueError` configuration failures and `PolicySetupError` combining
  configuration with state parsing for the `from_state*` factories. Python
  aliases `PolicyTransitionError`, `PolicyRequestError`, `PolicyStateError`.
- `CapabilityGrant`: validated capability/scope/expiry, `applies_to`, copies
  with a different scope or expiry, `as_dict`/`as_value`.
- `PolicyRequest`: Python-normalized request ids/digests, sorted deduplicated
  capabilities, forced `egress` for the egress side-effect level,
  `requires_approval`, `tool_class_change`, `capabilities`, `side_effect`,
  `as_dict`.
- `PolicyDecision`: `approved`/`denied`/`as_bool`, missing capabilities,
  actor/time, `as_dict`.
- `ApprovalRecord`: exact action binding, `action_key`, `key`, `resolved`,
  `as_dict`; `PolicyToolClass` cache entries.
- `PolicyEngine`: `new`, `fixed(now)`, `from_grants`, `from_capabilities`,
  `with_grants`, `with_now`, `with_clock`, `with_max_tracked_requests`,
  `grant`/`grant_capability`, `revoke`/`revoke_capability`, `decide` plus
  `evaluate`/`check`/`authorize`/`request`/`evaluate_request`,
  `request_approval`/`create_approval`, `approve`, `reject`, `abort`,
  `invalidate` (each also `*_by_id`), `get_approval`, `forget_tool_class`,
  `export_state`, `export_state_json`, `import_state`, `import_state_json`,
  `import_state_typed`, `from_state`, `from_state_with`, `from_state_json`,
  `clock`, `grants`, `capability_grants`, `max_tracked_requests`,
  `tracked_request_ids`, `approvals`, `approved_tool_classes`.
- `PolicyState`: validated export/import document with `parse`,
  `parse_json`, accessors, `validate`, `as_value`, `to_json`. `GrantSpec`
  models Python's iterable and mapping grant inputs. `DecisionContext`
  carries digest overrides, actor, reason and clock.
- Python aliases `ApprovalDecision`, `ApprovalRequest`, `DecisionStatus`,
  `PolicyApproval`, `PolicyApprovalRequest`, `PolicyRequestModel`,
  `PolicyStatus`.

### `kala_kernel::workflow`

- `WorkflowError` variants `Base`, `Transition`, `LimitExceeded`,
  `Approval`, `Callback`, with `python_class_name()`/`message()`; aliases
  `WorkflowTransitionError`, `WorkflowLimitExceeded`,
  `WorkflowApprovalError`.
- `WorkflowApproval`: validated ids/checkpoint/digests/status, pending and
  resolved invariants, `is_pending`, `is_approved`, `to_dict`/`as_value`,
  `from_dict`/`from_value`.
- `StepAttempt`, `WorkflowTransitionRecord`: immutable validated records with
  arbitrary-precision attempt/sequence counters and mapping conversion.
- `WorkflowState`: immutable validated state, terminal/cancelled predicates,
  `to_dict`/`as_value`, canonical `to_json`, `from_dict`/`from_value`,
  `from_json`.
- `WorkflowEvent`: `payload()` and `to_dict()` with the exact Python keys.
- `WorkflowEngine`: `new`, `with_sink`, `with_event_callback`,
  `with_payload_callback`, `with_journal_callback`, `with_clock`/`with_now`,
  `with_max_attempts`, `definition`, `state`/`current_state`, `start`, `run`,
  `advance`/`execute`/`step`, `resume`, `record_step_result`,
  `record_step_success`, `record_step_failure`/`retry_step`,
  `approve_checkpoint`/`approve`, `reject_checkpoint`/`reject`,
  `abort_checkpoint`/`abort`, `resolve_checkpoint`,
  `resolve_checkpoint_default`, `resolve_checkpoint_str`,
  `decide_checkpoint`, `decide_checkpoint_default`,
  `decide_checkpoint_str`, `cancel`, `request_replan`/`replan`, `serialize`,
  `snapshot`, `restore`/`restore_state`/`resume_from`, `restore_value`,
  `restore_json`, `state_digest`, `state_digest_of`, `action_digest`,
  `apply_transition`, `record_failure`, `fail_state`,
  `complete_if_terminal`, `invalidate_approval`, `make_approval`,
  `approval_matches`, `select_transition`, `get_transition`, `outgoing`,
  `attempt_count`, `attempt_limit`.
- `WorkflowEventSink`, `EventCallbackSink`, `PayloadCallbackSink`,
  `JournalCallbackSink`, `CollectingSink`; `EventJournalCallback` and
  `WorkflowEventCallback` aliases. `CheckpointContext` carries digest
  overrides, actor, reason and clock.
- Python aliases `WorkflowCheckpoint`, `WorkflowCheckpointStatus`,
  `WorkflowStateStore`.

### `kala_kernel::planning`

- `CapabilitySet` (`BTreeSet<Text>`), `PlanValidationCode` (all thirteen
  Python codes), `PlanValidationError` with `code`/`reason`, `field`,
  `step_id`, `details`, `as_value`, canonical `to_json`, and
  `PlanRejectedError`.
- `NormalizedPlan` (`Plan`/`ValidatedPlan`) with definition id, ordered
  steps, step ids, sorted required capabilities, configured maximum attempts,
  length/index/iteration.
- `PlanValidator`: `new`, `from_config`, `definition`,
  `allowed_capabilities`/`granted_capabilities`, `unrestricted`,
  `max_steps`, `max_attempts`, `max_total_attempts`, typed `validate`,
  dynamic `validate_values`/`validate_values_request`/`validate_json`,
  `validate_or_raise`,
  `is_valid`, `PlanConfigError`.
- `capability_set`, `combine_capability_sets`, `find_cycle` (deterministic
  depth-first path with the repeated start node, exactly as Python) and
  `topological_order` (native dependency-first convenience, iterative).

### Shared values

- `KernelTimestamp` and `IsoformatError` (`Invalid`/`Naive`): CPython
  `datetime.fromisoformat` spellings (calendar,
  basic and ISO-week dates, arbitrary separator, truncated comma/dot
  fractions, `±HH`, `±HHMM`, `±HH:MM`, offset seconds/fractions, `Z`,
  `24:00` rollover), requires an offset, normalizes to UTC, formats
  `datetime.isoformat()` with `+00:00`.
- `parse_model_json`/`ModelJsonError`: permissive ordinary model/state JSON
  with CPython last-wins duplicate keys, arbitrary integers, lone surrogate
  code points, and non-finite refusal. Canonical parsing remains
  `kala_json::parse_strict`.

## Compatibility contract

- Canonical JSON state and digest bytes are produced by `kala-json` with the
  unbounded model domain: sorted code-point keys, CPython ASCII escaping,
  arbitrary integers, and the 4,300-digit int conversion limit. Workflow
  state/action digests match live CPython exactly (a fixed gate checks
  `fb1cc7d2…2252` for a sample state and `4dd3b1f3…ab9d` for its transition).
- Policy approvals bind `(request_id, state_digest, action_digest)`. A
  changed digest, conflicting pending/approved record, revoked/expired
  capability, or denied scope invalidates or denies. Rejections/aborts are
  terminal and survive later invalidation attempts. Tool-class caches bind
  the exact action digest, scope and capability set.
- Workflow approvals bind the definition id/version, transition edge, step,
  current state digest and action digest. A changed state/action or native
  definition identity is invalidated, never replayed. Restore validates
  definition/version/state, contiguous declared transition history,
  declared steps, the replanning bound and pending-checkpoint consistency.
- Python `str.strip()`/`isspace()` codepoint rules (including
  `U+001C..=U+001F`) are used for identifiers and notes. Lone surrogates are
  preserved in policy/workflow identifiers and canonical escaping.
- Policy state import parses and validates the complete document before
  taking the engine lock; any failure leaves the previous engine exactly
  unchanged. State mutations are serialized by a mutex. Workflow event sinks
  are invoked without the engine mutex (source mutation order is retained),
  so a re-entrant sink cannot deadlock another thread.
- Unknown policy keys, wrong dynamic types (including bool-versus-int),
  duplicate/orphan records, wildcard grants and unsafe capabilities are
  rejected. Unknown workflow state keys are ignored as the Python loader does;
  wrong types for known fields are rejected.

## Tests

From `backend/crates/kala-kernel/`:

```bash
cargo fmt --check
cargo test --locked
cargo test --locked --release
cargo clippy --all-targets --locked -- -D warnings
cargo build --locked --release
cargo check --locked --target x86_64-pc-windows-msvc --all-targets
cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
```

129 tests in debug and release. The suite includes:

- `tests/policy_semantics.rs` (25 tests): security denial/approval/expiry/
  scope/revocation, stale and replayed approvals, terminal refusal, eviction
  tiers, tool-class exactness, state round-trip, dynamic strictness, Python
  strip/surrogate rules, bounded maps and concurrent atomic decisions.
- `tests/workflow_semantics.rs` (49 tests): every lifecycle branch,
  checkpoint approve/reject/abort/resolve/decide, retry and replanning
  bounds, cancellation, restore, event ordering/failure/re-entrancy,
  concurrency and exact CPython canonical/digest bytes.
- `tests/planning_semantics.rs` (17 tests): constructor bounds, every
  validation code and precedence rule, dynamic invalid/self-dependency
  inputs, capability intersections, deterministic cycles, topological order
  and a 5,000-step acyclic plan without stack overflow.
- `tests/python_differential.rs` (13 tests): exact-byte live Python
  differential for policy/workflow/planning scenarios, 220-op randomized
  policy history, 180-op workflow history, 160-op checkpoint history,
  offset/Unicode/C0 cases and the full plan validation matrix.
- `tests/export_import.rs` and `tests/workflow_restore.rs` (7 tests):
  legacy Python-exported state cross-reads, native exports read by Python,
  policy import rejection/default matrices (including exotic timestamp
  spellings) and workflow restore rejection matrices.
- Unit tests in each module (18 tests) and the test-only oracle
  `tests/oracle.py`.

A missing `.venv/bin/python` fails the differential tests; set
`KALA_KERNEL_PYTHON` to an absolute interpreter path elsewhere.

## Limits

- The Rust API is statically typed. Python keyword aliases that provide
  several mutually exclusive grant collections, duck-typed objects, and
  arbitrary exception objects from event callbacks cannot be represented.
  Sinks use `WorkflowEventSink` and return `WorkflowError`.
- Dynamic workflow state is stricter than Python's unvalidated dataclasses
  for `last_error`/`reason` (string or `null` only), and restore is stricter
  about forged history/step/replanning consistency. Python accepts those
  documents; native restore rejects them. Valid Python-exported states are
  accepted and re-serialized byte-identically.
- `WorkflowEngine::record_step_failure(error_code=None)` means the Python
  default `step_failed`; an explicitly passed Python `None` is not
  distinguished.
- Non-finite model JSON is refused at the parser; Python refuses NaN/Infinity
  only when a field is validated or serialized (unknown ignored fields could
  parse there).
- Sinks run outside the engine mutex. The mutation order relative to each
  event matches Python, but another thread can observe an intermediate state
  that Python's RLock would block.
- No persistence/memory/tool/runtime/HTTP composition, no packaging cutover,
  no Windows runtime execution and no performance/readiness claim. Windows
  results are compile/clippy only.
