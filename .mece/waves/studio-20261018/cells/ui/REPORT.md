# UI cell report — Kala Studio desktop workspace

## Status (current — repair pass 5, actual export/handoff binary contract)

All fixes from the actual-browser and actual-native reviews are in place
and verified with real checks. Typecheck clean, production build clean,
**125 tests passing across 13 files**, 0 production audit
vulnerabilities, 8 dev-only moderate audit findings (electron-builder
transitive chain, no criticals/highs). Layout from pass 4 is preserved
untouched. Packaging was validated end-to-end with a placeholder backend
(`electron-builder --linux dir` → exit 0; the asar contains
`electron/bootstrap.cjs` + `main.cjs` + `preload.cjs` + `package.json`,
and `resources/app/dist` + `resources/backend` are emitted), then the
placeholder and `release/` were removed.

Historical passes 1–4 are preserved below. Where an older section
conflicts with this one, **this section is authoritative**.

## Repair pass 5 — actual export/handoff binary contract

The conductor's real browser found the **Direct export** button was
always disabled and, more seriously, the client modelled a contract the
backend does not have.

### Actual backend contract (read-only: `src/kala_agent/studio/server.py`)

Both endpoints are a **single authenticated POST whose response body IS
the artefact bytes** — there is no metadata JSON and **no
`GET /api/studio/artifacts/...` route**:

- `POST /api/studio/export` — body `{kind, parameters, format}`; returns
  `Response` bytes with
  `content-disposition: attachment; filename="kala-studio-<kind>.<ext>"`
  and `content-type: model/stl` or `application/step`.
- `POST /api/studio/handoff` — body
  `{integration_id, kind, parameters, format}`; returns bytes with
  `content-disposition: attachment; filename="<integration_id>.<ext>"`
  and `x-studio-handoff: <id>;format=<fmt>;bytes=<n>`.
- Errors are JSON `{code, message, details}`: `422 invalid_request`,
  `503 step_unavailable`, `404 unknown_integration`, `409
  handoff_format_unsupported`, `401`/`403` auth.
- The registry gates handoff only on the row's **listed capability**, not
  on its native-automation status. `native_api_unavailable` (e.g.
  SOLIDWORKS + STEP) still renders a neutral file; `not_configured` rows
  are the only ones the UI keeps disabled.

### Root causes fixed

1. **Disabled direct button.** `canDownload` required
   `status === 'available_via_neutral_file'`, but the direct target
   reports `status: 'native'`, so it never enabled.
2. **Invented metadata + fetch contract.** `postExport`/`postHandoff`
   parsed a JSON `HandoffResponse`, then `downloadArtifact` issued a
   `GET /api/studio/artifacts/<name>` that does not exist. The old unit
   tests mocked that invented shape, so they passed against a false
   contract.

### Client / types changes

- `studio/src/types/studio.ts` — removed the wire-inaccurate
  `HandoffResponse`; added `ExportArtifact { blob, filename, bytes,
  format, integrationId }`, the unwrapped single-POST result.
- `studio/src/api/client.ts` — replaced the JSON+GET path with
  `sendBinaryRequest`: one POST with the bearer token, `redirect:
  'error'`, `no-store`, `credentials: 'omit'`; on `!ok` it reads the JSON
  error and maps `401/403` → `StudioAuthError` and everything else →
  `StudioApiError`; on success it reads `response.blob()` and reports the
  **real** `blob.size`. The filename comes from the
  `Content-Disposition` header through a new exported
  `sanitizeDispositionFilename` that accepts the RFC 6266 `filename*=`
  form or the quoted form, then reduces to a basename with control
  characters and quotes stripped (no path traversal). The handoff
  integration id comes from `x-studio-handoff` (falling back to the
  request payload). `downloadArtifact` and the dead `/artifacts/` URL are
  **deleted**.

### Panel changes

- `studio/src/components/export/ExportPanel.tsx` — `canDownload` now
  enables the direct target (`status: 'native'`) and any capability-
  listed handoff row whose status is `available_via_neutral_file` **or
  `native_api_unavailable`**; `not_configured` stays disabled. Download
  saves the returned blob in one step and reports `artifact.bytes`; the
  two-step "metadata then fetch bytes" path and its placeholder error are
  gone. The `native_api_unavailable` warning now states the truth — the
  studio cannot drive that tool's native automation, it writes a neutral
  file for the operator to import, and it **never launches or drives** the
  tool. Target options label those rows "neutral file (no native
  automation)" rather than "native API unavailable".

