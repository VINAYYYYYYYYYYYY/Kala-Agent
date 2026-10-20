# C1a: frontend data layer for design turns (no components, no CSS)

Read `../../WAVE.md`, ADR-0009 §7, and `../b1-api/REPORT.md` (exact JSON shapes and samples; this is the contract). Another worker (C0b) is restyling components and CSS at the same time. You own ONLY:
`studio/src/api/**`, `studio/src/types/studio.ts`, `studio/src/state/**` (store and controllers), new `studio/src/state/designs.ts` and `studio/src/state/journalPoller.ts` if useful, plus new test files `studio/test/design*.test.ts` and `studio/test/journalPoller*.test.ts`. Do NOT edit `components/**`, `styles/**`, `viewer/**`, or AppShell. Keep all existing tests green.

## Build
1. **Types** for DesignIntent requests, DesignResult, Drawing (views → visible/hidden polylines), Section (loops with `area_exact`), JournalPage, JournalVerify, ChatResponse `design?`, and Health `design_available`. Follow the B1 shapes exactly.
2. **Client methods** following the existing client conventions (bounded fetch, timeout, abort, runtime shape validation of responses, typed errors): `postDesign`, `getDesignDrawing(id, views)`, `getDesignSection(id, plane, offset)`, `getJournal(after, limit)`, `getJournalVerify()`, `exportDesign(id, format)`. Reject responses that fail validation; never coerce missing numbers to 0.
3. **Store design slice**: a bounded map (16, LRU) of design_id → DesignResult, `activeDesignId`, and per-design caches for drawing/section requests keyed by params, with status (idle/loading/error). Actions: `designReceived` (from chat or `postDesign`, which also activates it), `activateDesign`, `clearActiveDesign`. Selectors: `selectActiveDesign`, and `selectDisplayMeshSource` returning `'design' | 'server' | 'local' | 'none'`. A pure function `designToDisplayMesh(design)` produces the viewer mesh (Float32Array, etc.), memoizable by design_id.
4. **Journal merge**: server events (seq, type, ts, payload summary, digest) are stored separately from UI-local entries and merged in a selector ordered by time. Server events dedupe by seq. `chainStatus` is `'verified' | 'broken' | 'unknown'` with `first_bad_sequence` and `head_digest` from `/journal/verify`. Never set `'verified'` without a successful verify response.
5. **Journal poller** controller (pattern like previewController): every 1 s while a chat/design request is pending, 5 s idle, paused while `document.hidden`, exponential backoff on errors (max 30 s), `after = last seq`, bounded limit. Verify after each new design and every 30 s idle. Clean cancel on stop/unmount, StrictMode-safe. Wire start/stop where previewController is wired (`App.tsx` is allowed for this minimal wiring only).
6. **Chat send path**: when a ChatResponse has `design`, dispatch `designReceived`. Keep the existing chat behaviour.

## Tests
Validators (good and bad payloads, including the B1 samples), LRU eviction, selectors, the design-to-mesh conversion, the journal merge and dedupe, chainStatus transitions (including a broken chain), poller cadence and backoff with fake timers, hidden-tab pause, and StrictMode-safe start/stop.

Gates: `npm run typecheck && npm test && npm run build`. If a failure is in C0b-owned files, re-run once before reacting. Write `REPORT.md` with the exported store fields, selectors, and actions that the component worker (C1b) will use.
