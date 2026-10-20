# readiness-sync: make the native-readiness scanner and packaging tests match the r2 kala-studio refactor
Own ONLY `studio/scripts/native-readiness.cjs` (and its .d.cts), `studio/scripts/native-backend.cjs` (only if the failures require it), and `studio/test/nativeBackendPackaging.test.ts`. Read `/home/v9i/Projects/Kala-Agent/.mece/waves/rust-backend-20261019/cells/r2-studio-proxy/REPORT.md` and r2-followup's REPORT if present, plus `../d1-qa/REPORT.md` (the 7 failing tests).
r2 replaced `allowed_methods(path)` in `backend/crates/kala-studio` with a `RouteKind` table and moved the native primitives behind the non-default `native-primitives` feature. The readiness parser and the packaging-helper tests still expect the old shape, so 7 tests in `nativeBackendPackaging.test.ts` fail.
- Update the parser to read the current Rust route table (parse the real source; refuse to guess, as now).
- Report Rust-native routes vs proxied-to-worker routes separately.
- Update the readiness expectations: with r2 the native front door now covers the public routes via the proxy, so readiness must be computed, not hard-coded.
- Fix the packaging-helper tests that broke for the same reason, without weakening their safety checks.
Do not edit Rust or Python. Gates: the full studio `npm test` green (all 541+), plus typecheck. Write REPORT.md.
