# Worker session — coverage

Transport: T3 app-owned delegation (OpenCode worker, shared checkout)
Model: opencode-go/deepseek-v4.1-flash
Brief: `TASK.md` in this folder
Status: complete

## Scope observed

- Read `as1100-sheet-20261021/WAVE.md`, `SPEC.md`, `cells/qa/REPORT.md` and the
  final-wave `WAVE.md`.
- Compared the nine deleted test files and the trimmed `previewStatus` test
  (`git show HEAD:studio/test/<file>`) with the current tests and source.
- Wrote only `studio/test/**` plus this cell's `REPORT.md` and `SESSION.md`.
  Product source, backend, fixtures and packages were read-only.
- No commit, reset, clean, push, publication, credential read, remote API call
  or delegation. Tests ran with isolated `HOME`/data dirs under `mktemp`; the
  npm cache stayed at `/home/v9i/.npm`.

## Result

- 1109 → **1120** tests, 76 files unchanged.
- `npm run typecheck` pass; full `npm test` pass.
- 11 focused tests added, 2 existing tests strengthened.
- One source defect reported for a separate repair: edit turns do not detach
  the edit chip on `design_not_found` / `base_design_unavailable`, despite the
  WAVE contract. No source edit and no failing test were left behind.
