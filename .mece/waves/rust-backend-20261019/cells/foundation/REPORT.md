# Foundation cell report — Rust workspace, canonical JSON, event chain

**Status: implementation complete for this cell's slice; no commit, push,
deployment, model change, delegation, or performance claim was made.**
Windows foundation and the Python corpus were accepted by the conductor before
this cell started. Native Windows validation remains pending and no Windows
binary is claimed.

## 1. Ownership and files

Owned and created:

- `backend/Cargo.toml`, `backend/Cargo.lock`, `backend/.gitignore`,
  `backend/README.md`
- `backend/crates/kala-json/**`
- `.mece/waves/rust-backend-20261019/cells/foundation/REPORT.md` and
  `SESSION.md`

Read-only and untouched:

- `backend/compat/**`, `backend/fixtures/**` (Python corpus worker)
- `src/**`, `tests/**`, `studio/**`, `adapters/**`, packaging and docs
- `Rust+buid123/**`, `rust-implementation.md`
- `backend/crates/kala-geometry/**`, `backend/crates/kala-budget/**`
  (independent workers; excluded from the workspace, not edited)

`git status` for this cell shows only the new `backend/` and
`.mece/.../foundation/` paths. Pre-existing dirty user/Windows paths were not
modified.

## 2. Deliverables

### Workspace

- Standalone Cargo workspace, edition 2024, `rust-version = "1.85"`, resolver 3.
- `members = ["crates/kala-json"]` only.
- `exclude = ["crates/kala-geometry", "crates/kala-budget"]`; `cargo metadata`
  reports exactly one package, `kala-json`.
- Single reproducible `Cargo.lock` with registry checksums.
- `backend/.gitignore` ignores `target`; `Cargo.lock` is retained.
- `backend/README.md` records initial workspace status and scope limits.

### `kala-json` public API

```rust
pub fn parse_strict(payload: &[u8], limits: &Limits)
    -> Result<CanonicalValue, CanonicalJsonError>;
pub fn canonical_dumps(value: &CanonicalValue, limits: &Limits)
    -> Result<String, CanonicalJsonError>;
pub fn canonical_digest(value: &CanonicalValue, limits: &Limits)
    -> Result<Sha256Digest, CanonicalJsonError>;
pub fn strict_json_loads(payload: &[u8]) -> Result<CanonicalValue, CanonicalJsonError>;
pub fn canonical_json_dumps(value: &CanonicalValue) -> Result<String, CanonicalJsonError>;

pub enum CanonicalValue { Null, Bool, Integer(BigInt), Float(f64), Text(JsonText), Array, Object }
pub struct JsonText;      // Vec<u32> code points, lone surrogates allowed
pub struct Limits;        // bytes, string/key, arrays, depth, int digits
pub struct Sha256Digest;  // typed 32-byte digest, lowercase hex display

pub struct EventChainFields {
    pub sequence: u64,
    pub event_type: String,
    pub aggregate_id: String,
    pub workflow_id: Option<String>,
    pub run_id: Option<String>,
    pub occurred_at: String,
    pub payload_digest: Sha256Digest,
    pub previous_event_digest: Sha256Digest,
}
pub fn chain_preimage(&EventChainFields) -> Vec<u8>;
pub fn chain_digest(&EventChainFields) -> Sha256Digest;
pub fn payload_digest(payload_json: &str) -> Sha256Digest;
pub fn verify_chain(&[EventChainFields], Option<&[Sha256Digest]>)
    -> Result<ChainVerification, ChainVerifyError>;
```

`CanonicalValue` keeps integer-vs-float, negative zero, arbitrary-precision
integers and Python code-point key ordering. `JsonText` preserves lone
surrogates and exposes `to_utf8()` as an explicit failure when a Rust `String`
cannot represent the text. Production code contains no Python, subprocess, or
serde_json serialization path.

## 3. Compatibility behavior implemented

### Canonical JSON

- Key sorting by Unicode code point, before escaping.
- CPython `ensure_ascii=True` escaping: `\b \t \n \f \r \" \\`, `\u00xx`
  for remaining controls, DEL and BMP non-ASCII, UTF-16 surrogate pairs for
  supplementary code points, preserved lone surrogates.
- Strict parser combines adjacent escaped surrogate pairs exactly like CPython
  and leaves lone/high-followed-by-high escapes uncombined.
- CPython `repr(float)` formatting: shortest round-trip digits, decimal
  notation for decimal exponents `-4..=15`, scientific otherwise, explicit
  exponent sign, at least two exponent digits, integral `.0`, and `-0.0`.
  Ryu supplies shortest digits; the formatter applies CPython's thresholds and
  exponent spelling.
- Arbitrary-precision integer parsing/emission; CPython's default 4,300-digit
  int/str limit is enforced (configurable through `Limits`, `0` disables).
- Strict input: raw byte cap before UTF-8 decode; invalid UTF-8 distinguished
  from invalid JSON; duplicate keys rejected even with equal values; `NaN`,
  `Infinity`, `-Infinity`, overflow-to-infinity and non-finite floats rejected;
  underflow to `0.0` accepted; BOM, trailing data, invalid escapes and invalid
  number shapes rejected; top-level scalars accepted.
- Limits: 1,048,576 raw/canonical bytes, 32,768 string code points, 256 key
  code points, 2,048 array items, depth 48. Object entry count is **not**
  limited, matching Python `_normalize_json`; only arrays use the collection
  limit.
- Canonical byte cap is applied to the emitted ASCII bytes, not an unescaped
  UTF-8 estimate.
