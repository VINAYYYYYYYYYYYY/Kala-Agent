# kala-json

Native canonical JSON and journal event-chain hashing compatible with the Kala
Agent Python protocol. This crate is a member of the root `backend/` workspace;
it was accepted after byte-exact corpus and live-Python differential checks.

## Public API

- `parse_strict` / `canonical_dumps` / `canonical_digest` — strict bounded
  parsing and CPython-compatible canonical serialization (sorted code-point
  keys, ASCII escaping, shortest round-trip floats, arbitrary-precision
  integers, duplicate/non-finite rejection).
- `CanonicalValue`, `JsonText`, `Limits`, `Sha256Digest` — value model,
  code-point-faithful strings (lone surrogates preserved), explicit limits and
  digest type.
- `EventChainFields`, `chain_preimage`, `chain_digest`, `payload_digest`,
  `verify_chain` — the eight-part length-prefixed chain preimage, genesis
  linkage and typed-field verification.

`verify_chain` covers only typed fields. Persistence readers additionally
validate raw stored bytes, lowercase digests, canonical object bytes,
identifiers/timestamps and complete old-row chain integrity; this helper alone
is not full database verification. Stored objects must use the persistence
limits, not the default tool limits (depth 48 / 1 MiB).

The parser, validator, serializer and recursive value lifecycle use explicit
heap stacks. Tool input keeps the Python depth/size gates; persisted objects
are not silently bounded by those gates.

## Tests

From `backend/`:

```bash
cargo test -p kala-json --locked
cargo test -p kala-json --locked --release
cargo clippy -p kala-json --all-targets --locked -- -D warnings
```

33 accepted tests cover the Python corpus, deep/lifecycle behavior and a live
5,000-sample float differential plus structured Unicode/large-integer cases.
Python is a test oracle only; the library has no Python or subprocess calls.
See `../README.md` for workspace-wide compatibility rules and proof boundaries.
