# GPT-6.1-Sol review — changes required

Independent full workspace tests:62passed (23 JSON +39 persistence). Corpus/schema/cross-reader tests are useful, but additional out-of-tree probes reproduced the following defects. Persistence slice is NOT accepted yet.

## P1 — normalizing immutable chain fields hides tampering

`validate_event_row` trims event_type/aggregate_id/workflow_id/run_id into ValidatedEvent. `chain_fields` then hashes those trimmed strings rather than stored bytes. Reproducer: append("review.event","review-id","{}"); drop events_prevent_update; UPDATE events SET aggregate_id=' review-id ' WHERE sequence=1. Rust verify_chain and read both return Ok with the unchanged digest. Python verify_chain raises JournalIntegrityError: event1 failed chain verification. This violates covered-field tamper detection.

Hash raw stored identifier strings. Validate/normalize separately for returned Event API if needed. Applies to verify_chain, event_from_row, verified_tail and optional IDs. Test whitespace padding of each covered identifier without updating hashes, including middle rows, startup, get, filters matching nothing, append-head refusal. Also test a correctly rehashed raw padded row against Python so the repair does not merely substitute stricter refusal for raw-byte compatibility.

## P1 — failed COMMIT leaves a live transaction and pending writes

Database::transaction rolls back only when operation(connection) returns Err. COMMIT/RELEASE errors escape via ? from the success branch without cleanup. Reproducer: create review_parent(id INTEGER PRIMARY KEY) and review_child(parent_id INTEGER REFERENCES review_parent(id) DEFERRABLE INITIALLY DEFERRED); transaction inserts orphan123 and returns Ok. COMMIT returns FK error. Rust is_autocommit=false and SELECT COUNT(*) child=1 afterward. Python transaction catches commit failure, rolls back, in_transaction=false and rowcount=0. Subsequent Rust transactions can unknowingly become savepoints in this failed outer transaction.

Use reliable transaction/savepoint cleanup on closure errors, finalization errors and Rust panic unwinding, preserving outer transaction semantics and original errors. Test deferred-FK COMMIT failure, subsequent successful real commit, nested failure/outer rollback, and catch_unwind reuse. Do not weaken FK/durability or swallow errors to pass.

## P2 — timestamp reader changes accepted values/domain

Independent Python/native comparison:
- 2026-01-01T00:00:00.123456789Z: Python returns .123456Z; Rust retains123456789 nanoseconds (writer prints6digits later, but typed timestamp is wrong).
- 2026-01-01T00:00:60Z: Python rejects; Rust accepts/emits leap second. Writer must also reject chrono leap-second values impossible in datetime.
- Python accepts these but Rust rejects: 2026-W01-4T00:00:00+00:00; 2026-01-01X00:00:00+00:00; 2026-01-01T00:00:00+05:30:15; same +05:30:15.123456; 2026-01-01T00:00:00,123456Z.
Preserve native datetime.fromisoformat-style date/time/offset accepted forms and microsecond truncation, including offset seconds/fractions. Test these with live Python and valid/invalid date/offset boundaries; no Python runtime fallback. No exact timestamp-parser parity claim while known valid rows are rejected.

## Tracked follow-up / no broader cutover

The imposed persistence depth1024 limit rejects Python-accepted deeper stored objects. This remains a data-compatibility gap for eventual full cutover and needs an iterative/nonrecursive foundation path; cannot be described as identical accepted/rejected sets. Preserve readable deep data rather than resetting it. Foundation-owned changes must be separately coordinated, outside this focused repair ownership.

ConnectionGuard.with can panic if a held guard is reentered on the same thread after Database::close: guard itself does not retain a RefCell borrow. Normal code must not use a stale guard; review this public lifecycle edge and make failure explicit or prevent close while guards exist, with a regression if changed. Do not expand into CAS/repositories/core/provenance yet.
