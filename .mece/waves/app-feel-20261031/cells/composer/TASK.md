# Cell composer: the chat sidebar becomes a clean thread + composer

Read `../../WAVE.md` first. It holds the decisions, the contract and the rules. Look at `../../before/01-main.png`, `03-apikeys.png` and `05-design.png`. The left column is what you are cleaning.

## Build (in `components/chat/*`)
1. **Header.**
   - One slim row: the label `AGENT` (mono caps, as today's labels) and a `New chat` button, shown only when the thread has messages. It dispatches `chat/clear` with the same semantics as today's Clear.
   - Remove the brand box: the wordmark moved to the top bar (cell shell).
   - Remove the "BROWSER FALLBACK" line: connection moved to the status bar.
   - Remove the `API Keys` button and the inline `chat-panel-settings` key drawer: keys now live in Settings › Models & keys.
2. **Empty state.** When the thread is empty, show the 4 existing starter prompts as chips inside the thread area, with the same text and the same behaviour (they fill or send exactly as today), plus one short Newsreader-italic line. They disappear once a message exists and return after New chat. No separate "starters" band above the thread.
3. **Remove the Active geometry section** (`chat-panel-context`) from the chat. Cell shell moves it into Details › Properties.
4. **Composer** (bottom of the column; the only setup the user sees):
   - The prompt textarea and the Draft button: keep the label text `Draft Part Geometry` and its ↵ behaviour.
   - One row of two chip buttons under the prompt:
     - `[● <provider label> · <model or "default"> ▾]` (`aria-label="Model: …"`) opens a popover holding the existing `ProviderPicker` + `ModelPicker`, unchanged inside.
     - `[CAD: <OpenCascade|FreeCAD|Solid Edge|choose> ▾]` (`aria-label="CAD target: …"`) opens a popover holding the existing `CadTargetSelector`.
   - When `cadProvider` is null, the CAD chip reads `CAD: choose` in the existing vermilion attention style. Today's "first design turn asks you to choose" flow must still work.
   - Popovers: anchored above the chip, `role="dialog"`, close on Esc and on outside click, focus returns to the chip. Use a small local component (`components/chat/ComposerPopover.tsx`); no new dependency.
   - The remote-provider consent stamp stays visible in the composer **only when it is required** (a remote provider is selected and has not been consented).
   - Remove the `ROUTE …` line and the "Pick a configured provider…" help paragraph.
5. **Design result card (`DesignAgentCard`).**
   - Collapsed by default to a one-line summary: `✓ 6/6 checks · measured by OpenCascade (opt-in) 7.9.3`, or `✗ 2 failed …`, with the real counts and source.
   - A disclosure button (`aria-expanded`) reveals today's full card: the checks, SCRIPT/STEP copy/download and `Section A-A ↗`.
   - Failures auto-expand.
6. **Styles.** Put new rules in `styles/composer.css`. Delete the dead `chat-panel-*` rules (settings drawer, starters band, context, route) from `chat.css` and `studio.css`. Same tokens and look as today.

## Tests you own
- Update `ChatPanel`, `ChatAgentCard`, `chatJournal`, `DesignAgentCard`, `ProviderPicker`, `ModelPicker` and `CadTargetSelector` to the new structure: open the popover, then interact. Keep every behaviour assertion.
- Add tests for:
  - starters only when empty, and back after New chat;
  - the chip labels reflect the store;
  - the popover opens and closes (Esc, outside click);
  - the card is collapsed by default and auto-expands on failure;
  - the consent stamp shows only when required.

## Done when
- No type errors in your files, and your tests are green.
- `shots/empty-1440.png`: an empty thread.
- `shots/popover-1440.png`: the model popover open.
- `shots/design-1440.png`: after a Local helper + OpenCascade L-bracket turn. Pick OpenCascade in the CAD popover, type `L-bracket 100 x 80 x 60 mm, 8 mm wall`, press Draft and wait 15 s.
- REPORT.md is written.