### Tests added / replaced

- `studio/test/apiClient.test.ts` — removed the invented
  metadata tests; added a `binary export and handoff` suite with **real
  binary response fixtures** (a byte body copied into an `ArrayBuffer`
  plus `Content-Disposition`/`x-studio-handoff` headers): direct export
  POSTs exactly once and consumes the bytes with no follow-up GET;
  handoff reads the id/filename from headers; fallback filename when the
  header is absent; path-traversal filename is reduced to a basename;
  RFC 6266 `filename*` is decoded; `401` → `StudioAuthError`; `503
  step_unavailable` and `409 handoff_format_unsupported` surface honestly;
  redirects are refused; an abort becomes `StudioAbortError`.
- `studio/test/ExportPanel.dom.test.tsx` (new, 8) — real React panel with
  the client mocked to the actual `ExportArtifact` shape: direct STL and
  direct STEP each perform one POST and save the bytes; FreeCAD
  (available_via_neutral_file) offers STL/STEP handoff; SOLIDWORKS
  (`native_api_unavailable`, STEP only) enables a capability-supported
  handoff with the honest warning and never claims to launch the tool;
  an unlisted format is refused (STL option disabled) and OpenSCAD
  (`not_configured`) stays disabled; `401` and `503 step_unavailable`
  are presented; and no stray `fetch` (a would-be GET) is issued.

### Checks run in pass 5 (actual outputs)

- `npm run typecheck` — exit 0, no diagnostics.
- `npm run build` — exit 0; `dist/assets/index-*.js` 51.92 kB, react
  218.82 kB, three 554.13 kB (pre-existing chunk-size warning only). The
  built bundle contains **no** `artifacts` route string.
- `npm test` — vitest 5.0.5, **13 files, 125 tests, all pass**.

### Honest limitations (pass 5)

- **Real backend round-trip not performed by this worker.** The DOM and
  client tests use real binary fixtures shaped exactly like the backend's
  `export_primitive`/`handoff` responses; the conductor owns the actual
  browser recheck against the live backend.
- **`not_configured` rows stay disabled** even though the backend would
  render their listed capability. This is the permitted conservative
  behaviour ("may stay disabled") and avoids claiming a handoff the
  registry does not advertise as available.
- **Real STEP still depends on OCP.** With OCP absent the backend returns
  `503 step_unavailable`; the UI now presents that message instead of a
  fabricated success.

---

## Repair pass 4 — actual-browser CSS overflow + meta CSP

The conductor reproduced the built UI in a real 1280x800 browser: Connect
succeeded and primitives rendered, but `.app-shell` was 800 while
`.app-shell-body` was 712 with an implicit row of **1055**, the sidebar
`min-height: auto` stretched that row, the viewport/canvas grew to
**959px**, the dimension overlay landed at y≈1047, and the footer was
pushed to y≈764 (below the window).

Root cause: `.app-shell-body` is a grid whose single row was left
**implicit (`auto`)**, so it sized to the tallest column — the sidebar's
intrinsic content — instead of the shell height. `min-height: 0` alone on
the body/main could not constrain an implicit `auto` track.

Fix (all in `studio/src/styles/studio.css`):

- `.app-shell-body` — added `grid-template-rows: minmax(0, 1fr)` and
  `overflow: hidden` so the row is pinned to the shell height.
- `.app-sidebar` — added `min-height: 0` (overriding the grid item's
  default `auto`), `min-width: 0`, and `overflow-y: auto` so it scrolls
  internally instead of growing the row.
- `html, body, #root` — `overflow: hidden`; the app shell now owns all
  scrolling (no page-level scrollbar).
- `.boot-gate` — `overflow: auto` + `padding` so a tall boot card still
  scrolls now that the document cannot.
- Compact (`max-width: 1100px`) — the old block left an auto-sized row
  and a phantom `footer-area` grid area (the footer lives outside the
  main grid). Replaced with a fully bounded stacked layout:
  `grid-template-rows: auto minmax(0, 1.4fr) minmax(0, 1fr)` and areas
  `toolbar / viewport / rail`, so the 960x720 window gets a scrollable
  viewport above a scrollable rail with the footer always visible.
