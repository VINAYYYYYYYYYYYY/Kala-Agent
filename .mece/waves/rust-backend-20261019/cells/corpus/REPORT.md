# Python compatibility corpus for the full Rust backend — cell report

**Status: compatibility-fixture preparation complete and verified. No Rust
implementation, Cargo workspace, packaging cutover, or timing claim is included.**
The Windows-local milestone and its Windows/COM/NSIS evidence remain a gate, as
required by the wave brief.

## 1. Scope and ownership

Owned and created:

- `backend/compat/**` — reproducible Python generator, corpus library, README.
- `backend/fixtures/**` — versioned `corpus/v1` fixtures and README.
- `.mece/waves/rust-backend-20261019/cells/corpus/REPORT.md` and `SESSION.md`.

Read-only and untouched:

- All Python product code under `src/kala_agent/**` and `tests/**`.
- The current Go/Windows worker files (`adapters/solidedge*`, `.github/workflows`,
  docs, packaging).
- `Rust+buid123/` and `rust-implementation.md`.
- Any user database, token file, credential, or real data directory.

No commit, push, install, delegation, or package cutover was performed.
Pre-existing dirty paths from other workers were left unchanged; the only new
paths are `backend/**` and this cell's reports.

## 2. Deliverables

```
backend/compat/
  generate_corpus.py             single generation command + --check
  self_check.py                  structural + Python-reader + regeneration check
  corpuslib/
    common.py                    source pin, environment, fixed clocks
    canonical_json_cases.py      66 canonical JSON cases
    chain_cases.py               5 preimages + real schema-v2 SQLite fixture
    decimal_cases.py             85 Decimal/budget/duration/clamp cases
    http_cases.py                48 Studio + 49 core HTTP cases
    geometry_cases.py            6 meshes + STL + structural STEP
    provenance_cases.py          three LEGACY Python package identities
    cli_cases.py                 12 commands, 9 executions, launch contracts

backend/fixtures/
  README.md
  corpus/v1/
    manifest.json                source/oracle pin, coverage, hashes, gaps
    canonical_json.json
    event_chain.json
    schema_v2.sqlite3
    decimal_budget.json
    geometry_stl.json
    stl/{box,cylinder,sphere}_default.stl
    domain_provenance.json
    cli_launch.json
    studio_http.json
    core_http.json
```

Total fixture size: **1,024,660 bytes** across the 12 indexed files, plus
`manifest.json` (13 files in the directory). Large max-tessellation STL
bytes are not checked in; their SHA-256 and length are.

## 3. Reproduction and verification

From the repository root:

```bash
.venv/bin/python backend/compat/generate_corpus.py
.venv/bin/python backend/compat/generate_corpus.py --check
.venv/bin/python backend/compat/self_check.py
.venv/bin/python backend/compat/self_check.py --skip-regenerate
```

Results observed:

| Command | Result |
|---|---|
| generator | exit 0; 12 files; all declared routes covered |
| `--check` | exit 0; `determinism check passed` |
| `self_check.py` | exit 0; `self-check OK`; regeneration compared |
| `self_check.py --skip-regenerate` | exit 0 |
| `ruff check backend/compat` | all checks passed |
| `compileall backend/compat` | passed |

Oracle environment recorded in `manifest.json`:

- CPython 3.14.8, Linux x86_64, SQLite 3.54.0
- fastapi 0.141.1, pydantic 2.13.5, starlette 1.7.0, httpx 0.28.1, uvicorn 0.54.0
- `kala-agent` 0.1.0, cadquery-ocp 8.0.1.1.0
- Decimal context: precision 28, `ROUND_HALF_EVEN`, Emin/Emax ±999999,
  traps DivisionByZero/InvalidOperation/Overflow.

Source pin at generation:

- git commit `966c185bffa10f9d0524f3a2ee322c646729940a` (tree dirty because of
  the ongoing Windows worker and this new `backend/**`)