- All error messages match `JsonEnvelopeError` text for the corpus cases.

### Event chain

- Domain `kala-agent.event-chain.v1.sha256`, genesis `0`×64.
- Domain and all eight covered parts length-prefixed with 8-byte big-endian
  UTF-8 lengths: sequence, event type, aggregate id, workflow id or empty, run
  id or empty, occurred_at, payload digest, previous event digest.
- Payload digest is SHA-256 of the UTF-8 bytes of the persisted payload JSON,
  without re-serialization.
- Absent and empty optional ids collapse to the same preimage.
- `verify_chain` checks one-based contiguity, genesis linkage, previous-digest
  linkage and optional stored event digests; returns count and head.
- No SQLite write, schema migration, Merkle step, cache or JCS was added.

## 4. Corpus and differential coverage

Consumed read-only from `backend/fixtures/corpus/v1`:

- `canonical_json.json`: **66/66** cases executed. Every successful output is
  compared byte-for-byte and by SHA-256 and declared length; every error is
  compared by exact message. Cases cover exponent thresholds, `-0.0`, integral
  floats, subnormal/max finite, underflow, `2**70`/`-2**70`/`10**100`,
  controls, DEL, Greek/supplementary/raw Unicode, lone surrogates, key order,
  duplicates, non-finite constants, overflow, BOM, invalid UTF-8, trailing
  data, depth/string/key/array/byte boundaries and caller `max_bytes`.
- Python-only fixture sources are asserted honestly rather than skipped:
  non-string keys and raw `Decimal` are outside the typed value domain (their
  error vocabulary is checked), and non-finite float literals are constructed
  and refused with the expected message.
- `event_chain.json`: **5/5** preimage vectors, including Unicode and 300-byte
  parts and absent-vs-empty collapse; all five real database event rows
  (payload digest, canonical payload bytes, chain digest, previous digest);
  snapshot canonical bytes and digest; verified count/head from the fixture
  (`143fed75…99fc7`).
- `tests/corpus.rs` also re-parses every stored payload and checks that
  canonical re-serialization is byte-identical, exercising bigint, `-0.0`,
  `1e-05`, Unicode and surrogate escaping inside real persisted rows.

Live Python differential (CPython 3.14.8 from `.venv`; live product source
`src/kala_agent/tools/protocol.py`):

- **6,024 floats** (6,000 deterministic random finite binary64 bit patterns +
  24 curated boundaries) compared against `canonical_json_dumps`; every output
  also reparsed and re-dumped stably.
- **415 structured sources**: 120 random Unicode strings (ASCII/2-byte/BMP/
  surrogate/full-Unicode code points), 40 random objects with unique random
  escaped keys and mixed values, 40 random large signed integers (up to 300
  digits), 200 random decimal literals, 14 curated Unicode/bigint/nesting
  cases and one 2,049-key object.
- The test fails if no Python oracle is available; it does not silently skip.

## 5. Checks actually run

From `backend/` on Linux x86_64, rustc/cargo 1.99.1:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo test -p kala-json` | exit 0; 17 unit + 3 corpus + 3 differential tests passed, 0 failed/ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo build --release --workspace` | exit 0 |
| `cargo metadata --no-deps` | one package: `kala-json`; geometry/budget excluded |

Resolved key versions: `kala-json 0.1.0`, `num-bigint 0.4.8`, `ryu 1.0.23`,
`sha2 0.10.9`, `thiserror 2.0.21`; dev `serde_json 1.0.154` (with
`serde_core 1.0.229`, `zmij 1.0.23`, `itoa 1.0.18`). Oracle: CPython 3.14.8
(`.venv/bin/python`), `kala_agent` imported from this checkout.

No benchmark of any kind was run, and no speedup is claimed. The release build
is compilation evidence only.

## 6. Gaps and unfinished work

1. **Windows.** No Windows host/MSVC target build, no `.exe`, no COM/NSIS proof.
   The crate is portable Rust but Windows execution is unverified.
2. **Scope.** This is the foundation slice only. Contracts, SQLite schema
   v2/journal append, CAS, Decimal/budget, domains, HTTP services, CLI,
   geometry/STL and packaging remain unimplemented.
3. **Error precedence on multi-violation documents.** Python normalizes in JSON
   insertion order; this implementation validates after parsing and traverses
   sorted object keys. The error vocabulary and single-error corpus behavior
   match, but a document containing several independent limit violations can
   report a different one first.
4. **Digest-field typing.** `EventChainFields` stores typed 32-byte digests, so
   an invalid/uppercase stored digest string cannot be represented. Valid
   journal rows are constrained to lowercase 64-hex; no malformed-row hash
   parity is claimed.
5. **Python settings.** The 4,300-digit default is encoded in `Limits`; a
   process that changes `sys.set_int_max_str_digits` is not introspected.
6. **Oracle availability.** `cargo test` requires a working Python import of
   the live product for the differential test; this is intentional and
   documented, but means the test suite is not self-contained offline.
7. **Python-only source kinds.** Raw `Decimal` and non-string mapping keys have
   no typed Rust representation. This is a deliberate type-level refusal; the
   corpus expected messages are asserted textually, not produced by a generic
   serializer.
8. **Performance.** None measured; no comparison to Python and no optimization
   claim. Do not treat the release build as a benchmark.

The canonical JSON and chain contract is ready for dependent crates to consume
after reviewer acceptance. Integration of the excluded geometry/budget crates
and all persistence/service work remain for later cells.
