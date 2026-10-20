# C4 polish — ledger column overlap, SEQ clipping, one-line summaries

Cell: `c4-polish`, wave `design-turns-20261019`.
Read: `TASK.md` and the Opus review of `../c3-visual/shots/after-1440-pencil.png`.

Status: **done** for the journal component, its stylesheet, and its tests.
No commit, push, deployment, delegation, or credential read was performed.
Only `studio/src/components/journal/**`, `studio/src/styles/journal.css`, and
`studio/test/journalLedger.dom.test.tsx` were edited; the screenshot, the raw
DOM measurements, and this report are under the cell directory.

---

## 1. The three fixes

### 1.1 The digest copy button stays inside DIGEST

The copy button used to be an inline sibling of an `inline-block` digest chip
inside a 68 px fixed cell. The chip could not shrink (`max-width: 100%` plus
`white-space: nowrap`), so the 16 px button overflowed into CHAIN and covered
the chain verdict: the C3 shot renders it as `⧉k`.

The chip and the button are now one flex line inside the DIGEST cell:

```
<td class="journal-entry-digest-cell">
  <span class="journal-entry-digest-line">
    <code class="journal-entry-digest">…</code>
    <button class="studio-copy-button journal-digest-copy">…</button>
  </span>
</td>
```

* `.journal-entry-digest-line` is `display: flex; gap: 2px; min-width: 0`.
* The chip is `flex: 0 1 auto; min-width: 0; max-width: calc(100% - 18px)`, so
  it shrinks and ellipsizes before the button can move.
* The button is `flex: 0 0 auto`, and the cell clips (`overflow: hidden`) as a
  last line of defence.
* DIGEST is 80 px, CHAIN 42 px. CHAIN prints `ok`, `local`, and `broken` in
  full; it no longer wraps `local` to `loc al`.

Measured in Chromium 154 at 1440×900, real Python backend, FreeCAD design:

```
DIGEST cell   x 1312 → 1392   (80 px)
digest chip   x 1315 → 1369.61
copy button   x 1371.61 → 1387.61   (inside the cell by 4.39 px)
CHAIN cell    x 1392 → 1434   (42 px), text "ok"
copyOverlapsChain: false
```

The same holds for all ten server rows; no copy button crosses its cell edge.

### 1.2 SEQ and CHAIN headers are not clipped

The header cells were 20 px (SEQ) and 30 px (CHAIN), narrower than their own
bold, letter-spaced labels. Fixed table layout let the next cell's background
paint over the overflowing text, so SEQ read `SE(` and CHAIN read `CHAI`.

The six-column budget is now declared on the ledger as custom properties:

| column | width | what it must hold |
|---|---|---|
| SEQ | 28 px | `SEQ` header, three-digit sequence (`999`) |
| UTC | 52 px | `HH:MM:SS` |
| TYPE | 34 px | `TYPE` header (event types ellipsize, full value in title) |
| MESSAGE | remainder | one ellipsized summary line |
| DIGEST | 80 px | 8-character digest, chip frame, 16 px copy button |
| CHAIN | 42 px | `CHAIN` header, `local`, `ok`, `broken` |

Measured header scroll widths vs client widths at 1440: `SEQ 28/28`,
`UTC 52/52`, `TYPE 34/34`, `MESSAGE 77/77`, `DIGEST 80/80`, `CHAIN 42/42`;
no header had `scrollWidth > clientWidth`. At 1280 the compact four-column
ledger (`SEQ TYPE MESSAGE DIGEST`) measures `28 / 34 / 111 / 80`, also with no
clipped header.

### 1.3 MESSAGE is one line, full text available

Every MESSAGE cell now renders:

```
<td class="journal-entry-message">
  <span class="journal-entry-message-text" title="…full message…">…</span>
</td>
```

`.journal-entry-message-text` is a block with `white-space: nowrap;
overflow: hidden; text-overflow: ellipsis`. The full string stays in the DOM
(so screen readers still read it, and the accessible table semantics are
unchanged) and in the native title tooltip. Server summaries were already
type-specific (`summarisePayload`), e.g. the `provider.build.completed` row is
`freecad 1.1.4 · build completed · step 01e54555`; it is no longer wrapped
over 4–5 lines. Local operator notes use the same one-line treatment, so the
ledger stays one row per event; their full text is in the same title.

MESSAGE is 77 px wide at 1440 (a 320 px rail with six columns) and 111 px at
1280 (the compact rail drops UTC and CHAIN). The summary is therefore
ellipsized by design; the leading provider/verb tokens remain visible and the
whole message is one hover away.

---

## 2. Tests added

Added to `studio/test/journalLedger.dom.test.tsx` (5 new cases, 20 in the file):

| Test | Pins |
|---|---|
| `renders a server message as one line with the full text in the title` | The `provider.build.completed` row text is exactly `freecad 1.1.4 · build completed · step 01e54555`; every server row's `title` equals its full text; the cell still carries the full text. |
| `keeps the MESSAGE styling single-line and ellipsized` | The `.journal-entry-message-text` rule contains `white-space: nowrap`, `overflow: hidden`, and `text-overflow: ellipsis`. |
| `keeps every cell inside its column at 1440` | Cell boxes at 1440, all six columns. |
| `keeps every cell inside its column at 1280` | Cell boxes at 1280, compact four columns. |
| `keeps the copy button inside the DIGEST cell, not over CHAIN` | Structural containment (chip + button share the digest line) and the DIGEST width budget. |

