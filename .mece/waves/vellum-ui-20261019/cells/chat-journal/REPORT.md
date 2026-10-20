# P2b chat-journal — report

Status: **complete**. The left-zone calculation-pad chat and the right-bottom Agent & Kernel ledger are implemented in `studio/`. No commit, push, deploy, credential read, or further delegation was performed. No `electron/**`, `scripts/**`, `index.html`, or viewer/drafting files were touched.

## Files changed

New:

- `studio/src/styles/chat.css` — taped starter tabs, stamped header, calculation-pad table, message cards, consent stamp, composer.
- `studio/src/styles/journal.css` — ledger rows, kind-coloured tags, scroll body, jump control, live region, footer.
- `studio/test/chatJournal.dom.test.tsx` — 12 new DOM tests (see below).

Modified:

- `studio/src/components/chat/ChatPanel.tsx` — header stamp/connection dot, starter tabs, mono active-geometry table, message cards with role/time, Enter/Shift+Enter, `[Draft Part Geometry ↵]`, consent stamp / real route status, and a sticky thread that follows the newest turn unless the operator scrolls up. All existing send/cancel/clear/API-key/error behaviour retained.
- `studio/src/components/journal/JournalPanel.tsx` — `AGENT & KERNEL JOURNAL` header, honest chain stamp, mono rows, sticky autoscroll + jump control, throttled `aria-live="polite"` region, empty state, real origin/version footer.
- `studio/test/AgentWorkspace.dom.test.tsx` — old-layout assertions only: header title, composer placeholder/button label, journal footer origin, and `SESSION LOG` instead of `HASH-CHAIN —`. P2a concurrently updated the AuxView ortho assertion in this file.
- `studio/test/ManualConnect.dom.test.tsx` — old footer text `Connected to` replaced by the real journal-footer origin.

Not modified: `studio/src/components/shell/AppShell.tsx`, `studio/src/state/store.ts`, `studio/src/main.tsx`, `studio/src/types/studio.ts`. The journal reads `server.version` from the shared store directly; no shell or store patch was needed. The two stylesheets are imported from their owning components so no shared entry file had to be edited.

## Contract added

`JournalPanelProps` is unchanged and binding:

```ts
export interface JournalPanelProps {
  entries: ReadonlyArray<JournalEntry>;
  connectionStatus: ConnectionStatus;
  hashChainStatus: HashChainStatus; // only 'verified' renders HASH-CHAIN OK
}
```

`ChatPanel` still takes no props. Stable test/DOM hooks added:

- Chat: `.chat-panel[data-connection="desktop|browser|connecting|disconnected|error"]`, `.chat-panel-stamp`, `.chat-panel-connection`, `.chat-panel-starter[data-starter]`, `.chat-panel-context-table`, `.chat-panel-message-meta`, `.chat-panel-message-time`, `.chat-panel-consent[data-consent="required|granted"]`, `.chat-panel-provider-status`.
- Journal: `.journal-panel-stamp[data-chain="unknown|verified|broken"]`, `.journal-body`, `.journal-list`, `.journal-jump`, `.journal-live`, `.journal-panel-footer`.
- Preserved existing hooks: `#chat-provider-select`, `#chat-model-input`, `.chat-panel-prompt`, `.chat-panel-send`, `.chat-panel-cancel`, `.chat-panel-settings-toggle`, `.chat-panel-settings`, `.chat-panel-empty`, `.journal-list`.

## Gate results

Run from `/home/v9i/Projects/Kala-Agent/studio`:

| Command | Result |
| --- | --- |
| `npm run typecheck` | clean (exit 0) |
| `npm test` | **18 test files passed, 220 tests passed** |
| `npm run build` | success, 50 modules transformed; only the pre-existing >500 kB `three` chunk warning |

P2b added 12 tests: starter-tab seeding/no-autosend, Enter send, Shift+Enter newline, Enter with empty prompt, three real connection labels, chat thread stickiness (pinned vs scrolled up), ledger row/tag/digest rendering, real origin/version footer, empty state, honest unknown/verified/broken stamps, throttled live announcement, and journal sticky autoscroll + jump-to-latest. The full-suite count includes the P2a cell's tests, which were green at the final run.

## Visual smoke check

A throwaway local backend was started on **127.0.0.1:8766** with a non-secret test token and the built `studio/dist`, then driven with headless Chromium over CDP. The already-running :8765 backend was not touched and no token/credential file was read.

- **1440×900**: doc/shell/chat/journal all had `scrollWidth === clientWidth` (no horizontal overflow); left chat 339 px, right journal 319 px.
- **1280×800**: chat 279 px, journal 259 px, still zero horizontal overflow; the provider/model row collapses to one column. A second real turn sent at this width pinned the chat thread to the newest card (`scrollTop 595 = scrollHeight 928 − clientHeight 333`).
- **Vellum and Slate** both applied (`[data-theme]` switched), with the real footer `http://127.0.0.1:8766 · backend v0.1.0`.
- Real local-helper turns produced message cards and journal rows; the journal stayed at the newest row and the header stamp read `SESSION LOG`.

Screenshots (throwaway, outside the repo): `/tmp/opencode/chat-journal-1440.png`, `/tmp/opencode/chat-journal-1280.png`, `/tmp/opencode/chat-journal-slate-1440.png`.

## Known gaps / intentionally not faked

1. **No inline agent result tables or digest chips.** The backend `ChatResponse` has only `text`, `provider`, `model`, and `offline`; it carries no structured parameters, tool calls, estimates, or digests. The mono parameter table in the chat is therefore the **real active workbench geometry** (driving dimensions, closed-form envelope, closed-form volume) from `computeGeometryEstimate`, not a parsed or invented agent result.
2. **Journal categories are the frozen store kinds**, not the Stitch palette names: `preview`, `chat`, `tool`, `export`, `connection`, `system`. `connection` is shown as `[BRIDGE]`. No `plan`/`sketch`/`policy`/`error` rows are fabricated because the store has no such event kinds; system/connection errors appear under those real kinds.
3. **`HASH-CHAIN OK` is unreachable in this build.** `AppShell` still passes `hashChainStatus="unknown"` and there is no backend hash-chain endpoint, so the header shows the neutral `SESSION LOG` stamp. The verified/broken branches render only if a real backend report is wired through the existing prop.
4. **Chat autoscroll is additive and has no "jump to latest" control.** It pins to the newest turn while the operator is within 24 px of the bottom and releases when scrolled up; only the journal (per the brief) has a jump control. This is covered by the chat stickiness test.
5. **Remote consent is the existing real gate.** The consent stamp appears only for `openai`/`openrouter`/`codex` (the panel's existing `REMOTE_PROVIDERS`) and is wired to the same `allow_remote` checkbox; the local helper instead shows the real provider/route status (`configured`/`not configured`, `loopback only`).
6. **No mic/sketch icons** were added, because no such capture features exist.
7. **Backend version is real-or-empty**: `server.version` from the store, rendered as `backend v—` before the registry/health response arrives. No PID, latency, keytar, or FreeCAD version claims are made in these two panels.
8. **Styles are additive.** The P1 sketch rules remain in `studio.css`/`layout.css`; the new rules are scoped under `.chat-panel` / `.journal-panel` (with higher specificity) so they win regardless of bundle order. No P1 stylesheet was edited.
