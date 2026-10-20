# C1b-ledger — the session journal ledger and the design export

Cell: `c1b-ledger`, wave `design-turns-20261019`.
Spec: `../c1b-components/TASK.md` items 4 and 5 (ADR-0009 §7) plus the
binding "backend without design support" addendum.
Status: **complete** for the owned surface. All three gates are green.
No commit, push, delegation, screenshot, or credential read was performed.

---

## 1. Files

### Created (owned by this cell)

| File | Lines | Purpose |
|---|---:|---|
| `studio/src/components/journal/ledgerRows.ts` | 154 | The pure ledger view model: merges the local activity and the backend's stored events into the six-column rows, decides the CHAIN cell, and shortens a digest |
| `studio/src/components/journal/CopyButton.tsx` | 106 | The shared copy control (`CopyButton`, `copyTextToClipboard`); reports a refused clipboard instead of pretending |
| `studio/src/components/journal/JournalPanel.tsx` | 367 | Rewritten: the six-column ledger, the honest chain stamp, the newest-first toggle, the digest copy, the degraded-mode note |
| `studio/src/styles/journal.css` | 426 | Rewritten: the ledger table, the toolbar, the copy control (shared with the export receipt) |
| `studio/test/journalLedger.dom.test.tsx` | 291 | 13 DOM tests: the columns, the local-row rules, the stamps, the order toggle, the digest copy, the degraded mode |
| `studio/test/designExport.dom.test.tsx` | 207 | 8 DOM tests: the design export, the receipt with the sha256, the copy, the failures, the degraded mode |

### Modified (owned by this cell)

| File | Change |
|---|---|
| `studio/src/components/export/ExportPanel.tsx` | The active-design export block: a STEP/STL export of the compiled design, the receipt (filename, bytes, sha256 + copy), the journal rows, and the honest note for a backend without design routes |
| `studio/src/styles/studio.css` | **Only the `.export-panel*` selectors**: the design block, the receipt, the note, the compact action row |

### Modified (shared file, minimal additive wiring)

| File | Change (re-read immediately before the patch) |
|---|---|
| `studio/src/components/shell/AppShell.tsx` | 1 import (`selectChainStatus`), 2 selectors (`chainStatus`, `firstBadSequence`), and the `JournalPanel` props. The shell used to hard-code `hashChainStatus="unknown"`, so the ledger could never show the real verdict. Nothing else was touched. |

No store change was needed: `selectActiveDesign`, `selectChainStatus`,
`state.sessionJournal.events`, `state.sessionJournal.firstBadSequence`, and
`state.server.designAvailable` already exist in the C1a store, and
`exportDesign` already exists in the C1a client.

---

## 2. The ledger (item 4)

The journal panel body is now a real table:

```
SEQ | UTC | TYPE | MESSAGE | DIGEST | CHAIN
```

* **SEQ** — the backend sequence of a stored event; a local UI row has none
  and shows `—`.
* **UTC** — the wall-clock time of the instant in UTC (`HH:MM:SS`), with the
  full ISO-8601 instant in the cell's `title` and in the `<time dateTime>`.
* **TYPE** — the backend event type (`design.intent`, `artifact.stored`, …)
  or the local tag (`[PREVIEW]`, `[CHAT]`, `[EXPORT]`, `[TOOL]`, `[BRIDGE]`,
  `[SYSTEM]`).
* **MESSAGE** — the event summary for a stored event, the UI message for a
  local row.
* **DIGEST** — the short digest (a sha256 is cut to its first eight
  characters; a short value such as `512 bytes` is shown as it is) with a
  copy button that copies the **full** digest. The full value also stays in
  the chip's `title`.
* **CHAIN** — `local` for a UI row, `ok` for an event the chain covers,
  `broken` for an event the chain does not verify, `—` while no verification
  has been reported.

### The stamp (header)

| Chain state | Stamp |
|---|---|
| `verified` | `HASH-CHAIN OK` (stamp green) |
| `broken` + a first bad sequence | `CHAIN BROKEN @ 4` (redline) |
| `broken` without a sequence | `HASH-CHAIN BROKEN` (redline) |
| `unknown` | `SESSION LOG` (neutral) |

`HASH-CHAIN OK` is printed **only** when the store's `chainStatus` is
`verified`, and the store only becomes `verified` from a
`/journal/verify` response with `ok: true` (C1a). A local row, a page, or a
poll failure never lights the stamp.

### The order toggle

A `NEWEST FIRST` toggle in the panel toolbar (`aria-pressed`) reverses the
ledger. The default order is oldest-first with the newest row at the bottom,
which is what the existing sticky autoscroll and the "Jump to latest ↓"
control expect; in newest-first order the newest row is at the top and the
autoscroll pins to the top instead.

