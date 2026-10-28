# Cell edit-recovery — stale edit-base recovery

Status: **complete**. Repaired the missing edit-base recovery contract in the
shared turn machine and added focused regression coverage in a new test file.
Only `studio/src/state/chatTurn.ts`, the new
`studio/test/editBaseRecovery.dom.test.tsx`, and this cell's `REPORT.md` /
`SESSION.md` were written. No existing test file, other source file, backend,
fixture, package or wave document was touched. No commit/reset/clean/push,
remote call, credential read or delegation.

## Contract consumed

- `.mece/waves/as1100-sheet-20261021/WAVE.md:224`: `base_design_unavailable`
  (502) "matters to the UI: the UI treats it like `design_not_found` and
  detaches the edit chip."
- `backend/crates/kala-studio/src/server.rs`: `design_not_found` is the 404
  refusal of a `base_design_id` the store does not hold;
  `base_design_unavailable` is the 502 refusal when the stored design cannot
  be read as a base. Both are exact `StudioApiError` status+code pairs.
- Final-wave task brief: detach only for an edit request whose failure still
  belongs to the current turn and whose base is still the active design.

## Source change (`studio/src/state/chatTurn.ts`)

- Added `STALE_EDIT_BASE_STATUS` (`design_not_found` → 404,
  `base_design_unavailable` → 502) and `isStaleEditBaseError`, which requires
  a `StudioApiError` with the exact status+code pair.
- In the non-aborted `sendTurn` catch path, ownership is read **before**
  `chat/error` clears the clock: `activeTurn === turn` and
  `chat.turnStartedAt === startedAt`.
- After the normal error dispatch and journal row, `edit/detach` is
  dispatched only when all of these hold:
  - the input carried a non-null `baseDesignId` (a real edit request);
  - the cause is one of the two exact stale-base refusals;
  - the failing turn still owns the turn machine;
  - `designs.activeDesignId` is still that same base design.
- Nothing else changes: the real error message/kind, the journal, the user
  row with its `baseDesignId`, and the current design remain. There is no
  retry and no further `postChat` call. Cancellations and timeouts keep their
  existing early paths and never detach. A draft without a base, an unrelated
  server error, a changed active design, or a superseded turn leaves the chip
  attached.

## Regression tests (`studio/test/editBaseRecovery.dom.test.tsx`, NEW)

Actual turn machine with only `postChat` mocked, `bracketDesignResult` fixture
from `designFixtures.ts`, and the real `PencilPrompt` for the integration case:

1. `design_not_found` (404) detaches, while the error, user history/base id,
   journal row and active design survive; exactly one transport call.
2. `base_design_unavailable` (502) detaches the same way.
3. The next pencil draft after a 404 detaches the chip, keeps the visible
   error, and sends **no** `base_design_id` the second time (first payload
   still carries the base).
4. An unrelated `provider_unavailable` (503) keeps the chip attached.
5. The same 404 code on a draft that carried no base keeps the chip attached.
6. A failure arriving after the operator activated another design does not
   detach that new design; the error is still reported.
7. A late 404 from a turn superseded by a newer turn does not detach the chip
   or disturb the newer turn's reply.

## Gates (run in `studio/`)

Isolation: fresh `mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`; `npm_config_cache=/home/v9i/.npm`.
Temp roots removed afterwards. No real user data or credentials.

| Gate | Result |
|---|---|
| `npx vitest run test/editBaseRecovery.dom.test.tsx` | **7 passed / 1 file** |
| `npx vitest run test/chatTurn.dom.test.tsx test/pencilPrompt.dom.test.tsx` | **39 passed / 2 files** |
| `npm run typecheck` | **pass** (exit 0) |

The full suite and build were intentionally left to the parent's final gates,
per the brief.

## Known gaps / notes

- The shared checkout is dirty and `studio/src/state/chatTurn.ts` is untracked
  in git, so `git diff` does not show the edit; the parent should review the
  file itself. The added test file is also untracked.
- Status+code matching is deliberately strict. The backend emits exactly
  404/`design_not_found` and 502/`base_design_unavailable`; a same-named code
  with another status is treated as unrelated and does not detach.

## Requests

None. No contract change is needed.
