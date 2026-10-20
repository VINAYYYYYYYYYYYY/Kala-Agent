# Budget cell report — native Rust arbitrary-precision Decimal and ledger

Date: 2026-10-19
Cell: `budget` (wave `rust-backend-20261019`)
Crate: `backend/crates/kala-budget`
Status: implemented, repair round 2 complete and validated in isolation; ready for re-review. No commit, push, deployment, packaging cutover or benchmark claim.

## Scope and ownership

Owned and changed only:

- `backend/crates/kala-budget/**` (new crate, independent `Cargo.lock`, ignored `target/`)
- this cell's `REPORT.md` and `SESSION.md`

Not touched: fixtures, Python product/tests, `backend/Cargo.toml`/workspace lock, `crates/kala-json`, `crates/kala-geometry`, adapters, packaging, `Rust+buid123/`, `rust-implementation.md`, UI docs. The foundation worker had already excluded `crates/kala-budget` from `backend/Cargo.toml`, so validation used the crate's own manifest/lock/target.

## Delivered API

### Decimal (`kala_budget::decimal`)

- `Decimal`: exact finite sign/coefficient/exponent representation (`BigUint` coefficient, `i64` exponent) preserving trailing-zero scale and signed zero. Arbitrary coefficient length; construction exponents are limited only by CPython's own `MIN_ETINY`/`MAX_EMAX` bounds.
- `Decimal::parse` / `FromStr`: CPython grammar (whitespace, interior underscores, `.5`/`5.`, exponent range up to `PYTHON_MAX_EMAX`). The input is transformed exactly like `_PyUnicode_TransformDecimalAndSpaceToASCII`: every Unicode `Nd` digit is mapped to ASCII, all underscores are removed, and `decimal`'s whitespace set (Unicode `White_Space` plus `U+001C..U+001F`) is trimmed. Non-`Nd` numeric lookalikes (superscripts, fractions, Roman numerals) stay rejected; `NaN`/`sNaN`/`±Infinity` are rejected with `DecimalParseError::NonFinite`.
- Construction bounds match live CPython: the stored exponent (`written exponent - fraction digits`) must lie in `[PYTHON_MIN_ETINY, PYTHON_MAX_EMAX]` and a nonzero coefficient's adjusted exponent must stay `<= PYTHON_MAX_EMAX`. The written exponent is compared as decimal text first, so a 100,000-digit exponent is rejected without allocating a big integer, and harmless leading zeros are accepted.
- `Decimal::to_python_string` / `Display`: CPython `str(Decimal)` general-format algorithm (plain vs scientific thresholds, signed zero, stored exponent).
- `Decimal::as_tuple`, `digit_count`, `adjusted_exponent`, exact `compare`, numeric `PartialEq`/`Ord`, `negated`, `rounded_with_context`.
- `Context` (precision, `RoundingMode::HalfEven`, `Emax`, `Emin`, `etiny`) with `Context::python_default()` = 28 / half-even / ±999999. `Context::new` and `Decimal::from_parts` are documented trusted APIs that do not re-validate CPython bounds and do not panic on accepted inputs; `Decimal::parse` is the validating entry point.
- `Decimal::add`/`sub`/`add_with_context`/`sub_with_context`: native port of CPython libmpdec's `_mpd_qaddsub` + `mpd_qfinalize`. The "tiny" operand substitution bounds alignment to the precision window or the operand coefficient length, so exponents differing by millions never allocate a power of ten that size. Exact ties-to-even use libmpdec-style sticky rounding indicators; overflow traps as `DecimalError::Overflow`; underflow clamps to `Etiny` without trapping, as the default context does.
- `python_float_repr` keeps the public `Decimal::python_float_repr` wrapper but delegates to the reviewed `kala_json::format_float` (path dependency on `../kala-json`, no parent workspace change); non-finite values keep Python's `nan`/`inf`/`-inf` text.
- `python_float_parse` reproduces Python `float()` semantics: Unicode `Nd` digits in mantissa and exponent, underscores only between digits, and the Unicode `White_Space` set (which deliberately excludes `U+001C..U+001F`, unlike `Decimal`). `Decimal` and `float` normalization rules are kept separate.

### Counters, usage, duration (`counter`, `usage`, `duration`)