- Narrow (`max-width: 720px`) — `grid-template-rows: auto minmax(0, 1fr)`
  (sidebar band + workspace) with the sidebar capped at `max-height: 42vh`
  and scrollable.

CSP: removed `frame-ancestors 'none'` from the `<meta>` tag in
`studio/index.html`. `frame-ancestors` is **ignored from a meta tag** (the
CSP spec only honours it in a header), so it only produced a console
warning. It is still enforced at both real response boundaries:

- `studio/electron/main.cjs` sets `Content-Security-Policy` with
  `frame-ancestors 'none'` in `webRequest.onHeadersReceived`.
- The backend sets it in
  `src/kala_agent/studio/middleware.py` → `SECURITY_HEADERS`, applied to
  the served `index.html` by `StaticFileMiddleware._serve_file`.

No other directive was touched, so no policy was weakened. Added
`studio/test/layoutCss.test.ts` (12 tests) which pins the structural
invariants above and asserts the meta CSP dropped `frame-ancestors` while
the header enforcers kept it.

## Repair pass 3 — the three actual-runtime regressions

### 1. Preview timeout / cancel lifecycle (the stuck "Verifying…" bug)

`studio/src/state/previewController.ts` — the controller's own 8-second
timeout calls `controller.abort()`, so the rejected fetch is classified
as `StudioAbortError` → `kind: 'cancelled'`. The old `catch` returned on
`desc.kind === 'cancelled'` **before** the `timedOut` branch, so a real
timeout never dispatched anything and the store stayed `inFlight: true`
forever. Fixed:

- The `timedOut` branch now runs **first** and dispatches
  `preview/request-error { kind: 'timeout', message: 'request timed out' }`,
  which clears `inFlight`.
- Explicit `cancel()`/`dispose()` now `invalidate()` the request
  generation (and clear the fingerprint) and dispatch a new quiet action
  `preview/request-cancelled` (added to `store.ts`) that sets
  `inFlight: false`, `lastError: null`, `lastErrorKind: 'none'` while
  **preserving the local mesh**. A stale completion — even a
  non-cooperative one that ignores the abort — is dropped by the
  generation guard and can no longer overwrite newer state.
- `trigger()` still supersedes via `abortAll()` + generation bump and
  deliberately keeps the loading flag owned by the new request. Local
  preview stays visible throughout.

`studio/src/state/store.ts` — added `preview/request-cancelled` to the
`Action` union and reducer (quiet loading clear, keeps `preview.local`
and any last-good server mesh).

### 2. Native Electron entry (the "no window, no backend" bug)

Proven cause: the app main `.cjs` exited a `require.main === module`
guard, which is **false** in Electron 44 (the browser-process main entry
is Electron itself). Fixed by separating the bootstrap entry from the
tested helpers:

- New `studio/electron/bootstrap.cjs` is the real entry (now
  `package.json#main`). It requires `./main.cjs` and calls the exported
  `bootstrap()`. It contains **no** `require.main` guard.
- `studio/electron/main.cjs` is now side-effect-free on import: it
  exports `bootstrap()` (which performs `app.whenReady()` →
  permission handler → `main()`), alongside the existing helpers.
- Unexpected owned-backend exit **after readiness** now closes the app:
  `BackendProcess` gained `ready`/`expected` flags and an
  `onUnexpectedExit` callback; `main()` wires it to log, `shutdownOnce()`
  (so the force-kill timer is cleared), `dialog.showErrorBox(...)`, and
  `app.quit()`.
- `studio/electron-builder.json#files` now bundles
  `electron/bootstrap.cjs`; `scripts/desktop.cjs` launches
  `bootstrap.cjs`; `main.d.cts` declares `bootstrap`.

### 3. Manual Connect registry loading + lifecycle

`studio/src/components/shell/BootGate.tsx` — the manual form submit and
the bridge probe now share **one lifecycle-owned `RegistryHandle`** held
in a ref. The manual path (`attemptManual`) no longer creates an unowned
handle: it aborts any in-flight controllers on the shared handle, then
runs the same `loadRegistries` pipeline. A new `activeLoad` identity plus
the existing `alive` flag means superseded loads, unmounts, and retries
all drop their callbacks. Unmounting or retrying cancels in-flight manual
fetches too.

### 4. Store reset (build blocker)

