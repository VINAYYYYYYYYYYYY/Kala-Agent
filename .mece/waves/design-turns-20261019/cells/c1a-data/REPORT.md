# C1a — frontend data layer for design turns (no components, no CSS)

Cell: `c1a-data`. Spec: ADR-0009 §7 and §6, the B1 API contract
(`cells/b1-api/REPORT.md`), and the C1a task.
Status: **complete** for the owned surface. Gaps and honest notes are at the end.
No commit, no push, no delegation, and no credential reads were performed.

## Files

Created (owned by this cell):

| File | Lines | Purpose |
|---|---:|---|
| `studio/src/api/validators.ts` | 486 | runtime shape validation of every design-turn response; `StudioValidationError` |
| `studio/src/state/designs.ts` | 268 | the design slice: bounded LRU, drawing/section caches, the viewer-mesh conversion and its memo |
| `studio/src/state/journal.ts` | 351 | the session-journal slice: stored events, dedupe, chain verdict, the merged ledger |
| `studio/src/state/journalPoller.ts` | 250 | the poller controller (cadence, backoff, hidden-tab pause, verify) |
| `studio/src/state/designController.ts` | 112 | loads a drawing or a section into the caches |
| `studio/test/designFixtures.ts` | 280 | the B1 recorded payloads the tests are written against |
| `studio/test/designApi.test.ts` | 473 | validators and client methods (42 tests) |
| `studio/test/designStore.test.ts` | 358 | the LRU, the selectors, the mesh conversion, the merge, the chain verdict (22 tests) |
| `studio/test/designController.test.ts` | 100 | the drawing/section loader (5 tests) |
| `studio/test/journalPoller.test.ts` | 328 | the poller (16 tests) |

Edited (owned by this cell):

| File | Change |
|---|---|
| `studio/src/types/studio.ts` | the design-turn types (request and response), `ChatResponse.design?`, `HealthResponse.design_available?`, `DesignExportArtifact` |
| `studio/src/api/client.ts` | `postDesign`, `getDesignDrawing`, `getDesignSection`, `getJournal`, `getJournalVerify`, `exportDesign`; bounded timeouts; response validation; the chat → design dispatch; the health capability probe |
| `studio/src/state/store.ts` | the `designs` and `sessionJournal` slices in `UiState`, their actions and selectors, the design action creators |
| `studio/src/App.tsx` | **wiring only**: the journal poller is created and stopped where the preview controller is (this file is the only file outside my slice I touched) |

Not touched: `studio/src/components/**`, `studio/src/styles/**`,
`studio/src/viewer/**`, `AppShell`, `BootGate`, the backend, and every
`.mece/**` file except this report.

## Types (`studio/src/types/studio.ts`)

Request:

```ts
type DesignTemplateCall =
  | LbracketTemplateCall            // { template: 'l_bracket', width, depth, height, wall, bore_d?, hole_d?, hole_count?: 0|2|4, edge? }
  | SteppedShaftTemplateCall        // { template: 'stepped_shaft', segments: [{ diameter, length }] }
  | HousingTemplateCall;            // { template: 'housing_envelope', width, depth, height, wall }

interface DesignIntent {
  template?: DesignTemplateCall;    // exactly one of template / script
  script?: ConstructionScriptRequest;
  material?: string | null;         // DECLARED
  tolerances?: ReadonlyArray<string>; // DECLARED
  label?: string | null;
}
```

Response (exactly the B1 shapes): `DesignResult`, `DesignMeasured`,
`DesignBounds`, `DesignCheck` (`expected`/`measured` are
`boolean | number | [number, number]`), `DesignMesh`, `DesignArtifacts`,
`DrawingResponse` / `DrawingView` / `DrawingPolyline` (`visible` + `hidden`),
`SectionResponse` / `SectionLoop` (`area_exact`, `outer`, `bounds`,
`centroid`), `JournalPage` / `JournalEvent`, `JournalVerify`
(`ok`, `event_count`, `head_digest`, `first_bad_sequence`),
`DesignExportArtifact`.

Constants: `DESIGN_VIEW_NAMES`, `SECTION_PLANES`, `DESIGN_LRU_CAP` (16),
`JOURNAL_PAGE_LIMIT` (200), `SESSION_JOURNAL_CAP` (500).

`ChatResponse` gained `design?: DesignResult | null`; `HealthResponse` gained
`design_available?: boolean`.

## Client methods (`studio/src/api/client.ts`)

