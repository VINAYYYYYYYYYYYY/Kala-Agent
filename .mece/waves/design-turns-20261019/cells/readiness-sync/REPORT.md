# readiness-sync — native-readiness scanner and packaging tests for the r2 front door

Cell: `readiness-sync`, wave `design-turns-20261019`.
Status: **done**. The full studio test suite and typecheck are green.

Read first: `rust-backend-20261019/cells/r2-studio-proxy/REPORT.md`,
`r2-followup/REPORT.md`, `design-turns-20261019/cells/d1-qa/REPORT.md`, and
`backend/crates/kala-studio/src/server.rs` (the `RouteKind` table, mtime
01:12:45) plus `backend/crates/kala-studio/Cargo.toml` (`default = []`).

No Rust or Python file was edited. No commit, push, deploy, delegation, or
credential read was performed. Only the four files below plus this report
were touched.

---

## 1. Root cause of the 7 failures

`d1-qa` recorded the failures at 00:13, before the Rust wave refactored
`server.rs` at 00:39–01:12. The old scanner looked for
`fn allowed_methods(path: &str)` and a `vec![Method::…]` body; r2 replaced it
with:

* `fn classify(path) -> Option<(RouteKind, Option<String>)>` — the path table,
  including the `strip_prefix("/api/studio/design/")` +
  `strip_suffix("/drawing" | "/section")` family;
* `fn allowed_methods(kind: RouteKind) -> &'static [Method]` — the method
  table;
* `fn route(...)` — the dispatch where the geometry kinds call
  `proxy_route`.

The scanner found no arms and threw
`the native Studio route table was parsed but contained no route-methods`.
That exception also made `metadata.sourceParity.pythonSourceSha256` `null`,
which produced the unrelated-looking staging failure and the
`invalid or ambiguous` test failure (the readiness call ran before
`decidePackaging` could reject the argv).

The three readiness assertions were stale in the other direction too: they
pinned 14 public Python routes, 9 native routes, 5 missing routes, and
`ready === false`. The current public Python surface is **15** route-methods
(`GET /api/studio/providers/cad` was added to `server.py` by the parallel
provider work at 00:40), and the r2 front door now covers the public surface
itself, forwarding the geometry routes to the supervised worker.

---

## 2. Files changed

| Path | Change |
|---|---|
| `studio/scripts/native-readiness.cjs` | Parse the r2 `classify` / `allowed_methods` / `route` tables; split Rust-native from proxied routes; report both; compute readiness from the parsed surface. |
| `studio/scripts/native-readiness.d.cts` | Types for `parseRustRouteSurface`, the analysis split fields, and `report.native.rustNative*` / `proxied*`. |
| `studio/scripts/native-backend.cjs` | `nativeStep` is now true only for `--features native-step` or `--features native-primitives`, matching `default = []`; the marker check and message follow the new feature. |
| `studio/test/nativeBackendPackaging.test.ts` | New RouteKind synthetic fixtures, split/refusal tests, r2 readiness expectations, pending-report packaging fixtures, native-primitives marker tests. |

---

## 3. The scanner (public interface)

`parseRustRouteSurface(source)` (new, exported and typed) returns:

```ts
{
  routeMethods: string[];          // all front-door route-methods
  rustNativeRouteMethods: string[];// answered in Rust
  proxiedRouteMethods: string[];   // forwarded to the Python geometry worker
}
```

`parseRustRoutes(source)` keeps its old signature and returns `routeMethods`.

`analyzeStudioSources` now also returns `rustNativeRouteMethods`,
`rustNativeRouteMethodCount`, `proxiedRouteMethods`, `proxiedRouteMethodCount`.
`checkStudioReadiness().native` carries the same fields, the CLI text prints
the two groups separately, and the summary reports the split.

### How it parses, and what it refuses

* A comment/string/char-aware balanced-brace scanner extracts the real
  function bodies. No regex can run past the end of a function.
* `classify`: literal `"path" => RouteKind::X` arms and
  `strip_prefix("P")` / `strip_suffix("S") => RouteKind::X` family arms. The
  family path is `P + "{}" + S`, then normalized like a Python route.
* `allowed_methods`: `RouteKind::A | RouteKind::B => &[Method::GET, …]` arms.
  The method list must contain only recognised `Method::` values.
* Every classified kind must have exactly one method arm, and every method
  arm must be returned by the classifier. A mismatch, a duplicate, an empty
  table, or an arm shape the parser does not understand throws.
* When the source contains `fn proxy_route`, a parseable `fn route` must
  exist and name at least one proxied kind. The split comes from the real
  dispatch, not from a label or a comment.
* The pre-r2 `fn allowed_methods(path)` shape is refused
  (`could not find fn classify`), never guessed.

## 4. Current computed readiness

Parsed from the real sources:

