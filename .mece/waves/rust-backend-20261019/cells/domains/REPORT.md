# Domains cell report — native `kala-domains` + explicit Rust provenance

Status: **repair round 2 complete** in the new standalone crate; ready for
independent re-review. All three findings in `REVIEW-1.md` are addressed. No
commit, push, deploy, cutover, packaging change, root manifest/lock edit, Python
product/test change, adapter/frontend/desktop change, or other-cell change was
made. Model: `opencode-go/deepseek-v4.1-flash` variant max only. T3 task/child
IDs are in `LAUNCH.md`.

## Review findings resolved

1. **CPython-compatible workflow score.** `ReferenceWorkflowRun::score` now uses
   the same Neumaier compensated float summation as CPython 3.12+ `sum()`
   (`hi`/`lo`, `fabs(hi) >= fabs(x)` branch, `lo != 0 && isfinite(lo)` final
   step) before dividing by the score count and applying
   `clamp_unit_interval`. The review case
   `[0.356107, 0.301885, 0.49567, 0.042336]` now produces exactly Python's
   `0.299` instead of `0.298999`. Regressions cover the review case, reversed
   order, `[0.1] * 10`, cancellation-style sequences in the internal helper,
   zero/one bounds, mixed `None` scores, empty runs, repeated `1.0`, and
   deterministic arrays of lengths 1–73; the 67 real domain differential cases
   still compare `run.score()` bits.
2. **Permissive model JSON input.** `ParameterModel::from_json_str` and
   `from_json_bytes` now use a new iterative `model_json` parser instead of the
   strict canonical parser. Duplicate object keys use Pydantic last-wins
   semantics with the first insertion position retained; escaped spellings of
   the same key collapse; surrogate pairs decode to one code point; lone
   surrogate escapes are rejected as invalid JSON; non-finite literals
   (`NaN`, `Infinity`, `-Infinity`) and overflowing decimal literals reach the
   field range checks exactly as Pydantic does; extras are refused in document
   order before field validation. No `serde_json::Value` is involved, so
   `$serde_json::private::Number`-shaped keys/values remain plain data.
   Canonical/hash parsing remains strict and still rejects duplicates.
   Regression coverage spans all three models, both text/byte entry points,
   duplicate declared/extra keys, nested objects, private-number shapes,
   surrogate pairs/lone surrogates, `NaN`/overflow, and malformed documents.
3. **Truthful local-dependency provenance.** The native package source digest
   now binds the source trees and manifests of the accepted local dependencies
   `kala-budget`, `kala-json`, and `kala-contracts`, in addition to this crate's
   domain and shared Rust sources. `build.rs` hashes those files at build time
   and emits `rerun-if-changed` for them; no Python or runtime source lookup is
   used. Mutation tests copy the crate and dependency trees, mutate
   `kala-budget/src/compat.rs`, `kala-json/src/float.rs`,
   `kala-contracts/src/wire.rs`, and a dependency `Cargo.toml`, and prove the
   package/implementation identity changes while module digests stay stable.
   The exact source-identity boundary (and its exclusions) is stated below and
   in `PROVENANCE_BOUNDARY`.

## Scope delivered

New crate: `backend/crates/kala-domains/**` with its own workspace and
`Cargo.lock`. It ports the entire pure domain declaration/arithmetic layer:

- `src/kala_agent/domains/__init__.py`: parameter specifications, catalog
  steps, evaluation contexts, evaluator protocol/identity, package validation,
  manifests, capability policy enforcement, reference workflow runner,
  canonical JSON/digest helpers, schema digests, rounding/clamping constants;
- `mechanical/{parameters,simulation,evaluation,workflow}.py`;
- `building/{parameters,simulation,evaluation,workflow}.py`;
- `wing/{parameters,simulation,evaluation,workflow}.py`;
- all module `__init__` exports through `mechanical`, `building`, `wing`;
- a typed registry/dispatch layer for the three domain names.

No kernel/runtime/persistence/CAD/solver/network dependency was added. The
only external calls are test-time invocations of the repository `.venv` Python
as the read-only oracle.

Dependencies are read-only path dependencies: `kala-contracts` (typed contract
models), `kala-json` (canonical JSON and CPython float formatting), and
`kala-budget` (accepted Python `round(x, 6)`/clamp semantics). No accepted
library API was edited or worked around.

