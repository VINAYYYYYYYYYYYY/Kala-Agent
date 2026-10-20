# kernel-review: independent adversarial review of kala-kernel (security-critical)
Follow `../CONVENTIONS.md` for gates and honesty. You are a REVIEWER: do NOT edit `backend/crates/kala-kernel/src/**`. You may add ONLY new test files `backend/crates/kala-kernel/tests/review_*.rs` (and reuse `tests/support/mod.rs` read-only) plus this cell's `REVIEW.md`.
Read `../kernel/TASK.md`, `../kernel/REPORT.md` (especially "Review requests" and the intentional differences), `src/kala_agent/kernel/{policy,workflow,planning}.py`, `tests/security/test_policy.py`, and the kernel tests.
Probe and write failing-if-broken tests for:
1. Policy import/from_state with forged, default, missing, extra, duplicated, type-confused, and oversized shapes: the engine must be unchanged after every failure, and nothing may widen privileges.
2. Approval replay: a reused request_id with a changed action or state digest, an approval after expiry, an approval for a changed definition or native implementation identity, and terminal-decision finality.
3. Workflow restore/advance with forged history, unknown steps, out-of-order transitions, a changed definition identity, or stale approvals: no replay, no privilege widening. Confirm valid Python-exported states still restore and re-serialize byte-identically (live Python).
4. Event-sink failure and re-entrancy (a sink that panics, or calls back into the engine) and concurrent policy decisions from many threads: no deadlock, no torn state, and deterministic outcomes where Python's are.
5. Resource bounds: huge plans and cycles, deep JSON, and the tracking/pruning limits (no unbounded growth).
REVIEW.md: a verdict per probe (PASS / FAIL with a minimal repro), counts, and a recommendation ACCEPT or REPAIR with the exact defects. Gates: the full kala-kernel test suite including your new tests (debug and release), plus clippy -D warnings.