- 78 product Python files, tree digest
  `d12f292f69b874068a83aef07d07120f39967c98cc392cef2e5900015d4aa888`.

Regenerate after any product source change. The corpus is a same-tree oracle,
not a cross-revision comparison.

## 4. What is frozen

### 4.1 Canonical JSON — 66 cases

Uses only `kala_agent.tools.protocol.canonical_json_dumps`,
`strict_json_loads`, and `canonical_json_digest`.

Covered:

- CPython float formatting at the exponent thresholds: `1e-05` → `1e-05`,
  `1e-04` → `0.0001`, `1e15` → `1000000000000000.0`, `1e16` → `1e+16`,
  `1e-07` → `1e-07`, `1e+308` with explicit `+`; `-0.0`; `1.0` vs `1`;
  subnormal `5e-324`; max finite; underflow `1e-400` → `0.0`.
- Large integers exact: `2**70`, `-2**70`, `10**100`.
- Strings: ASCII controls, quote/backslash/tab/newline escapes, solidus not
  escaped, Greek `κ`, supplementary `😀`, raw UTF-8 input, lone high and low
  surrogate escapes (accepted and re-emitted as `\ud800`/`\udfff`).
- Key ordering, including Unicode keys sorted by code point before escaping.
- Booleans/null, numeric strings, mixed objects.
- Rejections: duplicate keys (even same value), `NaN`/`Infinity`/`-Infinity`,
  float overflow `1e999`, invalid JSON, empty/trailing data, UTF-8 BOM,
  invalid UTF-8, non-string keys, raw Decimal values.
- Boundaries: string 32,768/32,769; key 256/257; collection 2,048/2,049;
  depth 48/49; payload 1,048,576 bytes; caller-supplied `max_bytes` rejection.
- Expected errors record the actual `JsonEnvelopeError` type and message.

Important Rust consequence: Python accepts lone surrogate escapes. A Rust
`String` cannot represent them; a WTF-8/surrogate-preserving text type or a
declared divergence is required for byte parity.

### 4.2 Event chain and schema-v2 SQLite

- Domain: `kala-agent.event-chain.v1.sha256`.
- Genesis previous digest: 64 zeros.
- Every part, including the domain, is prefixed by an 8-byte big-endian UTF-8
  length. Nine length-prefixed parts total.
- Payload digest is SHA-256 of the canonical payload JSON bytes.
- Five independent preimage vectors, including workflow/run absent vs empty
  collapsing to the same preimage, Unicode parts, and 300-byte parts.
- Real database created by `Database`, `EventJournal`, `SnapshotStore`, and
  `DesignSpecRepository`.

Fixture facts:

- schema version 2; 10 tables including `sqlite_sequence`
- 2 migrations (`initial_persistence_schema`, `journal_event_hash_chain`)
- 3 triggers (`events_prevent_delete`, `events_prevent_update`,
  `events_require_event_chain`)
- 5 linked events, 1 workflow snapshot, 1 design spec
- `sqlite_master` normalized digest
  `a8b36de324421812e27eddcaa49487482b81e8293fbd4edb806b83dfbfb60b9a`
- verified chain head
  `143fed75e63b8fb67c9449b0fe353cf4d20fc1cb4223b20e5d70e95028999fc7`
- `PRAGMA journal_mode = wal`; the build connection enabled `foreign_keys`
  (per-connection setting, not a stored file property)

`self_check.py` copies the database to a temp path and re-opens it with the
live `Database` and `EventJournal`, verifies the chain, and compares event
rows, migrations, table counts, `sqlite_sequence`, and the schema digest.
Migration `applied_at` values are stabilized to a fixed value after migration;
this is declared in `event_chain.json`.

The SQLite file bytes are deliberately **not** asserted stable across SQLite
library versions. Logical rows and digests are the contract.

### 4.3 Decimal and budget — 85 cases

