# Cell tab-rail (Phase B) — report

The `#13a` paper-roll assembly tabs above the border, the pinned component
sub-tabs and the primitive tab.

## Gates

Every gate ran in `studio/` with `RUSTUP_HOME=/home/v9i/.rustup`,
`CARGO_HOME=/home/v9i/.cargo` and a fresh `mktemp -d` for `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`.
`/home/v9i/.local/share/kala-agent` was never touched, no credential/auth/
settings file was read, no real `claude`/`codex`/`opencode` CLI was run, and
the temp dir was removed at the end.

| Gate | Result |
|---|---|
| `npm run typecheck` | **pass** (exit 0). |
| `npm test` | **1184 passed / 1184** in **86 files** (the sheet-shell baseline was 813/813 in 59 files). This cell adds **42 tests** in 3 files; all pass. |
| `npm run build` | **pass** (exit 0; only the pre-existing three.js chunk-size warning). |

The typecheck and the build were blocked several times while the other Phase B
cells were mid-edit — `test/orthoNudge.test.ts` (ortho-sheet),
`src/viewer/SceneCanvas.tsx` (canvas-3d) and
`src/components/dock/ProcessTrail.tsx` (dock) each failed it in turn. Those
files are not this cell's; each failure was re-run and then reported, never
fixed here. The final run of all three gates is green.

## Files changed

### Created (owned)

- `studio/src/components/tabs/TabRail.tsx` — the rail itself: one tab per
  design of the store's LRU in LRU order (label truncated with a tooltip, the
  part-count badge `[n]`, the chevron, the `×`), the PRIMITIVE tab, the
  `[ + New Sheet ]` trigger, the pinned sub-tab row, the roving tabindex and
  the arrow-key navigation.
- `studio/src/components/tabs/PaperRoll.tsx` — the unfurling roll: a design's
  real component list, or today's parametric inspector under the PRIMITIVE
  tab.
- `studio/src/components/tabs/tabScript.ts` — the defensive reader of
  `design.script` (`readScriptComponents`, `readAssemblyRelations`,
  `singleComponentScript`, `primitiveTabLabel`, the sub-tab key helpers).
- `studio/src/components/tabs/subTabBuild.ts` — D12: pin a component, build
  it alone, report building/ready/error, retry.
- `studio/src/styles/tabs.css` — the rail, the index tabs, the pinned
  sub-tabs and the paper roll (unroll `cubic-bezier(0.22, 1, 0.36, 1)`,
  paper-curl shading and ruled lines as inline SVG textures,
  `prefers-reduced-motion` respected).
- Tests: `studio/test/TabRail.dom.test.tsx` (19), `studio/test/PaperRoll.dom.test.tsx` (6),
  `studio/test/tabScript.test.ts` (14).

### Modified (owned)

- `studio/src/components/inspector/Inspector.tsx` — added the **Reset
  parameters** row (the frozen `parameters/reset` action with
  `defaultParameters(kind)`); today the inspector had a kind switch that
  reset implicitly, but no explicit reset, and the roll needs one. Nothing
  else in the inspector changed: the kind switch, the vernier sliders, the
  bounds readout and the estimates behave exactly as before (the existing
  `test/VernierSlider.dom.test.tsx` coverage still passes).

### Not touched

`studio/src/state/store.ts`, `studio/src/state/scriptBuild.ts`,
`studio/src/types/studio.ts`, `studio/src/components/shell/AppShell.tsx`,
`studio/electron/**`, `studio/scripts/**`, `electron-builder.json`,
`index.html`. No new runtime dependency. No commits.

## Contract consumed

- **Types** (`studio/src/types/studio.ts`, frozen): `SubTab`,
  `DesignResult` (with `script`, `component_count`, `label`, `provider`,
  `measured_by`, `measured`), `ConstructionScriptRequest`,
  `PrimitiveKind`, `StudioParameters`, `BoxParameters`, `CylinderParameters`,
  `SphereParameters`.
- **Store** (frozen): read `designs.order` / `designs.designs` /
  `designs.activeDesignId`, `subTabs`, `activeSubTab`, `sheet`, `primitive`,
  `parameters`. Dispatched: `design/activate` (through `activateDesign`),
  `design/close` (through `closeDesign` — amendment 2; the backend copy is
  never deleted), `subTab/pin`, `subTab/built`, `subTab/error`,
  `subTab/unpin`, `subTab/activate`, `prompt/draft`.
- **Helpers** (frozen): `buildScript({script, provider, label, activate})`
  from `state/scriptBuild.ts`, `defaultParameters` from
  `viewer/localPrimitives.ts`, `useUiState` / `getState` / `dispatch` from
  `state/store.ts`.
