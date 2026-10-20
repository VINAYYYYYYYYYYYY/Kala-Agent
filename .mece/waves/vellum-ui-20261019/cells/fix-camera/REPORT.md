# REPORT — cell `fix-camera` (wave `vellum-ui-20261019`)

Fixes the two defects raised by the Opus review:

- **Bug 1 (high)** — the view snapped back and the GPU geometry was rebuilt on
  every unrelated re-render (journal append, aux-view tab, options drawer,
  theme switch, chat turn).
- **Bug 2 (medium)** — the Section A–A cutting plane was hard-coded at `z = 0`
  and labelled `Z = 0.00 mm · mid-plane`, which is only true for a mesh centred
  on the origin.

No commit, no push, no delegation, no credential reads. Only the owned files
were touched.

---

## 1. Files changed

| File | Change |
| --- | --- |
| `studio/src/components/shell/AppShell.tsx` | `displayMesh` is now built inside a `useMemo` keyed on `serverMesh`, `localMesh`, `primitive`, `parameters` only. The mesh object handed to the viewport is referentially stable across unrelated store changes. |
| `studio/src/viewer/SceneCanvas.tsx` | `applyMesh` re-frames the camera only when the bounds really changed (or on the first mesh); new `framedBounds` state; `resetView` still re-frames on demand and records the bounds it framed on; `rebuildSection` cuts at the real bounds mid-plane instead of `z = 0`. |
| `studio/src/viewer/draftingGeometry.ts` | New pure helpers `sectionPlaneZ`, `formatSectionPlaneLabel`, `boundsKey`, `shouldReframeBounds`; `computeSectionView` takes optional real bounds and labels the plane with the actual Z value; `SectionView` gained the optional `planeZMm` field. |
| `studio/src/components/drafting/DraftingSheet.tsx` | Section label only: `readSection` passes the mesh bounds to `computeSectionView`, and the cut note falls back to the real bounds mid-plane when the mesh carries no primitive descriptor. |
| `studio/test/AppShellMeshStability.dom.test.tsx` | **New** — DOM regression test for the mesh identity (4 tests). |
| `studio/test/draftingGeometry.test.ts` | +8 tests: real mid-plane labels with non-centred bounds, the no-bounds fallback, the no-negative-zero label, and the camera-framing predicate. |
| `studio/test/DraftingSheet.dom.test.tsx` | +1 test: the cut note names the real mid-plane for a mesh that is not centred. |

Nothing outside the owned list was edited. `electron/**`, `scripts/**`, the
backend, and the CSP were not touched.

---

## 2. What the contract now is

### `AppShell` → `DraftingSheet` → `SceneCanvas`

`displayMesh` identity is now a **stable contract**: it changes only when the
mesh source or the driving geometry changes. `SceneCanvas.applyMesh` therefore
runs only for a real geometry change, so the GPU buffers are not rebuilt and
the camera is not touched for unrelated workspace activity.

### Camera ownership (new rule, shared by viewport and tests)

```
shouldReframeBounds(previous, next):
  no next mesh            -> false   (nothing to frame)
  first mesh (no previous)-> true    (frame it once)
  bounds key changed      -> true    (frame the new bounds)
  bounds key unchanged    -> false   (keep the operator's camera)
```

`boundsKey` rounds every min/max value to `1e-6` (`toFixed(6)`), so float
noise is not a bounds change while a real move of the part is. The viewport
keeps `framedBounds` = "the bounds the camera is currently framed on"; it is
cleared when the mesh is cleared, so the next mesh frames again.

