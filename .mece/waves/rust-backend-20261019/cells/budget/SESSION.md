# Budget cell session log

Date: 2026-10-19
Worker: implementation sub-agent (OpenCode Go DeepSeek V4.1 Flash max)
Scope: `backend/crates/kala-budget/**` plus this cell's report/session only.

## Timeline

1. Read `TASK.md`, `WAVE.md`, `REVIEW.md`, `src/kala_agent/kernel/budget.py`, `RunBudget` and the shared contract validators, kernel tests (`tests/unit/test_kernel.py`), `backend/compat/corpuslib/decimal_cases.py`, and the 85-case `backend/fixtures/corpus/v1/decimal_budget.json`.
2. Confirmed foundation had already created `backend/Cargo.toml` with `exclude = ["crates/kala-geometry", "crates/kala-budget"]`; validated the new crate with its own manifest, lock, and target.
3. Probed CPython `decimal` behaviour directly (fixture venv) before implementation:
   - `str(Decimal)` general-format thresholds and signed-zero/exponent retention.
   - CPython default context (prec 28, ROUND_HALF_EVEN, Emax 999999, Emin -999999; traps InvalidOperation/DivisionByZero/Overflow; no clamp).
   - Zero-addition sign/exponent rules; `1 + 0E-1000000` growing to 28 digits; `1e999999 + 1e-999999`; overflow/underflow/cancellation beyond the context range; exponent parse limits.
   - Duration grid: `round(seconds * 1e6)` ties-even, and the double round-trip (`m/1e6` then re-grid) that differs for large microsecond counts (found a concrete mismatch at ~9.99e15 µs). The Rust implementation re-grids exactly like Python at each usage construction/addition.
   - JSON state: Python `json.loads` int-vs-float classification, `Decimal(str(float))` for float costs, Pydantic/`RunBudget` validation order, import tamper/rollback behaviour, reservation id trimming/length, `next_reservation` collision skipping, and exact exhaustion messages.
4. Evaluated `bigdecimal` 0.4.11: its `Context` has precision/rounding but no `Emax`/`Emin`, and `Context::add_refs` computes the exact sum before rounding, which would allocate for far-apart exponents. Rejected it and implemented a native Decimal on `num-bigint` following CPython libmpdec's `_mpd_qaddsub`/`mpd_qfinalize` algorithm (including the tiny-operand cutoff that bounds alignment and libmpdec sticky rounding indicators).
5. Implemented the crate modules: `decimal/{mod,parse,format,cmp,arith,round}.rs`, `counter`, `duration`, `dimension`, `errors`, `json`, `run_budget`, `usage`, `ledger`, `compat`.
6. Built the fixture gate first; fixed:
   - `serde_json` `arbitrary_precision` makes `Number` equality lexical (`2e-06` vs `2e-6`), so tests compare numbers by value via `tests/common/mod.rs` (integers exactly, mixed int/float like Python).
   - A decimal comparison bug when one operand was zero (zero has no meaningful adjusted exponent); explicit zero cases added.
   - Native state error text aligned to Python for usage/reservation paths (cost/duration/counter inner messages), and Python contract class mapping for parse/overflow/state variants.
7. Added live-Python differential oracle scripts (`tests/oracle_decimal.py`, `tests/oracle_ledger.py`) and `tests/differential_python.rs`; fixed an oracle kwargs-selection bug and error-class naming (`Overflow`, not `decimal.Overflow`, matching `type(e).__name__`).
8. Added native ledger semantics tests and the allocation-bounded far-exponent test.
9. Ran an ad-hoc extended differential (20,000 seeded decimal pairs × parse/add/sub, 3,000 durations, 1,000 sums) against the live Python oracle with zero mismatches; the temporary test file was removed, leaving the committed deterministic workload at 1,000 pairs + fixed edges.
10. Ran the required gates from the crate directory.

## Final validation snapshot

```
cargo fmt --check                          clean
cargo test                                 55 passed, 0 failed
cargo clippy --all-targets -- -D warnings  clean
cargo build --release                      success
cargo test --release                       repeated the 55-test gate cleanly
```

Corpus: all 85 decimal/budget fixture cases consumed (17 decode, 23 RunBudget, 20 usage, 13 duration, 1 ledger scenario, 11 clamp), no skipped or weakened case.

## Decisions / notes for the reviewer

- No fixed-width decimal or f64 cost: costs use exact coefficient/exponent; counters use `BigUint`.
- Duration is stored as Python stores it (grid float), while intermediate sums use arbitrary-precision microseconds; no saturation or wrapping.
- Import validation is completed before assignment, matching Python's transactional behaviour.
- `reserve`/`commit` intentionally preserve Python's mutation order around late decimal-overflow errors.
- The crate is self-contained and excluded from the parent workspace; its `Cargo.lock` is independent. Integration into the shared workspace is a foundation/conductor step after review.
- `clamp_unit_interval` is present only to consume the 85-case corpus; it is not a claim of domain-crate ownership.
- No commit, push, deploy, model switch, fixture/Python edit, or whole-backend/benchmark claim.
- Remaining parity limits are listed in `REPORT.md` (Pydantic validation text, typed-vs-dynamic kwargs, non-default decimal contexts, JSON key order/float lexical text, canonical bytes).

