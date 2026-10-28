# Cell pencil-prompt (Phase B) — the drafting-pencil prompt bar, its history, and the living graphite string

## Gates

Every gate ran in `studio/` with a fresh `mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`, and `npm_config_cache=/home/v9i/.npm`.
`/home/v9i/.local/share/kala-agent` was never touched, no credential/auth/settings file was
read, no real `claude`/`codex`/`opencode` CLI was run, and every temp dir was removed.

| Gate | Result |
|---|---|
| `npm run typecheck` | **fails on `test/titleBlock.dom.test.tsx` only** (6 errors, the title-block cell's file, mid-edit). Re-run: no error in any file of this cell. |
| `npm test` (this cell's files) | **47 passed / 47** in **4 files** (`pencilPrompt.dom.test.tsx` 21, `promptHistory.dom.test.tsx` 13, `graphiteString.dom.test.tsx` 6, `promptCss.test.ts` 6) |
| `npm test` (whole suite, at the time of the run) | **1099 passed / 1110**, 5 files failed — none of them a file of this cell (see below) |
| `npm run build` | `vite build` **succeeds** (only the pre-existing three.js chunk-size warning). The `npm run build` script stops at its first step, `tsc --noEmit`, because of the same title-block test file. |

The failures of the whole suite at the time of the run, all in files this cell does not own
and none caused by it:

- `test/shellStubs.dom.test.tsx` — 7 failures: the ToolStrip, ViewSwitcher, ScalePill,
  MiniMap, ProcessTrail, TitleBlock and EngineerCard stubs of the other Phase B cells.
  **The PencilPrompt and GraphiteString assertions of that shared file pass** (the pencil
  still carries the `data-stub="pencil-prompt"` mount point and the string still reads
  `data-graphite-string="idle"`).
- `test/AgentWorkspace.dom.test.tsx` — 2 failures: the `tool-strip` mount point of the
  canvas-3d cell and the engineer card of the title-block cell. The
  `.drafting-overlay-pencil [data-stub="pencil-prompt"]` and `.graphite-string` assertions
  of that shared file pass.
- `test/graphite.test.ts` (1), `test/journalLedger.dom.test.tsx` (1) — the viewer shader
  and the legacy journal CSS of other cells.

## Files changed

### Created (this cell owns them)

- `studio/src/components/prompt/PencilPrompt.tsx` — the pencil: ferrule (↺ history, Clear),
  collar (blocker chip / live clock / redline pills / route stamp), barrel (the input
  chamber, the edit chip, the character counter), nib (the wood-and-graphite cone that
  drafts). No props; it reads the store and calls `state/chatTurn.ts`.
- `studio/src/components/prompt/PromptHistory.tsx` — the prompt history popover: the whole
  thread, newest first, with the honest facts of every turn.
- `studio/src/components/prompt/GraphiteString.tsx` — the living graphite string: the hand
  drawn line, the scribble, the crystallised trail and the snapped line.
- `studio/src/components/prompt/graphite.ts` — the pure geometry of the string (seeded
  jitter, the cubic, its length, the scribble loop, the milestone dots).
- `studio/src/components/prompt/labels.ts` — the blocker's short name, the Engineer card
  section that resolves it, the route stamp, the failure code.
- `studio/src/components/prompt/history.ts` — the thread read as turns.
- `studio/src/components/prompt/useFocusTrap.ts` — the popover's focus trap (Tab wrap,
  Escape, focus back to the trigger).
- `studio/src/components/prompt/useReducedMotion.ts` — the motion preference, kept live.
- `studio/src/styles/prompt.css` — the pencil, the popover and the string. Tokens only; no
  gradients, no shadows, no raw px font sizes, no rounded corners.
- `studio/test/pencilPrompt.dom.test.tsx`, `studio/test/promptHistory.dom.test.tsx`,
  `studio/test/graphiteString.dom.test.tsx`, `studio/test/promptCss.test.ts` — 47 tests.

### Modified

- `studio/src/components/prompt/PencilPrompt.tsx` and `studio/src/components/prompt/GraphiteString.tsx`
  — the Phase A stubs were replaced by the real components. The frozen prop contract (no
  props) is unchanged.

No other file was touched: the store, the types, `AppShell.tsx`, `chatTurn.ts`,
`anchors.ts`, `components/chat/**`, the other stylesheets, `studio/electron/**`,
`studio/scripts/**`, `electron-builder.json` and `index.html` are untouched.

## Contract consumed (frozen, nothing added)

- `studio/src/state/chatTurn.ts` — `sendTurn({prompt, baseDesignId?})`, `abortTurn()`,
  `turnReadiness(state)`, `useTurnElapsed()`, `formatElapsed()`, `chatBadgeText()`. The
  pencil adds no turn logic of its own: the abort, the deadline, the journal rows and the
  design activation are the shared machine's.
- `studio/src/state/store.ts` — `promptDraft` + `prompt/draft`, `editDetached` +
  `edit/detach`, `commandCard/open {section}` + `commandCard.section`, `routing`
  (amendment 1), `chat.thread / inFlight / lastError / lastErrorKind / turnStartedAt`,
  `designs.activeDesignId`, `design/received`, `design/activate`, `chat/clear`,
  `selectActiveDesign`, `selectCadProvider`.
- `studio/src/components/shell/anchors.ts` — `useAnchorRef('pencil-tip')` (published by
  the pencil on the graphite point) and `useAnchorRect('pencil-tip' | 'trail-port')`.
- `studio/src/types/studio.ts` — `ChatUsage`, `ProviderId`; `studio/src/providerPolicy.ts` —
  `defaultModelLabel`, `requiresModel`, `isLocalProvider`.
- WAVE D16 (the edit chip), amendment 1 (the routing lives in the store), the pencil's
  geometry from § Shell geometry (centred, 20 px above the bottom rule,
  `min(660px, 60vw)` × 44 px) and SPEC § 4D (the pencil silhouette and the string).

Nothing was added to the contract, so there is no contract change for Opus to merge.

## What the pencil does

**The pencil.** Ferrule and eraser cap (48 px, an inline-SVG knurl band): `↺` opens the
prompt history, `Clear` empties the barrel — the draft only, never the thread. Hexagonal
barrel (flex): the input, bound to `promptDraft`, with twin 0.75 px bevel hairlines and
the placeholder *"Describe a part, assembly, or modification to draft…"*. Enter sends,
Shift+Enter keeps the newline, an IME composition never sends, and the barrel grows upward
to six lines while the text wraps. A mono counter appears from 3500 of 4096. Conical nib
(52 px): a wood cone ending in the graphite point `▶`; its accessible name is "Draft", it
is disabled with the reason as its tooltip while `turnReadiness` is not ready, and it
publishes the `pencil-tip` anchor on the point.

**The blocker chip.** In the pencil EVERY `turnReadiness` blocker is hard, `cad-target`
included: the nib stays disabled and a refused send keeps the prompt in the barrel. The
chip at the collar names the real blocker and opens the Engineer card on the section that
resolves it: CAD target → `cad`, consent / provider / model → `routing`, connection →
`cad`.

**The edit chip (D16).** `EDITING ‹label› ×` appears in the barrel when a design is
active, the route is not the local helper, and the design is not detached. `×` dispatches
`edit/detach {detached:true}`; any change of the active design re-attaches
(`edit/detach false`); a send while attached carries `baseDesignId`.

**In flight.** The collar shows the live elapsed clock (mono, one second tick, from
`chat.turnStartedAt`) and the two redline pills `✕ Abort` and `✎ Edit Prompt` (abort, put
the sent prompt back into the barrel, focus it). After a turn a compact dismissible error
line rides above the pencil with the real message; a timeout reads as a timeout and a
cancellation reads as a cancellation.

**The prompt history.** A top-layer dialog (focus trapped, Escape closes it, the focus
returns to the trigger). The whole thread, newest first: the prompt, the full reply
(scrollable), the honest provider and model label (`chatBadgeText`, so a subscription CLI
says that it used its own default model and never names one it did not report), the
duration, the tokens when the provider reported any, the failure code when there is one,
`Show design` when the turn built one, and `Re-use prompt`. `Clear history` asks for a
confirmation and then calls `chat/clear`.

**The graphite string.** A viewport-fixed, pointer-transparent SVG. Idle: nothing.
Generating: a hand-drawn cubic leaves the tip's right-edge centre, sweeps across into the
dock, is drawn out over ~600 ms and ends in an endless organic scribble at the trail port.
Success: the scribble pulls taut, the line becomes a straight dotted trail with three
milestone dots and a closing ring, and it fades after ~1.2 s. Failure: the line snaps at
55 % with a redline fracture and fades. The jitter is seeded from the turn, the anchor
rectangles are tracked live, the animation clock is paused while the document is hidden,
and with `prefers-reduced-motion: reduce` the string is a static dashed line while a turn
runs and nothing at all afterwards.

## Known gaps

1. **The `trail-port` anchor belongs to the dock cell.** The pencil publishes
   `pencil-tip`; `trail-port` must be published by the Process Trail's entry port. Until it
   is, the string has no end point and stays invisible (honest: no line to a port that does
   not exist). The anchors contract is frozen, so nothing has to change there — only the
   dock's mount.
2. **The four starter prompts of the legacy `ChatPanel` have no home in the SPEC's
   pencil.** They are not in the WAVE relocation table, not in SPEC § 4D, and not in this
   cell's list of what the popover shows, so they were not ported. They still live in
   `components/chat/ChatPanel.tsx` (and their coverage in `test/chatJournal.dom.test.tsx`),
   which qa removes in Phase C. See Request 1.
3. **The store keeps ONE error, not a per-turn log.** A failure code can therefore only be
   attributed to the newest turn, and only while that turn has no reply. An older turn
   without a reply says "No reply recorded" instead of borrowing the last error. No store
   change was made; the history is honest about what the sheet knows.
4. **`data-stub="pencil-prompt"` is kept on the pencil root.** Two shared test files this
   cell does not own (`test/shellStubs.dom.test.tsx` and `test/AgentWorkspace.dom.test.tsx`)
   look for that mount point, so the attribute stays and the root also carries the honest
   `data-pencil="pencil-prompt"`. qa may drop the `data-stub` marker when the stub test is
   rewritten.
5. **The route stamp in the collar is this cell's addition.** The WAVE names the collar
   for the blocker chip, the clock and the pills. The stamp answers "which route does this
   draft take?" from the store's routing (`ROUTE —` when no provider is chosen, the CLI's
   own default model named exactly as the CLI reports it). It is a small addition to the
   WAVE's list; say the word and it goes.
6. **The edit chip does not appear on the local route.** `isLocalProvider` covers `local`
   and `local-helper`, and the backend refuses an edit turn there
   (`edit_requires_model`), so showing the chip would promise something the turn cannot
   deliver.
7. **The barrel's auto-grow is measured in JavaScript** (`scrollHeight`, capped at six
   lines). In a headless DOM the measured height collapses to one line; in the app the
   browser measures it.

## Requests

1. **The starter prompts.** The four taped paper tabs of the legacy chat panel
   ("Size L-Bracket", "Size Stepped Shaft", "Housing Envelope", "STEP Handoff") are not in
   the relocation table and do not fit the 44 px pencil. I did not port them. If they must
   survive the removal of `ChatPanel`, tell me where they go — my suggestion is the prompt
   history popover, as four chips that only seed the barrel (never auto-send), which keeps
   the SPEC's pencil silhouette intact.
2. **The dock's entry port.** `useAnchorRect('trail-port')` is only useful once the Process
   Trail publishes that anchor. No contract change is needed (the anchor id exists), only
   the dock cell's mount. Please confirm who owns that mount.
3. Nothing else: the store, the types and `AppShell.tsx` were not touched, and no contract
   change is needed for this cell to work.
