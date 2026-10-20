# GPT-6.1-Sol review of migration inventory

Audit accepted as inventory and exploratory measurements, not an implementation or final optimization result.

## Decisions/corrections
1. Two public services retain DIFFERENT envelopes: Studio flat, core nested. The portable envelope extraction belongs to core transport; don't unify service contracts.
2. Canonical JSON must match actual Python oracle bytes, including exponent thresholds/sign/padding, -0.0, integral floats, ASCII Unicode escapes, supplementary/lone surrogates if accepted, large integers, duplicate keys, depth and byte limits. Never silently adopt JCS or plain serde_json output for hashes.
3. Source accepts arbitrary finite nonnegative Decimal values without fixed scale/range bounds. rust_decimal's fixed96-bit/28-scale domain may narrow valid inputs. Freeze wide/scale vectors before selecting representation; arbitrary-precision scaled decimal with Python-context arithmetic may be required. Preserve stored scale, exact counters and ties-even duration rounding.
4. Domain provenance must distinguish existing Python source identities from new Rust implementation identity. Preserve legacy data/digests on reads. New Rust provenance cannot silently claim Python source hashes describe Rust; an explicit versioned migration/provenance policy is needed before new core writes. Studio vertical does not depend on this decision.
5. COM itself is available natively to Rust on Windows; Python COM worker is not an unavoidable architecture requirement. Real Windows COM tests remain a gate. Any OCP/solver/vendor sidecar must be listed explicitly; don't label a Python-hosted backend a full Rust rewrite.
6. FreeCAD IS installed on this host (/usr/bin/FreeCADCmd,/usr/lib/freecad/lib, previously measured1.1.4). Audit only collected382 tests, so its statement "not installed" is inaccurate. No live FreeCAD run claimed from that audit.
7. Preserve full journal-integrity threat behavior and default SQLite durability. A head cache must not skip detection of old-row tampering; no weakening verification to manufacture speedup.
8. Exploratory baseline scripts are only under /tmp/opencode; make reproducible repository-owned oracle/harness before comparing. Repeat timings in a controlled run after concurrent implementation/tests quiesce.
9. Compare equivalent capabilities and same host/payloads/build modes. Report whole-process-tree RSS/startup including required OCP/vendor sidecars; separate backend-only RSS. Don't compare a Python process that loadedOCP to a Rust process omitting same capability without disclosure.
10. Preserve user-owned Rust+buid123 and rust-implementation.md. Add new backend workspace; no Python deletion/cutover until every replacement contract and packaging test passes.

## First vertical
After current Windows foundation integration checks: contract corpus +Rust canonical JSON/contracts, then standalone Rust Studio backend (all9 routes and security/static/provider behavior), native geometry/STL and existing desktop launch contract. Core persisted-data compatibility and write provenance follow. Entire backend goal remains active; initial Studio vertical is not completion.


## Persistence-specific source review
contracts.models.CanonicalJsonObject does not impose tools.protocol limits on stored objects. Preserve accepted large/deep/string/key/array values with explicit persistence Limits, not defaults. Sha256Digest::from_hex accepts uppercase; stored chain fields need raw lowercase validation before conversion. Typed field-chain helper alone is insufficient for payload/canonical/full-journal verification. See foundation/REVIEW-1.md.

## Studio serialization correction from implementation evidence
FastAPI response_model routes use Pydantic-core float formatting, which differs from CPython json.dumps; preview output must match actual fixture/live Pydantic bytes. Starlette/flat error responses may follow the CPython serializer. The earlier blanket guidance to apply CPython float repr to every HTTP body was too broad. Read live source/corpus per path; hash canonicalization remains CPython sortedASCII. Studio worker independently identified and tests this distinction.