## Repair round 2 (completed)

T3 task: node:delegated-task:command%3Amcp%3A19093ff3-9ea8-43bd-9489-e6da827626c1%3Adelegate-task%3Arust-backend-20261019-budget-r2-go
Child thread: thread:delegated-task:command%3Amcp%3A19093ff3-9ea8-43bd-9489-e6da827626c1%3Adelegate-task%3Arust-backend-20261019-budget-r2-go
Model: opencode-go/deepseek-v4.1-flash, max. Original predecessor terminal/no live turns. No competing writer; parent workspace/lock and other crates untouched.

### What was repaired (REVIEW-1 findings)

1. **Unicode `Nd` digits + `Decimal` whitespace (P2).** Probed CPython 3.14.8 (`_PyUnicode_TransformDecimalAndSpaceToASCII`, `str.isspace`, `unicodedata`) before coding. Added `src/unicode.rs` with a compact static `Nd` run table (71 runs / 760 codepoints, digit `(codepoint - start) % 10`) generated by `tools/generate_unicode_nd.py` (CPython 3.14.8, Unicode 16.0.0). `Decimal::parse` now maps `Nd` to ASCII, removes all underscores and trims `decimal`'s whitespace set (`White_Space` + `U+001C..U+001F`). `python_float_parse` maps `Nd` and handles underscores between digits, but trims only `White_Space` (float rejects `U+001C..U+001F`), so the two grammars stay separate. Superscripts/fractions/Roman numerals remain rejected because only `Nd` is mapped.
2. **Construction exponent bounds (P2).** `Decimal::parse` now enforces CPython's `libmpdec` checks on the *stored* exponent (`written - fraction digits`) in `[MIN_ETINY, MAX_EMAX]` plus the nonzero-coefficient adjusted-exponent bound. Long exponents are compared as text first (leading zeros accepted; >19 significant digits rejected without allocation). Verified against live Python for `1e-9223372036854775808`, `1e-1999999999999999998`, `99e999999999999999999`, zero coefficients, fraction shifts (`1.0e-1999999999999999996`, `0.1e1000000000000000000`) and 100,000-digit written exponents.
3. **Shared float formatter.** `Decimal::python_float_repr` keeps its public wrapper and delegates finite values to reviewed `kala_json::format_float` through a read-only path dependency; `nan`/`inf`/`-inf` text preserved.
4. **Panic/property review.** Documented `Decimal::from_parts` and `Context::new` trusted-API preconditions; exercised the new extreme accepted values through parse/format/compare/add/sub, the ledger state path and the allocation guard (2,000-digit coefficient at `MIN_ETINY` underflows in bounded memory). No unused Decimal operators were added.

### Repair regression evidence

- `tests/unicode_inputs.rs` (new, live Python oracle `tests/oracle_unicode.py`): full `Nd` table, all non-decimal numeric lookalikes, all 29 whitespace characters, 74 fixed + 300 seeded float strings compared to `repr(float)`, and 20 `RunBudget` JSON cases compared to `model_dump(mode="json")`.
- `tests/differential_python.rs`: +37 fixed Unicode/boundary pairs, +400 seeded construction-boundary pairs, +200 seeded mixed-syntax Unicode literals and +5 1,000-digit-coefficient boundary cases (parse/add/sub against live CPython); ledger sequence extended with Unicode budget/charge/state-import commands.
- `tests/ledger_semantics.rs`: Unicode/whitespace state import plus transactional rejection of lookalikes and out-of-range exponents, exact `MIN_ETINY`/`MAX_EMAX` storage.
- `src/run_budget.rs`, `src/decimal/parse.rs`, `src/duration.rs`, `src/unicode.rs`, `tests/far_exponent_memory.rs`: native unit/edge coverage for the same repairs, including a `.repeat(100_000)` exponent literal rejected as `ExponentOutOfRange` without allocation.

### Final validation snapshot (repair round 2)

An additional ad-hoc run (temporary test file, removed) compared 20,000 seeded mixed-syntax Unicode literals and 5,000 construction-boundary pairs × parse/add/sub against the live oracle with zero mismatches.

```
cargo fmt --check                               clean
cargo test --locked                            65 passed, 0 failed
cargo clippy --all-targets --locked -D warnings clean
cargo test --release --locked                  65 passed, 0 failed
cargo build --release --locked                 success
```

Corpus gate unchanged: all 85 fixture cases consumed, exactly 85 asserted, no skips. Ownership unchanged: only `backend/crates/kala-budget/**` plus this cell's `REPORT.md`/`SESSION.md`; parent `Cargo.toml`/`Cargo.lock` mtimes and contents untouched. No commit/push/deploy/model switch/fixture/Python-product/user-file edits; no delegation.

Round2 result read: completed, no pending child turns. Sol accepted default-context budget slice after independent65release tests, fmt/clippy/release and code review; see REVIEW-2.md. No active budget writer; crate read-only to consumers.
