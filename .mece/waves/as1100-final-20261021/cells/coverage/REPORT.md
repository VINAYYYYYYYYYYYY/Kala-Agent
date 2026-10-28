# Cell coverage — removed-UI test audit and targeted repairs

Status: **complete**. Audited the 111 tests QA deleted from the AS 1100
redesign against the new homes, the current source and the old test bodies
(`git show HEAD:studio/test/<file>`). Added **11 focused tests** and strengthened
**2 existing tests** in `studio/test/**`. Product source, backend and every
other cell's files were left untouched. No commit, reset, clean, push,
publication, credential read, remote call or delegation.

## Gates (in `studio/`)

Isolation: fresh `mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; `npm_config_cache=/home/v9i/.npm`.
No real user data was read or written.

| Gate | Before this cell | After this cell |
|---|---|---|
| `npm run typecheck` | pass | **pass** (exit 0) |
| `npm test` | **1109 passed / 76 files** (QA result, re-measured) | **1120 passed / 76 files** |
| `npm run build` | not run here (no source change) | parent gate |

Net: **+11 tests, +0 files**. The two strengthened tests add assertions
without changing the count.

## Method

1. Read `as1100-sheet-20261021/WAVE.md`, `SPEC.md`, the qa `REPORT.md` and the
   final-wave `WAVE.md`.
2. Extracted every `it`/`test` title from the nine deleted test files and the
   two trimmed `previewStatus` tests.
3. Read the deleted bodies and compared each assertion with the current
   component/source and the replacement test. A moved *name* was not accepted
   as proof; the assertion had to exist.
4. Added a test only where a surviving behaviour had no assertion anywhere in
   `studio/test/**`. Retired surfaces were not recreated.
5. Ran the focused files, then the full suite and typecheck in the isolated
   environment.

## Gaps closed (old test → missing assertion → new test)

| # | Old coverage (deleted file) | Gap found in the new home | Added test |
|---|---|---|---|
| 1 | `ChatPanel.dom.test.tsx` "sends the model the operator picked…", "lets a subscription CLI send without a model id and still needs the consent stamp" | No test asserted the turn payload built from `routing`: `provider`, `model`, `allow_remote` and the CLI's null model. `apiClient.test.ts` only proves the client forwards a payload it is given; `turnReadiness` only proves the blockers. The consent stamp could regress without any failing test. | `chatTurn.dom.test.tsx` **"sends the routed provider, the pinned model and the egress consent, and a CLI without a model"** |
| 2 | `ChatPanel.dom.test.tsx` "cleans its timer up when the panel is unmounted mid-turn" (and "stops when the reply lands") | The pencil tested that the clock *ticks*, not that its interval ends on reply or unmount. `useTurnElapsed` could leak an interval with no failing test. | `chatTurn.dom.test.tsx` **"ticks while the turn runs and leaves no interval behind on a reply or an unmount"** |
| 3 | `DesignAgentCard.dom.test.tsx` "marks every check and prints the expected value beside the measured one", "renders a solid_count check generically", "marks a failed check with a cross" | The trail is the only home of the check list, but its tests asserted ordering/status only. No assertion covered the printed `expected`/`measured` values, the generic numeric + unit row, or the `✗` fail mark. | `ProcessTrail.dom.test.tsx` **"prints the expected and measured value of every check, with a cross on a failure"** |
| 4 | `DesignAgentCard.dom.test.tsx` "shows no STEP or native chip when the provider delivered no such artefact" | `ArtifactsPanel` omits the native pill when `native_kind` is null and prints an explicit empty digest, but nothing tested that no dead download chip or invented digest appears. | `ProcessTrail.dom.test.tsx` **"offers no native artefact and invents no digest when none was stored"** |
| 5 | `DesignAgentCard.dom.test.tsx` "shows the script, STEP, and native artefact digests" | The replacement asserted only the script and STEP digests; a regression that dropped the native (FCStd) digest would pass. | **Strengthened** `ProcessTrail.dom.test.tsx` "shows the digest of each artefact that has one" with the native digest |
| 6 | `DesignAgentCard.dom.test.tsx` "does not present a not-reported design as verified or failed" | No test asserted the trail status for `notReportedDesignResult`; a false `VERIFIED` or `CHECKS FAILED` over unreported data would pass. | `ProcessTrail.dom.test.tsx` **"never presents a not-reported design as verified or failed"** |
| 7 | `AuxView.dom.test.tsx` "shows a loading state naming the plane" | The mini-map has the loading state, but only the error/retry path was tested. The plane name could disappear from the loading copy unnoticed. | `MiniMap.dom.test.tsx` **"names the plane while the section loads"** |
| 8 | `AuxView.dom.test.tsx` "marks an approximate area with ≈ instead of presenting it as exact" | No mini-map test covered `area_exact: false`; the exact/approximate distinction (a D1 honesty rule) was unverified. | `MiniMap.dom.test.tsx` **"marks an approximate area with ≈ instead of presenting it as exact"** |
| 9 | `AuxView.dom.test.tsx` "draws a circle for a cylinder cut normal to its axis", "draws a great circle for a sphere" | Only the box primitive profile was tested. The section lens could stop rendering circles for the other two inspector kinds. | `MiniMap.dom.test.tsx` **"cuts a cylinder and a sphere into their real circles"** |
| 10 | `AuxView.dom.test.tsx` "shows an honest unavailable state for an unsupported primitive" | The mini-map's unavailable branch was untested; it could draw a rectangle or claim the ANSI hatch over undefined geometry. | `MiniMap.dom.test.tsx` **"says so when the primitive section is unavailable"** |
| 11 | `ChatAgentCard.dom.test.tsx` "shows the offline local route and the measured reply time" | Prompt history tested a remote badge and a CLI default badge, but not the `local · offline` badge. | `promptHistory.dom.test.tsx` **"names the offline local route in the badge"** |
| 12 | `chatJournal.dom.test.tsx` "reports the real browser fallback and disconnected states" | The engineer card tested only the desktop mode. `browser-manual` and the unwired state (the old "Disconnected"/NO LOOPBACK behaviour) were unverified in their new home. | `engineerCard.dom.test.tsx` **"names the browser fallback and the unwired bridge"** |
| 13 | `chatJournal.dom.test.tsx` "does not send Enter when the provider is not configured" (its body actually presses Enter on an empty prompt) | The pencil's Enter tests always had text. The empty-prompt guard was untested. | **Strengthened** `pencilPrompt.dom.test.tsx` "sends on Enter, keeps the newline on Shift+Enter…" with the empty-barrel case |

Evidence: every added test was written against the current implementation and
passes. The full-suite delta is exactly the 11 new `it` blocks; the two
strengthened tests add assertions in place.

## Removed coverage accepted as deliberate (not recreated)

These were checked against `WAVE.md`, `SPEC.md` and the qa report; none is a
silent loss.

| Removed behaviour | Why it is not covered now |
|---|---|
| Starter prompt tabs, "only seeds the composer" | Region #11 is removed by SPEC; the pencil-prompt cell decision (WAVE § Phase A) explicitly declined the port. No surviving caller. |
| `default`/`matte`/`wireframe` surface materials and their two controls | D17 retires the material field and keeps one finish state (`graphite`, `cadColor`, `wire`); `ViewSwitcher` tests cover the replacement. |
| The chat column, bottom bar, Options drawer/cards, AuxView tabs/tolerance panel | Regions #2–4, #6–12, #15–17 removed (D2, D13). Their functions have new homes; the relocation rows are covered as listed below. |
| Design-card mass row (reported mass and density-derived mass) | SPEC § 4F omits mass, the backend declares no density for the recorded design, and qa §9.3 records this as an accepted gap. No new UI asserts a mass. |
| "Design turns need the Python geometry backend" note (`design_available: false` / not reported) | The chat column and aux view that carried it are retired and WAVE assigns no new home; qa §9.3 records it. The picker/target selector and journal poller still honour the capability, and the mini-map/ortho sheet use honest empty/error states. Residual risk: the operator sees no explicit sentence explaining why design controls are absent on such a backend. |
| Primitive "Tolerance — no model / VALUES: NONE" panel | The tolerance aux tab is retired; WAVE moves only the *declared design* material/tolerances to trail Dot 1. The primitive had no tolerance data by definition. |
| Design-card "Section A–A" jump pill | The section is reachable as the mini-map Section lens; the old pill's design-activation side effect is no longer a product requirement. |
| Options-drawer card scrolling/corner ticks/duplicate-heading checks | Those were properties of the retired drawer layout; the engineer card is a modal with its own focus/scroll behaviour, tested in `engineerCard.dom.test.tsx`. |
| Bottom-bar fixed ENV/VOL/AREA/SCALE order and bottom-bar preview chip | The bar is retired; metrology lives in trail Dot 2 (its own grid/order tested) and the scale in `ScalePill`. |

No surviving behaviour from the WAVE relocation table was found unreachable
without a test after the additions below. No retired UI was recreated.

## Relocation rows re-checked

All rows have a live component and a current assertion after this cell:
composer send/cancel/clock/deadline/errors (pencil + `chatTurn`); provider,
model, free-model policy, consent and keys (engineer card, pickers,
`ProviderPanel`, `modelCatalog`); CAD target and blocker (engineer card,
`CadTargetSelector`, pencil); bridge/reconnect/manual connect (engineer card,
`ManualConnect`); exports and downloads (`designExport`, `ExportPanel`, trail);
primitive inspector (paper roll); checks/metrology (trail); section/mini-map
(`MiniMap`); ortho 3-view (`OrthoSheet`, `orthoLayout`, `orthoFeatures`);
declare-only tolerances and journal/audit (trail + `journalLedger`); tool
toggles/measure (`ToolStrip`, `MeasureTool`); finish/theme (`ViewSwitcher`,
engineer card); tabs (`TabRail`).

## Source defect found (report only — not repaired)

**Edit turns do not detach the edit chip after `design_not_found` (404) or
`base_design_unavailable` (502).**

- The contract in `as1100-sheet-20261021/WAVE.md` (line 224) says: "One extra
  code matters to the UI: `base_design_unavailable` (502). The UI treats it
  like `design_not_found` and detaches the edit chip."
- Frontend evidence: `grep -rn "base_design_unavailable\|design_not_found"
  studio/src` matches only a comment in `components/prompt/labels.ts`; nothing
  dispatches `edit/detach`. `PencilPrompt.tsx` clears `editDetached` only when
  `activeDesignId` changes, and `sendTurn` keeps sending `base_design_id` after
  a failed edit turn.
- Backend evidence: `kala-studio/src/server.rs` returns `design_not_found` for
  a missing base and `base_design_unavailable` for an unusable stored script.
- Impact: once the base design has been evicted or becomes unusable, every
  subsequent Draft repeats the same failing edit turn until the operator finds
  the `×` on the chip. The error line does show the code, so the failure is not
  silent — the promised automatic detach is simply absent.
- Suggested separate repair: on a failed turn whose code is either of those
  two, dispatch `edit/detach {detached: true}` (a `PencilPrompt` effect on the
  failure code, or the turn machine's error path). Add a regression test that
  sends an edit turn, rejects with 404/502, and asserts the chip is gone and
  the next payload has no `base_design_id`. This cell did not add a failing
  test or touch source.

No other source defect was confirmed by this audit.

## Changed paths (all owned)

- `studio/test/chatTurn.dom.test.tsx` (+2 tests)
- `studio/test/ProcessTrail.dom.test.tsx` (+3 tests, 1 strengthened)
- `studio/test/MiniMap.dom.test.tsx` (+4 tests)
- `studio/test/promptHistory.dom.test.tsx` (+1 test)
- `studio/test/engineerCard.dom.test.tsx` (+1 test)
- `studio/test/pencilPrompt.dom.test.tsx` (1 strengthened)

No product source, backend, electron/scripts, fixture or package file changed.
No test file was deleted or renamed.

## Residual risks / notes

1. The defect above is the only behavioural contract breach found; it is
   outside this cell's write scope.
2. The `design_available` explanatory sentence has no home (accepted above).
3. The mass row remains retired by SPEC; if a later spec restores it, the
   reported and density-derived cases from `DesignAgentCard.dom.test.tsx`
   should be resurrected with it.
4. The added mini-map tests use the primitive kinds the inspector can produce
   (`box`, `cylinder`, `sphere`); the code's unknown-kind fallback is defensive
   and intentionally not exercised through a cast.
5. Temporary run directory was removed; `/tmp/opencode` and the real Kala data
   directory were never touched.