### The rules that keep it honest

* A local row is never dressed up as a chain event: no sequence, no digest
  it did not earn, and the chain cell says `local`.
* When the chain is broken, the rows **before** the first bad sequence stay
  `ok` (they are still covered by an intact prefix) and the bad row and
  everything after it say `broken`.
* The merge keeps the local rows in their own order and puts a backend row
  first when both share a millisecond. This is deliberately one notch
  different from the store's `selectMergedJournal`, which breaks a tie on
  the row id; the ledger is a view and the local order is the view's business.
* The copy control reports the real outcome: `data-state` is `copied`,
  `failed`, or `idle`, and the accessible name says so. A denied clipboard
  never looks like a successful copy.

---

## 3. The design export (item 5)

The Options drawer card 02 keeps its primitive/handoff export and gains an
**active design** block when a design is active:

* the design label and its first eight id characters;
* a STEP/STL picker and one `Export design …` button, side by side so the
  card keeps its height budget;
* the receipt: the **real** filename, the **real** byte count, and the
  sha256 the backend reported in `x-studio-digest`, with a copy button. When
  the backend sent no digest header the receipt shows `—` and offers no
  copy button — a digest is never invented;
* the export is also a real journal row (`Saved <file>` with the byte count
  and the short digest), and a failed export is reported as an error.

The receipt belongs to the design it was written from: activating another
design drops the old receipt instead of showing it next to the wrong part.

---

## 4. Backend without design support

When the backend reports `design_available: false`:

* the ledger shows one honest line — *"Design journal unavailable — this
  backend reports design_available: false, so only local activity is
  recorded here."* — and keeps every local row;
* the chain stamp stays the neutral `SESSION LOG` (nothing verified a
  chain);
* the export panel shows *"Design export needs the Python geometry backend;
  this backend reports design_available: false."* and offers no design
  export at all;
* the primitive export, the handoff registry, and every other panel keep
  working — nothing was removed.

When the backend has not reported the flag at all (`null`, e.g. a health
probe that has not answered yet) the ledger shows no note: the studio does
not claim a capability it has not been told about.

---

## 5. Contracts added

```ts
// components/journal/ledgerRows.ts
export type LedgerChain = 'ok' | 'broken' | 'pending' | 'local';

export interface LedgerRow {
  id: string;
  source: 'local' | 'server';
  kind: JournalKind;
  type: string;          // the TYPE cell
  message: string;       // the MESSAGE cell
  ts: number;            // epoch ms
  sequence: number | null;
  digest: string | null;
  chain: LedgerChain;
}

export function buildLedgerRows(
  local: ReadonlyArray<JournalEntry>,
  server: ReadonlyArray<SessionJournalEvent>,
  chainStatus: HashChainStatus,
  firstBadSequence: number | null,
  newestFirst?: boolean,
): LedgerRow[];

export function chainFor(sequence, chainStatus, firstBadSequence): LedgerChain;
export function chainLabel(chain: LedgerChain): string;
export function chainTitle(chain: LedgerChain): string;
export function shortDigest(digest: string): string;
export function journalKindFor(eventType: string): JournalKind;

// components/journal/CopyButton.tsx
export type CopyState = 'idle' | 'copied' | 'failed';
export function copyTextToClipboard(text: string): Promise<boolean>;
export function CopyButton(props: { value: string; label: string; className?: string }): ReactElement;

// components/journal/JournalPanel.tsx
export interface JournalPanelProps {
  entries: ReadonlyArray<JournalEntry>;
  connectionStatus: ConnectionStatus;
  hashChainStatus?: HashChainStatus;      // optional now: the store's verdict is the fallback
  firstBadSequence?: number | null;       // optional now: the store's value is the fallback
}
```

`hashChainStatus` and `firstBadSequence` became **optional** and fall back to
the store, so the ledger can never show a stale hard-coded verdict. The
shell passes the store's real values explicitly.

The copy control lives in `components/journal/` and is used by the export
receipt as well; the shared base styles (`.studio-copy-button`) live in
`styles/journal.css` because the ledger introduced the affordance. No other
cell's file was edited for it.

---

## 6. Tests

| File | Tests | What it covers |
|---|---:|---|
| `test/journalLedger.dom.test.tsx` | 13 | the six columns; a local row (`—`, `local`, no digest); a stored event (sequence, event type, summary, short digest + full digest in the title, `ok`); the UTC instant; the three stamp states; the broken-chain row marking; the newest-first toggle; the digest copy and the refused clipboard; the degraded mode and the "not reported yet" case |
| `test/designExport.dom.test.tsx` | 8 | the design block and its label/id; the export call with the design id and the format; the receipt with filename, bytes, and sha256; the journal row; the sha256 copy; the format picker; a backend that reported no digest; a failed export; no design → no block; `design_available: false` → the honest note and no block |