`studio/src/state/store.ts` — `resetStore()` previously wrote the private
`store.state` field from outside the class (TS2341 build failure). Now
`Store.reset()` is store-owned: it installs a fresh object graph from the
new `createInitialUiState()` factory and notifies subscribers. No private
bypass, no shared mutable global. `store.test.ts` asserts notification
count and object-graph independence across resets.

## Files changed in pass 3

- `studio/src/state/previewController.ts` — timeout-first completion,
  generation invalidation on cancel/dispose, quiet loading clear.
- `studio/src/state/store.ts` — `createInitialUiState()`,
  `Store.reset()`, `preview/request-cancelled` action.
- `studio/src/components/shell/BootGate.tsx` — shared
  lifecycle-owned handle for bridge + manual loads.
- `studio/electron/bootstrap.cjs` (new) + `electron/bootstrap.d.cts`
  (new) — real entry.
- `studio/electron/main.cjs` — exported `bootstrap()`, no auto-run
  guard, unexpected-exit handling, `dialog` import.
- `studio/electron/main.d.cts` — `bootstrap`, `BackendExitInfo`,
  `onUnexpectedExit`.
- `studio/electron-builder.json`, `studio/package.json`,
  `studio/scripts/desktop.cjs` — point at `bootstrap.cjs`.

## Tests added / expanded in pass 3

- `studio/test/previewLifecycle.test.ts` (new, 6) — real
  controller + real store + fake timers: the 8 s timeout clears loading
  and records `timeout`, then a retry succeeds; a timeout rejection
  classified as `cancelled` still completes; explicit cancel clears
  loading quietly and a non-cooperative stale completion cannot
  overwrite a newer preview; dispose clears loading; a superseded request
  cannot clobber a newer success; local fallback survives cancellation.
  **Verified the two timeout tests fail against the old ordering** (2
  failed) and pass with the fix.
- `studio/test/electronBootstrap.test.ts` (new, 9) — loads the **real**
  `bootstrap.cjs` via Node's native `require` with `electron` /
  `child_process` injected through a `Module._load` hook: startup runs
  even though `require.main` is not the module; exactly one backend
  spawns with explicit `--host 127.0.0.1 --port <chosen>`; the window is
  hardened (sandbox + contextIsolation + static preload + no
  nodeIntegration) and loads the trusted origin; an owned backend that
  exits after readiness shows a failure and quits without re-killing a
  dead child. Plus static wiring checks (no `require.main` anywhere,
  package/builder/launcher point at `bootstrap.cjs`, preload stays a
  static single-channel bridge).
- `studio/test/store.test.ts` (+2) — reset notifies subscribers and
  produces independent state graphs (no mutable global sharing).
- `studio/test/ManualConnect.dom.test.tsx` (3 → 5) — real React manual
  Connect now also asserts the initial box preview is rendered/requested,
  retry after failure returns to the form and reconnects, and unmounting
  mid-connect aborts the in-flight request signal.

## Checks run in pass 3 (actual outputs)

- `npm run typecheck` — exit 0, no diagnostics.
- `npm run build` — exit 0; `dist/assets/index-*.js` 50.88 kB, react
  218.82 kB, three 554.13 kB (pre-existing chunk-size warning only).
- `npm test` — vitest 5.0.5, **11 files, 97 tests, all pass**.
- `npm audit --omit=dev` — `found 0 vulnerabilities`.
- `npm audit` — 8 moderate, all dev-only in the
  `electron-builder → app-builder-lib → … → sprintf-js` chain.
- `node --check` on `bootstrap.cjs`, `main.cjs`, `preload.cjs`,
  `desktop.cjs`, `desktop-pack.cjs` — all OK.
- `node scripts/desktop-pack.cjs --linux dir` (with a placeholder
  `studio/backend/kala-studio`) — exit 0; asar contains
  `/electron/bootstrap.cjs`, `/electron/main.cjs`, `/electron/preload.cjs`,
  `/package.json`; `resources/app/dist` and `resources/backend` emitted.
  Placeholder + `release/` removed afterwards.

## Honest limitations (pass 3)

- **Real GUI launch not performed by this worker.** The conductor owns
  the final native launch; this worker does not spawn Electron (to avoid
  duplicate launches). The entry is now exercised by a real native
  `require` of the actual `bootstrap.cjs` with injected Electron, which
  is what proves the `require.main` regression can no longer recur.
- **Real packaged backend binary** `studio/backend/kala-studio[.exe]` is
  still owned by the delivery cell. Packaging was validated with a
  placeholder; production refuses to start without the real binary.