| Method | Route | Notes |
|---|---|---|
| `postDesign(connection, intent, signal?)` | `POST /api/studio/design` | validates the `DesignResult` |
| `getDesignDrawing(connection, designId, views?, signal?)` | `GET /api/studio/design/{id}/drawing?views=…` | `views` omitted ⇒ all four |
| `getDesignSection(connection, designId, plane, offsetMm?, signal?)` | `GET /api/studio/design/{id}/section?plane=XZ&offset=4` | plane allow-listed `XY\|XZ\|YZ` |
| `getJournal(connection, after = 0, limit = 200, signal?)` | `GET /api/studio/journal?after=…&limit=…` | the limit is clamped to the backend bound of 500 |
| `getJournalVerify(connection, signal?)` | `GET /api/studio/journal/verify` | the only source of the chain verdict |
| `exportDesign(connection, designId, format, signal?)` | `POST /api/studio/export` | body `{design_id, format}`; returns the bytes, the filename, and the `x-studio-digest` |

Conventions kept from the existing client:

- every call takes an `AbortSignal` and every call has its **own** budget
  (`DESIGN_COMPILE_TIMEOUT_MS` 60 s, `DESIGN_READ_TIMEOUT_MS` 20 s,
  `DESIGN_EXPORT_TIMEOUT_MS` 60 s, `JOURNAL_READ_TIMEOUT_MS` 10 s). An expired
  budget raises `StudioTimeoutError` (new) instead of hanging the UI;
- redirects are refused, the bearer is never logged, the token is never
  persisted;
- the new GET routes send the bearer (the backend guards them). The older
  public GET routes keep their old behaviour — `getHealth` and `getSettings`
  still send **no** `Authorization` header;
- a design id must be a 64-character lowercase hex sha256 before it can
  become a path segment, so a hostile id cannot traverse the path;
- **every response is validated before it is returned.** A payload that does
  not match the frozen contract raises `StudioValidationError` (with the
  dotted path of the offending field). A missing number is **never** coerced
  to `0` — a zero on the drafting sheet is a fabricated measurement.

`StudioValidationError` and the `parse*` functions live in
`studio/src/api/validators.ts` and are re-exported from the client.

## The store

### Fields added to `UiState`

```ts
state.designs: DesignState
  designs: ReadonlyMap<string, DesignResult>   // design_id -> design, LRU, max 16
  order: ReadonlyArray<string>                 // LRU order, least recently used first
  activeDesignId: string | null
  drawings: ReadonlyMap<string, DesignResource<DrawingResponse>>
  sections: ReadonlyMap<string, DesignResource<SectionResponse>>
  lastError: string | null

state.sessionJournal: SessionJournalState
  events: ReadonlyArray<SessionJournalEvent>   // the backend's stored events, deduped by sequence
  lastSequence: number                         // the next poll asks after this
  hasMore: boolean
  chainStatus: 'unknown' | 'verified' | 'broken'
  firstBadSequence: number | null
  headDigest: string | null
  eventCount: number | null
  lastError: string | null

state.server.designAvailable: boolean | null   // null = the backend did not say
```

`DesignResource<T>` is `{ status: 'idle' | 'loading' | 'ready' | 'error';
data: T | null; error: string | null }`. The task listed "idle/loading/error";
`'ready'` was added so a successful load is distinguishable from "never
requested" without inspecting `data`.

`SessionJournalEvent` is `{ sequence, eventType, aggregateId, occurredAt, ts,
summary, digest, eventDigest, previousEventDigest, payload }`.

### Selectors (all in `studio/src/state/store.ts`)

| Selector | Returns |
|---|---|
| `selectDesigns(state)` | the whole `DesignState` |
| `selectActiveDesign(state)` | the active `DesignResult`, or `null` |
| `selectDisplayMeshSource(state)` | `'design' \| 'server' \| 'local' \| 'none'` |
| `selectSessionJournal(state)` | the whole `SessionJournalState` |
| `selectChainStatus(state)` | `'unknown' \| 'verified' \| 'broken'` |
| `selectDrawing(state, key)` | `DesignResource<DrawingResponse> \| null` |
| `selectSection(state, key)` | `DesignResource<SectionResponse> \| null` |
| `selectMergedJournal(state)` | the merged ledger (see below) |

`selectMergedJournal` is **memoised**: `useSyncExternalStore` needs a
referentially stable snapshot, and a selector that built a new array on
every call would re-render forever. Every other selector returns a stable
reference.

`MergedJournalEntry` is
`{ id, ts, source: 'local' | 'server', kind, message, digest?, sequence?,
eventType?, chain }`. The merged list is ordered by time (oldest first);
rows with the same timestamp put the backend row first, because it is the
authoritative ledger row. A local row never carries a sequence or a chain
verdict of its own.

### Actions

