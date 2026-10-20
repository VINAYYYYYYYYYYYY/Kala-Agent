# C2 provider UI — CAD target selection, provider-labelled measurements, B2 view contract

Cell: `c2-provider-ui`, wave `design-turns-20261019`.
Specs read: `TASK.md` (including the Opus addendum), ADR-0011, ADR-0009,
`../b2-providers/REPORT.md`, `../c1a-data/REPORT.md`, and `../d1-qa/REPORT.md`.

Status: **complete** for `studio/src/**` and `studio/test/**`. No commit, push,
deploy, delegation, or credential read was performed. The selector and the
provider labels were unit/component tested; no live backend was called and no
screenshots were taken (final QA owns them).

---

## 1. What changed

### Data layer (the B2 contract)

| File | Change |
|---|---|
| `studio/src/types/studio.ts` | `CadProviderId` (`opencascade \| freecad \| solidedge`), `CAD_PROVIDER_IDS`, `CadProviderRecord`, `CadProvidersResponse`; `ChatRequest.cad_provider?: CadProviderId \| null`; `DesignRequest = DesignIntent + required provider`; `DesignMeasured` nullable per field plus `mass_g`; `DesignCheck.measured: CheckValue \| null` and `status: passed \| failed \| not_reported`; `DesignMesh` → `DesignView` (adds `bounds`, `solids`, `warnings`); `DesignResult` gains `provider`, `provider_version`, `measured_by`, and `view` replaces `mesh`; `DesignArtifacts` gains `native_sha256` / `native_kind`. |
| `studio/src/api/validators.ts` | `parseCadProviders` (closed id set, duplicate ids refused, strict null/undefined distinction); `requireCadProviderId`; nullable measured fields parsed as `null`, never coerced to 0; `checks[].status` required and cross-checked against `passed` and `measured`; `view` required, with imported `bounds`/`solids`/`warnings`; `step_sha256`, `native_sha256`, `native_kind` validated. A B1 payload that still carries `mesh` and no `view` is refused at `design.view`. |
| `studio/src/api/client.ts` | `getCadProviders` → `GET /api/studio/providers/cad` with the bearer, bounded timeout, validated payload; `postDesign` now takes `DesignRequest`, validates `provider` before the network call, and sends it; `ChatRequest` carries `cad_provider` unchanged to `/chat`. |
| `studio/src/state/designs.ts` | `designToDisplayMesh` reads `view` (positions/normals/indices/counts/engine/bounds/solids/warnings). The display bounds are the imported B-Rep box, and the provider's `measured.bounds` may be null without breaking the viewport. |
| `studio/src/state/store.ts` | `state.cadProvider` (session-only, initial `null`), `server.cadProviders` / `cadProvidersStatus` / `cadProvidersError`, `cad/provider-set`, `server/cad-providers*`, `selectCadProvider`, `selectCadProviderRecord`, `selectCadProviders`, `setCadProvider`; `selectDisplayMeshSource` reads `view`. |
| `studio/src/state/journal.ts` | summaries now name the provider (`design.intent`, `design.measured`, `provider.build.*`, `view.imported`, `chat.turn` adds `cad_provider`), and `not_reported` checks are neither counted as passed nor failed. `provider.*` / `view.*` are classified as tool rows. |

### Components

| File | Change |
|---|---|
| `studio/src/components/chat/CadTargetSelector.tsx` (new) | Composer CAD target. Rows come from `GET /providers/cad`; **no default**; unavailable rows are disabled with the backend reason in the row and button tooltips; OpenCascade is always labelled opt-in; the choice is written to the in-memory store only; probe errors show the backend code/details and offer Retry. |
| `studio/src/components/chat/ChatPanel.tsx` | Renders the selector in the composer, sends `cad_provider` (or `null` when unchosen), includes the CAD target in the tool journal row, quotes provider-measured context with `—` for unreported values, and renders the reason + available ids of a 503 `provider_unavailable`. |
| `studio/src/components/chat/DesignAgentCard.tsx` | Provider/version line and caption (`measured by FreeCAD 1.1.4`); `B-Rep Verified` only when `verified`; check marks and `data-status` for passed/failed/not_reported (`—` mark); `measured —` for a null value; STEP and native digest chips; design/view warnings. |
| `studio/src/components/chat/designCard.ts` | `measuredByLabel`, nullable `formatMm`/`envelopeSize`, provider-reported mass, native chip label (`FCStd`), `checkSummary` with failed/not-reported counts, provider-aware tooltips. |
| `studio/src/components/bottombar/BottomBar.tsx` | Source is `measured by <measured_by> <version>` (`data-source-kind="provider"`, `data-provider`), nullable ENV/VOL/AREA render `—`. |
| `studio/src/components/drafting/DraftingSheet.tsx` | Title block gains `MEASURED BY`; cut note uses the always-present imported `view.bounds`; passes the design provider to the viewport label. |
| `studio/src/components/drafting/AuxView.tsx` | Ortho footer names the imported view; tolerance panel/footer show the measuring provider. |
| `studio/src/components/export/ExportPanel.tsx` | Active-design block shows the provider/version and the script, STEP, and native artefact digests; export errors include API details. |
| `studio/src/viewer/SceneCanvas.tsx` + `viewer/designSource.ts` (new) | Design view label is `view: OpenCascade import (view-only)`, or `view: OpenCascade (opt-in) build` when the provider is `opencascade`; the helper is shared with the HLR sheet. |
| `studio/src/viewer/HlrDrawing.tsx` + `viewer/hlr.ts` | HLR heading carries the same view label; a null measured envelope draws no callouts rather than an invented size. |
| `studio/src/components/journal/ledgerRows.ts` | `provider.*` / `view.*` ledger rows are tool rows. |
| `studio/src/styles/*.css` | Selector, provider line, warnings, export artefact chips, HLR view label, and the provider-source bottom-bar state. |