- `Counter`: arbitrary-precision nonnegative integer wrapper over `BigUint`; parses/validates integer text, rejects negatives, never wraps.
- `BudgetUsage`: seven dimensions; counters are `Counter`; duration is stored as the same whole-microsecond float grid Python stores (`round(seconds * 1_000_000)` with ties-even, `microseconds / 1_000_000`); cost is exact `Decimal`.
- `BudgetUsage::add`/`sub`/`exceeds` (Python direction), `as_dict` (integers stay integers, duration float, cost `str(Decimal)`), `merge` for keyword-style charges.
- `BudgetSnapshot::remaining`: clamps counters/duration/cost at zero with Python's `max` tie/scale behaviour (`max(Decimal("0"), value)` returns the literal zero when not strictly greater).
- `UsageDelta` with aliases `tokens`, `artifact_size`, `replanning`; `ModelCallCharge`/`ToolCallCharge` with Python defaults (count 1).
- Duration helpers: `normalize_duration`, `duration_microseconds` (arbitrary-precision), `seconds_from_microseconds` (correctly-rounded rational conversion through exact decimal text), `python_float_parse`.

### Budget, ledger, errors (`run_budget`, `ledger`, `errors`, `dimension`)

- `RunBudget`: Python defaults and validators; JSON import/export keeps int-vs-float distinctions and arbitrary-size integers.
- `BudgetLedger`: thread-safe (`Mutex`) usage/reserve/commit/release, monotonic `reservation-N` ids skipping imported collisions, reservation insertion order, `available`/`remaining`, `snapshot`, `is_exhausted`, `can_consume`, record helpers and Python aliases (`ensure`, `charge`, `record`, `record_model_call(s)`, `record_tool_call(s)`, `record_replanning_attempt`).
- `import_state`/`export_state`: exact Python state keys, strict type/extra-key/duplicate/id-length/`next_reservation` checks, reservation-total consistency, and all-validation-before-mutation transactionality.
- `BudgetError` variants and `contract_error_name()` mapping to Python classes (`BudgetStateError`, `BudgetReservationUnknownError`, `BudgetReservationMismatchError`, `BudgetCounterError`, the seven `*BudgetExhaustedError` classes, `ValueError`, `OverflowError`, `Overflow`). Exhaustion messages match Python text (validated by the fixture and live differential).
- `compat::clamp_unit_interval` is included only so the 85-case decimal corpus can be consumed whole; production domain ownership remains with a future domain crate.

## Dependencies

`num-bigint 0.4`, `num-traits 0.2`, `serde_json 1.0` with `arbitrary_precision`, `thiserror 2.0` and a read-only path dependency on the reviewed foundation crate `kala-json` (for `format_float`), all locked independently in `backend/crates/kala-budget/Cargo.lock`. No fixed-width `rust_decimal`, no `f64` cost, no Python subprocess in production code, no new third-party parser dependency. The static Unicode `Nd` table is generated once from CPython 3.14.8 (Unicode 16.0.0) by `tools/generate_unicode_nd.py` and checked in; Python is not needed at build or runtime.

## Validation evidence

### Corpus: 85/85 cases, no skips

`tests/fixture_decimal_budget.rs` reads `backend/fixtures/corpus/v1/decimal_budget.json` and consumes:

| Group | Cases | Comparison |
|---|---:|---|
| `decode_cases` | 17 | exact sign, digits, exponent, `str` |
| `run_budget_cases` | 23 | exact JSON value for 17 accepts; 6 rejects asserted by category (negative/non-finite) |
| `usage_cases` | 20 | exact JSON value for single costs, sums and subtraction-identity inputs |
| `duration_cases` | 13 | exact JSON value including ties-even and summed grids |
| `ledger_cases` | 1 | full observation sequence: initial state, charges, reserve, commit with actuals, final state, import round-trip equality, cost-exhaustion class + exact message |
| `clamp_cases` | 11 | exact values/errors for the corpus `clamp_unit_interval` cases |
| **Total** | **85** | test asserts the total is exactly 85 |

Rejection cases compare native error categories rather than Pydantic's multi-line `ValidationError` text (see gaps). All accepted values, including the ledger state and exhaustion message, match the fixture exactly under numeric-aware JSON comparison.

### Live-Python deterministic differential

`tests/differential_python.rs` executes the same seeded workload against `.venv/bin/python` (stdlib `decimal` for arithmetic; real `kala_agent.kernel.budget` + contracts for ledger). A missing oracle fails the test loudly.