## Public API (crate root)

Declarations and evaluation:

- `ParameterSpec`, `ParameterDefault`, `CatalogStep`, `ParameterModel`
- `parameter_schema_digest`, `canonical_json`, `canonical_digest`
- `EvaluationContext`, `reference_context`, `reference_evaluation_epoch`,
  `REFERENCE_EVALUATION_EPOCH_ISO`, `Timestamp` re-export
- `EvaluatorImplementation`, `DomainEvaluator`, `DomainPackage`,
  `DomainPackageParts`, `DomainPackageApi`
- `CapabilityRequest`, `CapabilityGrantPolicy`, `enforce_capability_request`
- `run_reference_workflow`, `ReferenceWorkflowRun`, `StepOutcome`
- `DomainError` (`class_name`, exact `message`), `ParameterError`
  (`field_name`, `kind`, Pydantic-style `message`/`summary`), `DomainCallError`
- `require_identifier`, `require_digest`, `require_name`,
  `require_capabilities`, `parse_checkpoint_kind`, `clamp_unit_interval`

Domain and dispatch:

- `mechanical`, `building`, `wing` modules with declared constants, parameter
  models, simulations, evaluators, workflows, reference runs;
- `DomainName`, `DomainParameters`, `package`, `package_for`, `packages`,
  `manifests`, `evaluate`, `evaluate_typed`;
- `source_audit` (`SourceRoots`, `DependencySourceRoot`, `LOCAL_DEPENDENCIES`,
  `crate_source_roots`, `digest_domain`, `source_files`,
  `current_domain_digests`, `embedded_identity`) and `provenance`
  (`native_source_identity`, `EvaluatorImplementation`, `legacy`,
  `provenance_compatibility`, `ProvenanceCompatibilityRecord`).

Parameter models accept `kala_json::CanonicalValue` objects (or JSON
text/bytes) through `ParameterModel::from_value`, `from_json_str`,
`from_json_bytes`, and expose `to_wire_value`, `to_wire_json`,
`to_wire_json_ascii`, `to_canonical_value`, `to_canonical_json`.
`from_json_str`/`from_json_bytes` are the permissive Pydantic model-input
boundary; `from_value` and all canonical/hash paths remain strict. No
`serde_json::Value` is on the parameter path.

## Provenance scheme

Parameter-schema digests are computed from the same declaration payload and
remain byte-identical to Python. Evaluator/package identities use the native
scheme:

- digest version: `evaluator-implementation-digest-v3-rust`
- language marker: `rust`
- module name: `kala_domains.<domain>.evaluation`
- module digest: SHA-256 of `src/<domain>/evaluation.rs`
- package source digest: canonical digest of sorted `[label, sha256]` pairs
  covering:
  - every `.rs` under `src/<domain>/` (`rust-package:` labels);
  - every shared `.rs` directly under `src/` (`rust-shared:` labels);
  - every `.rs` under each accepted local dependency's `src/`, plus its
    `Cargo.toml` and optional `build.rs`
    (`rust-dependency:<crate>/<path>` labels);
- implementation digest: canonical digest binding scheme, language, step id,
  qualified name, module/package source digests, and canonical capabilities.

`build.rs` computes this at build time and embeds the module/package source
digests as constants. It does not invoke Python and does not read Python source
files. `src/source_digest_core.rs` is compiled by both the build script and the
library, so `source_audit` uses the exact same algorithm. `rerun-if-changed`
covers the dependency source trees and manifests.

**Exact source-identity boundary:** the identity covers the bytes of this
crate's Rust sources and the accepted local dependency source trees/manifests
listed above. It does **not** cover external crates (`num-bigint`, `sha2`,
`idna`, `unicode-casefold`, etc.), the Rust compiler version, target triple,
profile flags, linker, generated machine code, or built binaries. No executable
or binary digest is claimed.

Current embedded source identities:

| domain | module digest | package source digest |
| --- | --- | --- |
| mechanical | `dc986cf5209d6b9ec2afc153ad4d4edc10350d01f3f6b530d04208d23365e73e` | `bc5d35ca031ffb74bb36cae8a4d4da41d761b4ef454369a4691b2a451533c98a` |
| building | `37e033ef036236c55ba0ce44eb8422194d1988fe2b6b285e68405712981662b8` | `816b9daf175cfc77ed725885d6e7dcdafb29a69fd536797c7e4cb68c486ca740` |
| wing | `187c805bc734aff6aa324aa6c35376f5551b50f4cbb39374ce50b19e3221002e` | `196bb8c8be5f72b109f3d3c34dbb2b4d92076b9bb35af5ba5d2d6b03c8271643` |

Native/legacy identity comparison:

| domain | native package digest | legacy Python-v2 package digest | schema digest (preserved) |
| --- | --- | --- | --- |
| mechanical | `a66389708e95ccff02bb1e4011983017107471fc4c1bcc8aed5925d718cec25e` | `97a2ebade1532e16864d81bc1f42b725c584c14250d1ae1f946eff1d4d5937b5` | `9fb3d2b068184cedff55fe47664f3cd98fabe0528e4324bdeb9bfc16de284721` |
| building | `f654a28d9f63f96be11a5e2465998f2195eb8de2bf49c19b2d5bff285df0f10c` | `a6d193634794b7a0f28af54ea451924314d61e803ced03610efdbfd6570f40b7` | `8ba15360e4e181e5a78686a84394039a27e176477c0edcb72197b4c104ad8b45` |
| wing | `c76046248dc7579a5a0f4f2cf3bed5a699b92048dc65757390d6c5946cc0ab63` | `108cd68bf1ec08088f526e5ef0b8f2034199298fe7dc443e7c425e46d2358c71` | `898306c1388c1449d0049693aa66e90493f1aef7e84acaed160c8f48bc41d15e` |

Native implementation digests:

| step | native Rust | legacy Python-v2 |
| --- | --- | --- |
| mechanical.constraint-evaluation | `0e357bce18ac2428114c0d85731636ae130399a3a0f2bf6e1cd6281c0eda8002` | `62281ffb23e7732669de991bc7bac6acf21b05404e8ec2ccbd3bdb68a01a419b` |
| mechanical.geometry-check | `6ffa9114f27c954ed6e3b4c5a4ed6cb07b5f53854dd9090f339e0d739ed3b27b` | `a3c6ed56837677e22faff1206fb8dd5517e2c4b94e9932a84e540b3f91c5bc6b` |
| mechanical.load-check | `b97c9281924311fc35507ecd533bb53dbfca1089fabb545a215710b5232551f3` | `f72ecbc5b34f7f18f0723a759954ea400e863ce619c776b8b994b3c8eb9bceba` |
| mechanical.material-check | `83872d2e8d52e6fabadd2d15b761e57e7ce660d8399941ad46e2accc3d6d044a` | `acafdfa1334cfc090ae552f247384358f5f95d4acacacb4d709706ee839cdf35` |
| building.area-model | `f98b9dcf942148d8b4b42e6fe60b37ca76f4c2c09df3fb71b3762fc6649c4f23` | `17c7b04d7be4f4999e4bdd9ef0180aa3ed6ae5cd4aaf8702772db36e4d1aea74` |
| building.storey-model | `021a82b503f23d4644b60706e782ec048c7bb066e16171c0a90c12644dbd427e` | `6413241c2e547294b449ae570bfc2bac613ed1c47c36fa471bad69f52889b1ad` |
| building.thermal-envelope | `b9c39baec27a8b1dc8418ede2dc7c3e757bd98dee6d8e9c7f43af6492a96eb37` | `059e747203ac38c87d629e518ff5de4e686e6166aa42b1f15a91e1ac798f05fa` |
| wing.aerodynamic-surrogate | `79159c986b532cd651e02d4f49d135f74b8e1cefea32ffcc9ef4f9ed96acd3e5` | `a038013612a991ab6da4caaf8290c61a2cadc40e6be1c2636e0486beb00c7d77` |
| wing.planform-geometry | `88ed801b1ae3330d5e620621f885e4e48c8f3aec4eea6398bdad98a45fa998ef` | `0935c2012b7086fd24941ead52d5370b167a9ce86dcc9970f108b13517afadc3` |
| wing.span-load | `7e4fe73b1a7c67bb98d6d8add2cd4ff4107f1e7e320fc32735741052e87d6817` | `c0d4b43c4d5c66da474aa6b11440e22f544d0d3b2bbab9d57f5aa2cf4db7f75d` |