The tests use the recorded B1 payloads from `test/designFixtures.ts`
(`bracketJournalPage`, `bracketJournalVerified`, `bracketJournalBroken`,
`bracketDesignResult`) — the real shapes the backend cell recorded, not
invented ones.

**No pre-existing test was edited or weakened.** `test/chatJournal.dom.test.tsx`
and `test/ExportPanel.dom.test.tsx` still pass untouched: the ledger keeps
the `.journal-entry`, `.journal-entry-tag`, `.journal-entry-message`, and
`.journal-entry-digest` class names and the `.journal-list` scroll
container, so the old coverage of the ledger rows, the empty state, the
stamp, the live region, and the sticky autoscroll still holds.

---

## 7. Gates

Run in `studio/` (Node 22, vitest 5, TypeScript 5.7 strict with
`exactOptionalPropertyTypes` and `noUncheckedIndexedAccess`):

| Gate | Command | Result |
|---|---|---|
| Types | `npm run typecheck` | **clean** (no output) |
| Tests | `npm test` | **39 files, 530 tests passed** (0 failed) |
| Build | `npm run build` | **green**, `✓ built in 486 ms` |

The 530 = 485 at the start of this cell's last run + the parallel workers'
new suites (`BottomBarDesignReadout`, `DesignAgentCard`, …) + this cell's
21 new tests. The build's "chunks are larger than 500 kB" warning is the
pre-existing `three.js` bundle, not this cell.

This cell's own suites: `npx vitest run test/journalLedger.dom.test.tsx
test/designExport.dom.test.tsx` → **21 passed** (13 + 8).

The type-scale guard (`test/designSystem.test.ts`) passes: no raw px
`font-size` outside `tokens.css`, no size below 9 px, no rounded control
corner. Every new style uses the C0b tokens (`--text-*`, `--space-*`,
`--hairline`, `--stamp`, `--redline*`, `--prussian`, `--sheet`, `--card`,
`--margin`, `--ink`, `--construction`).

**A parallel worker's file broke the build twice during this cell** (a
half-written `src/styles/drafting.css` and an `exactOptionalPropertyTypes`
error in `test/AuxView.dom.test.tsx`, both owned by `c1b-drawing`). Both
were re-run, not edited; the build is green now that the owning worker
finished.

---

## 8. Gaps and honest notes

1. **The right rail is 320 px wide and the ledger has six columns.** The
   table is laid out with a 296 px minimum and the message column takes the
   remainder, so at 1440 the six columns fit; below 1400 px (a 260 px rail)
   the ledger scrolls horizontally inside its own pane, and the document
   never scrolls. The six columns were a binding requirement, so the ledger
   scrolls rather than dropping a column.
2. **The chain stamp has two broken texts.** `CHAIN BROKEN @ n` when the
   backend named the first bad event, and the older `HASH-CHAIN BROKEN` when
   it did not. The second form keeps the pre-existing ledger contract
   intact (`test/chatJournal.dom.test.tsx` pins it) and is never printed
   when the backend reports a sequence — which it always does when it
   reports `ok: false`.
3. **The journal poller is gated by C1a, and that gate is one notch wider
   than the Opus rule.** `studio/src/App.tsx` starts the poller when
   `designAvailable !== null`, so a backend that explicitly answers
   `design_available: false` is still polled (the Rust backend, which
   reports no flag at all, is correctly left alone). `App.tsx` belongs to
   `c1a-data`, so this cell did not edit it; the one-line fix is
   `designAvailable !== true`. The UI is honest either way: the ledger shows
   the degraded note and never a chain claim.
4. **The ledger's local tie-break differs from the store's merge.** The
   store's `selectMergedJournal` orders equal-timestamp local rows by their
   id; the ledger keeps the local rows in the order the shell recorded them.
   The two agree on every real session (the ids are monotonic there).
5. **The Options card 02 grows when a design is active.** The design block
   is deliberately compact (the format picker and the button share one row),
   but the export card is the tallest of the four, and the C0b invariant
   "card 02 has no outer scroll at 1440×900" was measured before this
   cell. Final QA should re-measure the drawer with an active design.
6. **The digest chip shows eight characters.** The full digest is one
   `title` hover and one click away, and the copy button puts the full
   value on the clipboard.
7. **The degraded note is driven by an explicit `false`.** A backend that
   never answered the health probe shows no note, because the studio must
   not claim a capability it has not been told about.
8. **No live backend was called.** Every test stubs `fetch` or the client
   module; the payloads are the ones the backend cell recorded.