- `decimal_parse_and_arithmetic_match_live_python`: 1,000 seeded random pairs plus 71 fixed pairs (scale, signed zero, far exponents, all-nines carries, overflow/underflow, malformed literals, Unicode digits/whitespace, construction bounds), 400 seeded construction-boundary pairs, 200 seeded mixed-syntax Unicode pairs and 5 wide-coefficient boundary cases; compares parse tuple/str and add/sub tuple/str with Python.
- `duration_grid_matches_live_python`: 516 duration values (ties, subnormals, huge values) and 154 duration-sum pairs, including Python's re-gridding of stored floats (the >2^53-µs double round-trip case is exercised).
- `ledger_command_sequences_match_live_python`: 61 commands covering charges, reservations, commit-with-actual, release, failure/rollback, import/tamper/round-trip (including `state.budget` extra-key text parity), JSON int/float state costs, JSON integer `-0` normalization, `can_consume`, `is_exhausted`, signed/arbitrary counters, far-exponent cost overflow, duration-grid overflow in `remaining`, and Unicode-text budget/charge/state inputs; compares returned JSON and, for budget errors, the Python class name and exact message.

An additional ad-hoc run (not committed; temporary test copy removed) compared **20,000** seeded decimal pairs × parse/add/sub, **3,000** duration values and **1,000** duration sums against the same live oracle with zero mismatches.

### Repair round 2 regressions (added after REVIEW-1)

These tests fail on the pre-repair parser and pass now:

- `tests/unicode_inputs.rs` (3 tests, live Python via `tests/oracle_unicode.py`):
  - every Unicode `Nd` codepoint reported by `unicodedata` (760 codepoints, Unicode 16.0.0) parses to its digit value through `Decimal::parse`, `to_python_string` and `python_float_parse`;
  - every numeric-but-not-decimal codepoint (superscripts, fractions, Roman numerals, ...) is rejected by both parsers;
  - all 29 `str.isspace()` codepoints are trimmed by `Decimal`, while `float` accepts all of them except `U+001C..U+001F` (matching Rust `char::is_whitespace`);
  - 74 fixed float strings (Unicode digits/exponents/underscores/whitespace, `inf`/`nan` forms, grammar edges) plus 300 seeded mixed-syntax samples compare `text` exactly against Python `repr(float)`, which also exercises the `kala_json::format_float` reuse;
  - 20 `RunBudget` JSON cases (Unicode strings, `MIN_ETINY`/`MAX_EMAX` boundaries, lookalikes, string-typed counters) compare accept/reject and the exact `model_dump(mode="json")` values.
- `tests/differential_python.rs` additions: 37 fixed Unicode/whitespace/boundary pairs, 400 seeded construction-boundary pairs (stored exponent within ±4 of `MIN_ETINY`/`MAX_EMAX`, random coefficient/fraction/sign, zero coefficients included), 200 seeded mixed-syntax Unicode literals and five 1,000-digit-coefficient boundary cases; all are compared against live CPython for parse, add and sub.
- `tests/differential_python.rs` ledger sequence additions: `new` budget with Unicode `max_cost`/`max_duration_seconds`, a Unicode-text charge, and state imports with Unicode `cost`/`duration_seconds` strings and a `MIN_ETINY` cost.
- `tests/ledger_semantics.rs` addition: native state import accepts Unicode numeric text and `Decimal`-only whitespace, rejects lookalikes and out-of-range exponents transactionally, and stores the exact `MIN_ETINY`/`MAX_EMAX` boundary values.
- `src/run_budget.rs` unit test: JSON entry path accepts Unicode/whitespace inputs, rejects lookalikes, `float`-only whitespace and both construction-bound overflows.
- `tests/far_exponent_memory.rs` additions: `MIN_ETINY` values underflow to `0E-1000026` in bounded memory, including a 2,000-digit coefficient at the bound; the peak-allocation assertion stays under 1 MiB.

An additional ad-hoc run (not committed; temporary test file removed) compared **20,000** seeded mixed-syntax Unicode literals and **5,000** construction-boundary pairs × parse/add/sub against the same live oracle with zero mismatches.

### Native tests

`tests/ledger_semantics.rs` (15 tests): default zero-cost exhaustion, exhaustion priority order, arbitrary-size counters without wrapping (including exact error text), release-in-any-order duration grid, commit actual/atomicity/unknown/mismatch, transactional import for 20 tamper shapes, scale-insensitive reservation equality, huge `next_reservation` ids, export/import round-trip, clamped `available`, exported state numeric shapes, integer duration import + float-overflow rejection, unknown dimension, Unicode/whitespace state import with boundary values, and two concurrency tests (800 serialized charges; 200 unique concurrent reserve/release ids).

`tests/far_exponent_memory.rs` (1 test) uses a tracking global allocator and asserts peak heap < 1 MiB while adding/subtracting values whose exponents differ by ~2,000,000 and ~1,000,000, plus overflow/underflow/cancellation and extreme construction-bound cases; peak stays at a few kilobytes and runtime is far below the 5 s guard.