- **Frozen props honoured**: `TabRail` still takes **no props** and is
  mounted once by AppShell in the top margin. `nav.tab-rail` with
  `aria-label="Sheet tabs"` and the `SHEET · <title>` label are kept, so
  `test/shellStubs.dom.test.tsx` stays valid.

## What the rail does

1. **Design tabs.** One tab per design of the store's LRU, in the store's
   `order` (least recently used first). The label is truncated with the full
   label as its tooltip, the badge is the design's own `component_count`, the
   chevron unfurls the roll, the `×` calls `closeDesign(designId)` — the tab
   leaves the sheet, the backend keeps the design.
2. **The active tab** is raised, its bottom rule is open and it sits on the
   sheet's own rule, so the line runs through it.
3. **The PRIMITIVE tab** appears when no design is active and is labelled
   from the real parameters (`BOX 40×20×30`, `CYLINDER ⌀40×40`,
   `SPHERE ⌀25`).
4. **`[ + New Sheet ]`** clears the active design (`clearActiveDesign`), drops
   the active sub-tab and, only when the pencil draft is empty, writes the
   real design starter into `prompt/draft` — it never overwrites what the
   operator has typed.
5. **Keyboard.** The rail is a `role="tablist"` with a roving tabindex: the
   selected tab is the only one in the Tab order, Arrow/Home/End move the
   focus, Enter/Space activate.
6. **The paper roll** unfurls below the tab with the unroll easing
   `cubic-bezier(0.22, 1, 0.36, 1)`, the paper-curl shading and the ruled
   lines (inline SVG, no CSS gradient, no drop shadow) and no animation at
   all under `prefers-reduced-motion: reduce`. A design roll lists the
   components of `design.script.components[].name` with each component's step
   count and its FUSED / PLACED mark (read from `script.assembly`). The
   PRIMITIVE roll holds today's inspector.
7. **Pinned sub-tabs (D12).** Clicking a component in the roll pins a mini
   sub-tab `└─ <name> ×` under its parent tab (several side by side). The
   pin dispatches `subTab/pin` and builds the component alone through
   `buildScript` with exactly
   `{summary, components:[that component], assembly:[{op:'place',component:name}]}`,
   the parent's own `provider` and `activate:false`; the result is
   `subTab/built`, a refusal is `subTab/error` with the backend's own
   message. A ready sub-tab shows the component's real measured result
   (provider, size, volume) in its tooltip and switches the sheet with
   `activateDesign(childDesignId)` + `subTab/activate {key}`; the parent tab
   switches back with `subTab/activate null`. A building sub-tab shows its
   progress, a failed one shows the message and a **Retry**. Pinning the same
   key twice never rebuilds.

## Known gaps

1. **A pinned component also appears as a full rail tab.** The frozen
   `buildScript` with `activate:false` keeps the built design in the store's
   LRU, so the child design shows up in the tab order next to the parent's
   pinned sub-tab. The sub-tab is the operator's shortcut; the LRU is the
   session record. **Solved in the rail in Follow-up 1** (the child is hidden
   from the tab list while its sub-tab is pinned).
2. **The LRU reads least-recently-used first.** The rail shows the store's
   `designs.order` as it is, so the active design is the right-most tab. One
   line changes it to most-recent-first. **Solved in Follow-up 1**: the rail
   keeps its own first-seen order, so no tab moves when another is activated.
3. **The sub-tab chips hang about 8 px into the drafting field.** The 40 px
   top margin is sheet-shell's geometry; the chips are absolutely positioned
   inside their tab column so they escape the rail's horizontal scroll
   clipping, but they still overlap the sheet's top rule. The visual result
   needs the Phase D live check.
4. **The roll is a popover without a focus trap.** It takes the focus when
   it opens, closes on Escape (focus returns to the tab), on its own Close
   button and on a click elsewhere (the focus stays where the operator
   clicked), and the tab that opened it toggles it. It is not modal, so no
   trap is needed.
5. **The inspector's surface-material control is still inside the roll.** It
   writes `viewport.material`, which the store maps onto `finish`; the View
   Switcher (D17) owns the finish. Nothing is duplicated or contradictory,
   but the material row is a legacy leftover of the primitive inspector.
   **Solved in Follow-up 1**: the roll's inspector no longer renders the row.
6. **`prompt/draft` is only written when the draft is empty.** The starter
   never replaces a draft the operator has already written.
7. **Gate blockers outside this cell, seen while the other cells were
   mid-edit** (re-run and reported, never fixed here): `test/orthoNudge.test.ts`
   (ortho-sheet) cast the frozen `ConstructionScriptRequest` to a mutable
   shape and needed `as unknown as`; `src/viewer/SceneCanvas.tsx`
   (canvas-3d) passed `{width,height}` where `{x,y}` was expected;
   `src/components/dock/ProcessTrail.tsx` (dock) referenced an undeclared
   `downloadError`; `test/ProviderPanel.dom.test.tsx` (title-block) failed
   three API-key tests while that cell moved the panel. All three gates are
   green in the final run.

