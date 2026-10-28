# Worker session — edit-recovery

Transport: T3 app-owned delegation (OpenCode worker, shared checkout)
Model: opencode-go/deepseek-v4.1-flash
Brief: `TASK.md` in this folder
Status: complete

## Scope observed

- Read the final-wave `WAVE.md` / `TASK.md`, the source contract
  `as1100-sheet-20261021/WAVE.md`, the backend refusal sites, the store's
  `edit/detach` and `chat/*` reducers, the pencil, and the existing
  `chatTurn` / `pencilPrompt` fixtures.
- Wrote only `studio/src/state/chatTurn.ts`, the new
  `studio/test/editBaseRecovery.dom.test.tsx`, and this cell's `REPORT.md` /
  `SESSION.md`. Existing tests and all other source were read-only.
- No commit, reset, clean, push, publication, credential read, remote API call
  or delegation. Tests ran under fresh `mktemp` `HOME`/data dirs; npm cache
  stayed at `/home/v9i/.npm`. Temp roots were removed.

## Result

- `design_not_found` (404) and `base_design_unavailable` (502) now detach the
  edit chip at the shared `sendTurn` layer, only when the failing turn still
  owns the machine and its base is still the active design.
- Error, journal/history and current design are preserved; no retry or extra
  remote call. Unrelated errors, plain drafts, changed designs and superseded
  turns do not detach.
- 7 focused tests added in the uniquely owned new file; 39 related existing
  tests still pass; `npm run typecheck` passes. Full suite/build left to the
  parent's final gates as briefed.
