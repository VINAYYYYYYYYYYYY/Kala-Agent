# Cell pencil-prompt (Phase B): the drafting-pencil prompt bar, its history, and the living graphite string

Read `../../WAVE.md` in full: D1, D2, D16, the relocation table, the contract (including amendment 1, routing state) and the rules. Read `../../SPEC.md` § 4D. Read `../sheet-shell/REPORT.md` for the frozen props, `state/chatTurn.ts` and `components/shell/anchors.ts`. Read the old `components/chat/ChatPanel.tsx`: every composer behaviour it has must exist in the pencil.

## Do
1. **`PencilPrompt.tsx`** (centred, 20 px above the bottom rule, `width: min(660px, 60vw)`, 44 px high). It has three parts.
   - **Ferrule and eraser cap (48 px, knurled band):**
     - `↺` opens Prompt History;
     - Clear empties the barrel (the draft only).
   - **Hexagonal barrel (flex):**
     - The input, with twin bevel hairlines and the placeholder "Describe a part, assembly, or modification to draft…". It is bound to `promptDraft`.
     - Enter sends, Shift+Enter is a newline, and IME composition never sends. The barrel grows upward to about 6 lines while the text wraps.
     - A mono character counter appears from 3500 of 4096.
   - **Conical nib (52 px):** a tapered wood-and-graphite cone ending in the graphite point `▶`. It is the Draft/Submit button.
     - Its accessible name is "Draft". Disabled with the reason as its tooltip when `turnReadiness` is not ready.
     - Register the `pencil-tip` anchor on the point.
2. **Blocker chip.** In the pencil EVERY `turnReadiness` blocker is hard, including `cad-target`: the nib stays disabled. (`sendTurn` keeps `cad-target` soft only for the legacy panel; Opus decision.) When not ready, a chip at the collar names the real blocker and opens the engineer card on the right section:
   - CAD target → `commandCard/open {section:'cad'}`;
   - consent, provider or model → `'routing'`;
   - connection → `'cad'`.
3. **Edit chip (D16).** When a design is active, the provider is not `local`, and `editDetached` is false: show `EDITING ‹label› ×` in the barrel.
   - × dispatches `edit/detach {detached:true}`.
   - Any change of the active design re-attaches (`edit/detach false`).
   - A send while attached carries `baseDesignId`.
4. **In flight.**
   - The collar shows the live elapsed timer (mono, 1 s tick, from `chat.turnStartedAt`).
   - It shows two redline pills: `✕ Abort` (`abortTurn`) and `✎ Edit Prompt` (abort, put the sent prompt back in the barrel, focus it).
   - After a turn, a compact dismissible error line above the pencil shows the real message. A timeout reads differently from a user cancel.
5. **Prompt History popover** (focus-trapped, Esc closes). The whole thread, newest first. Each entry shows:
   - the prompt;
   - the full reply text (scrollable);
   - the honest provider and model label (reuse today's default-model label helpers);
   - the duration;
   - tokens, when `usage` was reported;
   - a failure code, when there is one;
   - `Show design` (activates it) when the turn built one;
   - `Re-use prompt`.
   - At the bottom, `Clear history` (with a confirm) calls `chat/clear`, as today's Clear did.
6. **`GraphiteString.tsx`** (a viewport-fixed SVG overlay, `pointer-events:none`).
   - **Idle:** nothing.
   - **Generating** (`chat.inFlight`):
     - A hand-drawn graphite path from the `pencil-tip` rect (right-edge centre) to the `trail-port` rect: a cubic curve that sweeps across into the dock. Use deterministic jitter seeded per turn and a ~600 ms draw-in.
     - At the trail-port end, an endless organic circular scribble.
   - **Success:** the scribble pulls taut. The path animates into a straight dotted line (`• ── • ── ◉`), which fades after about 1.2 s.
   - **Failure:** the string snaps (a short redline break), then fades.
   - Track anchor rects live with `useAnchorRect`. Pause rAF when the document is hidden.
   - **`prefers-reduced-motion`:** a static dashed line while generating, nothing afterwards.
7. **Tests:**
   - send, Enter / Shift+Enter / IME, Clear;
   - the blocker chip for each blocker, and its card section;
   - the edit chip: attach, detach, re-attach on design change, `baseDesignId` sent;
   - Abort and Edit Prompt (fake timers);
   - the elapsed timer;
   - history entries (default-model label, usage, failure, Show design, Re-use, Clear with confirm);
   - the GraphiteString states, including reduced motion (mock anchor rects).

## Own
- `studio/src/components/prompt/**` (`PencilPrompt.tsx`, `GraphiteString.tsx`, `PromptHistory.tsx` and helpers).
- The new `studio/src/styles/prompt.css`, and the tests of these.
- You may import from `components/chat/**`, but do not edit it (title-block owns it).
- The store, types, AppShell and `chatTurn.ts` are frozen: use "Requests" in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Other cells edit concurrently: if a failure is in a file you do not own, re-run once, then report it. No commits. Clean up /tmp. Write `REPORT.md`.
