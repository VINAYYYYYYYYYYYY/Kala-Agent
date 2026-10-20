# Fix: camera snaps back and GPU rebuild on unrelated re-renders (found by Opus review)

Own ONLY: `studio/src/components/shell/AppShell.tsx` (the displayMesh construction), `studio/src/viewer/SceneCanvas.tsx`, `studio/src/viewer/draftingGeometry.ts`, `studio/src/components/drafting/DraftingSheet.tsx` (only the section label), and the matching tests in `studio/test/`. Same rules as `../../WAVE.md`.

## Bug 1 (high): view resets on every journal append, tab switch, or drawer open
`AppShell.tsx` (~line 60) builds `displayMesh` inline with `new Float32Array(...)` on every render, and AppShell subscribes to `journal`, `auxView`, `optionsOpen`, `theme`, and more. Every re-render passes a new `mesh` object to SceneCanvas. Its `useEffect([props.mesh])` then calls `applyMesh`, which rebuilds all GPU geometry AND calls `frameOnBounds`. The result: the user orbits, sends a chat message or clicks a tab, and the camera snaps back.
Fix (both parts):
1. `useMemo` the displayMesh, keyed on `serverMesh`, `localMesh`, `primitive`, and `parameters` only.
2. In SceneCanvas `applyMesh`, re-frame the camera only when the bounds actually change (compare a bounds key, say values rounded to 1e-6) or on the first mesh. Keep the user's camera when only the material, theme, or identical bounds change. `resetView` still re-frames on demand.
Regression tests: (a) a DOM test where AppShell re-renders because of an unrelated store change (append a journal entry, switch auxView) and the SceneCanvas mesh prop identity stays the same; (b) a unit test of the bounds-change predicate.

## Bug 2 (medium): the section plane is hard-coded at z = 0 and labelled "mid-plane"
`SceneCanvas.rebuildSection` uses `const z = 0`, and `draftingGeometry.ts` labels it `Z = 0.00 mm · mid-plane`. That is only true for meshes centred on the origin. Use the real bounds mid-plane `z = (z0 + z1) / 2` in both places, and label it with the actual value (for example `Z = 15.00 mm · mid-plane`). Add a test with non-centred bounds.

## Gates
`npm run typecheck && npm test && npm run build`, all green. Write `REPORT.md` in this folder: diff summary, tests added, and gate counts. No commit, delegation, or edits outside the owned files.
