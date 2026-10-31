# Composer cell — report (app-feel wave, 2026-10-31)

## Scope delivered

The left column is now only the agent thread and the composer:

- **Header**: one slim row — `AGENT` (mono caps) on the left, `New chat` on the
  right, and only while the thread has messages. `New chat` aborts an in-flight
  turn and dispatches `chat/clear` exactly as the old Clear did.
- **Empty thread**: one Newsreader-italic line plus the four existing starter
  tabs, inside the thread area. They leave with the first message and return
  after `New chat`.
- **Composer**: prompt textarea (same placeholder, Enter drafts / Shift+Enter
  keeps the newline) and the same `[Draft Part Geometry ↵]` button, then one
  row of two chips:
  - `[● <provider> · <model or "default"> ▾]` — `aria-label="Model: …"`;
  - `[CAD: <name or "choose"> ▾]` — `aria-label="CAD target: …"`, vermilion
    attention while no CAD target is chosen.
- **Popovers** (`ComposerPopover.tsx`): anchored above the chip,
  `role="dialog"`, close on Escape and outside click, focus returns to the
  chip. They hold the unchanged `ProviderPicker` + `ModelPicker` and
  `CadTargetSelector`.
- **Consent stamp**: rendered only while a remote provider is selected and
  consent has not been granted.
- **Design card**: collapsed by default to the real summary
  (`✓ 5/5 checks · measured by OpenCascade (opt-in) 7.9.3`); the disclosure
  (`aria-expanded`) opens the full measured table, checks, digest chips,
  downloads, warnings and `Section A–A ↗`. Failed checks auto-expand.

## What moved where (one home per control)

| Old home | New home |
|---|---|
| Brand stamp `KALA // MECHANICAL DESIGN AGENT` in chat | top bar (shell cell) |
| `BROWSER FALLBACK` connection line | status bar (shell cell) |
| `API Keys` button + inline key drawer in chat | Settings › Models & keys; the model picker's action dispatches `options/open {card:"providers"}` |
| `Active geometry` table (`chat-panel-context`) | Details › Properties (shell cell) |
| Provider / Model / CAD target blocks under the prompt | model chip popover and CAD chip popover |
| `ROUTE …` line and the "Pick a configured provider…" paragraph | removed |
| `Clear` (always visible) | `New chat` (only with messages) |
| Full design card always open | one-line summary + disclosure, failures auto-open |
| 4 starter chips in a band above the thread | chips inside the empty thread |

Nothing was added to the store contract or `types/studio.ts`.

## Files changed

- `src/components/chat/ChatPanel.tsx` — new structure (header, empty thread,
  composer chips, consent gating); removed geometry-context code and the key
  drawer; the only new import wiring is `ComposerPopover` and the CAD display
  name helper.
- `src/components/chat/ComposerPopover.tsx` — **new**, local chip + popover.
- `src/components/chat/DesignAgentCard.tsx` — summary/disclosure, failure
  auto-expand; full body retained unchanged inside.
- `src/components/chat/CadTargetSelector.tsx` — only exported
  `cadProviderDisplayName` (used by the CAD chip); behaviour unchanged.
- `src/styles/chat.css` — rewritten to the thread + composer structure; dead
  rules (brand stamp, settings drawer, intro band, context table, route,
  provider-status line, granted-consent variant) removed; CAD target rules
  kept for the popover.
- `src/styles/composer.css` — chip row, chip states, popover panel, inline
  picker lists.
- `src/styles/studio.css` — old `chat-panel-*` block replaced by the bounded
  three-row `.chat-panel` base (all other chat rules now live in `chat.css`);
  removed the dead chat selectors from the single-column guard list.
- `test/ChatPanel.dom.test.tsx` — popover-based provider/model interaction,
  Settings › Models & keys assertion, plus new tests for starters/New chat,
  chip labels, popover open/close + focus, consent visibility.
- `test/DesignAgentCard.dom.test.tsx` — opens the disclosure for record-based
  assertions; new tests for collapsed summary, failure auto-expand and the
  not-reported summary; the old "geometry context" test now asserts the chat
  no longer duplicates Details › Properties.
- `test/chatJournal.dom.test.tsx` — connection assertions moved to the
  panel's `data-connection` state (the visible label is the status bar's).
- `test/providerFixtures.ts` — `pickProvider` opens the model chip first when
  the picker is inside the popover (still works standalone).

`ProviderPicker.dom.test.tsx`, `ModelPicker.dom.test.tsx` and
`CadTargetSelector.dom.test.tsx` needed no changes: the controls themselves
are unchanged.

## Verification actually run

All commands with a throwaway `HOME` / `KALA_*_DATA_DIR` / `XDG_DATA_HOME`.

- `npx tsc -p tsconfig.json --noEmit` — **0 errors**.
- `npx vitest run` on the seven owned files (`ChatPanel`, `ChatAgentCard`,
  `chatJournal`, `DesignAgentCard`, `ProviderPicker`, `ModelPicker`,
  `CadTargetSelector`) — **7 files, 116 tests, all passed**.
- `npx vitest run test/designSystem.test.ts -t "type scale"` — 5 passed,
  6 skipped (the whole file's other assertions belong to the shell/settings
  cells).
- `npx vite build --outDir /tmp/kala-composer-dist --emptyOutDir` — built.
- Served with `.mece/waves/app-feel-20261031/tools/serve.sh 8793
  /tmp/kala-composer-dist` and captured with `tools/shot.mjs` at 1440×900.

Full suite in the shared checkout at hand-off: **811 passed, 31 failed**, all
31 in whole-app files that still assert the old shell/options/bottom bar
(`AgentWorkspace`, `ManualConnect`, `designSystem`, `layoutCss`). Per WAVE.md
those are the integration step's; I did not edit them.

## Screenshots (`cells/composer/shots/`)

- `empty-1440.png` — empty thread: `AGENT`, one note, four starter tabs,
  prompt, model chip, vermilion `CAD: choose`, Draft.
- `popover-1440.png` — model popover open above the chip (provider trigger,
  model search, Free only, Reload, consent-ready model picker).
- `design-1440.png` — after Local helper + OpenCascade,
  prompt `L-bracket 100 x 80 x 60 mm, 8 mm wall`, Draft, 20 s wait. The
  backend answered `local · offline`; the collapsed card reads
  `✓ 5/5 checks · measured by OpenCascade (opt-in) 7.9.3` (the real count for
  that run, not the 6/6 example). The design also rendered on the sheet and
  the status bar shows `OpenCascade`.

Control count at 1440×900 in the visible app: empty screen 22 controls
(`tools/shot.mjs` count), down from the pre-wave 36 in `before/01-main.png`.

## Left over / notes for integration

- Whole-app tests above (`AgentWorkspace`, `ManualConnect`, `layoutCss`,
  `designSystem`) still encode the old shell and Options drawer; they need the
  integration pass. `layoutCss` also reads only `studio.css`, so it must point
  its `.chat-panel` row assertion at `auto minmax(0, 1fr) auto` and its
  `.bottom-bar` heights at the new 28 px status bar.
- At 360 px the two chips ellipsize long provider names
  (`Local Deterministic H…`); the full text is on each chip's `aria-label` and
  in the open pickers. No extra title attribute was added to avoid duplicate
  tooltip sources.
- Nothing else outstanding in the composer cell.
