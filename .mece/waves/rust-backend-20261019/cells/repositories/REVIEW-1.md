# Sol typed repositories review — accepted within documented limits

Original task terminal/no live child turns. Accepted as a standalone typed repository layer for future consumers; root workspace membership/application cutover is not part of this acceptance.

Independent checks on final code:

- All 33 offline locked release tests passed: seven CAS/concurrency, three live Python cross-reader, seven memory-model, eight round-trip and eight schema-integrity tests.
- fmt and all-target/all-feature clippy-Dwarnings passed.
- Independent out-of-tree consumer `/tmp/kala-sol-repository-review`: 51 extra live Pydantic/Rust MemoryRecord comparisons, zero mismatches. Covered ID whitespace/ranges, Pydantic Unicode/control trimming and astral content lengths, NUL/surrogates, metadata duplicates/privateNumber/arbitrary data, aware/numeric/boundary timestamps, expiry equality/offset comparisons and duplicate model keys. Compared exact model wire bytes for accepted inputs and rejection for invalid inputs.

Source reviewed: every repository SQL/order/tie breaker against actual Python, append-only revision CAS and atomic spec/revision insertion, pending versus decided approval conditional UPSERT, tool-result call/identity checks, memory expiry/source filtering, typed error fields, all MemoryRecord validation/serialization, timestamp bridge and private transaction error adapter. Existing accepted Database transaction/savepoint/rollback logic remains authoritative. The adapter records the typed closure error and forces the accepted rollback path using a private sentinel; failed begin/commit and successful closure errors remain distinguishable. It does not duplicate the transaction manager or schema.

The release tests independently exercised real Python-written SQLite -> Rust readers and Rust-written SQLite -> Python readers with raw model_json/timestamp comparisons, finality/FK/trigger failures, two-connection CAS/approval races with one winner, orphan prevention, rollback/connection reuse, deep/large metadata and tampered stored-model rejection. No unresolved acceptance finding.

Explicit boundaries retained: native Windows check blocked by bundled SQLite C toolchain/lib.exe availability, no Windows runtime claim; typed i64 list limits and SQLite sequence-range error differ in representation from Python arbitrary integer/type exceptions; typed StoredModel errors replace Pydantic exceptions. Legacy rows/provenance are never rewritten on read. Source fixture was not opened during conductor review; worker disclosed an earlier sidecar checkpoint and current tests copy the fixture/sidecars before opening. Root manifest/lock remains delivery-owned; no source deletion, commit/push/deploy, benchmark or full migration/cutover claim.
