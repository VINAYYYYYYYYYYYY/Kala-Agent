# Foundation cell session log

## Scope

Implement the first native Rust slice: Cargo workspace plus `kala-json`
canonical JSON and journal event-chain digest, byte-compatible with the
accepted Python corpus. Own root workspace/lockfile and the crate; read-only for
the Python corpus and source. No delegation, commit, push, deploy, model change,
or edits to user-owned/other-worker files.

## Sequence

1. Read `TASK.md`, `WAVE.md`, `REVIEW.md`, audit/corpus reports, the corpus
   README/generators, `src/kala_agent/tools/protocol.py`,
   `src/kala_agent/persistence/chain.py`, and the v1 fixtures.
2. Confirmed actual Python behavior with the `.venv` (3.14.8): float repr
   thresholds, escaped surrogate-pair combination, duplicate/non-finite/BOM/
   trailing errors, integer digit limit, object-vs-array collection limits.
3. Created `backend/Cargo.toml`, `backend/.gitignore`, and the `kala-json`
   crate. Chose edition 2024, MSRV 1.85, and only `members=["crates/kala-json"]`
   with geometry/budget excluded.
4. Implemented code-point `JsonText`, typed `CanonicalValue`, bounded
   `Limits`, exact Python error messages, strict parser, canonical serializer,
   CPython float formatter (ryu digits plus CPython thresholds/exponent
   spelling), SHA-256 digest, and typed `EventChainFields` preimage/digest/
   verification.
5. Added corpus tests that read the real Python fixtures read-only, including
   every success byte/SHA and every error message, all five chain preimages and
   the real SQLite event rows.
6. Added a test-only Python differential runner:
   `tests/python_oracle.py` imports the live product; the Rust test sends 6,000
   deterministic random finite f64 bit patterns plus curated boundaries, and
   415 structured Unicode/key/large-integer/decimal-literal sources.
7. Repairs during validation:
   - corrected fixture path (`../../fixtures`, not `../../../`);
   - made random object-key generation duplicate-free;
   - corrected the ASCII byte-cap unit expectation;
   - matched Python's object behavior: collection count is limited for arrays
     only, not objects;
   - added CPython's 4,300-digit int/str limit;
   - replaced a `verify_chain` assertion with a typed error.
8. Ran formatter, all tests, clippy with `-D warnings`, release build and
   `cargo metadata`; documented results in `REPORT.md` and `backend/README.md`.

## Decisions

- `JsonText` stores `Vec<u32>` rather than WTF-8 so sorting, `len`, escaping
  and lone-surrogate semantics map directly to Python code points.
- Canonical hashing never uses `serde_json`; it is a dev-dependency only for
  reading fixture/test data.
- Float formatting uses ryu only for shortest digits; CPython's lower exponent
  threshold differs from ryu's pretty formatter, so the output is reformatted.
- `EventChainFields` uses typed `Sha256Digest` fields and preserves the
  domain/8-byte-big-endian preimage exactly.
- Unsupported Python source domains (Decimal, non-string keys) are refused by
  the type system and asserted as explicit fixture expectations, not skipped.

## Actual verification

- `cargo fmt --all --check` — pass
- `cargo test -p kala-json` — 23 tests pass (17 unit, 3 corpus, 3 differential)
- `cargo clippy --workspace --all-targets -- -D warnings` — pass
- `cargo build --release --workspace` — pass
- `cargo metadata --no-deps` — only `kala-json`
- Oracle: CPython 3.14.8, live `kala_agent.tools.protocol`
- 66/66 canonical JSON corpus cases, 5/5 chain preimages, 5/5 database events
- 6,024 float comparisons, 415 structured comparisons

## Open items

- Native Windows/MSVC build, COM and binary proof remain pending.
- No SQLite/journal write, HTTP, contracts, CLI, Decimal, geometry or budget in
  this slice.
- Multi-violation error precedence differs because validation happens after
  parsing in sorted key order; single-error corpus messages match exactly.
- No performance measurements or claims.