### Tests

| File | Coverage |
|---|---|
| `studio/test/CadTargetSelector.dom.test.tsx` (new, 7 tests) | no default; three live rows/versions; unavailable disabled + reason tooltip; click records in the session store; choice survives remount; no probe without design-turn support; failed probe + retry. |
| `studio/test/designApi.test.ts` | B2 accept; nullable measured → `null` not 0; B1 `mesh` refused at `design.view`; provider/measured_by required; unknown provider; missing measured key; view bounds/solids; status/measured/passed contradictions; missing status; native digest; provider probe client (bearer + validation); `postDesign` sends provider and refuses missing/unknown before the network; `cad_provider` sent as chosen or `null`. |
| `studio/test/designStore.test.ts` | `designToDisplayMesh` reads `view` (including differing view/measured bounds and a not-reported design); `view.solids`; CAD selection initial null, reducer, unknown id, clear, non-persistence, probe status; provider-labelled journal summaries and not-reported check counting. |
| `studio/test/DesignAgentCard.dom.test.tsx` | provider line/caption; `—` for every unreported measured field; not-reported check mark/status; verified stamp absent when unverified; script/STEP/FCStd chips; provider context row. |
| `studio/test/BottomBarDesignReadout.dom.test.tsx` | `measured by FreeCAD 1.1.4`, `data-provider`, and `—` for unreported values. |
| `studio/test/DraftingSheet.dom.test.tsx` | clay viewport shows `view: OpenCascade import (view-only)`; opt-in OpenCascade variant; title-block `MEASURED BY`; cut note from view bounds with null measured bounds. |
| `studio/test/HlrDrawing.dom.test.tsx`, `test/hlr.test.ts` | drawing view label; no callouts when the provider reported no envelope; `measuredEnvelope` null. |
| `studio/test/AuxView.dom.test.tsx` | `SOURCE: OCP HLR · VIEW: IMPORT`; tolerance footer names the provider. |
| `studio/test/designExport.dom.test.tsx` | active-design provider line and script/STEP/FCStd digests. |
| `studio/test/ChatPanel.dom.test.tsx` | sends the selected `cad_provider`, sends `null` when none, renders a 503 `provider_unavailable` reason and available ids. |
| `studio/test/AppJournalPollerGate.dom.test.tsx` | new `getCadProviders` mocked for the `design_available: true` poller case. |
| `studio/test/designFixtures.ts` | replaced the B1 sample with the B2 FreeCAD 1.1.4 sample (`design_id` ≠ `script_sha256`), added the synthetic not-reported design and the three-record probe response. |

`studio/test/nativeBackendPackaging.test.ts` and `studio/scripts/**` were **not
touched**.

---

## 2. D1 QA gaps

| D1 gap | Status after this cell |
|---|---|
| 2. Provider selector did not exist; QA shimmed `cad_provider` | **Fixed.** Selector sends `cad_provider`; no page-level shim is needed. |
| 3. `DesignResult.mesh` vs `view`; C1a validator expected `mesh` | **Fixed.** Validator, store, and every consuming component use `view`; the B1 shape is refused. |
| 1. Native readiness suite red from the parallel Rust refactor | **Green at final run without this cell touching it.** The parallel owner updated `studio/scripts/native-readiness.cjs` and `studio/test/nativeBackendPackaging.test.ts` at 01:36. This cell only observed the result. |
| 4. XZ section offset fixed at 4 mm | Unchanged (the ADR criterion-4 cut); not a provider-UI issue. |
| 5. Card 02 body still scrolls with a receipt | Unchanged; the C0b card pattern. The new provider/digest rows are inside the existing active-design block; the receipt's `max-height` bound was not changed. |
| 6. 1280 journal table gutter 256/249 with `overflow-x: hidden` | Unchanged; no horizontal bar. |
| 7. No remote provider called, no credential read | Still true for this cell. |

