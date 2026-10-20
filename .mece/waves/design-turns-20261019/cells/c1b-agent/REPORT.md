# C1b-agent — the agent card and the bottom-bar readout for design turns

Cell: `c1b-agent`. Spec: `../c1b-components/TASK.md` items **1** and **6** plus
the binding "backend without design support" addendum, ADR-0009 §7, the B1 API
contract (`../b1-api/REPORT.md`), the store contract (`../c1a-data/REPORT.md`),
and the C0b design system (`vellum-ui-20261019/cells/c0b-system/REPORT.md`).

Status: **complete** for the owned surface. All three gates are green.
No commit, push, delegation, screenshot, or credential read was performed.
Nothing outside the owned files was touched, with one exception: a single
minimal additive change to `studio/src/state/store.ts` (listed in §4).

---

## 1. Files

### Created (owned by this cell)

| File | Lines | Purpose |
|---|---:|---|
| `studio/src/components/chat/designCard.ts` | 218 | the pure helpers of the card: the measured rows, the digest chips, the check formatting, the declared-density rule, and the honest note for a backend without design support |
| `studio/src/components/chat/DesignAgentCard.tsx` | 210 | the card component: measured table, checks, digest chips with copy, the verification stamp, the jump-to-section pill |
| `studio/src/styles/bottombar.css` | 19 | the bottom-bar rules for the one state the primitive preview never reaches (the measured-design readout) |
| `studio/test/DesignAgentCard.dom.test.tsx` | 307 | 17 DOM tests for the card, the chat turn, and the no-design-backend mode |
| `studio/test/BottomBarDesignReadout.dom.test.tsx` | 99 | 6 DOM tests for the bar readout with and without a design |

### Edited (owned by this cell)

| File | Change |
|---|---|
| `studio/src/components/chat/ChatPanel.tsx` | renders the design card under the turn that produced a design; renders the one-line note when the backend cannot compile designs; the geometry-context block quotes the active design's measured values when there is one; the chat response now carries the design id |
| `studio/src/components/bottombar/BottomBar.tsx` | the readout comes from the active design's measured values with the source "OCP measured"; without a design the bar keeps its primitive behaviour |
| `studio/src/styles/chat.css` | the agent-card styles (all type tokens, no raw px font size) |

### Not touched

`studio/src/components/drafting/**`, `studio/src/viewer/**`,
`studio/src/components/journal/**`, `studio/src/components/export/**`,
`studio/src/styles/drafting.css`, `studio/src/styles/journal.css`,
`studio/src/api/**`, `studio/src/types/**`, `studio/src/state/designs.ts`,
`studio/src/state/journal*.ts`, `studio/electron/**`, `studio/scripts/**`,
`electron-builder.json`, and every Python file.

---

## 2. The agent card (c1b-components item 1)

The card is rendered under the assistant turn that compiled a design. It shows
exactly what the backend measured, and nothing that the backend did not
report.

### The measured table

| Row | Value | Source |
|---|---|---|
| `volume` | `35571.42 mm³` | `measured.volume_mm3` |
| `area` | `11889.10 mm²` | `measured.surface_area_mm2` |
| `envelope X` | `60.00 mm` | `measured.bounds`, the measured envelope |
| `envelope Y` | `45.00 mm` | `measured.bounds` |
| `envelope Z` | `40.00 mm` | `measured.bounds` |
| `mass` | **only when a density was DECLARED** | `measured.volume_mm3` × the declared density |

The table's caption says `Measured by the geometry kernel (OCP)`, and every
row's tooltip names the bounds range or the kernel that produced it.

**The mass rule.** The frozen B1 `DesignResult` carries **no density at all**,
so the mass row is omitted today — a mass computed from a material *name*
("aluminium ⇒ 2.70 g/cm³") would be a fabricated number, and the ADR forbids
it. The rule is implemented rather than skipped: `readDeclaredDensity()`
reads a declared `declared_density_g_cm3` (g/cm³) and refuses a missing,
non-numeric, or non-positive value. The moment the backend declares a
density, the row appears with `mass = volume_mm3 / 1000 × density` in grams.
This is the only derived number in the card, and it is derived from a
declared input.

### The checks

One row per backend check, each with a `✓`/`✗` mark
(`role="img"`, `aria-label` "pass"/"fail"), the check label, and the expected
value beside the measured one:

```
✓  Envelope X   expected [-30, 30]   measured [-30, 30] mm
```

The `unit` is appended to the measured value except for `bool`, where it
would only add noise. A check that fails is marked in redline; the whole
row is not restyled, so the expected/measured pair stays readable.

### The digest chips

