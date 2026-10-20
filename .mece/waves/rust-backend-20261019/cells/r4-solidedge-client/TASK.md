# r4-solidedge-client: Rust client for the Solid Edge bridge (ADR-0010 "Solid Edge track", ADR-0008)

Read `../../TAKEOVER.md`, `docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md`, `docs/adr/0008-solid-edge-windows-bridge.md`, `docs/construction-semantics.md`, and the Python reference you are porting: `adapters/solidedge/src/kala_agent_solidedge/{compile.py,client.py,journal.py,paths.py,errors.py}`, the wire contract `adapters/solidedge-bridge/src/kala_solidedge_bridge/{protocol.py,auth.py,errors.py,version.py}`, and their tests.

Rust is ONLY the connection and lowering layer here: it turns a ConstructionScript into bridge protocol v1 operations and talks to the bridge. Solid Edge does all the geometry. Do NOT port `oracle.py` or any geometry or volume computation.

Own: NEW `backend/crates/kala-solidedge-client/**` (standalone `[workspace]` with a local Cargo.lock, like the other standalone cells) and this cell's REPORT.md. Consume the accepted `backend/crates/kala-contracts` and `kala-json` read-only via path dependencies, if they already model ConstructionScript; otherwise define minimal serde types that round-trip the Python wire JSON exactly. Do not edit any other crate, the root manifests/lock, Python, adapters, or studio/**.

## Build
- `compile`: ConstructionScript → bridge operation list, refusing everything outside the supported subset with the same error codes and messages as `compile.py`.
- `client`: loopback-only HTTP client to the bridge with the same auth scheme as `auth.py` (token from env or file exactly as the Python client resolves it, never logged), protocol version negotiation, bounded timeouts and body sizes, typed errors mapped from the bridge's error envelope, and job submit/poll/result as in `client.py`.
- `journal`/`paths`: the same on-disk locations and record format as the Python client, so either client can read the other's records.

## Tests
- Differential lowering: for every script in `tests/fixtures/construction_parity.json` (read-only) and the Python compile tests' cases, the Rust lowering output equals the Python `compile.py` output byte-for-byte as canonical JSON. Run the live Python reference with `PYTHONPATH=adapters/solidedge/src:adapters/solidedge-bridge/src:src /home/v9i/Projects/Kala-Agent/.venv/bin/python -I` in tests or a generator. Refusal cases match too.
- Client against an in-process mock bridge implementing `protocol.py` semantics: auth required, version mismatch, error envelope mapping, timeout, oversize response, job lifecycle.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked` (debug and release), plus a Windows target `cargo check --target x86_64-pc-windows-gnu` if the target is installed (report it if not).

No live Solid Edge or Windows claim. Write REPORT.md: public API, test counts, differential results, and remaining gaps.