## Requests

1. ~~**A pinned sub-tab's child design stays in the LRU.**~~ **Solved in the
   rail in Follow-up 1** — no contract change: the rail reads `subTabs` and
   hides every design that is a pinned sub-tab's `childDesignId`. The child
   stays in the store's LRU (the session record) and is reached through its
   sub-tab; unpinning it makes it a normal tab again.
2. **A "retry" action for an errored sub-tab.** `subTab/pin` refuses a key
   that already exists, so the retry path has to `subTab/unpin` and then
   `subTab/pin` again (which also clears `activeSubTab` if it pointed at that
   tab). A `subTab/retry {key}` (or a `subTab/pin` that resets an errored
   tab to `building`) would make the retry exact. **Declined by Opus** — the
   unpin + pin retry stays as it is.
3. **Nothing else is needed from the store, the types or AppShell.** The
   rail works with the frozen contract as it stands: `design/activate`,
   `design/close`, `subTab/*`, `prompt/draft` and `buildScript` are enough.

---

## Follow-up 1 (Opus review, accepted with three small follow-ups)

All three were solved inside this cell's own files; **no contract change was
needed**.

### 1. A pinned component is shown only as a sub-tab (Request 1, solved in the rail)

The rail already reads `subTabs`, so it now derives the set of designs that
are a pinned sub-tab's `childDesignId` and leaves them out of the tab list.
The child stays in the store's LRU — it is the session record — and is
reached through its sub-tab. Unpinning the sub-tab makes the child a normal
tab again (it keeps its first-seen position).

While the sheet shows such a child, the **parent tab stays raised** and its
sub-tab carries the active mark: the operator always sees which tab the
sheet belongs to.

Files: `studio/src/components/tabs/TabRail.tsx`
(`childDesignIds`, `activeChildParentId`).

### 2. The tabs no longer jump around (Gap 2)

The rail keeps its **own first-seen order** of the session's designs: a
component-local ref maps a design id to the sequence in which the rail first
saw it, a design that left the store drops out of the map, and a design that
arrives later appends at the right. The store's LRU still moves when a
design is activated; the rail does not follow it. The active tab is shown by
its raised style, not by its position.

The helper `stableDesignOrder(designs, firstSeen, sequence)` is exported from
`TabRail.tsx` so the rule is testable on its own.

File: `studio/src/components/tabs/TabRail.tsx`.

### 3. The primitive roll no longer repeats the surface material (Gap 5)

The finish lives in the View Switcher (D17), so the roll's inspector does
not render the surface-material row. `Inspector` gained the optional prop
`showSurfaceMaterial` (default `true`, so the legacy Options card is
unchanged); the paper roll passes `showSurfaceMaterial={false}`. The rest of
the inspector — the kind switch, the vernier sliders, the reset, the bounds
and the estimates — is untouched, and
`studio/src/components/panels/SurfaceMaterialControl.tsx` (another cell's
file) is not touched at all.

Files: `studio/src/components/inspector/Inspector.tsx`,
`studio/src/components/tabs/PaperRoll.tsx`.

### Tests (this cell: 42 in 3 files, all passing)

- `test/TabRail.dom.test.tsx` — the tab order is the rail's first-seen order;
  **activating an older tab does not reorder** (the store's LRU does move,
  the rail does not); a design that arrives later appends at the right; a
  pinned component's child design is **not** a tab while its sub-tab is
  pinned and becomes one again when it is unpinned; the parent tab stays
  raised while the sheet shows the child; the primitive roll has **no**
  surface-material row.
- `test/PaperRoll.dom.test.tsx` — the primitive roll carries the inspector
  and **not** the surface material; the design roll, the honest empty list
  and the close action are unchanged.

### Gates after Follow-up 1

| Gate | Result |
|---|---|
| `npm run typecheck` | **pass** (exit 0) |
| `npm test` | **1184 passed / 1184** in **86 files** |
| `npm run build` | **pass** (exit 0; only the pre-existing three.js chunk-size warning) |

The gates were run in `studio/` with `RUSTUP_HOME=/home/v9i/.rustup`,
`CARGO_HOME=/home/v9i/.cargo` and a fresh `mktemp -d` for `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; the temp
dir was removed afterwards. `/home/v9i/.local/share/kala-agent` was never
touched, no credential/auth/settings file was read, no real
`claude`/`codex`/`opencode` CLI was run, and nothing was committed.

### Requests after Follow-up 1

None. Request 2 (a `subTab/retry` action) is declined, so the retry stays
the unpin + pin path; the store, the types and AppShell are untouched.
