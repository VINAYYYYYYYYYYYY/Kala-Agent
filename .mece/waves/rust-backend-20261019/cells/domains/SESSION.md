# Domains session log

## Identity and policy

- Cell: `rust-backend-20261019/cells/domains`
- T3 task/child IDs: recorded in `LAUNCH.md`
  (`...rust-backend-20261019-domains-r1-go-review52`)
- Model used for all implementation: `opencode-go/deepseek-v4.1-flash`
  variant max only. No delegation, provider switch, fallback model, commit,
  push, deploy, cutover, or packaging action.
- Ownership observed: new `backend/crates/kala-domains/**` plus this cell's
  `REPORT.md`/`SESSION.md` only. Root workspace/lock, other crates, Python
  product/tests, adapters, frontend/desktop/build/packaging, fixtures, and user
  references were read-only.

## Work performed

1. Read `TASK.md`, `LAUNCH.md`, `WAVE.md`, `SYNTHESIS.md`, the accepted
   `contracts/REVIEW-3.md`, the entire Python domain source
   (`domains/__init__.py` plus all 15 mechanical/building/wing modules), the
   unit/security/integration domain tests, accepted `kala-contracts`,
   `kala-json`, and `kala-budget` public surfaces, and
   `backend/fixtures/corpus/v1/domain_provenance.json`.
2. Created a standalone Cargo workspace/crate with a local lock, read-only path
   dependencies, and no root manifest edit.
3. Ported shared declarations, all parameter models/specs/defaults, materials
   and units, all three deterministic simulations, all ten evaluators, all
   workflow/catalog declarations, package validation/manifests, capability
   enforcement, the reference workflow runner, and the typed domain registry.
4. Added build-time native Rust source hashing (`build.rs` +
   `src/source_digest_core.rs`) and runtime audit/recompute support
   (`src/source_audit.rs`). Embedded module/package source identities replace
   only evaluator/package identity; parameter-schema digests are preserved.
5. Added an explicit legacy Python-v2 compatibility module and record, with
   fixture-verified historical constants and the re-preparation/re-approval
   boundary. Native identities were not relabelled and legacy values were not
   reused.
6. Built a live-Python differential oracle and tests covering normalized
   parameters, schema bytes, every simulation field, every evaluation, workflow
   stopping/status/scores/findings, invalid inputs, and capability decisions.
7. Added native declaration/contract tests, substitute-evaluator identity and
   capability tests, serializer-tag/bigint boundaries, source mutation tests,
   and embedded-versus-recomputed digest checks.
8. Ran fmt, locked debug/release tests, Linux all-target/all-feature clippy
   with `-D warnings`, release build, Windows GNU check/clippy, Windows MSVC
   check, and a manual touch-rebuild plus clean-rebuild determinism check.

## Key technical decisions

- Parameter input uses `kala_json::CanonicalValue` (and strict JSON text/bytes)
  exclusively. No serde value or serializer tag can reinterpret domain input.
- Canonical output and float formatting reuse `kala-json`; contract results and
  manifests reuse `kala-contracts`; `round(x, 6)`/clamp reuse the accepted
  `kala-budget` helper rather than a new rounding implementation.
- Arithmetic order mirrors the Python expressions exactly (including
  multiplication/parenthesisation order and the wing bounded station loop).
- Evaluator identities are explicit native declarations supplied by each
  evaluator, because Rust has no Python-style runtime source introspection.
  Shipped identities are build-embedded; custom evaluators use a validated
  public constructor.
- Native source digest labels (`rust-package:`, `rust-shared:`) deliberately
  differ from Python's labels, and the scheme/language are part of the digest
  payload.
- The registry is an object-safe trait plus typed `DomainParameters` enum, so
  future application/core API code can dispatch by name without losing the
  concrete parameter types.

## Failures and focused repairs

No product-semantics defect was found by the live differential. Issues fixed
during bring-up:

1. Initial compile issues (module re-export names, `LazyLock` deref at trait
   object coercion, non-const `sqrt` in a `const`) were corrected in one pass.
2. Test-harness corrections, not product repairs:
   - fixture path depth;
   - several values initially listed as invalid are valid Pydantic coercions
     (`"storey_count": "4"`, `"storey_height_m": "3.0"`, graceful numeric
     strings/bools) and were moved to positive cases;
   - the exact capability grant was initially in the denial matrix and moved to
     the success case;
   - the missing-transition checkpoint fixture needed a reachable-state bypass
     edge so it exercised checkpoint validation rather than workflow
     reachability.
3. One legacy Wing planform constant was mistyped and corrected against the
   read-only corpus fixture; the fixture comparison test now pins all legacy
   values.