- **8 dev-only moderate advisories** remain (no safe fix in the
  electron-builder 26.x line).
- Writing to the store from a disposed preview controller (the quiet
  cancel) is intentional and safe (module singleton), but means a store
  subscriber can observe one final `preview/request-cancelled` after
  unmount.

---

## Historical record — repair pass 2 + desktop

> The sections below describe the earlier repair passes and are kept for
> traceability. Where they conflict with the pass-3 section above, the
> pass-3 section is authoritative.

## What changed in this round (desktop portion)

- `studio/electron/main.cjs` —
  - `buildBackendInvocation` now always appends `--host 127.0.0.1` and
    `--port <chosen>` to argv. The backend CLI defaults `--port` to 8765
    and does NOT read `KALA_STUDIO_PORT`; without the explicit `--port`
    flag the readiness probe was polling a port the backend never bound.
    The helper is exported so the spawn argv can be asserted directly.
  - `resolveBackendCommand` now uses `app.isPackaged` (loaded lazily
    via `require('electron').app`) instead of `process.resourcesPath`
    truthiness, because `process.resourcesPath` is defined in dev as
    well as in packaged builds. Dev mode prefers the project `.venv/bin/python`
    (or `.venv/Scripts/python.exe` on Windows) before falling back to
    `KALA_STUDIO_PYTHON` or a generic `python`.
  - Top-level `app.whenReady()` is now gated behind
    `require.main === module && process.versions.electron`, so the
    module loads cleanly under plain Node for tests.
  - `validateIpcSender`, `attachSecurityPolicy`, `registerIpc`, and
    `BackendProcess` are exported for unit tests.
  - `shutdownOnce()` runs once across `closed`, `window-all-closed`,
    and `before-quit` to avoid double shutdown.
- `studio/electron/main.d.cts` — TypeScript declarations for the new
  exported helpers (constructable `BackendProcess`, etc.).
- `studio/scripts/desktop.cjs` — simplified to ONLY prepare the UI
  build and start Electron with `process.execPath` and
  `require.resolve('electron/cli.js')`. No backend spawn, no token
  file on disk, no `npm.cmd`/`npx.cmd`. Forwards `process.argv.slice(2)`
  to Electron.
- `studio/scripts/desktop-pack.cjs` — forwards `process.argv.slice(2)`
  to electron-builder's CLI entrypoint (`require.resolve` over several
  candidate paths, no shell), and always injects `--publish never`
  unless the caller already passed `--publish`. Spawn errors and child
  exits are surfaced as non-zero exits.
- `studio/electron-builder.json` — `linux.desktop` now follows the
  `LinuxDesktopFile` schema from `app-builder-lib`: `{ entry: { Name,
  Comment, Categories, Keywords, StartupWMClass } }`. Removed unused
  `copyright` and `protocols`. Config validates under the installed
  `electron-builder 26.15.3` (verified by running
  `electron-builder/out/cli/cli.js --linux dir`, which proceeds through
  the config-load and packaging steps until it complains about the
  missing `studio/backend/` directory, which is the delivery cell's
  responsibility).
- `studio/package.json` — added `author.name` so electron-builder's
  required-field warning is silenced. No version bumps beyond what was
  already merged.
- `studio/test/electronMain.test.ts` — 17 mocked tests that exercise
  the real shipped module without booting Electron:
  - `buildBackendInvocation` produces argv containing `--host
    127.0.0.1 --port <chosen>` and never leaks the token.
  - The `--host`/`--port` flags are preserved alongside any existing
    args.
  - Spawning an actual child with the produced argv yields argv that
    matches what the backend CLI would observe.
  - `pickFreePort` returns a port in the IANA-registered range and
    actually releases the port (no `EADDRINUSE` on retry).
  - `generateToken` produces a base64url string of at least 16
    characters and distinct tokens across calls.
  - `validateIpcSender` rejects each failure mode (destroyed sender,
    wrong sender identity, wrong frame identity, unparseable frame
    url, origin mismatch) and accepts the matching-origin happy path.
  - `BackendProcess` records `error` / `exitCode` / `exitSignal` /
    `exited` correctly when the spawn fails with ENOENT or the child
    exits unexpectedly; `shutdown()` is idempotent.
  - `desktop-pack.cjs` source contains `process.argv.slice(2)`,
    `--publish`, `'never'`, and `require.resolve`, proving argv
    forwarding and `--publish never` injection without dragging
    electron-builder into a vitest run.
  - `desktop.cjs` source does NOT contain any `spawn` of the backend
    and does NOT write a token file, proving the launcher stays a
    thin Electron starter.

