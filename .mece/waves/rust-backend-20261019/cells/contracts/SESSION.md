# Contracts cell — session notes

## Launch record

- Wave: `rust-backend-20261019`, cell `contracts`
- T3 task: `node:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-contracts-r1-go-retry46`
- Child thread: `thread:delegated-task:command%3Amcp%3A3835ea6b-b602-40a5-985a-4572190f2d42%3Adelegate-task%3Arust-backend-20261019-contracts-r1-go-retry46`
- Implementation model: `opencode-go/deepseek-v4.1-flash` variant max.
- Review model: GPT-6.1-Sol (conductor), not run by this worker.
- No delegation, provider switch, commit, push or deploy.

## Work performed

1. Read `TASK.md`, `WAVE.md`, `SYNTHESIS.md`, the audit report and the actual
   Python contracts (`models.py`, `construction.py`, `__init__.py`) as the
   source of truth. Read the accepted `kala-json`/`kala-budget` APIs and the
   persistence canonical-object rules for compatibility context.
2. Created only `backend/crates/kala-contracts/**` as a standalone workspace
   with its own lockfile. Added read-only path dependencies on `kala-json`
   and `kala-budget`; no parent manifest/lock or other crate edits.
3. Implemented the full public model surface:
   - all enums and values, all models, defaults, extra-field refusal;
   - identifier/digest/media-type/URI normalization;
   - Pydantic `AwareDatetime`/speedate timestamps;
   - arbitrary-precision nonnegative integers and scale-preserving Decimal;
   - canonical JSON objects with the persisted unbounded domain;
   - all cross-field invariants;
   - ordered wire JSON, sorted/ASCII canonical JSON and SHA-256 digests.
4. Implemented construction scripts: nested-profile flattening and aliases,
   profile/shape inference, component sequence state machine, assembly
   references, analysis ordering, edit/reading bounds, and Python-compatible
   `ConstructionError` message selection.
5. Built a live differential oracle (`tests/oracle/contracts_oracle.py` +
   `tests/python_differential.rs`) and 450 cases. Also added Rust-only
   invariant, scalar, construction, canonical-depth and shared-corpus tests.
6. Repaired divergences found by the oracle and targeted probes:
   - `PlanStep.max_attempts`/`WorkflowDefinition.max_replanning_attempts`
     defaults are 1, not 0;
   - required tuple fields (`states`, `available_step_ids`, `options`,
     `subject_ids`, construction `steps`/`components`) now report missing
     fields instead of silently defaulting;
   - timestamp epoch millisecond inference and speedate negative-float
     rounding;
   - `_`/U+2212 timestamp spellings and epoch forms `1.`, `.5`, `+.5`;
   - Pydantic `string_unicode` refusal for `NonEmptyText` lone surrogates;
   - Pydantic tuple/type messages used by construction loaders;
   - named-profile fallback short-circuiting for truthy non-string or blank
     `profile["type"]`;
   - review P2 repairs: model JSON last-wins duplicate keys, and ordered
     construction profile alias interpretation across string/byte entry points,
     nested scripts, readings and loader consumers.
7. Wrote `README.md`, this report and session notes. No new work was started
   for dependent cells.

## Important decisions

- Contracts stay architecturally below persistence; the accepted persistence
  canonical-object limits were mirrored rather than introducing a
  `kala-persistence` dependency (which would pull bundled SQLite into every
  consumer).
- The public API exposes separate wire, canonical and typed forms so callers
  cannot accidentally hash HTTP bytes.
- Ordinary model JSON entry points now use Python/FastAPI last-wins duplicate
  handling. Construction JSON/bytes entry points preserve object insertion
  order for profile aliases; the old `CanonicalValue` loader path remains
  deterministic and is documented as order-lossy.
- `serde_json` is used only as the permissive ordered model-input parser;
  every number/string is converted explicitly to `CanonicalValue`. No
  `serde_json::Value` reaches a model field, and `WireValue`/`CanonicalValue`
  preserve arbitrary integers, signed zero and surrogate code points.
- Construction JSON/bytes entry points preserve Python insertion order for
  profile aliases, including nested scripts and loaders. Only
  `from_value`/`load_*(&CanonicalValue)` is deterministic canonical order, as
  documented and tested separately.

## Verification performed

Run from `backend/crates/kala-contracts`:

- `cargo fmt --all -- --check` — pass.
- `cargo test --locked` — 43 tests pass (15 unit, 27 Rust integration, 1 live
  Python differential; 0 failed, 0 ignored).
- `cargo test --locked --release` — 43 tests pass.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` — pass.
- `cargo build --locked --release` — pass.
- Live oracle: 450 cases (226 accepted, 224 rejected), all wire/canonical/
  digest/status/round-trip comparisons match CPython 3.14.8 with the repository
  `.venv`; shared `domain_provenance.json` manifests validate read-only.
- Rust-only: 5,000-depth and >2 MiB canonical objects, arbitrary integers,
  signed zero and lone-surrogate canonical fields.

## Remaining / handoff

- Awaiting GPT-6.1-Sol review and acceptance.
- Root workspace/lock unification and any dependent writer integration are
  outside this cell.
- No unresolved build/test errors. Known source-limited differences are listed
  in `REPORT.md` section 6.
- No Windows/COM, performance, packaging or cutover claim was made.