Frozen evidence:

- Stored scale is preserved: `"1.00"`, `"0.10"`, `"0.2"`, `"100.000"`,
  `"1.2300"`, `"1E+3"`, and `"0E-1000000"` round-trip through
  `BudgetUsage.as_dict()` / `RunBudget.model_dump(mode="json")`.
- `Decimal("0.10") + Decimal("0.2") = Decimal("0.30")`; `0.1 + 0.2 = 0.3`.
- Extremes accepted by the budget source: `1e-999999`, `1e+999999`,
  38-digit coefficients, >28 fractional scale, `0E-1000000`.
- Context arithmetic is real and observable: `1e+40 + 1e-30` becomes
  `1.000000000000000000000000000E+40`; the 38-digit value plus `1e-40`
  rounds to 28 significant digits.
- Duration grid: `round(seconds * 1_000_000)`; examples
  `1.5e-06 → 2e-06`, `2.5e-06 → 2e-06`, `1.2345675 → 1.234568`.
- Budget ledger export/import round trip, reservation commit, and cost
  exhaustion.
- `clamp_unit_interval` ties-to-even examples.

`rust_decimal`'s 96-bit/28-scale domain is **insufficient** for the recorded
input set. A scale-preserving arbitrary-precision representation with an
explicit Python-compatible context is required.

### 4.4 Studio HTTP — 9 routes, 48 cases

All nine routes:

| Method | Path |
|---|---|
| GET | `/api/studio/health` |
| GET | `/api/studio/integrations` |
| GET | `/api/studio/providers` |
| GET | `/api/studio/settings` |
| POST | `/api/studio/settings` |
| POST | `/api/studio/preview` |
| POST | `/api/studio/export` |
| POST | `/api/studio/handoff` |
| POST | `/api/studio/chat` |

Covered behavior:

- health before/after storing and clearing a synthetic provider key
  (`offline` transition), integrations, providers, settings
- preview box default/custom, cylinder max, sphere max
- STL export box default and sphere max; STEP export box
- handoff STL FreeCAD; handoff STEP SolidWorks
- local deterministic chat; OpenAI missing model / `allow_remote=false` /
  missing key; Codex unavailable / `allow_remote=false`
- flat error envelope `{code,message,details}`
- 421 bad Host, 403 cross-origin (GET and OPTIONS preflight), 401 missing and
  wrong bearer token with `WWW-Authenticate: Bearer realm="kala-studio"`,
  413 declared and chunked oversized bodies, 404 unknown route, 405 method,
  409 unsupported handoff format, 404 unknown integration, 422 validation and
  parameter cases, 400/503 provider refusals
- static index, asset, SPA fallback, `/api/*` reservation
- security header set on every response; `server`, `x-powered-by`, and
  `set-cookie` absent
- binary STL bodies are stored by SHA-256 and length; the default box STL is
  also a byte artifact. STEP bodies are structural only.

### 4.5 Core HTTP — 17 live routes, 49 cases

The live route table and OpenAPI both expose **17 operations total**:

| Method | Path |
|---|---|
| GET | `/health` |
| GET | `/api/v1/domains` |
| GET | `/api/v1/domains/{domain}` |
| POST | `/api/v1/domains/{domain}/evaluations` |
| POST | `/api/v1/revisions` |
| GET | `/api/v1/revisions` |
| GET | `/api/v1/revisions/{revision_id}` |
| POST | `/api/v1/revision-commits` |
| GET | `/api/v1/revision-commits/{request_id}` |
| POST | `/api/v1/revision-commits/{request_id}/approve` |
| POST | `/api/v1/revision-commits/{request_id}/reject` |
| POST | `/api/v1/workflows/mechanical/reference` |
| GET | `/api/v1/workflows/{workflow_id}` |
| GET | `/api/v1/workflows/{workflow_id}/checkpoints` |
| POST | `/api/v1/workflows/{workflow_id}/checkpoints/approve` |
| POST | `/api/v1/workflows/{workflow_id}/checkpoints/reject` |
| GET | `/api/v1/events` |

