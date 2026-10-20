# kala-contracts

Native, typed Rust contracts for the Kala Agent backend. This crate is the
reviewed foundation that domains, repositories, the kernel, tools and the core
HTTP service consume. It is a port of the frozen Pydantic surface in
`src/kala_agent/contracts/{models,construction}.py`; Python is used only by the
test-time differential oracle.

This crate is a standalone Cargo workspace with its own `Cargo.lock`, the same
integration pattern as `kala-studio` and `kala-step`. It depends only on the
accepted `kala-json` and `kala-budget` crates by read-only path. It does not
edit a parent manifest, does not own an HTTP route, and does not perform any
filesystem or Python runtime work.

## Three representations, deliberately

| Form | Method | Used for |
|---|---|---|
| Typed value | every model struct | validated in-process consumers |
| Wire JSON | `ContractModel::to_wire_json` / `to_wire_json_ascii` | HTTP response bodies (`model_dump(mode="json")`, field order preserved) |
| Canonical JSON | `ContractModel::to_canonical_json`, `canonical_digest` | hashing and durable identity (sorted keys, ASCII escaping) |

Do not hash wire bytes and do not send canonical bytes on the wire. The wire
form preserves field order and UTF-8 text; the canonical form sorts object keys
by Unicode code point and escapes non-ASCII exactly like
`kala_agent.tools.protocol.canonical_json_dumps`. Canonical serialization is
delegated to `kala_json`; no serde representation is used at any point, so
arbitrary integers, signed float zero, lone-surrogate code points and ordinary
user keys are preserved.

`to_canonical_json()` uses the tool-protocol limits that
`canonical_json_digest` uses (1 MiB, depth 48, ...). When hashing a persisted
model whose embedded `CanonicalJsonObject` is deliberately unbounded, pass
`model_json_limits()` to `to_canonical_json_with` / `canonical_digest_with`
instead.

## Getting a typed value

```rust
use kala_contracts::{ContractModel, Requirement, RunBudget};

let requirement = Requirement::from_json_str(
    r#"{"id":" req-1 ","title":" Preserve state ","kind":"functional","tags":["b","a"]}"#,
)?;
assert_eq!(requirement.id(), "req-1");
assert_eq!(
    requirement.tags().iter().map(|tag| tag.as_str()).collect::<Vec<_>>(),
    vec!["a", "b"]
);

let budget = RunBudget::from_json_str(r#"{"max_cost":"1.00"}"#)?;
assert_eq!(budget.max_cost().to_python_string(), "1.00");
# Ok::<(), kala_contracts::ContractError>(())
```

Models are immutable value objects: fields are private, and the only
constructors are the validating entry points:

- `Type::from_json_str` / `Type::from_json_bytes` — Pydantic-compatible model
  JSON: duplicate object keys use last-wins semantics, and construction
  models retain object insertion order for profile aliases. Strict canonical
  objects and hashes are unaffected.
- `ContractModel::from_value(&kala_json::CanonicalValue)` — the lossless input
  API for consumers that already parsed JSON (including lone surrogates and
  arbitrary integers).
- `Type::from_json_str` for `CanonicalJsonObject` remains the persisted-object
  path with no tool-protocol limits.

`RunBudget` additionally has consuming setters (`with_max_cost`, ...) for
programmatic construction; every other model is built from validated input.

## Public types

Models: `Requirement`, `DesignSpec`, `DomainPackageManifest`, `ArtifactRef`,
`DesignRevision`, `PlanStep`, `WorkflowTransition`, `WorkflowDefinition`,
`ToolDescriptor`, `ToolCall`, `ToolResult`, `EvaluationMetric`,
`EvaluationResult`, `ApprovalRequest`, `RunBudget`, `Provenance`.

Construction models: `ComponentStep`, `ComponentScript`, `AssemblyStep`,
`ConstructionScript`, `AnalysisStep`, `AnalysisScript`, `EditStep`,
`AnalysisReading`, plus `load_construction`, `load_analysis`, `load_reading`,
the order-preserving `load_*_json_str`/`load_*_json_bytes` variants and their
Python-compatible `ConstructionError`.

Enums: `RequirementKind`, `RequirementPriority`, `RequirementStatus`,
`SideEffectLevel`, `WorkflowStatus`, `StepStatus`, `ToolRunStatus`,
`ApprovalKind`, `ApprovalStatus`, `ArtifactClassification`, `RevisionAction`.
Every enum has `ALL`, `as_str`, `parse`, `Display` and `FromStr`.

Scalars and support types: `Identifier`, `MediaType`, `ArtifactUri`,
`CanonicalJsonObject`, `Sha256Digest`, `FiniteNonNegativeFloat`,
`UnitInterval`, `NonNegativeInteger`, `NonNegativeDecimal`, `NonEmptyText`
(alias for validated `Text` fields), `Timestamp`, `Text`, `WireValue`,
`ContractModel`, `ContractError`, `ContractIssue`.