---

## 3. Gates (run in `studio/`)

| Command | Result |
|---|---|
| `npm run typecheck` | **clean**, exit 0 |
| `npm test` | **42 files, 592 tests, 592 passed** (2.7–3.1 s). All green, including `nativeBackendPackaging`. |
| `npm run build` | **green** (`built in ~0.4–0.6 s`). Only the pre-existing `three.js` chunk warning: `three-BnHQmCIC.js` 559.64 kB / 138.50 kB gzip. |

Baseline at the start of this cell was 41 files / 541 tests with 7 failures, all
in `nativeBackendPackaging.test.ts`, caused by the Rust route-table refactor
recorded by D1. The parallel owner of that file/script fixed it mid-cell; the
final run above is fully green. No gate was skipped or filtered in the final
run.

---

## 4. Behaviour notes (honest)

1. **The probe is gated on `design_available === true`.** `GET /providers/cad`
   is part of the design-turn surface. An older backend (flag absent) and a
   backend without OCP (`false`) have no usable view importer, so the composer
   shows the honest capability note instead of probing a route that may not
   exist. When the flag is true, the selector calls the route with the bearer
   and the retry control exists for a transient failure.
2. **No default is deliberate.** `state.cadProvider` starts null, is never
   written to localStorage, and is cleared only by a full store reset. With no
   choice the chat request carries `cad_provider: null`; the backend replies
   with its provider-choice prompt and builds nothing. The selector shows
   "No CAD target selected — the first design turn will ask you to choose."
3. **Unavailable rows are evidence, not guesses.** The row is disabled, shows
   `unavailable`, and carries the backend `reason` in both the button and
   wrapper `title` (some browsers do not show a tooltip on a disabled button).
4. **`measured_by` is the source of truth for the measurer.** The UI never
   labels OpenCascade as the measurer of a FreeCAD/Solid Edge part. The viewer
   label is a separate statement: it always names the OpenCascade import of the
   provider's STEP, and only says "opt-in build" when `provider === 'opencascade'`.
5. **`not_reported` is not a failure.** A null measured property prints `—`; a
   `not_reported` check prints a `—` mark and `measured —`, and the stamp says
   "not reported". `B-Rep Verified` is shown only when `verified === true`.
6. **The mass row** uses the provider's `measured.mass_g` when present; the old
   declared-density derivation remains only when a backend declares a density
   and reports a volume. No density is inferred from a material name.
7. **`checks[].status` is required by the validator** and must agree with
   `passed` and the nullability of `measured`. This is stricter than a client
   that ignores `status`; it makes a contradictory payload impossible to
   render as a pass.
8. **The B1 sample was replaced.** `bracketDesignResult` is now the B2 FreeCAD
   1.1.4 sample from `../b2-providers/REPORT.md`; its `design_id` deliberately
   differs from `script_sha256`, and the view arrays are shortened (a real view
   has hundreds of vertices) while every measured number is the recorded one.
9. **No live round trip.** All API tests stub `fetch`; the selector tests mock
   `getCadProviders`. The exact live payloads are the ones B2 recorded, not a
   fresh run against the Python backend.
10. **No screenshot / visual check.** The task explicitly leaves screenshots to
    final QA; only component and CSS assertions were run.

---

## 5. Files changed by this cell

`studio/src/types/studio.ts`, `studio/src/api/validators.ts`,
`studio/src/api/client.ts`, `studio/src/state/designs.ts`,
`studio/src/state/store.ts`, `studio/src/state/journal.ts`,
`studio/src/components/chat/CadTargetSelector.tsx` (new),
`studio/src/components/chat/ChatPanel.tsx`,
`studio/src/components/chat/DesignAgentCard.tsx`,
`studio/src/components/chat/designCard.ts`,
`studio/src/components/bottombar/BottomBar.tsx`,
`studio/src/components/drafting/DraftingSheet.tsx`,
`studio/src/components/drafting/AuxView.tsx`,
`studio/src/components/export/ExportPanel.tsx`,
`studio/src/components/journal/ledgerRows.ts`,
`studio/src/viewer/SceneCanvas.tsx`, `studio/src/viewer/HlrDrawing.tsx`,
`studio/src/viewer/hlr.ts`, `studio/src/viewer/designSource.ts` (new),
`studio/src/styles/chat.css`, `studio/src/styles/studio.css`,
`studio/src/styles/drafting.css`, `studio/src/styles/bottombar.css`, and the
tests listed in §1.

No file outside `studio/src/**` and `studio/test/**` was edited.