Checks:

- 201 start vs 200 resumed for the mechanical workflow
- revision create/commit/approve/reject, list/read, location headers
- domain evaluate/list/read
- workflow checkpoints and approve/reject
- events list and filters
- nested error envelope `{error:{code,message},details}` (never unified with
  Studio)
- 421 Host, 403 Origin including OPTIONS preflight, 401 missing/wrong token,
  413 declared/chunked, 404 unknown domain/revision/prepared/workflow/route,
  405 method, 409 fork/stale digest/decided checkpoint/finished workflow,
  422 extra field/parameters/query/JSON/content type/identifier
- security headers on every response; `server`/`x-powered-by`/`set-cookie`
  absent

IDs are deterministic: `rev-<design>-r<sequence>`, commit request IDs derived
from the two digests, supplied workflow IDs. No generated-ID normalization was
needed.

### 4.6 Geometry, STL, and STEP

Six meshes at default and maximum tessellation:

| Mesh | Vertices | Triangles | Bounds (mm) | STL bytes | STL SHA-256 |
|---|---:|---:|---|---:|---|
| box default | 24 | 12 | ±5 each axis | 684 | `e6c04f2a…d2c7dfd5` |
| box max (`subdivisions=64`) | 25,350 | 49,152 | ±5 | 2,457,684 | `6ae74307…da2febde` |
| cylinder default | 198 | 192 | ±5 | 9,684 | `0c87786b…a8548243` |
| cylinder max (256×64) | 17,221 | 33,280 | ±5 | 1,664,084 | `cd67e50d…770efcec` |
| sphere default (24×32) | 761 | 1,472 | ±5 | 73,684 | `ea50ff2e…202419e3` |
| sphere max (96×128) | 12,257 | 24,320 | ±5 | 1,216,084 | `b7b27e47…cc32ea61` |

- Every mesh is watertight under the position-keyed edge check (each undirected
  edge used exactly twice).
- Positions, normals, and indices have compact-JSON SHA-256 digests, lengths,
  and first/last samples.
- Signed volume is recorded with a tolerance note; it is diagnostic, not a
  bit-exact contract.
- Default box/cylinder/sphere STL files are checked in byte-for-byte. Max STLs
  are digest/length only.
- Sphere-max STL SHA-256 begins `b7b27e47…32ea61`, matching the audit's
  independent measurement.
- STEP: OCP 8.0.1.1.0 is available and a default box export succeeded
  (15,440 bytes). The fixture records ISO prefix, header lines, timestamp
  presence, `MANIFOLD_SOLID_BREP` count 1, and length. STEP bytes and SHA-256
  are informational only; the header timestamp forbids byte equality.
- The STEP 503-unavailable branch is not captured because OCP is installed.

### 4.7 Domain package provenance — LEGACY Python identity

`domain_provenance.json` records, with `provenance_kind:
"legacy_python_source"` and `rust_identity_policy_required: true`:

| Domain | Package | Version | Package digest |
|---|---|---|---|
| building | `building.storey-envelope` | 0.1.0 | `a6d193634794b7a0f28af54ea451924314d61e803ced03610efdbfd6570f40b7` |
| mechanical | `mechanical.bracket` | 0.1.0 | `97a2ebade1532e16864d81bc1f42b725c584c14250d1ae1f946eff1d4d5937b5` |
| wing | `wing.parametric-tapered` | 0.1.0 | `108cd68bf1ec08088f526e5ef0b8f2034199298fe7dc443e7c425e46d2358c71` |

Also recorded: `evaluator-implementation-digest-v2`, each evaluator's module
digest/package digest/implementation digest, manifest, schema digests, and the
seven source files per package plus ancestor `__init__` files.

