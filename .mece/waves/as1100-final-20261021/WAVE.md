# AS1100 final verification — 2026-10-21

**Superseded by the user's UI rollback request. Do not continue this redesign or its review follow-ups.** The Studio UI is being restored to pre-redesign HEAD under `../ui-revert-20261021/`; backend work is retained. The user requires no commit.

Resume after the previous provider's rate limit. Preserve the entire dirty working tree. Source contract: `../as1100-sheet-20261021/WAVE.md` and `SPEC.md`.

Two shared-checkout OpenCode workers run concurrently through T3 app-owned delegation:
- coverage: DeepSeek V4.1 Flash. Audit removed UI tests against new homes; own only `studio/test/**` additions/repairs and its cell report. Product source read-only.
- backend-review: MiniMax M3.1 Flash Free. Review edit-turn/usage backend diff and run isolated meaningful backend checks. Product source and tests read-only; own only its report.

Parent reviews real diffs, runs frontend gates, checks live behavior, builds and smoke-tests AppImage. No commits, reset, push or publication. No real user data writes. Credentials never printed.

Acceptance: no lost reachable behavior in the redesign; frontend typecheck/tests/build pass; relevant Rust checks pass; real CAD draft/edit/export verified where available; packaged AppImage smoke passes.

## Follow-up cells

- whitespace (DeepSeek): removed AppShell's extra trailing blank line; `git diff --check` passes. No behavior change.
- coverage verified 111 removed tests against actual new assertions, added 11 tests and strengthened two; full suite 1120/1120 passes in 76 files. Found a source contract breach: missing/unusable edit-base errors never detach the edit chip.
- edit-recovery (DeepSeek): owns `studio/src/state/chatTurn.ts` and NEW `studio/test/editBaseRecovery.dom.test.tsx` only. Existing test files stay with coverage. Fix stale edit recovery with guards for a different active design or newer turn; retain real error and never retry automatically.

Parent live model/edit/nudge/export evidence is in `LIVE-VALIDATION.md`. First package build and native OpenCascade/FreeCAD smoke pass; package must be rebuilt after edit-recovery lands.