Material changes, theme changes, and grid/axes/dimension toggles never
re-frame. `resetView` (bottom-bar "Reset View" and the sheet's reset tool)
still re-frames on demand and updates `framedBounds` so a later mesh with the
same bounds does not fight it.

**Deliberate exception:** switching the render mode (`pencil2d` ↔ `clay3d`)
still re-frames, because the field of view changes from 24° to 38° and the part
has to be re-fitted. That is a deliberate operator action, not an unrelated
re-render, and the task did not list it among the cases to preserve.

### Section A–A plane (new rule)

```
sectionPlaneZ(bounds) = (bounds.min[2] + bounds.max[2]) / 2
```

- `SceneCanvas.rebuildSection` draws the redline plane at that Z.
- `computeSectionView(kind, parameters, bounds?)` labels it with the real
  value: `Z = 15.00 mm · mid-plane` (box), `· normal to axis` (cylinder),
  `· great circle` (sphere).
- With no bounds the primitive contract is used: every solid this studio
  builds is centred on the origin, so the plane is `Z = 0.00 mm`.
- The label never prints `-0.00 mm`.
- `SectionView.planeZMm` exposes the plane value in a machine-readable form.

`computeSectionView` gained an **optional** third argument, so the existing
two-argument call sites (notably `AuxView.tsx`, which this cell does not own)
keep compiling and keep their current behaviour.

---

## 3. Tests added

**`studio/test/AppShellMeshStability.dom.test.tsx` (new, 4 tests)** — the
viewport module is replaced by a recorder that captures the `mesh` prop, and
the boot gate is bypassed (both are covered by their own tests). A server
preview is dispatched first so the mesh is the *server* mesh, the one that was
being rebuilt on every render.

1. appending a journal row → the shell re-renders (the render count grows) and
   the viewport still holds the **same** mesh object;
2. switching the aux-view tab → same mesh object;
3. opening the options drawer and switching the theme → same mesh object;
4. control case: a genuinely new server preview **does** produce a new mesh
   object, so the identity assertions are not vacuous.

Verified to fail without the fix: with the old inline `displayMesh`
construction, tests 1–3 fail with
`expected { kind: 'box', …(9) } to be { kind: 'box', …(9) }` ("serializes to
the same string" — i.e. a new object with identical contents), and pass again
after restoring the memo.

**`studio/test/draftingGeometry.test.ts` (+8 tests)**

- `section cutting plane`
  - non-centred bounds `z ∈ [0, 30]` → `sectionPlaneZ === 15`, label
    `Z = 15.00 mm · mid-plane`, profile still 40 × 20 mm;
  - cylinder → `Z = 10.00 mm · normal to axis`; sphere →
    `Z = 10.00 mm · great circle` (both with non-centred bounds);
  - no bounds / `null` bounds → the centred-contract fallback
    `Z = 0.00 mm · mid-plane`;
  - `formatSectionPlaneLabel` never emits `-0.00` and keeps a real negative
    value (`Z = -12.50 mm · mid-plane`);
- `camera framing predicate`
  - re-frames the first mesh and any mesh whose bounds changed;
  - keeps the camera for identical bounds (including a fresh object with the
    same extents);
  - ignores float noise below one micrometre (`boundsKey` equal);
  - nothing to frame without a mesh.

**`studio/test/DraftingSheet.dom.test.tsx` (+1 test)** — a mesh with
`bounds.min[2] = 0`, `bounds.max[2] = 30` renders
`CUT A–A · Z = 15.00 mm · mid-plane`.

The pre-existing assertion `CUT A–A · Z = 0.00 mm · mid-plane` still holds for
the centred local box, and the `AuxView` assertions
(`Z = 0.00 mm · mid-plane`, `normal to axis`, `great circle`) are unchanged.

No behavioural coverage was deleted.

---

## 4. Gates

Run in `/home/v9i/Projects/Kala-Agent/studio`:

| Gate | Command | Result |
| --- | --- | --- |
| Type check | `npm run typecheck` | **green** — no output, exit 0 |
| Tests | `npm test` | **green** — 20 test files passed, **237 tests passed** (0 failed) |
| Build | `npm run build` | **green** — `✓ built in 357ms`, chunks emitted (`index`, `react`, `three`) |

Baseline before this cell: 19 files / 224 tests passing. After: 20 files /
237 tests passing (**+1 file, +13 tests**). The pre-existing chunk-size
warning from Vite (`three` chunk > 500 kB) is unchanged and unrelated.

---

## 5. Known gaps / follow-ups for other owners

1. **`AuxView.tsx` still shows the centred plane** (file not owned by this
   cell). It calls `computeSectionView(props.primitive, props.parameters)` and
   already receives `props.bounds`. One-line follow-up for the P2a owner:
   `computeSectionView(props.primitive, props.parameters, props.bounds)`.
   Behaviour is identical for every mesh this studio builds (all are centred
   on the origin), so nothing is wrong on screen today; it only matters for a
   mesh that is not centred.
2. **The SceneCanvas section plane has no DOM-level test.** `rebuildSection`
   lives inside `createScene` and needs a real WebGL context, which
   happy-dom does not provide. The arithmetic is covered by the
   `sectionPlaneZ` unit tests and the label is covered by the DraftingSheet
   DOM test; the remaining wiring is the single call
   `sectionPlaneZ(currentBounds)` in `rebuildSection`.
3. **Nothing is fake or stubbed by this cell.** All new numbers are real
   values derived from the real mesh bounds. No tolerance, verification, or
   hash-chain value was introduced.
