# UI rollback — execution report

- Project: `/home/v9i/Projects/Kala-Agent`
- Manifest: `.mece/waves/ui-revert-20261021/manifest.json`
- Recorded HEAD / current HEAD: `2ca57bbb56b23e359813025f90c0e79ebfa77824` (unchanged)
- Scope: `studio/src`, `studio/test` only. No commit, no staging, no reset, no `git clean`, no broad directory deletion, no tests/build.

## Pre-flight checks

- `tracked_restore` = 52 entries; `.mece/waves/ui-revert-20261021/tracked-paths.txt` = 52 lines, same set and same order.
- `new_delete` = 78 entries; all 78 present before the run.
- Every path in both lists is under `studio/src/` or `studio/test/` (0 out-of-scope paths).
- HEAD before the run matched the manifest head.

## Action 1 — restore tracked files (worktree only)

```
git restore --source=2ca57bbb56b23e359813025f90c0e79ebfa77824 \
  --worktree --pathspec-from-file=.mece/waves/ui-revert-20261021/tracked-paths.txt
```

Result: 52 files restored from the recorded HEAD into the worktree. Index untouched (no `--staged`).

## Action 2 — delete new (untracked) files

Deleted individually, exactly the 78 paths in `new_delete`, via per-path file removal.

- Deleted: 78
- Already absent: 0
- Files present from `new_delete` after the run: 0

## Action 3 — empty directories

Removed only directories that our deletions emptied, and only because they are then useless empty trees in git:

- `studio/src/components/dock`
- `studio/src/components/ortho`
- `studio/src/components/prompt`
- `studio/src/components/titleblock`
- `studio/src/components/tabs`

Left alone (pre-existing empty directories, contain no manifest paths): `studio/src/app`, `studio/src/components/viewer`, `studio/src/utils`. No directory under `studio/test` was removed.

## Verification (actual output)

| Check | Result |
| --- | --- |
| `git diff -- studio` | empty |
| `git diff --stat -- studio` | empty |
| `git status --short -- studio` | 0 lines |
| `git ls-files --others --exclude-standard -- studio` | 0 lines |
| `git rev-parse HEAD` | `2ca57bbb56b23e359813025f90c0e79ebfa77824` (unchanged) |
| staged index (`git diff --cached \| sha256sum`) | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` — matches manifest `index_sha256` (empty diff) |
| `preserved_product_sha256` | 20/20 match, 0 mismatches |

## Untouched (outside owned scope)

`git status --short` outside `studio/` is unchanged: 17 modified `backend/**`, `docs/**`, `src/kala_agent/**` files plus 6 untracked paths (3 wave directories under `.mece/waves/` and the 3 preserved-but-untracked product files `backend/crates/kala-studio/src/providers/usage.rs`, `backend/crates/kala-studio/tests/edit_turns.rs`, `docs/adr/0012-edit-turns-and-usage.md`). Their content was not modified by this cell; the product files still hash to the manifest values.

No tests, builds, packaging, staging, or commits were performed — reserved for the parent.