No more than two focused repairs were applied to any single failing check.

## Verification snapshot

- `cargo fmt --all --check`: pass.
- `cargo test --locked`: 25 tests pass (4 lib, 14 contract, 3 differential,
  4 source mutation).
- `cargo test --locked --release`: same 25 pass.
- Linux `cargo clippy --locked --all-targets --all-features -- -D warnings`:
  pass.
- `cargo build --locked --release`: pass.
- Windows GNU all-target check and clippy `-D warnings`: pass (compile only).
- Windows MSVC all-target check: pass (compile only).
- Rebuild determinism: mtime-only rebuild and clean release rebuild produced
  the same generated source-digest file
  (`sha256 179d6f9a991f9fc5e4886d25426f4202429331026a2286bd1538dcf7531fd95f`).
- Live oracle: Python 3.14.8 / Pydantic 2.13.5.

Exact digests, case counts, API and limits are in `REPORT.md`.

## Handoff state

- No live child turns; work is terminal for this cell's original task.
- No commit/push/deploy/cutover performed.
- Native crate is ready for independent review. If review requests changes,
  recompute all embedded digests via a normal rebuild and update `REPORT.md`
  identity values; do not introduce Python source digests as native identities.
- Remaining product integration (repository/application migration, native
  revision writes, re-approval flow, packaging, Windows runtime) belongs to
  later cells and was intentionally not started here.

## Repair round 2 (REVIEW-1 findings)

The conductor authorized this fresh repair round for the same cell and owned
paths. Model remained `opencode-go/deepseek-v4.1-flash` variant max only; no
delegation, provider/model switch, commit, push, deploy, source deletion, or
cutover.

1. **Workflow score summation.** Replaced the naive accumulator in
   `ReferenceWorkflowRun::score` with a line-for-line port of CPython 3.14's
   Neumaier `CompensatedSum` used by builtin `sum()` (including the
   `fabs(hi) >= fabs(x)` branch and the `lo != 0 && isfinite(lo)` final step).
   The review's four-score case now matches Python `0.299`. Added an internal
   bit-level unit test plus a 19-case live oracle matrix over custom
   `ReferenceWorkflowRun`s (order, length, `None`, bounds, repeated values,
   deterministic random arrays). Real domain workflow score bits remain in the
   67-case differential.
2. **Permissive model JSON boundary.** Added `src/model_json.rs`, an iterative
   ordered parser used only by `ParameterModel::from_json_str`/`from_json_bytes`.
   It follows Pydantic `model_validate_json`: last-wins duplicate keys with
   first insertion position, escaped-key collapse, surrogate pairs decoded,
   lone surrogates rejected, non-finite/overflowing numeric literals passed to
   field range validation, ordered extra rejection, 4,300-digit integer limit,
   and no serde value anywhere. Canonical/hash and `from_value` paths remain
   strict. Added a 35-case live model-JSON differential across all three
   domains and a local regression test.
3. **Local dependency provenance.** Extended `source_digest_core` and
   `build.rs` to hash `kala-budget`, `kala-json`, and `kala-contracts` source
   trees (`Cargo.toml`, optional `build.rs`, `src/**/*.rs`) into the native
   package source digest. Added `SourceRoots`/`DependencySourceRoot` audit API,
   dependency `rerun-if-changed` entries, and a temporary-copy mutation test
   for each dependency source and manifest. Documented the exact boundary:
   accepted local dependency source/manifests are included; external crates,
   compiler, target triple, profile flags, and binaries are not.

## Round 2 verification snapshot

- `cargo fmt --all --check`: pass.
- `cargo test --locked`: 29 tests pass (5 lib, 15 contract, 4 differential,
  5 source mutation).
- `cargo test --locked --release`: same 29 pass.
- Linux all-target/all-feature clippy `-D warnings`: pass.
- `cargo build --locked --release`: pass.
- Windows GNU all-target check and clippy `-D warnings`: pass (compile only).
- Windows MSVC all-target check: pass (compile only).
- Live oracle: Python 3.14.8 / Pydantic 2.13.5; 67 parameter cases, 51 invalid
  cases, 35 model-JSON cases, 19 score cases, 9 capability scenarios.
- Rebuild determinism: release generated source-digest file SHA-256
  `01ffdd014182080ed72886ff40a20cb01adebdb94aaae9283ec7911326053a09`
  across mtime-only and clean release rebuilds.

No more than two focused repairs were applied to any single failing check. The
new dependency-provenance values and the full changed-value explanation are in
`REPORT.md`. Handoff state is unchanged: no commit/push/deploy/migration, and
remaining application/repository integration stays with later cells.