Plus 42 unit tests across decimal parse/format/compare/arithmetic/rounding (including the Unicode table and construction bounds), duration, counters, usage, budget and ledger.

### Commands actually run (crate directory, after repair round 2)

```
cargo fmt --check                              -> clean
cargo test --locked                            -> 65 passed, 0 failed
  (42 lib + 3 differential + 3 unicode_inputs + 1 far-exponent + 1 fixture + 15 ledger + 0 doc)
cargo clippy --all-targets --locked -D warnings -> clean
cargo test --release --locked                  -> 65 passed, 0 failed
cargo build --release --locked                 -> success
```

The 85-case corpus gate (`tests/fixture_decimal_budget.rs`) still asserts exactly 85 consumed cases, including the Unicode-insensitive fixture entries.

No benchmark headline is claimed; no end-to-end/service/whole-backend completion is claimed.

## Known gaps and limitations

1. **Pydantic text parity.** `RunBudget`/`state.budget` validation failures use native messages, not Pydantic's multi-line `ValidationError`; error class categories are preserved (`Validation`/`State`). Usage/reservation/state text was aligned where the live differential compares it.
2. **Typed API vs Python dynamic kwargs.** The Rust API is statically typed; unknown keyword names or Python duck-typed values cannot be passed. Service wrappers must reject unknown JSON/kwargs before calling. `Counter`/`Duration` aliases are provided.
3. **Context coverage.** Only the CPython default context is first-class. `Context` supports explicit precision/`Emax`/`Emin` with half-even, but not `clamp=1` or other rounding modes; multiplication, division, quantize, powers and special values are intentionally not implemented (not used by the budget kernel). Python callers who mutate the global default context are not mirrored.
4. **Decimal exception surface.** Arithmetic overflow returns `BudgetError::Decimal(DecimalError::Overflow)` (`contract_error_name() == "Overflow"`), not a Python `decimal.Overflow` object; underflow/inexact/rounded/subnormal flags are not exposed. No result can be `NaN`/`Infinity` because non-finite input is rejected at parse.
5. **JSON representation.** State object key order follows serde (sorted) rather than Python insertion order, and float lexical text can differ (`2e-6` vs Python's `2e-06`) while parsed numeric values are equal. Parsed-value parity is tested; canonical byte/digest parity remains foundation-owned. Float *text* produced by `Decimal::python_float_repr` is now the reviewed `kala_json::format_float` output and is compared against live Python `repr(float)`.
6. **Partial-mutation quirk.** `reserve`/`commit` mirror Python's order, where a decimal overflow during the final reserved/usage addition can leave `next_reservation`/the reservation entry mutated (Python behaves the same). Import and charge validation themselves are atomic.
7. **Threading model.** `Mutex` provides serialization; Python's reentrant `RLock` semantics are not reproduced. There are no internal reentrant calls, and poisoning is recovered rather than panicking.
8. **Counter type edge cases.** Booleans/integer subclasses are Python runtime concerns; the Rust API cannot express them. Negative values and invalid text are rejected with `CounterError`.
9. **Integration.** The crate remains an explicit workspace exclusion with its own lock; it has no service/adapter wiring, persistence, or canonical-byte integration yet. `clamp_unit_interval` is only a corpus compatibility helper in this crate.
10. **No performance evidence.** The far-exponent test asserts bounded allocations and a loose time guard only; no comparative benchmark or optimization headline is reported.
11. **Foundation lock coupling.** The crate's independent `Cargo.lock` now pins `kala-json`'s transitive requirements (`ryu 1.0`, `sha2 0.10`, workspace `num-bigint`/`thiserror`). If the foundation worker changes those version requirements, `cargo test --locked` in this crate must be refreshed with a normal `cargo build`/`cargo update -p`. The parent manifest and lock were not touched.
12. **Written-exponent short-circuit.** Exponent magnitudes longer than 19 digits are rejected without big-integer allocation. This is exact for every constructible input (the accepted magnitude bound is 19 digits, and the only permitted 20-digit window would need a ~10^19-character fraction); the case is unreachable in practice and not covered as an accepted path.
13. **Unicode table provenance.** The embedded `Nd` table matches CPython 3.14.8 / Unicode 16.0.0 and is verified against the live oracle in `tests/unicode_inputs.rs`. A future Python Unicode database update would fail that differential test and requires re-running `tools/generate_unicode_nd.py`; the build itself never runs Python.
14. **Trusted constructors.** `Decimal::from_parts` and `Context::new` do not re-validate CPython construction/context rules (documented preconditions). The crate's parsing/JSON entry points are the validating surface; out-of-contract typed values are exercised by tests for absence of panics but are not claimed as CPython-parity inputs.