| Surface | Count |
|---|---:|
| Python public `/api/studio/*` route-methods | 15 |
| Python worker-only `/internal/v1/*` route-methods | 4 |
| Rust front door, all | 16 |
| — Rust-native | 6 |
| — proxied to the geometry worker | 10 |
| Missing Python route-methods | 0 |
| Native-only (allowed, not parity) | 1 (`POST /api/studio/import`) |
| Missing DTO capabilities | 0 |

Rust-native: `GET /health`, `GET /integrations`, `GET /providers`,
`GET /settings`, `POST /settings`, `POST /chat`.
Proxied: `/preview`, `/export`, `/handoff`, `/design`, `/design/{}/drawing`,
`/design/{}/section`, `/journal`, `/journal/verify`, `/providers/cad`,
`/import` (all methods as in the Python table).

`ready` is computed as `missingRouteMethods.length === 0 &&
missingCapabilities.length === 0`; it is **true** for the current sources,
and the test asserts that equality instead of pinning a literal. The
`POST /api/studio/import` native-only route is reported but does not fail the
diff (same rule as before).

## 5. Packaging-helper tests

* The staging fixture now asserts `metadata.nativeStep === false` for the
  default build, because r2's `default = []` links no native geometry. This
  is the truthful value, not a weakened check.
* The former "rejects a no-default-features binary" test became: a
  marker-less binary stages as a proxy-only build (`nativeStep: false`), but
  the same binary with `features: 'native-primitives'` is refused with
  `built without the native primitives but metadata says nativeStep=true`.
  A marker-bearing binary with `features: 'native-step'` stages with
  `nativeStep: true`. The marker requirement is therefore preserved exactly
  where the metadata claims native STEP.
* The "preserves a previous good stage when replacement validation fails"
  test now opts into `native-primitives`, so the marker-less bad binary still
  defeats candidate validation and the old stage is preserved.
* Packaging-gate tests (`decision` blocks/permits, effective electron-builder
  targets, invalid-argv fail-closed) now use an explicit synthetic
  *pending* report and an explicit *ready* report. The checks themselves
  (installer target blocked unless ready; `--dir` labelled and not release
  eligible; invalid/ambiguous argv fails closed) are unchanged. The real
  report is no longer used as a stand-in for "pending", which was the stale
  hardcoding the task asked to remove.
* Added a test that a classifier/method-table mismatch and the legacy
  `allowed_methods(path)` shape are refused.

## 6. Gates (studio/)

| Command | Result |
|---|---|
| `npm run typecheck` | **clean**, exit 0 |
| `npm test` | **42 files, 592 tests passed**, exit 0 |
| `npx vitest run test/nativeBackendPackaging.test.ts` | **39 tests passed** |
| `node scripts/native-readiness.cjs --report-only` | exit 0; READY, 15 public / 4 internal / 6 native / 10 proxied |
| `node scripts/native-backend.cjs --help` | exit 0 |
| `node --check` on both changed CJS helpers | pass |

A transient full-suite failure in `test/DesignAgentCard.dom.test.tsx` came
from C2's in-flight edit at 01:28–01:36 (it expected the lowercase digest
label `fcstd` while the concurrent source produced `FCStd`). It disappeared
on the rerun after C2 updated that test at 01:36:52; no file outside this
cell was edited.

## 7. Remaining gaps (honest)

1. **`desktop-pack.cjs` still requires `metadata.nativeStep === true`**
   (line ~252) before invoking electron-builder. That file belongs to the
   Rust delivery cell, not this one. With r2's default proxy-only build the
   truthful metadata is `nativeStep: false`, so release packaging remains
   blocked by that stale rule even though the readiness report is green.
   The delivery cell must decide how the Python/OCP geometry worker is
   packaged (the cross-wave option B) and update that rule; this cell did not
   weaken or bypass it.
2. **The stage still rejects Python/OCP trees.** That is the delivery cell's
   "no Python in the native distribution" rule. The geometry worker cannot
   be included until that rule is changed, as
   `CROSS-WAVE-CONFLICT.md` records.
3. **The scanner is intentionally shape-strict.** A future route-table
   refactor that renames `classify`/`allowed_methods`/`route`/`proxy_route`
   or uses a new arm shape will make the readiness check exit 2 rather than
   report a guessed surface. That is the intended fail-closed behaviour; the
   parser must be extended with the new shape.
4. **The Python public count is 15, not D1's 14.** `GET
   /api/studio/providers/cad` was added to `src/kala_agent/studio/server.py`
   at 00:40 by the parallel provider work. The test pins the current real
   surface; if that Python surface changes again, the test and the parser
   fixture must follow.
5. No Windows runtime, packaging, or installer claim is made. The changed
   helpers are platform-neutral Node code; the test suite ran on Linux.