## What was NOT changed in this round

- Renderer code (`src/**`), `localPrimitives.ts`, the reducer store,
  the preview controller, the IPC client, the connection form, the
  BootGate, the App boot wiring, the right-rail tabs, the chat panel,
  the export panel, the inspector, the SceneCanvas. These were
  preserved from the previous pass per the conductor's instruction.
- `pyproject.toml`, `README.md`, `docs/**`, `packaging/**`,
  `.github/workflows/**`, `studio/backend/**`, the existing
  `AppBoot.dom.test.tsx`, `ChatPanel.dom.test.tsx`, and the unit tests
  for the API client / connection / store / preview controller.

## Checks run (actual outputs)

- `npm run typecheck` — exits 0, no diagnostics.
- `npm run build` — bundle written to `dist/`; main bundle 50 kB,
  React 219 kB, Three 555 kB.
- `npm test` — 8 files, 75 tests, all pass.
- `npm audit --omit=dev --audit-level=high` — `found 0
  vulnerabilities`.
- `npm audit --audit-level=high` — 8 moderate, all in the
  `electron-builder → app-builder-lib → @electron/get → global-agent
  → roarr → sprintf-js` chain. No criticals, no highs.
- `node ./node_modules/electron-builder/out/cli/cli.js --config
  electron-builder.json --linux dir` — config loads cleanly under
  installed `electron-builder 26.15.3`. The run proceeds through
  electron download / native dep install / packaging steps and stops
  only because `studio/backend/` is missing (delivery cell owns that).
- `node --check` on every CJS file — syntax OK.

## Honest limitations / known blockers

- **Backend not yet bundled.** `studio/backend/kala-studio[.exe]` does
  not exist in this checkout; the delivery cell drops it there. The
  Electron main refuses to start (exits 1 with an actionable message)
  if the file is missing. The dev path uses
  `python -m kala_agent.studio` from the repo `.venv` when present.
- **Electron download on this machine.** The local `electron` install
  did not download the native binary, so a real `npm run desktop` GUI
  launch is not exercised here. Tests cover the actual helper argv
  shape via a real spawn to `process.execPath`, which is the meaningful
  contract for the backend invocation.
- **electron-builder moderate advisories.** Same eight as the prior
  pass; no safe stable upgrade available.
- **Argv tests are static source-grep checks for the launcher and
  packager scripts.** A behavioural test would require running
  electron-builder, which would take minutes; the static check is the
  pragmatic trade-off and matches what the conductor asked for.

## Changed files (added or substantially modified in this pass)

- `studio/src/styles/studio.css` — root grid uses `auto minmax(0,1fr) auto`
  with a single `minmax(0,1fr)` column. Footer no longer uses
  `grid-area: footer-area` so it stops participating in the main grid
  template. Middle area is bounded so the canvas stays central. The
  narrow-window media query still references `footer-area`; that key is
  used only inside the `@media` block where the layout is rewritten.
- `studio/src/App.tsx` — preview controller is created in an effect;
  the same effect calls `next.trigger(primitive, parameters)` so the
  viewport is never a blank canvas after Connect. StrictMode
  double-mount is handled because the controller lives in a ref and the
  effect cleanup disposes the one it created.
- `studio/src/components/shell/BootGate.tsx` — rewritten: each effect
  invocation allocates a fresh `RegistryHandle`, registers three
  `AbortController`s (one for `/health`, one for `/integrations`, one
  for `/providers`), each with a 5 s timeout. Cleanup flips the handle's
  `alive` flag and aborts every controller the handle owns. StrictMode
  double-mount cannot leave a stale "dead" handle that the second
  setup reuses. Retry now bumps a `retrySeed` dependency so the user can
  retry after a real failure.
- `studio/src/components/shell/RightRail.tsx` — new tabbed rail
  (Inspector / Export / Assistant) so a single 360 px column fits on
  1280x800 without the chat panel sliding below the fold.
- `studio/src/components/shell/AppShell.tsx` — uses `RightRail`
  instead of stacking `Inspector + Export + ChatPanel` vertically.