One chip per artefact digest: `script` always, `step` only once a STEP
artefact exists (`artifacts.step_sha256` is `null` until then). The chip
prints the **first eight characters** of the SHA-256 and carries a copy
button that puts the **full** digest on the clipboard
(`aria-label="Copy script sha256"`). The button shows `copied` for 1.5 s and
`data-copy-failed="yes"` when the browser refuses the write, so a failed
copy never looks like a successful one.

### The stamp

`B-Rep Verified` is a Prussian stamp and is rendered **only** when
`verified === true`. When the design is not verified the stamp is replaced
by the honest count (`2/3 checks pass`); the string "B-Rep Verified" never
appears in that state. With no checks at all, no stamp is rendered at all.

### The section pill

`Section A–A ↗` does exactly the two things the spec asks for:

```ts
activateDesign(design.design_id);                 // makes this design active
dispatch({ type: 'auxView/set', view: 'section' }); // switches AuxView to Section
```

Both are plain store actions; no prop drilling and no AppShell edit.

### The card reads the store, never a copy

The card is given a `designId`, not a `DesignResult`. It resolves the
design through the store (`state.designs.designs`), so the card always
shows the store's copy of the measurement and never a stale snapshot. If
the design LRU has evicted it, the card says so in one honest line
("This design is no longer held in the session store; the design cache
evicted it.") instead of rendering an empty table.

---

## 3. The bottom-bar readout (c1b-components item 6)

With an active design the bar quotes the **measured** values and names the
source:

| Item | Value | Source |
|---|---|---|
| `ENV` | `60×45×40 mm` | the measured bounds |
| `VOL` | `35571.42 mm³` | `measured.volume_mm3` |
| `AREA` | `11889.10 mm²` | `measured.surface_area_mm2` |
| source | `OCP measured` | — |

The millimetre units are kept (not rounded into cm³) because the readout
must be the value the kernel measured. The source span carries
`data-source="OCP measured"` and a new `data-source-kind="ocp"` styling
hook; `data-source` still mirrors the printed text, so the existing
`BottomBar.dom.test.tsx` assertion is unchanged.

Without a design the bar is **byte-for-byte the old behaviour**: the
primitive's closed-form estimate, the `server` / `local est.` / `—` source,
the `ENV · VOL · AREA · SCALE · source` order, and the honest `FIT` scale
statement. `SCALE` is never touched by a design turn: the viewport still
frames the part to fit, so the sheet still carries no fixed model-to-paper
ratio.

---

## 4. The wiring edits (minimal, additive, and listed as required)

### `studio/src/state/store.ts` — three lines

The chat turn had to be able to say **which** design it produced; the design
itself stays in the design slice, so the store is not duplicated.

| Line | Change |
|---|---|
| `ChatMessage` | added `designId?: string` (the design the turn compiled) |
| `Action` | `chat/response` gained `designId?: string` in its payload |
| `reducer` | `if (action.response.designId !== undefined) message.designId = action.response.designId;` |

Nothing was renamed, removed, or changed in behaviour; `exactOptionalPropertyTypes`
is honoured, so the field is only ever set when it is present. The file was
re-read immediately before each of the three patches (the parallel C1b
workers may edit the same file).

### `studio/src/components/chat/ChatPanel.tsx` — the caller

`postChat` already hands the validated design to the store (C1a's
`design/received` dispatch), so the panel only forwards the id:

```ts
...(response.design ? { designId: response.design.design_id } : {}),
```

### `studio/src/AppShell.tsx` — **not** edited

The card and the bar both read the store directly, so no shell wiring was
needed. The only shell-side wiring in this wave belongs to the drawing
worker (the design mesh in the viewport and the sheet tabs).

---

## 5. A backend without design support

`designSupport(server)` in `designCard.ts` is the single rule:

| Backend state | What the panel says |
|---|---|
| `design_available: false` | `Design turns need the Python geometry backend; this backend reports design_available: false.` |
| `design_available` absent (null) **and** the health probe answered | `Design turns need the Python geometry backend; this backend did not report design_available.` |
| `design_available: true` | nothing — no note is rendered |
| the health probe has not answered yet | nothing — the panel stays quiet, because a boot-time flash would be a claim the frontend cannot make yet |

The note is rendered **once**, in the agent-card block of the chat panel
(`.chat-panel-agent-card`, `role="note"`), so it is visible even before the
operator sends a turn. The wording distinguishes "reports false" from "did
not report the flag", because those are different facts.

Everything else keeps working in that mode: the composer, the four starter
prompts, the provider selection and consent stamp, the API-key drawer, the
active-geometry context block, the preview, and the whole primitive
pipeline. A DOM test asserts this explicitly.

---

## 6. The geometry context block

The chat panel's "Active geometry" card showed the *primitive's* closed-form
estimate. With a compiled design active, the sheet shows the design, so the
block now quotes the **design's** measured values instead (kind `design`,
the design's label, the measured envelope, the measured volume). Two
different numbers for the same "active geometry" would be a contradiction on
the operator's desk. Without a design the block is unchanged.

---

## 7. Tests

| File | Tests | What it covers |
|---|---:|---|
| `test/DesignAgentCard.dom.test.tsx` | 17 | the measured table and the exact measured numbers; the mass row absent with no declared density and present with one; the checks with ✓/✗ and the expected/measured pair; the stamp present only when verified; the digest chips (script always, STEP only when it exists); the copy button putting the **full** digest on the clipboard; the section pill activating the design and switching AuxView to Section; the card rendered for a turn that produced a design and absent for one that did not; the geometry context quoting the design; the three no-design-backend notes; the primitive features still working |
| `test/BottomBarDesignReadout.dom.test.tsx` | 6 | the measured envelope, volume, and area in the bar; the source "OCP measured" and its styling hook; the fixed readout order and the honest `FIT` scale; the unchanged primitive behaviour with and without a local preview |

The fixtures are the recorded payloads from `test/designFixtures.ts` (the
criterion-2 L-bracket: 60 × 45 × 40 mm, volume 35 571.416 047 mm³, the
three recorded checks). No test invents a measurement.

---

## 8. Gates

Run in `/home/v9i/Projects/Kala-Agent/studio` (Node 22, vitest 5, TypeScript
5.7 strict with `exactOptionalPropertyTypes` and `noUncheckedIndexedAccess`):

| Gate | Command | Result |
|---|---|---|
| Types | `npm run typecheck` | **clean** (no errors) |
| Tests | `npm test` | **39 files, 530 tests passed** (0 failed) |
| Tests (this cell) | `npx vitest run test/DesignAgentCard.dom.test.tsx test/BottomBarDesignReadout.dom.test.tsx` | **23 passed** (17 + 6) |
| Build | `npm run build` | **built in 334 ms** |

The suite is shared with the two parallel C1b workers, so the 530 is their
files and mine together; this cell adds 23 of them. The build's "chunks are
larger than 500 kB" warning is the pre-existing `three.js` bundle, not this
cell.

**Transient failures seen while the drawing worker was mid-refactor.** Two
earlier full runs reported failures that were all inside the *drawing
worker's* files, never in this cell's:

| Run | Failure | Owner |
|---|---|---|
| 1 | `test/designSystem.test.ts` (type-token guard) and a `drafting.css` PostCSS syntax error (`Unexpected }` at line 1240) | `src/styles/drafting.css` — c1b-drawing |
| 2 | `test/HlrDrawing.dom.test.tsx` and the same type-token guard | c1b-drawing |
| 3 | `npm run typecheck`: `test/AuxView.dom.test.tsx` — `designController: DesignController \| null \| undefined` vs `DesignController \| null` under `exactOptionalPropertyTypes` | c1b-drawing |

Per the task's rule the runs were repeated rather than "fixed": this cell
must not edit a parallel worker's files, and no file outside this cell's
ownership was modified to make a gate pass. After the drawing worker
settled, three consecutive full runs were green (typecheck clean, 530/530
tests, build green). This cell's own two suites passed in every run,
including the three failing ones.

---

## 9. Gaps and honest notes

1. **The mass row never renders today.** The B1 contract has no density, so
   the row is omitted. The rule is implemented and tested against a design
   that declares one, but the honest statement is that no shipped backend
   can currently produce a mass. A density table in the frontend was
   deliberately **not** added.
2. **The card is bound to the active design, not to the turn's design
   geometry.** A turn's card shows the design it names; the *sheet* follows
   the store's active design. The section pill is what moves the sheet to
   that design. A turn from an evicted design shows the honest "no longer
   held" line.
3. **The "B-Rep Verified" stamp is the backend's `verified` flag.** The ADR
   defines it as OCP validity plus measured-vs-declared envelope; it says
   nothing about strength or manufacturability, and the card does not
   suggest otherwise.
4. **The note is rendered once per panel, not once per turn.** Repeating it
   under every assistant turn would be noise; the single line in the agent
   card is the honest one.
5. **The no-design note is gated on the health probe.** Before `/health`
   answers, the panel says nothing about design turns. The poller in the
   data layer uses the same `design_available` flag, so the panel and the
   ledger never disagree.
6. **`data-source-kind` is a new attribute on the source chip.** It is
   additive: `data-source` still holds the printed text, so the existing
   `BottomBar.dom.test.tsx` contract is untouched.
7. **No screenshot was taken.** Final QA (D1) owns the visual evidence; the
   card's CSS was written against the C0b tokens and passes the
   `designSystem.test.ts` guard (no raw px font size, no rounded corner).
8. **The sheet tabs, the HLR drawing, the Tolerance tab, and the journal
   ledger belong to the drawing and ledger workers.** This cell only
   contributes the agent card and the bar readout.