The two layout cases are DOM-driven but happy-dom has no layout engine, so they
build each cell's box from the widths the production CSS declares
(`--journal-col-*` in `journal.css`) and the text the rendered DOM contains.
Text width uses the conservative 9 px × 0.62 em advance (JetBrains Mono is
0.6 em), so a box can only be too wide. Each case asserts:

* every adjacent pair of cell boxes is disjoint (`right ≤ next.left`);
* each cell's narrowest content (SEQ header + `999`, `HH:MM:SS`, `CHAIN`
  header + `broken`, digest chip + frame + gap + 16 px button) fits its column;
* MESSAGE keeps at least 70 px at 1440 and 100 px at 1280;
* CHAIN is present and not clipped at 1440.

This model is validated by the real-browser measurements saved in
`shots/after-1440-ledger-measurements.json`; the actual widths match the
declared budget and no pair of adjacent cells overlaps.

---

## 3. Gates

Run in `studio/` on the final tree:

| Command | Result |
|---|---|
| `npm run typecheck` | **clean**, exit 0 |
| `npm test` | **43 files, 619 tests, 619 passed** (2.8 s) |
| `npm run build` | **green** (`built in 402ms`); only the pre-existing three.js chunk-size warning |

---

## 4. Live reproduction and after-shot

* Backend: `KALA_STUDIO_TOKEN=<throwaway> .venv/bin/python -m kala_agent.studio
  --data-dir /tmp/opencode/c4/data --static-dist studio/dist --port 8792`
  (`design_available: true`, FreeCAD `1.1.4`).
* Headless Chromium 154 over CDP, 1440×900 and 1280×800, using the C3 capture
  harness (`/tmp/opencode/c4/capture.cjs`).
* Prompt: *"Design a machined aluminum L-bracket for a 40 mm mounting plate,
  8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting
  holes."*
* CAD target: **FreeCAD 1.1.4**.
* Result: 17 ledger rows = 7 local + 10 server, `SEQ 1–10`, 64-character
  digests, `CHAIN ok`, stamp `HASH-CHAIN OK`.

| File | Shows |
|---|---|
| `shots/after-1440-ledger.png` | FreeCAD L-bracket turn at 1440: clip-free `SEQ UTC TYPE MESSAGE DIGEST CHAIN` header, copy button inside DIGEST, `ok`/`local` in CHAIN, one-line summaries. |
| `shots/after-1440-ledger-measurements.json` | Raw DOM boxes at 1440 and 1280: header widths, digest chip/button rects, copy containment, message title equality, row count, document overflow. |

The before state is the frame Opus reviewed,
`../c3-visual/shots/after-1440-pencil.png`: `SE(` header, `⧉k` collision, and
4–5-line messages.

Measured after-values:

| Viewport | SEQ | UTC | TYPE | MESSAGE | DIGEST | CHAIN | header clipped | doc overflow |
|---|---|---|---|---|---|---|---|---|
| 1440×900 | 28 | 52 | 34 | 77 | 80 | 42 | none | 0 |
| 1280×800 | 28 | — | 34 | 111 | 80 | — | none | 0 |

---

## 5. Files changed

* `studio/src/components/journal/JournalPanel.tsx` — digest flex line,
  message span with full-text title.
* `studio/src/styles/journal.css` — column budget, header/cell padding,
  digest flex containment, one-line message, CHAIN nowrap.
* `studio/test/journalLedger.dom.test.tsx` — summary, containment, and
  two-width layout coverage.

No other source, test, backend, or configuration file was touched.

---

## 6. Honest notes

1. **The MESSAGE column is narrow by construction at 1440** (77 px on a
   320 px rail with six fixed columns). The summary is clipped after roughly
   14 characters; the complete text is in the cell's title. This is the
   explicit trade-off requested: single line, full text available. At 1280
   the compact rail gives MESSAGE 111 px because UTC and CHAIN are already
   documented as hidden there (their values live in the row title).
2. **TYPE remains ellipsized** (34 px). It was already ellipsized at 50 px
   for long event types; the full type is the tag's title, and colour still
   carries the kind.
3. **Local operator notes are also one line now.** The task called out server
   rows, but leaving local rows at 9 wrapped lines would have defeated the
   compact ledger. Their complete message is in the same title, and no text
   was removed from the DOM.
4. **The tooltip is the disclosure, not a click-to-expand row.** The task
   allowed "expandable detail or title"; a per-row disclosure would add a
   second interaction to a scan surface and make the narrow cell taller. The
   full text remains keyboard/screen-reader accessible as cell text.
5. **No remote provider was called and no credential was read.** The token
   was a throwaway value passed through the local environment; every number
   above is either the real FreeCAD result or a real DOM measurement.