These values cannot be reproduced from Rust sources. They must be pinned as
explicit versioned legacy constants, or replaced by a visible
`implementation_digest_version` bump. Silent drift is unacceptable.

### 4.8 CLI and package launch

- 12 public core commands: `init`, `token`, `serve`, `domains`, `evaluate`,
  `run-mechanical`, `workflow-status`, `approve`, `reject`,
  `prepare-revision`, `commit-revision`, `events`.
- Exit codes recorded: 0, 1, 2, 3. Nine executions cover init, domains,
  passing evaluation, failing hard-constraint evaluation (exit 3), events,
  unknown workflow, unknown prepared commit, usage error, and `--help`.
- Command output is captured with `<DATA_DIR>` normalization; help prose is
  truncated after 20 lines with a full SHA-256/length.
- Studio launch: `python -m kala_agent.studio`, default `127.0.0.1:8765`,
  `--host`/`--port`/`--data-dir`/`--static-dist`, token from
  `KALA_STUDIO_TOKEN` or the studio token file, loopback-only validation,
  exit codes 0/1/2.
- Core serve: `127.0.0.1:8000`, `--host`/`--port`, loopback-only,
  `KALA_AGENT_DATA_DIR`, exit codes 0/1/2/3.
- Electron contract recorded from the audit (not re-measured): `--host
  127.0.0.1 --port <n>`, `KALA_STUDIO_TOKEN`, readiness ≤15 s, SIGTERM then
  SIGKILL at 3 s, packaged path `resources/backend/kala-studio[.exe]`.

## 5. Determinism, normalization, and safety

- Application timestamps use a fixed one-second sequence from
  `2026-01-01T00:00:00Z`.
- `EventJournal.append` uses the wall clock in production. HTTP and CLI
  generation inject a fixed subclass for `kala_agent.persistence.journal.datetime`
  so event timestamps and dependent digests are reproducible. This is a
  fixture override, not a product change; it is recorded in both fixtures.
- Core `/health` `data_dir` and CLI temporary paths are `<DATA_DIR>`.
  Nothing else is normalized. Meaningful differences in keys, floats, scale,
  envelopes, headers, and digests are preserved.
- The synthetic core/studio bearer tokens and provider key are fixture-only,
  never live credentials. The provider key is cleared in the same run.
- No external LLM, network, user database, token store, or user credential is
  used. Studio provider cases are local, invalid, or unconfigured.
- SQLite file bytes and STEP bytes are informational; logical rows and
  structural fields are asserted.
- No timing/RSS numbers are included, and none are presented as speedups.

## 6. Gaps and discrepancies

1. **Core route count.** The task/audit says 18 core routes; the live route
   table and OpenAPI expose 17 total (root `/health` plus 16 under `/api/v1`).
   All 17 are covered. The extra route could not be located. This is a
   documentation/audit discrepancy, not an uncovered known route.
2. Windows/COM/NSIS/Windows CI was not exercised. No live Windows proof is
   claimed. OCP is available on Linux; FreeCAD was not run.
3. HTTP is captured through Starlette `TestClient`, not a live uvicorn socket.
   Transport-generated `Date` and `Server` headers are therefore not frozen.
4. The Studio STEP 503-unavailable path is not captured because OCP is
   installed. STEP is captured structurally for the default box only;
   cylinder/sphere STEP is not in the corpus.
5. Core HTTP uses a fresh temporary database. It does not include a frozen
   core data-directory snapshot or a multi-process writer run.
6. The CLI `serve` command is resolved but not started; no live socket launch
   is recorded.
7. Static traversal/symlink edge cases remain covered by the existing Python
   security tests, not this corpus.
8. `KALA_STUDIO_MESH_LIMIT` is documented in `docs/studio-limitations.md` but
   absent from the implementation; it is not frozen.
9. Pydantic validation message text is frozen for the recorded package
   versions and can drift on upgrade. The corpus pins those versions.