Action creators (from `state/store.ts`):

```ts
designReceived(design: DesignResult): void   // stores the design AND activates it
activateDesign(designId: string | null): void // an unknown id is refused
clearActiveDesign(): void                    // the viewport falls back to the preview
```

Raw actions, for a caller that prefers `dispatch`:

```ts
{ type: 'design/received'; design: DesignResult }
{ type: 'design/activate'; designId: string | null }
{ type: 'design/clear-active' }
{ type: 'design/drawing-start' | 'design/drawing-success' | 'design/drawing-error'; key; … }
{ type: 'design/section-start'  | 'design/section-success'  | 'design/section-error';  key; … }
{ type: 'journal/server-page'; page: JournalPage }
{ type: 'journal/server-verify'; verify: JournalVerify }
{ type: 'journal/server-error'; message: string }
{ type: 'server/capabilities'; designAvailable: boolean | null }
```

Cache keys (from `state/designs.ts`, re-exported by `state/store.ts`):

```ts
drawingCacheKey(designId, views)   // `${designId}|front,iso`  (or `|all` for no views)
sectionCacheKey(designId, plane, offsetMm?)  // `${designId}|XZ|4`  (or `|default`)
```

### The viewer mesh

```ts
designToDisplayMesh(design: DesignResult): DesignDisplayMesh
// { designId, positions: Float32Array, normals: Float32Array, indices: Uint32Array,
//   bounds: BBox, vertexCount, triangleCount, engine, warnings, source: 'design' }
```

Pure: the same design always yields the same mesh. The bounds are the
kernel-**measured** envelope (`measured.bounds`), not the mesh extents.
`createDesignMeshMemo(cap = 16)` memoises it by `design_id` with a bounded
LRU, so a long session cannot leak a typed array per design.

### The journal merge and the chain verdict

- The backend's events live in `state.sessionJournal.events`, **separate**
  from the UI-local ledger in `state.journal`. A local row is not a chain
  event and is never presented as one.
- Server events are **deduped by sequence**: a page that repeats an event
  the store already holds does not double the ledger.
- `chainStatus` is `'unknown'` until a `/journal/verify` response arrives.
  It becomes `'verified'` **only** from a verify response with `ok: true`,
  and `'broken'` (with `first_bad_sequence`) from one with `ok: false`.
  A page, a poll failure, and a local row never change the verdict. A
  failed verify leaves the previous verdict alone and records the error.
- `summarisePayload(eventType, payload)` builds the one-line summary shown
  in the ledger. Every number in it is read from the payload itself
  (`design.measured` ⇒ `volume 35571.416 mm^3 · area 11889.102 mm^2 · 1
  solid`). An unknown event type falls back to at most three scalar payload
  values.

## The journal poller (`studio/src/state/journalPoller.ts`)

```ts
createJournalPoller(options): JournalPoller
// options: { connection, getAfter(), isBusy(), getLastDesignId(), isHidden?(), limit? }
// poller:  { start(), stop(), isRunning(), __test? }
```

- **1 s** while a chat or design request is pending (`isBusy()`),
  **5 s** while the studio is idle;
- **paused** while the document is hidden (`document.hidden`, or the
  injected `isHidden()`); the poller keeps its place and resumes as soon as
  the tab is visible again;
- **exponential backoff** on a failed poll: 1 s, 2 s, 4 s, 8 s, 16 s, then
  capped at **30 s** (`backoffDelay(n)`). A successful poll returns it to
  the normal cadence;
