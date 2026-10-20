# Contracts cell — R2 session notes

## Launch and state

- Wave: `rust-backend-20261019`, cell `contracts`.
- Launch record: `LAUNCH-R2.md`; original task result was terminal/completed
  with no live child turns before this repair.
- Implementation model: `opencode-go/deepseek-v4.1-flash` variant max.
- Review model: GPT-6.1-Sol (conductor), not run by this worker.
- No delegation, provider/model switch, commit, push, deploy or cutover.
- Ownership: `backend/crates/kala-contracts/**` plus this cell's `REPORT.md`
  and `SESSION-R2.md` only. No parent workspace/lock, accepted crate/shared
  source, Python product/test, Studio/process, adapter, packaging or user-file
  edit. The standalone workspace and local `Cargo.lock` remain.

## Work performed

1. Read `TASK-R2.md`, original `TASK.md`, current `REPORT.md`, `SESSION.md`,
   `REVIEW-1.md`, `REVIEW-2.md` and the actual Python contract source. Consumed
   the three confirmed REVIEW-2 reproductions with live Python and the current
   Rust crate semantics.
2. Replaced the serde-based permissive input parser with a private
   `src/input.rs` ordered representation:
   - explicit-stack parser (no recursion, arbitrary nesting);
   - Python `json.loads` duplicate semantics: last value wins, first insertion
     position retained;
   - codepoint-faithful `\u` escapes including lone surrogates and surrogate
     pairs, arbitrary integers up to CPython's 4,300-digit conversion limit,
     finite float rules and signed zero;
   - iterative ordered-to-`CanonicalValue` conversion and iterative
     `OrderedValue` cleanup;
   - no serde `Value`/private Number representation, so every arbitrary key
     and object shape is plain data.
3. Updated `ContractModel::from_json_str`/`from_json_bytes` to use the ordered
   parser and convert once. No public signature changed; the ordered API is
   `pub(crate)`, so there is no newly exposed ordered public API.
4. Reworked construction profile handling in `src/construction.rs`:
   - `ProfileScope` applies hints only at `ComponentStep`,
     `ComponentScript.steps`, `ConstructionScript.components[].steps` and
     `AnalysisReading.rewrite.steps`;
   - arbitrary extra/attribute subtrees are never traversed for a `profile`
     key;
   - first-alias choice uses the ordered representation; duplicate alias keys
     retain first position/last value like Python;
   - all `from_json_*` and `load_*_json_*` entry points pass the correct scope.
5. Fixed full uniqueness in `src/models.rs`: `canonicalize_texts` and
   `canonicalize_identifiers` now detect any duplicate with a hash set while
   preserving order for unsorted fields (constraints/findings); sorted
   identifier fields still sort after validation. `unique_digests`/`unique_by`
   were checked and already compared all earlier values.
6. Removed direct `serde_json` from `Cargo.toml`; the local `Cargo.lock` was
   regenerated. It remains only transitively through accepted `kala-budget`
   (and as a dev-dependency in `kala-json`'s own test graph).
7. Updated crate rustdoc/README for the ordered parser, the absence of the
   fallback edge, the fixed profile paths and the Pydantic
   `model_validate_json` difference.
8. Added `tests/parser_contracts.rs` (Rust regressions, deep-stack cases) and
   extended `tests/oracle/contracts_oracle.py` +
   `tests/python_differential.rs`:
   - oracle cases now carry an `entry` of `str` or `bytes`;
   - 18 bytes-entry cases exercise `from_json_bytes` and
     `load_*_json_bytes`;
   - raw duplicate/marker/surrogate JSON is stored as the original text and
     reaches Rust unchanged;
   - a transport test asserts duplicate and escaped-marker text survives the
     oracle JSON envelope.
9. Rewrote the relevant `REPORT.md` sections and added the R2 evidence log.

## Focused attempts

- Uniqueness/private-tag repair: one focused pass; no second attempt against
  the same failure was needed.
- Last-wins/codepoint repair: this is the allowed second focused repair for
  that failure family; it passed the first time after implementation.
- No full-migration, Windows-runtime, performance, HTTP-422 or packaging claim
  is made.

## Verification performed

From `backend/crates/kala-contracts`:

- `cargo fmt --all -- --check` — pass.
- `cargo test --locked` — 50 tests pass (15 unit, 33 integration, 2 live
  differential; 0 failed, 0 ignored).
- `cargo test --locked --release` — 50 tests pass.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` — pass.
- `cargo build --locked --release` — pass.
- `cargo check --locked --all-targets --all-features --target
  x86_64-pc-windows-gnu` — pass (compile/check only, not executed).
- Windows-target clippy `-D warnings` — pass (compile/check only).
- Live oracle — **486 cases: 253 accepted, 233 rejected**, all wire/canonical/
  digest/status/round-trip comparisons match CPython 3.14.8; construction error
  messages match exactly; 18 cases use bytes entry points.
- R2-specific live cases include nonadjacent/normalized duplicate
  constraints/findings/references, marker-shaped objects nested with siblings/
  escaped spelling, duplicate + escaped surrogate + arbitrary bigint,
  duplicate alias keys with order, loader string/bytes entry points, and strict
  canonical-string duplicate refusal.
- Rust-only deep tests — 50,000-level `attributes_json`, 50,000-level ignored
  profile body and 50,000-level arbitrary extra subtree, all without stack
  overflow.

## Remaining / handoff

- Awaiting GPT-6.1-Sol review and conductor acceptance.
- Root workspace/lock unification and dependent writer integration remain
  outside this cell.
- Known source-limited differences are listed in `REPORT.md` section 6,
  including the Pydantic direct `model_validate_json` surrogate behavior (the
  source HTTP path is `json.loads` + `model_validate`), exotic IDNA2003 tables,
  ordinary error-string exactness and compile-only Windows status.
- No unresolved build/test errors.