10. Domain package digests are legacy Python source identity, not Rust
    identity. A migration/provenance policy is still required before new core
    writes.
11. No timing, RSS, startup, or throughput measurement is included. Those
    must be repeated after a release Rust build exists on a quiescent host.

## 7. Recommended Rust public types

These are shaped by the fixtures, not by a preference for a crate.

### Canonical JSON

```rust
pub struct Sha256Digest([u8; 32]);

pub enum CanonicalValue {
    Null,
    Bool(bool),
    Integer(BigInt),          // 2**70 and wider must stay exact
    Float(f64),
    Text(JsonText),           // must represent lone surrogates or declare divergence
    Array(Vec<CanonicalValue>),
    Object(BTreeMap<JsonText, CanonicalValue>),
}

pub enum CanonicalJsonError {
    NotJson, InvalidUtf8, DuplicateKey, NonStringKey, NonFinite,
    DepthExceeded, StringTooLong, KeyTooLong, CollectionTooLarge,
    ByteLimitExceeded, NotSerializable,
}

pub fn canonical_json_dumps(value: &CanonicalValue) -> Result<String, CanonicalJsonError>;
pub fn strict_json_loads(bytes: &[u8]) -> Result<CanonicalValue, CanonicalJsonError>;
```

Use a CPython-`repr` float formatter (shortest round-trip; exponent below
1e-4 or at least 1e16; two-digit exponent; explicit `+`) and an ASCII escaper.
Do not use JCS or plain `serde_json` output for hashes.

### Chain and persistence

```rust
pub struct ChainFields {
    pub sequence: u64,
    pub event_type: String,
    pub aggregate_id: String,
    pub workflow_id: Option<String>,
    pub run_id: Option<String>,
    pub occurred_at: String,
    pub payload_digest: Sha256Digest,
    pub previous_event_digest: Sha256Digest,
}

pub fn chain_digest(fields: &ChainFields) -> Sha256Digest;
pub struct ChainVerification { pub event_count: u64, pub head_sequence: Option<u64>, pub head_event_digest: Option<Sha256Digest> }
```

Keep the 8-byte big-endian length prefix on every part including the domain.
Preserve default SQLite durability and the append-only triggers; do not add a
head cache that skips old-row tamper detection.

### Decimal and budget

```rust
pub struct ScaledDecimal { coefficient: BigInt, scale: i32 } // arbitrary precision, scale-preserving
pub struct DecimalContext { precision: u32 /* 28 */, rounding: HalfEven }
pub struct DurationMicros(i64);

pub struct BudgetUsage { pub cost: ScaledDecimal, pub duration: DurationMicros, /* counters */ }
pub struct RunBudget { pub max_cost: ScaledDecimal, /* other limits */ }
pub struct BudgetLedgerState { pub version: u32, pub budget: RunBudget, pub usage: BudgetUsage, pub reserved: BudgetUsage, pub reservations: Vec<BudgetReservation>, pub next_reservation: u64 }
```

Do not choose `rust_decimal`'s fixed 96-bit value as the public domain. If it
is used internally for a bounded subset, convert at the boundary and preserve
scale/exponent. Duration rounding is ties-to-even on the microsecond grid.

### HTTP contracts

```rust
pub struct StudioErrorEnvelope { pub code: String, pub message: String, pub details: Vec<String> }
pub struct CoreErrorEnvelope { pub error: CoreErrorDetail, pub details: Vec<String> }
pub struct CoreErrorDetail { pub code: String, pub message: String }

pub struct HttpFixtureCase { pub id: String, pub request: HttpRequestSpec, pub expected: HttpResponseSpec }
pub struct HttpRequestSpec { pub method: String, pub path: String, pub headers: BTreeMap<String, String>, pub body: HttpBodySpec }
pub enum HttpBodySpec { None, Json(CanonicalValue), RawBytes(Vec<u8>), Chunked(Vec<Vec<u8>>) }
pub struct HttpResponseSpec { pub status: u16, pub headers: BTreeMap<String, String>, pub headers_absent: Vec<String>, pub body: ExpectedBody }
pub enum ExpectedBody { ExactJson(CanonicalValue), ExactBytes(Vec<u8>), DigestAndLength { sha256: Sha256Digest, length: u64 }, StructuralStep(StepStructure), PreviewSummary(PreviewSummary) }
```