- `after` is the last sequence the store has (`getAfter()`), the page size
  is bounded (200, the backend's own bound is 500);
- when a page says `has_more`, the next page is read straight away instead
  of waiting for the cadence (capped at 20 back-to-back pages so a broken
  backend cannot turn the poller into a tight loop);
- the chain is **verified after every new design** and **every 30 idle
  seconds**;
- `stop()` clears the timer and aborts the request in flight; `start()` after
  a `stop()` builds a fresh loop, so React StrictMode's double mount cannot
  leave two pollers running.

### Wiring

`studio/src/App.tsx` creates the poller in an effect (not a memo) next to the
preview controller, starts it when the connection is ready, and stops it in
the cleanup. The poller only runs when the backend reports the design-turn
feature (`state.server.designAvailable !== null`); see the note below.

## The chat → design dispatch

`postChat` now validates the reply and, when the reply carries a design,
dispatches `design/received` — the design is stored and becomes the active
one. `postDesign` does the same with the design it compiled. The chat
thread itself is untouched: the panel still renders exactly the reply text,
the provider badge, and the measured reply time.

The dispatch lives in the client because the client is the only layer that
sees the raw reply, and `studio/src/components/chat/**` is owned by the
component cell. The store stays the single source of truth: the components
read `selectActiveDesign` and never parse a response. A caller that
already holds a `DesignResult` (a replayed turn, a test fixture) can
dispatch it directly with the `designReceived(design)` action creator.

## Tests

| File | Tests | What it covers |
|---|---:|---|
| `test/designApi.test.ts` | 42 | every validator with good and bad payloads (including the B1 samples and the missing-number cases), the six client methods, the URL/header/bearer contract, the timeout, and the chat → design dispatch |
| `test/designStore.test.ts` | 22 | the LRU and its eviction, the activation rules, `selectDisplayMeshSource`, the mesh conversion and its memo, the cache statuses, the merge, the dedupe, the chain transitions (verified / broken / never-verified) |
| `test/designController.test.ts` | 5 | the drawing/section loader, the warm cache, the error state, the disposal |
| `test/journalPoller.test.ts` | 16 | the cadence (busy and idle), the backoff and its 30 s cap, the hidden-tab pause, the drain, the verify cadence, the stop/restart (StrictMode) safety, the abort on stop |

The fixtures in `test/designFixtures.ts` are the payloads the backend cell
recorded from a real criterion-2 run: the 60 × 45 × 40 mm L-bracket, volume
35 571.416 047 mm³, its HLR front view, its criterion-4 section cut
(2 321.460 183 660 mm², one outer loop, one inner Ø10 loop), the first
journal page, and both verification verdicts (intact and tampered).

## Gates

Run in `studio/` (Node 22, vitest 5, TypeScript 5.7 strict with
`exactOptionalPropertyTypes` and `noUncheckedIndexedAccess`):

| Gate | Command | Result |
|---|---|---|
| Types | `npm run typecheck` | **clean** (no errors) |
| Tests | `npm test` | **434 passed** in 2.5 s (33 files) |
| Tests (this cell only) | `npx vitest run test/designApi.test.ts test/designStore.test.ts test/designController.test.ts test/journalPoller.test.ts` | **85 passed** (42 + 22 + 5 + 16) |
| Build | `npm run build` | **built in 484 ms** |

The 434 = the 348 in the pre-existing files + the 85 new ones in this
cell's four test files. (The `test/design*.test.ts` glob also matches the
component cell's `test/designSystem.test.ts`; the row above names the four
files explicitly.) The build's "chunks are larger than 500 kB" warning is
the pre-existing `three.js` bundle, not this cell.

## Gaps and honest notes

1. **The poller only runs against a backend that advertises the feature.**
   `getHealth` now records the backend's `design_available` flag in
   `state.server.designAvailable`, and the poller starts only once that
   flag is present. An older backend has no `/journal` route at all, and
   polling it would fill the ledger with 404s. Consequence: a B1 backend
   *without* the OCP bindings (`design_available: false`) still has the
   journal routes but is not polled. The flag is the only signal the
   frontend has, and the backend owns the semantics of it.
2. **The API layer dispatches into the store.** `postChat` and `postDesign`
   dispatch `design/received`, and `getHealth` dispatches
   `server/capabilities`. This is a deliberate layering choice, not an
   accident: the component cell owns `components/**`, and the client is the
   only layer that sees the raw reply. The dependency is
   `api → state/store`; `state/store` never imports `api/client`, so there
   is no cycle.
3. **The cache status has a fourth value.** The task listed
   `idle/loading/error`; the slice uses `idle | loading | ready | error`, so
   "loaded" is distinguishable from "never requested" without reading
   `data`.
4. **A failed poll does not clear the chain verdict, and a successful
   verify does not clear the poll error.** The two are separate facts: the
   verdict comes from `/journal/verify` and nothing else, and the poll
   error is cleared by the next successful page.
5. **The merged ledger is ordered oldest-first.** C1b's "newest-first"
   toggle is a view concern: reverse `selectMergedJournal(state)`.
6. **The section and drawing caches are keyed by parameters, not by
   design.** Two different views of one design are two entries; the
   `drawingCacheKey` / `sectionCacheKey` helpers are the only way to build
   a key.
7. **`designToDisplayMesh` copies the arrays.** The memo means the copy is
   made once per design, not once per render.
8. **The mesh in the B1 samples is abbreviated.** The recorded sample
   carries `"..."` inside the mesh arrays; the fixture uses a small valid
   triangle mesh with the real measured numbers, because a validator cannot
   read an ellipsis.
9. **No live backend was called.** Every test stubs `fetch`; the shapes are
   the ones the backend cell recorded, not a round trip against a running
   studio.