`Text` preserves Python code points, including lone surrogates. Use
`as_str()` when the text is valid UTF-8, `as_json_text()` for canonical
handling and `to_string_lossy()` only for diagnostics.

## Compatibility rules implemented

- `extra="forbid"` on every model; unknown fields are refused.
- Pydantic `StringConstraints(strip_whitespace=True, 1..=16_384)` trims only
  Unicode `White_Space`; Python `str.strip()` additionally trims
  U+001C..U+001F where the Python source calls it (identifiers, digests,
  media types, construction text).
- strict booleans and strict nonnegative integers; arbitrary-precision
  integers are `BigInt`, never `u64`.
- finite/nonnegative floats accept negative zero and reject booleans.
- nonnegative `Decimal` keeps textual scale and signed zero through the
  accepted `kala-budget` implementation.
- identifiers, lowercase digests, lowercase ASCII media types.
- artifact URI parser: scheme allowlist, credentials/query/fragment refusal,
  IDNA host normalization, IPv4/IPv6 canonicalization, port rules, strict
  percent decoding, control/backslash/traversal refusal and POSIX path
  normalization.
- Pydantic `AwareDatetime`: RFC 3339 with `T`/`t`/space/`_` separators, `Z`
  or `±HH:MM` / `±HHMM`, microseconds truncated to six digits, epoch numeric
  input with speedate's millisecond inference and float rounding, `Z`
  serialization for UTC and preserved non-UTC offsets.
- canonical JSON object validation: strict parse, duplicate-key and
  non-finite refusal, top-level object only, canonical `ensure_ascii=True`
  output, no tool-protocol size/depth limits (CPython's 4,300-digit integer
  conversion limit is retained).
- all cross-field invariants: unique/sorted collections where Python sorts,
  package signature pairs, revision self-parent, plan self-dependency,
  workflow state/step references and reachability, destructive-tool side
  effects, tool-result failure/result rules, evaluation uniqueness, approval
  decision rules, provenance self-parent.
- construction flattening, profile aliases/inference, operation sequencing,
  assembly references, analysis ordering, edit and reading bounds, with the
  Python loader's cleaned error-message selection.

## Known limitations

- The ordered model-input parser (used by `from_json_str`/`from_json_bytes`) is
  a private explicit-stack parser and ordered value owned by this crate. It
  implements Python `json.loads` semantics: duplicate keys are last-wins with
  the first insertion position retained, escaped lone surrogates and arbitrary
  integers (up to CPython's 4,300-digit conversion limit) are preserved, and
  the parser, canonical conversion and cleanup are iterative. It has no
  combined duplicate/surrogate edge case and no fixed nesting ceiling.
- The source HTTP path is `json.loads` followed by Pydantic
  `model_validate`. Pydantic's direct `model_validate_json` parser rejects an
  escaped lone surrogate (`json_invalid`) even when there are no duplicate
  keys, so this crate intentionally follows the HTTP path rather than that
  parser-specific limitation.
- `from_value(&CanonicalValue)` has no insertion-order metadata. Construction
  profile aliases therefore use deterministic canonical order there. HTTP/JSON
  consumers should use `from_json_str`/`from_json_bytes` or the
  `load_*_json_str`/`load_*_json_bytes` loaders, which preserve Python object
  order.
- Profile hints are applied only at the fixed construction positions
  (`ComponentStep`, `ComponentScript.steps`,
  `ConstructionScript.components[].steps`, `AnalysisReading.rewrite.steps`).
  Arbitrary extra or attribute subtrees are never walked looking for a
  `profile` key.
- `CanonicalJsonObject` supplied as JSON string text still strictly refuses
  duplicate keys. An ordinary parsed object that contained duplicates has
  already been collapsed by the model-input parser before that validator, just
  like Python's `json.loads` + before-validator path.
- IDNA uses UTS #46 with Python's transitional mappings (ß→ss, final sigma,
  join controls removed). For ordinary names this matches CPython's
  IDNA2003 codec; exotic code points whose Unicode 3.2 nameprep tables differ
  from current UTS #46 data may differ.
- Pydantic error strings are not reproduced verbatim for ordinary contract
  models. Construction loader messages (which are product-visible) are
  compared byte-for-byte by the differential test.
- Canonical object depth is unbounded in Rust even where CPython's recursion
  limit would reject the same document; Rust is a safe superset there.

## Test and verification commands

```bash
cargo fmt --all -- --check
cargo test --locked
cargo test --locked --release
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --release
```

The differential test imports the live repository package from
`../../../.venv/bin/python` (or `KALA_PYTHON`) and compares wire bytes,
canonical bytes and SHA-256 for 480+ valid/invalid cases covering every model,
construction branch, URI security case, Decimal scale, timestamp form,
large/deep canonical object, duplicate/surrogate combinations and
`from_json_str`/`from_json_bytes` plus construction loader entry points.

No performance, full-migration, Windows/COM or packaging claim is made here.
