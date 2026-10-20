# Native shared contracts — cell report

Status: R2 focused repair complete, ready for GPT-6.1-Sol review of
`REVIEW-2.md`. The three newly confirmed regressions are repaired: full
sequence uniqueness for order-preserving text fields, the serde
arbitrary-precision private-marker collision, and duplicate + escaped-surrogate
model input. The earlier `REVIEW-1.md` repairs (ordinary model JSON last-wins
duplicates and insertion-order construction aliases) remain intact. No commit,
push, deploy, workspace unification, core HTTP or kernel cutover was performed.
Python is a test-only oracle; production code never invokes Python.

## 0. R2 repair summary

Ownership stayed exactly `backend/crates/kala-contracts/**` plus this cell's
`REPORT.md`/`SESSION-R2.md`. No parent workspace/lock, accepted crate, shared
corpus/generator, Python product/test, Studio/process, adapter, packaging or
user-file edit was made. The standalone workspace and local lock are retained.

Resolved `REVIEW-2.md` objections:

1. **Nonadjacent duplicates bypassed uniqueness.** `canonicalize_texts` and
   `canonicalize_identifiers` checked only `windows(2)`. They now use a full
   hash-set duplicate check while retaining original order for unsorted fields.
   `DesignSpec.constraints` and `EvaluationResult.findings` refuse adjacent,
   separated and whitespace-normalized duplicates; accepted values keep input
   order. `requirement_ids`/`artifact_ids` remain sorted after the same check.
2. **`$serde_json::private::Number` was reinterpreted.** `serde_json::Value`
   is no longer used anywhere in the crate. Model input uses a private
   explicit-stack `OrderedValue` parser; the marker-shaped object is ordinary
   user data at any depth, with siblings and with escaped key spelling.
3. **Duplicate + escaped surrogate was refused.** The ordered parser preserves
   duplicate last-wins semantics (first insertion position retained),
   code-point-faithful `\uD800` escapes, arbitrary integers and insertion order
   in one representation. There is no strict-parser fallback that drops
   duplicate/order support, and no serde private representation to collide
   with user keys.
4. **Deep input/cleanup and profile traversal.** Parsing, conversion to
   `CanonicalValue`, `OrderedValue` clone-free cleanup and `CanonicalValue`
   cleanup/validation/dump are iterative. Profile hints are applied only at the
   four known construction paths; arbitrary extra/attribute subtrees are never
   walked to find `profile`.

The strict shared `kala-json` API and all canonical/hash parsers were not
changed or made permissive. `CanonicalJsonObject` supplied as raw string text
still refuses duplicate keys.

## 1. Public API for dependent cells (review gate)

Dependent writers must wait for conductor acceptance before using this API in
another crate. The API is documented in
`backend/crates/kala-contracts/README.md` and rustdoc on every public item.

The R2 change adds **no public API**. The ordered parser/`OrderedValue` is
`pub(crate)`. Existing public signatures and compatibility are unchanged:
`ContractModel::from_json_str`/`from_json_bytes`, `to_wire_*`,
`to_canonical_*`, `canonical_digest*`, `from_value`, construction `load_*` and
the `load_*_json_str`/`_bytes` variants.

Crate: `backend/crates/kala-contracts` (standalone workspace + own
`Cargo.lock`, like `kala-studio`/`kala-step`). It is not a member of the root
workspace and no root manifest or lock was touched.

### Core trait

```rust
pub trait ContractModel: Sized {
    const NAME: &'static str;
    fn from_value(value: &kala_json::CanonicalValue) -> Result<Self, ContractError>;
    fn to_wire_value(&self) -> WireValue;
    fn from_json_str(text: &str) -> Result<Self, ContractError>;
    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError>;
    fn to_wire_json(&self) -> Result<String, ContractError>;      // UTF-8, field order
    fn to_wire_json_ascii(&self) -> String;                        // ASCII transport
    fn to_canonical_value(&self) -> Result<CanonicalValue, ContractError>;
    fn to_canonical_json(&self) -> Result<String, ContractError>; // protocol limits
    fn to_canonical_json_with(&self, limits: &Limits) -> Result<String, ContractError>;
    fn canonical_digest(&self) -> Result<Sha256Digest, ContractError>;
    fn canonical_digest_with(&self, limits: &Limits) -> Result<Sha256Digest, ContractError>;
}
```

