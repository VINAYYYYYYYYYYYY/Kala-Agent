# kala-persistence

Native SQLite schema v2, append-only verified event journal, canonical workflow
snapshots and the stored-object JSON domain for Kala Agent. This crate is a
member of the root `backend/` workspace and was accepted after repair round 2
and deep-JSON compatibility checks.

## Public API

- `Database` — one serialized SQLite writer, frozen v1/v2 migrations, foreign
  keys and WAL, SAVEPOINT-nested transactions with rollback on errors, failed
  finalizers and panics; guarded close.
- `EventJournal` — verified append/read/get with full stored-chain verification
  before every read, raw stored bytes in chain preimages and Python-compatible
  identifier normalization only in the typed view.
- `SnapshotStore` — canonical snapshot bytes and digest verification on every
  read.
- `canonicalize_json_object` / `persistence_limits` — the larger
  `contracts.models.CanonicalJsonObject` domain (no tool-protocol depth/byte
  caps; iterative parser/validator/encoder/lifecycle).

Stored chain strings must be exactly 64 lowercase hexadecimal characters;
`Sha256Digest::from_hex` accepts uppercase and is not used as that validation.
Journal appends verify the tail as the Python source does; reads and `get`
verify all old rows.

## Tests

From `backend/`:

```bash
cargo test -p kala-persistence --locked
cargo test -p kala-persistence --locked --release
cargo clippy -p kala-persistence --all-targets --locked -- -D warnings
```

65 accepted tests cover schema/migration equality, journal fixtures and
security, snapshots, canonical domain, deep JSON, identifier whitespace and
real Python cross-readers in both directions. Typed domain repositories,
migrations from arbitrary existing user databases and a Windows build remain
outside this slice. See `../README.md` for proof boundaries.