Keep Studio flat and core nested envelopes distinct. A shared transport
extractor may be used by core transport, but the service contracts must not be
unified.

### Geometry

```rust
pub struct Mesh { pub positions: Vec<f64>, pub normals: Vec<f64>, pub indices: Vec<u32> }
pub struct Bounds { pub min: [f64; 3], pub max: [f64; 3] }
pub enum PrimitiveSpec { Box { size: [f64; 3], subdivisions: u32 }, Cylinder { radius: f64, height: f64, radial: u32, vertical: u32 }, Sphere { radius: f64, rings: u32, sectors: u32 } }
pub struct StlBinary(pub Vec<u8>);
pub struct StepStructure { pub iso_prefix: bool, pub manifold_solid_brep_count: u32, pub length: u64, pub header_timestamp_detected: bool, /* no byte equality */ }
```

Positions/normals are `f64` internally and packed to `f32` in binary STL.
OCP STEP remains an explicit sidecar/optional capability; do not imply the
Rust process owns an exact B-rep kernel if it does not.

### Provenance and identity

```rust
pub enum PackageIdentity {
    LegacyPythonSource(Sha256Digest),
    RustVersioned { implementation_digest_version: u32, digest: Sha256Digest },
}
pub struct DomainProvenance { pub package_id: String, pub version: String, pub identity: PackageIdentity, pub source_files: Vec<SourceFileDigest>, pub legacy_warning: bool }
```

The legacy Python digests are pinned references, not the identity of Rust
sources.

### CLI

```rust
pub enum CliExitCode { Ok = 0, Error = 1, Usage = 2, FailedOutcome = 3 }
pub struct CliEnvelope<T> { pub command: String, pub data: T, pub status: String }
pub struct CliErrorEnvelope { pub error: CliErrorDetail }
pub struct StudioLaunchContract { pub host: String, pub port: u16, pub token_env: String, pub readiness_seconds: u64, pub term_grace_seconds: u64 }
```

## 8. Review corrections honored

- Studio flat vs core nested envelopes are separate fixtures; no unification.
- Canonical JSON is actual CPython bytes; no JCS/serde substitution.
- Decimal is treated as unbounded and scale-preserving; `rust_decimal` fixed
  domain explicitly rejected as the public representation.
- Domain provenance is marked LEGACY Python source identity; a Rust identity
  policy is required before new core writes.
- COM is not assumed to require a Python worker in the corpus; Windows COM
  proof remains pending.
- FreeCAD is reported installed on this host but was not run; no live claim.
- Journal integrity and default durability are preserved in the database
  fixture (append-only triggers + full chain verification).
- The oracle/harness is repository-owned, not only under `/tmp/opencode`.
- No equivalent-capability or RSS claim is made; timing comparison is deferred.
- `Rust+buid123/` and `rust-implementation.md` were not touched.

## 9. Handoff

This corpus is ready for the W1 canonical JSON/contracts gate and the W3
Studio vertical. Before core write cutover, the reviewer must settle:

1. lone-surrogate text representation (WTF-8 vs explicit divergence);
2. arbitrary-precision scaled decimal representation and context;
3. legacy domain package digest pinning vs versioned replacement;
4. the 17-vs-18 core route discrepancy in the audit/task text;
5. a real Windows host for COM/NSIS/CI proof.

No Cargo workspace, lockfile, Rust source, packaging change, or benchmark was
created by this cell.
