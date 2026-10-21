# Cell ui-fixes: the necessary UI-only findings of audit-ui

Read `../../WAVE.md` (the rules; the UI invariants from `.mece/waves/vellum-ui-20261019/WAVE.md` § Invariants all apply, above all **no fake data**) and `../audit-ui/AUDIT.md` (each finding has evidence and a suggested fix).

## Fix these (Opus adjudication)
- **U-01 (MUST)** A live elapsed timer while a chat or design turn is in flight (mono, 1 s tick, cleaned up on unmount, `prefers-reduced-motion`-safe). Keep the existing post-reply elapsed chip consistent with it.
- **U-02 (MUST)** A client deadline for `postChat` through the existing `boundedSignal` pattern: `CHAT_TURN_TIMEOUT_MS = 30 * 60_000`, with a comment explaining it covers the backend worst case (two model calls of up to 180 s each, plus two CAD builds whose deadline is being re-sized by the design-budget cell). It exists only to unstick a wedged backend. On expiry, show an honest timeout message distinct from a user cancel.
- **U-04 (MUST)** The ledger summary shows a `chat.turn` `failure` code (e.g. "failed: provider_timeout") instead of reading as a normal design request.
- **U-05 (MUST)** When the reply model is `default` (or empty) for `claude_code`, `codex_cli` or `opencode_cli`, show it honestly, e.g. "Codex CLI default model (chosen by Codex)", with a hint that picking a model in the model picker pins and shows it. Do the same in the picker's default row. Never invent a model name: the CLIs do not report it.
- **U-07 (SHOULD)** For a CLI provider whose list is unavailable, give the real reason (the CLI was not found or is unavailable, with the provider's own `reason` text if the API gives one), not "not configured server-side". Do not offer "Open API keys" for CLI providers.
- **U-08 (SHOULD)** View-provenance labels say what is true: the view is the OpenCascade import of the provider's STEP (view-only), e.g. "VIEW: OCC IMPORT OF FREECAD STEP" / "view: OpenCascade import (view-only) of the FreeCAD STEP". For the opt-in OpenCascade build: "…import of the OpenCascade (opt-in) STEP". Include the old `SOURCE: OCP HLR` drawing label: it should say it is an OpenCascade hidden-line view of the imported model.
- **U-09 (SHOULD)** "Clear key" must not claim success for an environment-sourced key. Use the settings row's source field. If the API does not expose the source, STOP on this item and report the exact field needed.
- **U-10, U-11 (SHOULD)** Render the stored server-preview error and the journal-poll error (real messages, in the existing visual language, dismissible or self-clearing on recovery).
- **U-12 (SHOULD)** Show ONE Codex row: when both `codex` (the historical alias) and `codex_cli` are listed, hide the alias in the picker (keep it accepted if a stored selection references it, mapping to `codex_cli`).
- **U-13 (SHOULD)** Keyboard access to the 3D viewport: focusable (`tabIndex=0`), visible focus ring, arrow keys orbit, +/- zoom, `0` resets, and an aria label naming the keys. Keep `role` semantics correct.

NOT in this cell: U-03 (the FCStd download needs a backend route) and U-06 (backend cancel). Those come later. Skip U-14..U-16.

## Rules
Own `studio/src/**` and `studio/test/**` only. Do not touch `studio/electron/**`, `studio/scripts/**`, `electron-builder.json`, or any backend file. Add tests for every behaviour (fake timers for U-01/U-02). Never delete behavioural coverage.
Gates: `npm run typecheck`, `npm test` (baseline 46 files / 683 tests), `npm run build`, all green. No commits. Clean up /tmp.

## REPORT
`REPORT.md` here: per finding, what changed (files) and the test that proves it, the gate counts, and anything stopped or left open.