Models are immutable, private-fielded value objects. Accessors use Python field
names in Rust snake case (`revision_id()`, `created_at()`, `artifacts()`, ...).
Arrays return slices, optional fields return `Option`.

### Types

- Models: `Requirement`, `DesignSpec`, `DomainPackageManifest`, `ArtifactRef`,
  `DesignRevision`, `PlanStep`, `WorkflowTransition`, `WorkflowDefinition`,
  `ToolDescriptor`, `ToolCall`, `ToolResult`, `EvaluationMetric`,
  `EvaluationResult`, `ApprovalRequest`, `RunBudget`, `Provenance`.
- Construction: `ComponentStep`, `ComponentScript`, `AssemblyStep`,
  `ConstructionScript`, `AnalysisStep`, `AnalysisScript`, `EditStep`,
  `AnalysisReading`; `load_construction`, `load_analysis`, `load_reading` and
  the order-preserving `load_*_json_str`/`load_*_json_bytes` variants;
  `ConstructionError`, `RevisionAction`, the operation sets and `MAX_*`
  constants.
- Enums: `RequirementKind`, `RequirementPriority`, `RequirementStatus`,
  `SideEffectLevel`, `WorkflowStatus`, `StepStatus`, `ToolRunStatus`,
  `ApprovalKind`, `ApprovalStatus`, `ArtifactClassification`; all expose
  `ALL`, `as_str`, `parse`, `Display`, `FromStr`.
- Scalars/support: `Identifier`, `MediaType`, `ArtifactUri`,
  `CanonicalJsonObject`, `Sha256Digest`, `FiniteNonNegativeFloat`,
  `UnitInterval`, `NonNegativeInteger`, `NonNegativeDecimal`, `NonEmptyText`
  (validated-`Text` alias), `Timestamp`, `Text`, `WireValue`, `ContractError`,
  `ContractIssue`, `model_json_limits`.

### Consumer mapping

- Repositories: parse stored JSON with `from_value` or `from_json_bytes`;
  hash durable state with `to_canonical_json_with(&model_json_limits())` and
  `canonical_digest_with`; never hash wire bytes.
- Core HTTP: read the raw request body, call `from_json_bytes` (do not use a
  lossy `serde_json::Value` first), validate, then return `to_wire_json()`.
- Kernel: `RunBudget` has consuming `with_*` setters; timestamps compare as
  absolute instants and serialize in Pydantic form.
- Tools: descriptor/call/result/artifact models with strict booleans,
  arbitrary-size nonnegative integers and the exact failure-state rules.
- Domains: specs, revisions, evaluation, provenance and package manifests;
  package `digest` values are preserved as supplied provenance data, never
  recomputed or spoofed.
- Construction adapters: use the ordered JSON loaders
  (`load_construction_json_str`/`_bytes`, and the analysis/reading variants)
  and typed getters; `ConstructionError` carries the cleaned Python-visible
  message. `load_*` on a `CanonicalValue` is order-lossy by construction.

### Example

```rust
use kala_contracts::{ContractModel, Requirement};

let requirement = Requirement::from_json_str(
    r#"{"id":" req-1 ","title":" Preserve state ","kind":"functional","tags":["b","a"]}"#,
)?;
assert_eq!(requirement.id(), "req-1");
assert_eq!(requirement.tags()[0].as_str(), "a");
let wire = requirement.to_wire_json()?;          // HTTP response bytes shape
let hash = requirement.canonical_digest()?;      // sorted/ASCII hash input
# Ok::<(), kala_contracts::ContractError>(())
```

## 2. Scope and ownership

New paths only:

```
backend/crates/kala-contracts/
  Cargo.toml, Cargo.lock, .gitignore, README.md
  src/{lib,error,input,text,wire,scalar,uri,timestamp,models,construction}.rs
  tests/{canonical_object,construction_contracts,model_contracts,
         parser_contracts,python_differential,scalars,shared_corpus}.rs
  tests/oracle/contracts_oracle.py
.mece/waves/rust-backend-20261019/cells/contracts/{REPORT.md,SESSION-R2.md}
```

No Python product source/test, frontend/desktop, adapter, shared
fixture/generator, root manifest/lock, `kala-json`, `kala-budget`,
`kala-geometry`, `kala-step`, `kala-studio`, `kala-persistence`,
user-reference or packaging file was modified. The crate-local test oracle
`tests/oracle/contracts_oracle.py` was extended for the R2 regressions.

## 3. Implementation notes