What changed and why:

- **Parameter-schema digests: unchanged.** The declaration bytes and meaning
  still match Python exactly.
- **Step/workflow declaration bytes: unchanged.**
- **Evaluator implementation digests: changed from round 1** because the
  package source identity now includes the local dependency source trees.
- **Package and manifest digests: changed from round 1** for the same reason.
  The parameter-schema digest member is preserved.
- **Legacy Python-v2 values: unchanged historical constants.** They still
  identify Python source bytes and are never used as native identities.

`provenance_compatibility(domain)` records
`implementation_identity_equivalent = false` and
`requires_reparation_and_reapproval = true`, and its boundary text now includes
the local-dependency source scope and exclusions. It verifies legacy members
against `backend/fixtures/corpus/v1/domain_provenance.json` read-only. Stored
Python-v2 revisions remain immutable readable legacy records. Newly computed
native states/revisions must carry native package digests; legacy prepared
actions/approvals/commit bindings require explicit re-preparation/re-approval
when identities differ. No repository/application migration was implemented
here.

## Parity and test evidence

Live oracle: Python 3.14.8 / Pydantic 2.13.5 in the repository `.venv`.
Oracle path: `tests/oracle/domains_oracle.py`; Rust differential:
`tests/python_differential.rs`. A missing oracle fails the gate.

The differential compares, per valid parameter case:

- exact normalized parameter JSON bytes in field order and canonical digest;
  exact parameter-schema digest;
- every simulation field bit-for-bit (18 mechanical, 18 building, 14 wing);
- every evaluation contract's canonical bytes (IDs, subjects, timestamps,
  passed flags, metrics, units, rounded scores, findings);
- workflow status, ordered/completed steps, blocked step, definition identity,
  per-outcome status/reason, flattened findings, and mean score bits;
- violation lists in declaration order.

Case counts:

- 67 valid parameter cases (3 reference defaults + 19 boundary + 45
  deterministic random), each evaluated through every step until a hard stop;
- 51 invalid-input cases compared by first field and exact Pydantic message;
- 35 raw model-JSON document cases across all three models, compared by
  normalized wire bytes and canonical digest, or by first error field/message
  (document-level parse errors by `json_invalid`/`model_type` class);
- 19 workflow-score cases (`ReferenceWorkflowRun` with valid
  `EvaluationResult`s), compared by score bits, including the review case;
- 9 capability scenarios.

Capability scenarios cover: no policy with a declared capability, empty grant,
surplus grant, exact grant, package-only capability, step+package capabilities,
missing package capability, and a capability-free run. Denial is checked before
any evaluator runs (recording evaluator call count is zero); exact request
package id, step order, and sorted capabilities are compared. Python and Rust
exception class/message matched for every denial.

Other native tests cover:

- all declared defaults/specs/field order, cross-field invariants, material
  variants, enum keys, boolean/string/integral-float coercions, numeric
  underscore rules, Unicode-digit rejection, NaN/infinity/overflow rejection,
  range messages, and `$serde_json::private::Number`-shaped keys;
- Pydantic permissive model JSON: duplicate last-wins (text and bytes), escaped
  key spelling, surrogate pairs, lone-surrogate rejection, non-finite literals,
  nested objects, extra ordering, and private-number shapes;
- context UTC normalization, IDs, prefix behavior, contract metric/result
  validation, duplicate metric/finding refusal, and score bounds;
- package consistency failures, checkpoint/transition mismatches, evaluator
  capability mismatch, substitute evaluator identity changes with identical
  numbers, capability canonicalisation/denial, workflow empty/duplicate/unknown
  steps and first-failure stopping;
- registry order/defaults/dispatch and manifest validity;
- native scheme markers, capability binding in implementation digests;
- source mutation of evaluator, sibling simulation, constants, shared helper,
  and each local dependency source/manifest changes the expected native
  digests; unrelated domains do not change for a domain-only mutation;
  identical copies/restoration are deterministic;
- embedded build constants equal a fresh source recomputation.

