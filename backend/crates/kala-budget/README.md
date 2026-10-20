# kala-budget

Native arbitrary-precision `Decimal`, counters, microsecond durations and the
run-budget ledger compatible with the Kala Agent Python kernel. This crate is a
member of the root `backend/` workspace and was accepted after repair round 2.

## Public API

- `Decimal` — exact finite sign/coefficient/exponent representation preserving
  scale and signed zero; CPython parse grammar and construction bounds; add/sub
  with the default 28-digit half-even context; `to_python_string`; float
  formatting delegated to `kala_json::format_float`.
- `Counter`, `BudgetUsage`, `BudgetSnapshot`, `UsageDelta` — arbitrary-precision
  counters, microsecond duration grid, exact cost and Python-compatible
  arithmetic.
- `RunBudget`, `BudgetLedger` — validation, reservations/commit/release,
  exhaustion semantics, state import/export and Python error-class mapping.

`Context::new` and `Decimal::from_parts` are trusted native constructors, not
validating user-input entry points. This is the default-context Decimal/ledger
slice, not a complete arbitrary-context Decimal library; Pydantic multi-line
validation text is not reproduced and wrappers must own response formatting.
Canonical state hashing must use `kala-json` losslessly, not a serde dump.

## Tests

From `backend/`:

```bash
cargo test -p kala-budget --locked
cargo test -p kala-budget --locked --release
cargo clippy -p kala-budget --all-targets --locked -- -D warnings
```

65 accepted tests cover all 85 corpus cases, 760 Unicode-digit/whitespace/
lookalike cases, boundary/duration/ledger differentials, far-exponent memory
bounds and a live Python oracle. See `../README.md` for workspace-wide
compatibility and platform boundaries.