### Representations

- Wire form = `json.dumps(model.model_dump(mode="json"), ensure_ascii=False,
  separators=(",", ":"))`; field order preserved. It is what Starlette returns.
- Canonical form = sorted keys, ASCII escaping, compact bytes, delegated to
  `kala_json::canonical_dumps`; this is the only form used for hashing.
- `to_wire_json_ascii` matches `ensure_ascii=True` transport.
- Model JSON is parsed with the crate's private ordered parser (last-wins
  duplicate keys with first insertion position retained, insertion order
  preserved, escaped lone surrogates and arbitrary integers supported) and
  then converted explicitly to `kala_json::CanonicalValue`; no serde value
  reaches a model field. `WireValue` preserves ordered fields, `BigInt`
  integers and code-point text; `CanonicalValue` preserves arbitrary integers,
  signed zero and lone surrogates. Canonical object strings and all hashes
  remain strict.

### Dependencies

Read-only path dependencies on accepted `kala-json` and `kala-budget`; `idna`
1.1 (UTS #46 host processing), `num-bigint`, `num-traits`, `thiserror`,
`unicode-casefold`. `serde_json` was removed from this crate in R2; it remains
only a transitive dependency of accepted `kala-budget`. No
dependency on `kala-persistence`: contracts are the lower layer, and importing
it would put bundled SQLite into every consumer. The accepted persistence
canonical-object rules are mirrored exactly (strict parse, duplicate/non-finite
refusal, top-level object, CPython 4,300-digit integer limit, unbounded
size/depth). If the conductor prefers a shared helper after workspace
unification, this can be swapped without changing the public API.

### Model input and construction order

`from_json_str`/`from_json_bytes` use a private ordered parser matching
FastAPI's `json.loads` + Pydantic `model_validate` pipeline: duplicate keys
last-wins with the first insertion position retained; escaped lone surrogates,
arbitrary integers (CPython 4,300-digit conversion limit) and object insertion
order all survive together. There is no serde value and no strict fallback, so
the former duplicate+surrogate edge no longer exists. Construction models
override the JSON entry points, apply Python's first-alias profile
interpretation to the ordered input at fixed construction paths, and then run
the existing validator. `load_*_json_str`/`load_*_json_bytes` expose the same
path to loader consumers. A `CanonicalValue` supplied to `from_value`/`load_*`
has no insertion metadata and uses deterministic canonical order; that path is
documented and tested separately.

The parser, ordered-to-canonical conversion and ordered cleanup use explicit
heap stacks. Tests exercise 50,000-level attribute input, a 50,000-level
ignored profile body and a 50,000-level extra subtree; the profile hint never
walks the latter.

### Canonical object domain

`CanonicalJsonObject` accepts JSON text or a parsed `CanonicalValue::Object`,
strictly parses duplicate keys/non-finite numbers/trailing input, and stores
canonical ASCII JSON. `model_json_limits()` uses `usize::MAX` for bytes,
strings, keys, collections and depth; it retains CPython's 4,300-digit integer
conversion limit because the source parser/serializer enforce it. Tool-protocol
1 MiB/48-depth limits are never applied. Rust is a safe superset for documents
deeper than CPython's recursion limit; tests use 50,000-level raw model input
and >2 MiB canonical values.

### Timestamps

Pydantic `AwareDatetime`/speedate compatibility was implemented from observed
and documented behaviour: `T`/`t`/space/`_` separators, optional seconds,
dot/comma fractions truncated to six digits, `Z`/`z`, `±HH:MM`/`±HHMM`, U+2212
minus, epoch integers and strings, speedate's 2e10 milliseconds inference, its
negative-float rounding quirk, nanosecond truncation, `Z` for UTC and preserved
non-UTC offsets, absolute-instant equality/ordering. Year range is the Python
`datetime` domain `1..=9999`.

### URI

`urlsplit`-compatible authority/path parsing, scheme allowlist, credential and
query/fragment refusal, port rules, IPv4/IPv6 canonicalization (including
IPv4-mapped dotted output, RFC 5952 compression and scope ids), IDNA with
Python transitional mappings, strict percent decoding, control/backslash/
traversal checks and POSIX path normalization.

### Construction

All models, aliases, nested-profile flattening, inference, sequence rules,
assembly references, analysis ordering, edit/reading bounds and loader error
selection are ported. `ConstructionError` reproduces the Python loader's
cleaned first message, including the "unknown operation anywhere wins" rule.

## 4. Differential corpus and oracle

`tests/oracle/contracts_oracle.py` imports the live repository package from
`../../../.venv/bin/python` (or `KALA_PYTHON`) and emits **486 cases: 253
accepted, 233 rejected**. The oracle uses the source HTTP path,
`json.loads(raw)` followed by `model_validate`/the construction loader, and
keeps the original raw string in `input_json`; it never parses then
re-serializes that input, so duplicate keys reach Rust unchanged.
`tests/python_differential.rs` feeds the same raw text through
`from_json_str`/`from_json_bytes` and the `load_*_json_str`/`_bytes` entry
points (18 bytes-entry cases) and compares:

- wire bytes with field order (`ensure_ascii=True`, plus UTF-8 wire when
  encodable),
- canonical sorted/ASCII bytes,
- SHA-256 of canonical bytes,
- accepted/rejected status, and exact `ConstructionError` messages,
- cross-reader round trip: wire JSON is re-parsed and must reproduce identical
  wire and canonical bytes.

Coverage includes every model and enum value, defaults, optional/null fields,
extra fields, missing required arrays, strict bool/number/string behaviour,
identifier/digest/media-type/URI normalization and all security refusals,
IDNA/IPv4/IPv6/port/path cases, Decimal scale/signed zero/large values,
timestamp spellings/offsets/fractions/epoch and negative rounding, canonical
object duplicate/non-finite/1.1 MiB/100-depth/lone-surrogate cases, all
construction operations and failure branches, Python last-wins duplicate keys,
both construction profile alias orders (nested scripts, readings and loader
consumers), and the real shared `domain_provenance.json` manifests read-only.

R2 additions specifically cover nonadjacent and whitespace-normalized duplicate
constraints/findings/references, marker-shaped arbitrary objects at several
nested positions with siblings/escaped spelling/numeric values/surrogate
values, duplicate keys combined with escaped surrogates and arbitrary
integers, duplicate alias keys with first-position retention, and the
distinction between a raw canonical object string (duplicates refused) and an
ordinary parsed object (duplicates already collapsed).

Rust-only tests additionally cover 50,000-level raw input and profile subtrees,
5,000-level canonical depth, >2 MiB canonical values, arbitrary integers,
signed zero, lone surrogates and byte-stable canonical object output.

Python source package digests are **not** treated as Rust implementation
digests. The corpus manifests' `digest` fields are validated as input data;
nothing claims a Rust source digest, and no legacy digest was hardcoded or
recomputed.

## 5. Verification (this host, 2026-10-19)

Environment: Linux x86_64, rustc/cargo 1.99.1, CPython 3.14.8 in
`/home/v9i/Projects/Kala-Agent/.venv`.

All commands run in `backend/crates/kala-contracts`:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo test --locked` | 50 tests pass (15 unit + 33 integration + 2 live differential; 0 failed/ignored) |
| `cargo test --locked --release` | 50 tests pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo build --locked --release` | pass |
| live Python differential | 486/486 cases and all round trips match (253 accepted, 233 rejected; 18 bytes-entry cases) |
| `cargo check --locked --all-targets --all-features --target x86_64-pc-windows-gnu` | pass |
| Windows-target clippy `-D warnings` | pass |

No performance, full-migration, Windows/COM, packaging or cutover claim is
made. Tests run on Linux; the Windows target was compiled/checked only, not
executed.

## 6. Known compatibility differences / unresolved objections

These are deliberate or source-limited; none is a test failure.

1. Model JSON entry points use the crate's private ordered parser with
   Python/FastAPI last-wins duplicate semantics (first insertion position
   retained), escape/codepoint fidelity, arbitrary integers and insertion
   order. There is no serde representation and no strict fallback. Strict
   canonical objects and hashes remain strict in all paths.
2. `from_value(&CanonicalValue)` and `load_*` on a `CanonicalValue` have no
   insertion metadata. Construction profile aliases then use deterministic
   canonical order. JSON/HTTP consumers use `from_json_str`/`from_json_bytes`
   or `load_*_json_str`/`load_*_json_bytes`, which preserve Python order; this
   is tested for both alias orders, duplicate keys, nested component scripts,
   construction scripts, readings and loader entry points.
3. The source HTTP path is `json.loads` + Pydantic `model_validate`. Pydantic's
   direct `model_validate_json` parser rejects escaped lone surrogates with
   `json_invalid` even without duplicate keys, while `json.loads` +
   `model_validate` accepts them. Rust follows the HTTP path and reproduces
   the latter. This is a Pydantic entry-point difference, not a Rust mismatch
   with the source transport; direct `model_validate_json` is not claimed.
4. IDNA host processing uses UTS #46 transitional rules, which match ordinary
   IDNA2003 names (ß→ss, final sigma, join controls) but exotic code points
   may differ from CPython's Unicode 3.2 nameprep tables.
5. Ordinary Pydantic model error strings are informative but not guaranteed
   verbatim; construction loader messages are compared exactly. The brief's
   contract is model/field behaviour and normalized serialization.
6. Rust accepts persisted canonical objects and raw model input deeper than
   CPython's recursion limit (safe superset); accepted bytes still match for
   documents Python can produce.
7. `NonEmptyText` fields reject lone surrogates like Pydantic's
   `StringConstraints`; construction `summary`/`verdict` and canonical object
   strings preserve them like the Python model, whose UTF-8 wire encoding then
   fails as Python does.
8. `to_wire_json` refuses model integers above CPython's 4,300-digit
   serialization limit; model JSON input cannot contain them either.
9. No `serde::Serialize`/`Deserialize` impls by design: they would invite
   lossy big-int/surrogate conversion. Consumers use `WireValue`,
   `to_wire_json` and `from_json_bytes`.
10. Windows is compile/check-only in this cell; no runtime, COM or Solid Edge
    execution claim is made.

## 7. Not done / follow-ups

- Root workspace integration and lock unification are the conductor's task
  after acceptance.
- Typed repositories, CAS, kernel/policy/workflows/tools/runtime, core HTTP,
  CLI and packaging were not touched.
- No dependent writer should begin before this report is accepted.
- Possible follow-ups: shared canonical helper after workspace unification;
  exact IDNA2003 tables if exotic-host parity becomes a product requirement;
  core HTTP 422 error-string parity (not claimed here).

## 8. R2 evidence log

Commands were run from `backend/crates/kala-contracts` on Linux x86_64:

- Repro 1 before repair (review shape): `DesignSpec` constraints `["a","b","a"]`
  was accepted by the old `windows(2)` check. After repair, Rust and live
  Python both reject it with `collection values must be unique`; the
  whitespace-normalized `[" a ","b","a"]` case rejects in both, and the
  accepted `["b","a","c"]` case preserves order in wire/canonical bytes.
- Repro 2 before repair: `attributes_json` containing
  `{"$serde_json::private::Number":"42"}` was turned into a serde number and
  rejected as "value must be a JSON object". After removing serde from the
  input path, live Python and Rust both preserve the object, including
  numeric values, nested objects, array siblings and `\u0024` escaped spelling.
- Repro 3 before repair: duplicate `"id"` plus `attributes_json:
  {"text":"\ud800"}` was refused by Rust while live
  `DesignSpec.model_validate(json.loads(raw))` accepted id=`last` and preserved
  the canonical surrogate text. After repair both accept, for `from_json_str`
  and `from_json_bytes`; a marker key and a >100-bit integer in the same
  object also survive.
- Construction: duplicate `summary`/`verdict` keys with escaped surrogate text
  and both profile alias orders match live Python through
  `load_construction_json_str`, `load_construction_json_bytes`,
  `load_reading_json_str` and `load_reading_json_bytes`, including an escaped
  alias key spelling and nonadjacent duplicate alias keys.
- Persisted canonical strings: `attributes_json` text
  `"{\"a\":1,\"a\":2}"` is still refused, while the ordinary parsed object
  `{"a":1,"a":2}` is accepted last-wins exactly as Python's `json.loads` path.
- Deep input: a 50,000-level `attributes_json` with duplicate keys and a
  surrogate, plus a 50,000-level ignored profile body and extra subtree, parse
  and clean up without stack overflow. Profile hints visit only the fixed
  construction paths.
- Live differential: 486 cases, 253 accepted, 233 rejected, all wire/canonical/
  digest/status/construction-message/round-trip comparisons match CPython
  3.14.8; 18 cases use the bytes entry point. An oracle-transport test asserts
  duplicate and escaped-marker text reaches Rust unmodified.
- `cargo test --locked` and `--release`: 50 tests pass each; `cargo fmt
  --check`, native/Windows-target clippy `-D warnings`, native release build
  and Windows GNU target check pass.