- `studio/src/components/inspector/Inspector.tsx` — drift detection now
  returns an empty array when local and server bounds are identical and
  only renders the message when at least one axis exceeds the 0.01 mm
  tolerance. `describeBoundsDrift` returns `DriftEntry[]` typed
  explicitly.
- `studio/src/components/sidebar/Sidebar.tsx` — workspaces are now real
  panels under the navigation. Each workspace renders a content-aware
  panel: CAD → CAD integrations compact list, Analysis → analysis
  integrations compact list with the representative five tools the
  backend registers, Providers → provider list. Status badges use the
  exact backend enum (`available_via_neutral_file` /
  `native_api_unavailable` / `not_configured`).
- `studio/src/components/panels/IntegrationsPanel.tsx` — accepts an
  optional `category` filter and a `compact` flag (no notes in compact
  mode). Status label uses the new concise "Neutral file" wording.
- `studio/src/components/export/ExportPanel.tsx` — compact form: target
  `<select>` + format `<select>` (capability-filtered, options that the
  target cannot satisfy are disabled with "(not supported by target)")
  + one Download button. Native-only and not-configured targets show
  their honest label and disable the button. Capabilities are intersected
  against the supported-format set, not the raw registry string array.
- `studio/src/components/chat/ChatPanel.tsx` — model is optional for
  the local helper (`providerId === 'local' | 'local-helper'`); API
  modes still require it. Cancel button now dispatches
  `chat/error kind: 'cancelled'` so the `inFlight` state clears and the
  operator can immediately re-send. The "Cancelled" status is shown in
  the thread but not as an error.
- `studio/src/api/client.ts` — every request checks
  `signal?.aborted` *before* `fetch` and throws `StudioAbortError`
  immediately when true, so a pre-aborted caller cannot leak a fetch.
  Authorization header is applied per method through a small helper.
- `studio/src/state/previewController.ts` — `requestServer` tracks a
  `timedOut` boolean (set by the timeout timer, not by `controller.abort`
  called from elsewhere). A timed-out request dispatches
  `preview/request-error kind: 'timeout'` instead of `cancelled` so the
  UI can distinguish "slow server" from "user cancelled".
- `studio/src/viewer/SceneCanvas.tsx` — `OrbitControls.enableDamping`
  disabled; the continuous `requestAnimationFrame` loop is gone, so the
  scene renders only on demand (mesh change, viewport change,
  OrbitControls.change, resize). WebGL creation is wrapped in a
  try/catch that renders a graceful fallback element when WebGL is
  unavailable instead of letting the error propagate to React.
- `studio/electron/preload.cjs` — static file included in the
  electron-builder `files` list. Only exposes `kalaStudio.getConnection`.
  No `nodeIntegration`, no `shell.openExternal`.
- `studio/electron/main.cjs` — IPC validation now requires `sender ===
  win.webContents` AND `frame === win.webContents.mainFrame` AND the
  parsed `URL(frame.url).origin === trustedOrigin`. No `startsWith`
  checks. `setWindowOpenHandler` always returns `deny`; there is no
  `shell.openExternal` call. No JS is written to `userData`. Production
  refuses to start when the packaged backend is missing at
  `resources/backend/kala-studio[.exe]`. Backend child is reaped on
  window close, app quit, and unexpected child exit.
- `studio/scripts/desktop.cjs` — the launcher starts the backend in
  dev mode and hands the port/token to Electron via env vars with
  `KALA_STUDIO_MANAGED=1`. The main process trusts that flag instead of
  spawning a second backend.
- `studio/scripts/desktop-pack.cjs` — resolves the
  `electron-builder` JS entrypoint directly (`require.resolve('electron-builder/out/cli/cli.js')`)
  and invokes `process.execPath` with it. No `spawnSync('npm'/'npx')`,
  no shell shim that breaks on Windows `.cmd`.
- `studio/electron-builder.json` — `linux.desktop` is now an OBJECT
  (the official schema is `{Name, Comment, Categories, ...}`).
  Removed unused `protocols` and `copyright` blocks. Backend binary
  via `extraResources` is preserved; static preload stays in `files`.

### Tests added or expanded

- `studio/test/AppBoot.dom.test.tsx` — real React + StrictMode +
  `happy-dom` test of `<App />`. Boots a connection from a mocked
  desktop bridge, asserts all three registry fetches fire, asserts the
  shell renders after StrictMode double-mount, asserts the retry button
  re-runs bootstrap and recovers.