Rebuild determinism was checked outside `cargo test`: the release
`native_source_digests.rs` SHA-256 was
`01ffdd014182080ed72886ff40a20cb01adebdb94aaae9283ec7911326053a09`; an
mtime-only touch rebuild and a `cargo clean --release -p kala-domains` full
rebuild both produced byte-identical generated constants.

## Verification commands/results

Environment: rustc/cargo 1.99.1, Linux x86_64, offline locked builds.

- `cargo fmt --all --check` — pass.
- `cargo test --locked` — pass, **29 tests**: 5 lib, 15 domain-contract,
  4 differential, 5 source-mutation.
- `cargo test --locked --release` — pass, same 29 tests.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` — pass.
- `cargo build --locked --release` — pass.
- `cargo check --locked --target x86_64-pc-windows-gnu --all-targets
  --all-features` — pass (compile/check only).
- `cargo clippy --locked --target x86_64-pc-windows-gnu --all-targets
  --all-features -- -D warnings` — pass (compile/check only).
- `cargo check --locked --target x86_64-pc-windows-msvc --all-targets
  --all-features` — pass (compile/check only).
- Live Python differential — pass (included in both test runs above).

No Windows runtime, MSVC link/run, CAD/CAE, solver, packaging, or performance
claim is made. Windows evidence is compile/check evidence only.

## Known limits / explicit gaps

- Native evaluator identity is declared by each evaluator through
  `implementation()`. Rust cannot discover a substitute's source at runtime;
  shipped evaluators use build-embedded source identities and custom evaluators
  use a validated constructor. Tests prove mutated source copies change the
  identity algorithm.
- The source identity covers local dependency source/manifests, not external
  crates, compiler, target triple, profile flags, or built binaries; no binary
  digest is claimed.
- `CatalogStep` is more strongly typed than the Python dataclass:
  `max_attempts` is `u32` (no bool/float acceptance) and
  `estimated_duration_seconds`/checkpoint kind are typed. Shipped declarations
  and consumer-relevant behavior match.
- `ParameterError` represents the first Pydantic error with its field, type
  name, and message; multi-error aggregation and the full error payload are not
  reproduced.
- Document-level malformed JSON returns `json_invalid` with a generic
  `Invalid JSON: value must be valid JSON` message rather than jiter's exact
  line/column text. Field-level duplicate/coercion/range errors match the
  checked Pydantic field and message.
- Pydantic's field-level `float_type`/`int_type` labels are represented by the
  crate's `Type` kind with the exact Pydantic message; the live invalid-case
  matrix compares field and message, not every Pydantic type label.
- `require_capabilities` rejects a bare string at compile time (typed
  collection) rather than at runtime; invalid identifiers still produce the
  Python `DomainParameterError` message.
- Package source-digest labels are native (`rust-package:`, `rust-shared:`,
  `rust-dependency:`), not Python's `package:`/`ancestor:`; this is intentional
  and documented.
- No downstream repository/application migration, native revision write, or
  re-approval workflow was implemented or exercised. That boundary is only
  documented and recorded.
- No full backend or Windows runtime verification; this cell owns only the
  standalone domain crate and its reports.

## Owned files changed

New files under `backend/crates/kala-domains/`:

- `Cargo.toml`, `Cargo.lock`, `build.rs`, `.gitignore`, `README.md`
- `src/lib.rs`, `error.rs`, `input.rs`, `model_json.rs`, `spec.rs`,
  `parameters.rs`, `context.rs`, `package.rs`, `capability.rs`, `workflow.rs`,
  `registry.rs`, `provenance.rs`, `source_audit.rs`, `source_digest_core.rs`
- `src/mechanical/{mod,parameters,simulation,evaluation,workflow}.rs`
- `src/building/{mod,parameters,simulation,evaluation,workflow}.rs`
- `src/wing/{mod,parameters,simulation,evaluation,workflow}.rs`
- `tests/domain_contracts.rs`, `tests/python_differential.rs`,
  `tests/source_mutation.rs`, `tests/oracle/domains_oracle.py`
- this `REPORT.md` and `SESSION.md`

No root manifest/lock, other crate, Python product/test, adapter, frontend,
desktop, build, packaging, fixture, generator, or user-reference file was
edited.
