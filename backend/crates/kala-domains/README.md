# kala-domains

Native Rust declaration and arithmetic layer for the Kala Agent domain
packages. This crate replaces the pure domain package surface in
`src/kala_agent/domains/{__init__,mechanical,building,wing}/`:

- shared parameter specifications, catalog steps, evaluation contexts,
  evaluator identities, manifests, capability enforcement, and the
  deterministic reference workflow runner;
- the mechanical bracket package (plate, hole pattern, material, load case);
- the building storey/envelope package (transmission-only thermal surrogate);
- the wing package (planform, span load, finite-wing surrogates);
- a typed registry and dispatch surface for `building`, `mechanical`, and
  `wing`.

It does **not** implement CAD, CAE, a solver, a building code, or a certified
analysis path, and it never imports `kernel`, `runtime`, `persistence`, tools,
the API, or the CLI. Every evaluator is a bounded pure function.

## Public API

- `ParameterSpec`, `CatalogStep`, `ParameterDefault`, `parameter_schema_digest`
- `EvaluationContext`, `reference_context`, `reference_evaluation_epoch`
- `DomainEvaluator`, `EvaluatorImplementation`, `DomainPackage`,
  `DomainPackageParts`, `DomainPackageApi`
- `CapabilityRequest`, `CapabilityGrantPolicy`, `enforce_capability_request`
- `run_reference_workflow`, `ReferenceWorkflowRun`, `StepOutcome`
- `DomainError`, `ParameterError`, `DomainCallError`
- `DomainName`, `DomainParameters`, `package`, `packages`, `manifests`,
  `evaluate`, `evaluate_typed`
- `mechanical`, `building`, and `wing` modules with their declared constants,
  parameter models, simulations, evaluators, workflows, and reference runs
- `source_audit` for recomputing the native source digests
- `provenance_compatibility` and `provenance::legacy` for the explicit
  legacy-Python versus native-Rust identity record

Parameter input is a `kala_json::CanonicalValue` object (or strict JSON text),
never a lossy serde value. Integer, float, boolean, and string JSON kinds stay
distinct, and an input key such as `$serde_json::private::Number` is ordinary
data. `ParameterModel::from_json_str`/`from_json_bytes` match Pydantic's
`model_validate_json` input boundary: duplicate object keys use last-wins
semantics with the first insertion position retained, escaped surrogate pairs
decode to one code point, and extras are refused in document order. Canonical
and hash parsing remains strict about duplicates. The models follow Pydantic's
accepted coercions for these bounded fields: numeric strings (with Python
underscore rules), integral floats, booleans, enum keys, range bounds,
NaN/overflow rejection, and the mechanical/building/wing cross-field
invariants. Errors carry Pydantic's message text and the actual Python domain
exception class in `DomainError::class_name`.

Canonical JSON, CPython float formatting, contract models, and Python
`round(x, 6)` semantics come from the accepted read-only `kala-json`,
`kala-contracts`, and `kala-budget` crates.

## Provenance scheme

Parameter-schema digests are byte-identical to the Python values because they
cover the same declaration payload. They are *not* relabelled.

Evaluator identities are a new, explicit native scheme:

- version: `evaluator-implementation-digest-v3-rust`
- language marker: `rust`
- module name: `kala_domains.<domain>.evaluation`
- module digest: SHA-256 of `src/<domain>/evaluation.rs`
- package digest: canonical digest over every `.rs` under `src/<domain>/`, every
  shared `.rs` directly under `src/`, and the source trees and manifests of the
  accepted local dependencies `kala-budget`, `kala-json`, and `kala-contracts`
- implementation digest: canonical digest binding step id, qualified name,
  module/package source digests, declared capabilities, scheme, and language

`build.rs` hashes those native sources at build time and embeds the
module/package digests as constants. Python, Python packages, and Python source
files are not available to or used by the build. `source_audit` applies the same
shared rules at test time, so embedded values are verified against the current
tree and mutations of evaluator, sibling simulation, constants, shared-helper,
and local-dependency sources change the native digests. A clean rebuild from
identical sources produces identical constants. External crates, the compiler,
target triple, profile flags, and built binaries are outside this source
identity.

Legacy Python-v2 digests live only in `provenance::legacy` and in
`ProvenanceCompatibilityRecord`. They remain immutable historical identifiers.
Native package/evaluator digests differ by design. New native states and
revisions must carry the native digests; legacy prepared actions, approvals, and
commit bindings require explicit re-preparation and re-approval when the
identity differs. This crate performs no repository/application migration.

## Verification

`cargo test --locked` runs native declaration/contract tests, the live-Python
differential oracle, and the source mutation tests. The oracle requires the
repository virtualenv (`.venv/bin/python`, overridable with `KALA_PYTHON`) and
fails loudly if it is absent. It compares normalized parameter bytes, schema
bytes/digests, every simulation field bit-for-bit, evaluation contract bytes,
workflow status/stop order/scores/findings, invalid-input messages, and
capability denial/success including "denied before any evaluator ran".