- `studio/test/ChatPanel.dom.test.tsx` — real React test of
  `<ChatPanel />` against the actual store. Confirms Send is enabled
  for the local helper with no model, and confirms cancelling an
  in-flight request clears `chat.inFlight` and re-enables Send.

## Checks run (actual outputs)

- `npm run typecheck` — exits 0, no diagnostics.
- `npm run build` — bundle written to `dist/`:
  - `dist/assets/index-DGl_Wt46.js` (50 kB / 14 kB gzip)
  - `dist/assets/react-BCnKyWd2.js` (219 kB / 68 kB gzip)
  - `dist/assets/three-C822UgBH.js` (555 kB / 137 kB gzip)
  - `dist/assets/index-*.css` (12 kB)
- `npm test` — vitest 5.0.5, 7 files, 57 tests, all pass.
- `npm audit --omit=dev --audit-level=high` — `found 0 vulnerabilities`.
- `npm audit --audit-level=high` — 8 moderate, all in the
  `electron-builder → app-builder-lib → @electron/get → global-agent →
  roarr → sprintf-js` chain. No criticals, no highs. The latest
  electron-builder 26.x line does not yet have a release that pins a
  patched sprintf-js; force-installing incompatible ranges is unsafe,
  so these remain documented.
- `node --check` on every CJS file (`main.cjs`, `preload.cjs`,
  `desktop.cjs`, `desktop-pack.cjs`) — syntax OK.
- `electron-builder.json` validates as JSON (comments stripped):
  `appId=com.kala-agent.studio`, `linux.target=['AppImage']`,
  `win.target=[nsis x64]`, `linux.desktop` is an object with
  `Name="Kala Studio"`.
- `vite preview` and `vite dev` both serve the bundled UI with HTTP 200
  on their loopback ports (`4173` and `5173` respectively). They do not
  conflict with the conductor's `localhost:8765` backend.

## Honest limitations / known blockers

- **Backend not yet running in CI for this cell.** The desktop launcher's
  smoke flow starts the backend in dev mode, but no live backend is
  bundled with this wave. Real wiring against `http://127.0.0.1:8765`
  is exercised by the `AppBoot.dom.test.tsx` and `ChatPanel.dom.test.tsx`
  tests via mocked bridges that return the same shape; the actual desktop
  launch needs the Python backend to be importable from the repo, which
  is a backend cell concern.
- **electron-builder moderate advisories.** Eight moderate findings
  remain in the dev dependency tree (sprintf-js via app-builder-lib).
  There is no published electron-builder release that pins a fixed
  version of the transitive `sprintf-js`, and forcing incompatible
  overrides would break packaging on the supported Electron and
  Node lines. The production-only audit is clean. Documented in this
  report and in `studio/README.md` so the conductor can decide whether
  to accept or to ask the delivery cell to ship a vendored override.
- **WebGL fallback is a static message.** When WebGL is unavailable
  (sandboxed iframes, headless agents, restricted drivers) the
  viewport shows a status banner instead of a 3D mesh. Inspector,
  export, and chat panels still work; the visualising path is the only
  thing disabled.
- **Static desktop entry still depends on the packaged backend binary
  landing at `studio/backend/kala-studio[.exe]`.** Until the delivery
  cell produces it, `npm run desktop:pack` refuses to run, by design —
  it never falls back to an arbitrary Python in production.

## What was *not* redone in this pass

- Geometry modules (`localPrimitives.ts`) and the `localPrimitives.test.ts`
  are unchanged from the previous pass; the conductor confirmed box
  and cylinder are correct and the brief said "Do not spend time
  rewriting already-passing geometry from scratch." The sphere has
  backend-side issues flagged in `REVIEW-NOTES.md` that the backend
  cell owns.
- `pyproject.toml`, `README.md`, `docs/**`, `packaging/**`,
  `.github/workflows/**`, `studio/backend/**` — not touched; the
  delivery cell owns those.
- Lockfile-driven dependency upgrades were scoped to vitest and the
  optional transitive chain; no platform-specific `@rollup/...` package
  is in `package.json` (verified by `grep`), and the lockfile resolves
  the optional native binaries via `optionalDependencies` so Windows
  and Linux builds pick the right platform-specific package without
  pinning it in our `dependencies`.