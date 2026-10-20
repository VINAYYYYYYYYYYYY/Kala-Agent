# Kernel cell session log

Date: 2026-10-19
Cell: `kernel` (wave `rust-backend-20261019`)
Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**
Status: terminal for this round; implementation and validation complete in
isolation, awaiting independent review. No live child turn, delegation,
provider/model switch, commit, push, deploy or cutover was performed.

## Inputs read

- Wave instructions `.mece/waves/rust-backend-20261019/{WAVE.md,SYNTHESIS.md}`,
  `cells/kernel/{TASK.md,LAUNCH.md}`, and accepted-cell reports/READMEs
  (`foundation`, `budget`, `contracts`, `workspace`, `persistence`,
  `artifacts`, `process`) for ownership and compatibility cautions.
- Full source: `src/kala_agent/kernel/{policy.py,workflow.py,planning.py}`,
  `src/kala_agent/kernel/{budget.py,__init__.py}` for budget boundary
  awareness, and the accepted `kala_contracts` models.
- Tests: `tests/security/test_policy.py`, `tests/unit/test_kernel.py`,
  `tests/unit/test_kernel_workflow_branches.py`,
  `tests/unit/test_kernel_memory_planning.py`, plus the relevant
  approval/revision integration/security passages
  (`test_mechanical_loop.py`, `test_revision_commit.py`,
  `test_tool_executor.py`, `test_model_residency.py`, `test_runtime.py`) for
  policy/workflow consumer semantics.
- Accepted Rust APIs: `backend/crates/kala-json` (`CanonicalValue`,
  `canonical_dumps/digest`, `Limits`), `kala-contracts`
  (`WorkflowDefinition`, `WorkflowTransition`, `PlanStep`, enums, `Text`,
  `Timestamp`, canonical-object parsing, `model_json_limits`) and
  `kala-budget` boundaries.

## Work sequence

1. Created the new standalone crate with `[workspace]` and local
   `Cargo.lock`; path dependencies only on accepted `kala-contracts` and
   `kala-json`, plus `chrono`, `indexmap`, `num-bigint`/`num-traits`.
2. Ported the CPython `datetime.fromisoformat`/`isoformat` domain into
   `time.rs` (aware-only kernel timestamps, microsecond truncation, `Z`,
   24:00, basic/week dates, offset seconds/fractions, UTC normalization).
3. Ported the permissive model/state JSON loader with last-wins duplicates,
   arbitrary integers, surrogate escapes and non-finite refusal.
4. Ported policy in full, including exact approval keying, capability scope/
   expiry, tiered pruning, terminal refusal survival, tool-class cache,
   strict state parse/import and transactional replacement. Added
   `PolicyError::Key` to preserve Python's `KeyError` for indexed missing
   request/approval fields.
5. Ported workflow in full, including mutable-free records, exact transition/
   attempt ordering, checkpoint state machine, canonical state/action digests,
   restore consistency, callback adapters and out-of-lock event delivery.
6. Ported planning in full, including all codes/details and first-error
   precedence; replaced recursive cycle discovery and topological ordering
   with explicit-stack implementations; added dynamic `validate_values` for
   raw self-dependency/invalid-step precedence.
7. Built a test-only Python oracle and exact-byte differential harness;
   added targeted, randomized, cross-reader, rejection-matrix, concurrency and
   re-entrancy tests.
8. Wrote this crate README and the cell report.

## Differential/verification iterations (failures not hidden)

All discrepancies found by the live oracle were fixed or explicitly
documented as intentional strictness. No oracle normalization was used.

- Initial `time` tests incorrectly expected lowercase `z` and date-only
  values to be aware; corrected to CPython behavior (lowercase `z` invalid,
  date-only naive) and fixed an expected offset arithmetic value.
- Planning cycle detail initially omitted Python's doubled final node
  (`a -> b -> a -> a`); fixed to match `_find_cycle` + error rendering.
- Workflow oracle result records include `result_json` only on success and
  `state_json` only in some branches; the native harness was aligned exactly.
- Workflow state parsing produced `"status must be a string"` for an integer
  status where Python raises `"workflow status is invalid"`; fixed status/
  checkpoint/sequence/attempt type precedence to match.
- Policy missing indexed request/approval fields raised `PolicyStateError`;
  Python raises `KeyError`; added the `Key` variant/message.
- The workflow restore differential initially included forged history that
  Python accepts; that case was moved to the native strictness test and is
  documented rather than normalized away.
- Clippy formatting/auto-deref/import issues in tests were repaired; final
  fmt and `-D warnings` are clean.

No review failure was received during this round, so no formal repair round
was needed; these were implementation iterations within the worker turn.

## Final commands/results

```
cargo fmt --check                                    clean
cargo test --locked                                  129 passed / 0 failed
cargo test --locked --release                        129 passed / 0 failed
cargo clippy --all-targets --locked -- -D warnings   clean
cargo build --locked --release                       success
cargo check --target x86_64-pc-windows-msvc --all-targets
                                                     success
cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings
                                                     clean
```

Test breakdown: 18 lib, 5 export/import, 17 planning, 25 policy, 13 live
Python differential, 2 workflow restore, 49 workflow, 0 doctests.

## Ownership check

`git status` shows only `backend/crates/kala-kernel/**` as new in this cell
plus these two report files. All other modified/untracked paths belong to
other authorized waves (design-turns, delivery, domains, repositories,
Studio) and were not edited. Root manifests/locks and accepted crate sources
are unchanged by this cell.

## Remaining risks/limits

See `REPORT.md` and the crate README: typed API vs Python dynamic kwargs and
callback exception types; stricter workflow restore for forged consistency;
`record_step_failure(None)` mapping; non-finite model JSON parse timing;
sinks outside the mutex; Windows compile-only proof; and no
persistence/tool/runtime/HTTP composition or performance claim.
