# A2 — Studio hidden-line drawing and section geometry

Cell: `a2-drawing`. Spec: ADR-0009 §3 and acceptance criterion 4.
Status: **done, library level**. No commit, no push, no delegation, no credential reads.

## Files

| File | Lines | Note |
|---|---|---|
| `src/kala_agent/studio/design/drawing.py` | 856 | new — HLR views + section loops |
| `tests/unit/studio_design/test_drawing.py` | 476 | new — 32 tests |

Nothing else was touched. `src/kala_agent/studio/design/__init__.py`,
`compile_ocp.py` and `measure.py` (A1) were read for context only, never
edited. A1 had already created `design/__init__.py` before this cell started,
so no empty placeholder was needed and no package file was created here.
The module imports nothing from `kala_agent.application`, from `adapters/`
or from `backend/`: it only imports OCP and the standard library.

## Public interface

```python
from kala_agent.studio.design.drawing import (
    hlr_views, section, default_section_offset,
    Drawing, ViewDrawing, Polyline, Bounds2D,
    Section, SectionLoop,
    ViewName, SectionPlane, DrawingError,
    DEFAULT_CHORD_TOLERANCE_MM, DEFAULT_SECTION_CHORD_TOLERANCE_MM,
    DEFAULT_VIEW_NAMES, MAX_POINTS_PER_VIEW, MAX_SECTION_POINTS,
)
```

### `hlr_views(shape, *, views=("front", "top", "right", "iso"), chord_tolerance_mm=0.05, max_points=20000) -> Drawing`

Runs `HLRBRep_Algo` + `HLRBRep_HLRToShape` once per view and returns the
visible and hidden pencil lines in **view-plane millimetres**.

| View | Viewer at | Horizontal axis `u` | Vertical axis `v` | Depth axis |
|---|---|---|---|---|
| `front` | −Y | +X | +Z | Y |
| `top` | +Z | +X | +Y | Z |
| `right` | +X | +Y | +Z | X |
| `iso` | (+X, −Y, +Z) | (1,1,0)/√2 | (−1,1,2)/√6 | (1,−1,1)/√3 |

The three orthographic views are the third-angle arrangement of ADR-0009.
The projection direction handed to OpenCascade points *from the part towards
the viewer*; this was verified against a box with a boss on one face (the
boss is visible only when the direction points at it).

`Drawing` holds `views: tuple[ViewDrawing, ...]`, `chord_tolerance_mm`,
`max_points`, plus `view(name)`, `view_names`, `total_point_count`.
`ViewDrawing` holds `name`, `visible: tuple[Polyline, ...]`,
`hidden: tuple[Polyline, ...]`, `truncated: bool`, plus `point_count` and
`polyline_count`. `Polyline` holds `points: tuple[tuple[float, float], ...]`
and `closed: bool`, plus `point_count`, `bounds` (`Bounds2D` with `width`,
`height`, `center_u`, `center_v`) and `length_mm`.

### `section(shape, plane="XZ", offset_mm=None, *, chord_tolerance_mm=0.01) -> Section`

Runs `BRepAlgoAPI_Section` against the plane and returns the closed loops of
the cut in the plane's own coordinates.

| Plane | Normal | `u` | `v` | Offset means |
|---|---|---|---|---|
| `XY` | +Z | X | Y | z = offset |
| `XZ` | +Y | X | Z | y = offset |
| `YZ` | +X | Y | Z | x = offset |

`offset_mm=None` (the default) is the mid-plane of the shape's measured
bounds normal to Y, which is the ADR's default section position;
`default_section_offset(shape)` returns that number on its own.

`Section` holds `plane`, `offset_mm`, `loops: tuple[SectionLoop, ...]`,
`chord_tolerance_mm`, `truncated: bool`, plus `outer_loops`, `inner_loops`,
`area_mm2` (net enclosed area, holes removed) and `point_count`.
`SectionLoop` holds `points`, `area_mm2` (always positive), `outer: bool`,
plus `bounds`, `centroid` and `point_count`.

Outer/inner classification is by containment: a loop inside a larger loop is
a hole, a loop that is not inside any larger loop bounds the material.
Nesting is handled by counting how many larger loops contain a loop, so a
loop inside a hole counts as material again.

### Conventions the numbers obey

* **Chord tolerance.** Curves are sampled with
  `GCPnts_QuasiUniformDeflection`, so a polyline never strays further than
  the tolerance from the true curve. Views use the ADR default of 0.05 mm.
  The section uses a tighter 0.01 mm because ADR-0009 pins the section's
  loop dimensions to ±0.05 mm and its area to 0.1 % — those are measured on
  the returned loops, so the section has to be sampled tighter than the
  sheet's display tolerance. With 0.05 mm the Ø10 loop's extent error is
  0.047 mm and the area error 0.042 %; with 0.01 mm they are 0.020 mm and
  0.009 %.
* **Point caps.** Every polyline starts and ends on the *exact* end points
  of its curve: the first and last samples are the caps of the edge, not
  approximations of them. A closed curve repeats its first point at the end
  so the polyline is explicitly closed, and a curve that collapses to a
  single point yields a one-point polyline instead of being dropped.
* **No OCP types escape.** Every public result is a frozen dataclass of
  plain floats and tuples; `dataclasses.asdict` serialises it directly for
  the HTTP layer.

## Measured results on the ADR L-bracket

The bracket is built in the test with OCP (`BRepPrimAPI_MakeBox` +
`BRepPrimAPI_MakeCylinder` + `BRepAlgoAPI_Fuse`/`Cut`): base
x∈[−30,30], y∈[0,45], z∈[0,8]; upright x∈[−30,30], y∈[0,8], z∈[0,40];
Ø10 bore along Y at x=0, z=24; 4× Ø5.5 base holes at x=±21.75,
y∈{16.25, 36.75}. Its measured volume is 35 571.416 047 mm³, the ADR
criterion 2 value.

**Criterion 4 — section by the XZ plane at y = 4:**

| Loop | Kind | Area | Extent | Centre | Points |
|---|---|---|---|---|---|
| 1 | outer | 2400.0000 mm² | 60.0000 × 40.0000 mm (u −30…30, v 0…40) | (0.0000, 20.0000) | 6 |
| 2 | inner | 78.3333 mm² | 9.9803 × 10.0000 mm | (0.0000, 24.0000) | 50 |

* exactly **one** outer loop and **one** inner loop,
* outer loop 60 × 40 within ±0.05 mm ✓, area 2400.0000 mm² ✓,
* inner loop Ø10 centred at (x=0, z=24) within ±0.05 mm ✓,
* net enclosed area **2321.6667 mm²** against the ADR's 2321.460 mm² →
  **+0.2067 mm² = +0.0089 %** (tolerance 0.1 %) ✓,
* the base holes (y = 16.25, 36.75) are missed, as the ADR requires: no
  other loop appears.

**Hidden lines of the bore in the front view:** the front view carries 8
visible and 21 hidden polylines. The Ø10 bore appears twice, as it must:
as a *visible* closed loop (the rim of the opening in the front face at
y = 0) and as a *hidden* closed loop (the far rim at y = 8), both centred on
(0, 24) in view-plane millimetres. The base holes contribute the hidden
lines at x = ±21.75 ± 2.75.

**View sizes** (chord tolerance 0.05 mm, no truncation):

| View | Visible | Hidden | Points |
|---|---|---|---|
| front | 8 | 21 | 206 |
| top | 11 | 20 | 268 |
| right | 10 | 32 | 292 |
| iso | 28 | 43 | 310 |

## Test coverage (32 tests)

* **Criterion 4 (7 tests)**: exactly one outer and one inner loop; the outer
  loop is the 60 × 40 rectangle (±0.05 mm, area 2400 mm²); the inner loop is
  the Ø10 bore at (0, 24) (±0.05 mm on the extent, the bbox centre and the
  area centroid); the net area is 2321.460 mm² within 0.1 %; the base holes
  are missed; the loops are closed chains; the section reports its plane,
  offset and tolerance.
* **Hidden lines of the bore (2 tests)**: the front view has hidden lines,
  one of them a closed Ø10 loop at (0, 24); the bore opening is also drawn
  as a visible closed loop.
* **The four views (6 tests)**: the drawing holds front/top/right/iso, all
  non-empty and untruncated; the front view's visible envelope is exactly
  x∈[−30,30], z∈[0,40]; the top view is measured in x and y (0…45); the
  right view is measured in y and z (0…40); the iso view is a real
  isometric projection of the part.
* **Sections of other parts and planes (6 tests)**: a box with a Ø10 through
  hole sectioned at y = 15 gives one 40 × 20 outer loop and one Ø10 inner
  loop; a plane that misses the part gives no loop and a zero area; the XY
  and YZ planes work; a YZ cut along the axis of the hole correctly splits
  the part into two outer loops of 30 × 5; an unknown plane is refused; the
  default offset is the mid-plane (22.5 mm) and a section without an offset
  uses it.
* **The public contract (10 tests)**: the results are plain frozen
  dataclasses of floats and tuples; a frozen result cannot be edited; a null
  shape is refused with `DrawingError`; the point caps are the exact ends of
  the curve and a closed polyline closes on itself; a tighter chord
  tolerance yields more points; an invalid chord tolerance is refused; the
  point limit truncates the view and says so; the default chord tolerances
  are the ADR's 0.05 mm and the section's 0.01 mm; the drawing is
  deterministic across two runs.

## Gates

| Gate | Command | Result |
|---|---|---|
| Tests | `.venv/bin/python -m pytest tests/unit/studio_design/test_drawing.py -q` | **32 passed** in 0.70 s |
| Lint | `.venv/bin/ruff check src/kala_agent/studio/design/drawing.py tests/unit/studio_design/test_drawing.py` | **All checks passed!** |
| Format | `.venv/bin/ruff format --check <same two paths>` | **2 files already formatted** |
| Types | `.venv/bin/mypy --python-version 3.12 <same two paths>` | **Success: no issues found in 2 source files** |
| Full unit suite | `.venv/bin/python -m pytest tests/unit -q` | **1352 passed, 1 warning** in 6.25 s |

The one warning is the pre-existing `StarletteDeprecationWarning` about
`httpx` vs `httpx2` raised by `fastapi.testclient`; it is not from this
cell.

**Pre-existing failure, not fixed here:** the repo-wide mypy gate
(`.venv/bin/mypy`, i.e. `files = ["src", "tests"]` from `pyproject.toml`)
fails before it checks any source file:

```
.venv/lib/python3.14/site-packages/numpy/__init__.pyi:737: error:
Type statement is only supported in Python 3.12 and greater  [syntax]
```

`pyproject.toml` pins `python_version = "3.11"` while the venv is Python
3.14 with numpy 2.5.4, whose stubs use the 3.12 `type` statement. The
failure reproduces with this cell's two files removed from the tree, and
A3's report records the same thing. `pyproject.toml` is not this cell's
file, so it was not changed. With `--python-version 3.12` (or 3.14) both
of this cell's files type-check clean under strict mode.

## Gaps and honest notes

1. **Criterion 4 is covered at library level only.** The `/drawing` and
   `/section` routes are B1's work; the ADR's "45° hatching between outer
   and inner loops" and the "HLR pencil lines" on the sheet are C1's. The
   module returns the loops and the lines, it does not draw them.
2. **The third-angle *arrangement* is not computed.** Each view is returned
   in its own view-plane millimetres with the origin at the global origin;
   placing the three views on the sheet (and scaling them) is the UI's job.
   The views are already in the right relative orientation.
3. **Hidden lines can coincide with visible ones.** The front view's
   visible seam at z = 8 (the edge on the front face) and the hidden edge
   at y = 8 project onto the same 2D line. Both are kept because they are
   different edges in 3D, so a UI that strokes hidden lines dashed over
   solid lines will show a dashed line on top of a solid one there. The
   kernel's output is reported as it is; suppressing the overlap is a
   drawing-style decision for the UI.
4. **The section keeps only closed wires.** The section edges are chained
   into wires with `ShapeAnalysis_FreeBounds.ConnectEdgesToWires_s` and a
   wire that does not close is left out, as ADR-0009 asks for closed loops.
   In every geometry exercised here (the L-bracket on XZ and XY, the box
   with a hole on XZ and YZ) the kernel produced closed wires only — 2, 5, 2
   and 2 respectively — so nothing was actually dropped. The count of
   dropped open chains is nevertheless **not** reported: `Section` has no
   `open_loop_count`, so a caller cannot tell a clean cut from a cut that
   had leftovers.
5. **The section's chord tolerance is 0.01 mm, not the ADR's 0.05 mm.**
   This is a deliberate choice (see the conventions above) and it is the
   only number in the module that is not the ADR's literal value. It is
   exposed as `DEFAULT_SECTION_CHORD_TOLERANCE_MM` and as a keyword
   argument, so B1 can tighten or loosen it.
6. **A drawn circle's extent is only as good as the chord tolerance.** In
   the views (0.05 mm) the Ø10 bore is drawn 9.9767 × 9.9534 mm, so the
   view test asserts the drawn bore within 0.1 mm rather than ±0.05 mm.
   The ±0.05 mm the ADR states is asserted on the *section* loop, which is
   sampled ten times tighter.
7. **Outer/inner classification uses the area centroid** for the
   containment test. That is exact for convex holes (the ADR case) but a
   pathological concave hole whose centroid falls outside the hole could in
   principle be classified against the wrong loop. Nested holes are handled
   by the containment depth.
8. **The `iso` view is one fixed standard isometric** from (+X, −Y, +Z).
   There is no user-selectable view direction, and no rear, left, bottom or
   rotated view.
9. **The point budget truncates, it does not adapt.** When a view exceeds
   20 000 points the module stops adding polylines and sets
   `ViewDrawing.truncated = True`; it does not coarsen the chord tolerance
   to fit. The same holds for the section (loops are dropped largest-first
   and `Section.truncated` is set). The polyline *length* is not bounded,
   only the point count.
10. **No `to_dict()`/schema helper.** The results are plain dataclasses so
    `dataclasses.asdict` works, but B1 has to write the pydantic schema and
    the flattening (e.g. turning `tuple[tuple[float, float], ...]` into a
    list of `{u, v}` points) for the API.
11. **The module needs OCP at import time**, like A1's compiler. The
    `design` package deliberately does not re-export it, so importing
    `kala_agent.studio.design` stays kernel-free.

---

# Repair 1 — the section areas are measured, not approximated

Review finding (Opus): `section()` reported `area_mm2` for the loops and the
net area of the section from the *discretized* polylines, so the UI would
show 2321.6667 mm² where the ADR's value is 2321.460 mm², and 78.3333 mm²
for a Ø10 bore whose area is 78.5398 mm². The ADR rule is that a displayed
number is measured by OCP, so the areas are now measured on the exact faces
of the cut.

No commit, no push, no delegation, no credential reads. Only
`src/kala_agent/studio/design/drawing.py` and
`tests/unit/studio_design/test_drawing.py` were touched; A1's and A3's files
were read for context only.

## What changed

| File | Before | After |
|---|---|---|
| `src/kala_agent/studio/design/drawing.py` | 856 lines | 1145 lines |
| `tests/unit/studio_design/test_drawing.py` | 476 lines, 32 tests | 576 lines, 35 tests |

**The areas are measured by OpenCascade on exact faces.**

* Every closed wire of the cut becomes a planar face
  (`BRepBuilderAPI_MakeFace(wire, OnlyPlane=True)`) and its area is measured
  with `BRepGProp.SurfaceProperties_s`. The area of the bore is therefore
  the area of the circle, not the area of the polygon that approximates it.
* The net area is measured on the face of every region of material: the
  face of the outer wire with the wires of its holes added. A hole wire is
  added *reversed* against the outer wire, which is how the kernel reads a
  hole; a hole wire with the same orientation would be read as a second
  region and the face would be the union of the two. An island inside a
  hole is a region of material of its own and is added back, so a nested
  cut is measured correctly.
* The polylines are kept for drawing only. The chord tolerance now bounds
  the *drawing*, not the area: the section's 0.01 mm tolerance is still
  what the loop dimensions are read off, but the areas no longer depend on
  it at all.

**No approximation is ever presented as a measurement.**

* `SectionLoop` gained `area_exact: bool` and `Section` gained
  `area_exact: bool`. The flags are set explicitly on every result the
  module builds and say whether the number is a kernel measurement.
* A face is only used when it describes the loop it was built from. Two
  guards reject a face the kernel built wrongly:
  * a wire that touches itself (a cut that grazes a hole) is not a simple
    loop — the face covers the regions the wire walks over, not the region
    it bounds. The wire is checked by counting the points its edges start
    at: a point that starts two edges means the wire touches itself;
  * the measured area must agree with the drawn loop within the area
    between the chords and the curves, which is bounded by the length of
    the loop times the chord tolerance. A face that disagrees by more than
    that does not describe the loop.
  When a face is rejected the area of the drawn polyline is used and the
  flag says `False`.
* The net area falls back to the signed sum of the loop areas (outer
  minus inner) when a net face cannot be built or does not agree with the
  loops it is built from. That sum is exact as long as every loop area was
  measured, and the flag says so.
* A section whose point budget dropped a loop is not exact either: the
  area no longer covers the whole cut, so `Section.area_exact` is `False`.

## Public interface (changes for B1)

```python
SectionLoop(points, area_mm2, outer, area_exact=False)
Section(plane, offset_mm, loops, area_mm2, chord_tolerance_mm, truncated=False, area_exact=False)
```

* `SectionLoop.area_exact` — **new field**: true when `area_mm2` is a
  kernel measurement, false when the area of the drawn polyline was used.
* `Section.area_mm2` — **changed from a property to a stored field**. It
  was the signed sum of the loop areas; it is now the area measured on the
  face of the net region. Same meaning, measured instead of approximated.
* `Section.area_exact` — **new field**: true when the net area is a
  kernel measurement of the whole cut.
* The field order of `Section` changed (`area_mm2` is now the fourth
  field). Construct `Section` with keywords.

## Measured results after the repair

**Criterion 4 — the L-bracket cut by the XZ plane at y = 4:**

| Loop | Kind | Area | Expected | Relative error | `area_exact` |
|---|---|---|---|---|---|
| 1 | outer | 2399.999 999 999 999 5 mm² | 2400 | 1.9e-16 | True |
| 2 | inner | 78.539 816 339 744 83 mm² | 25π | 0.0 | True |
| net | — | 2321.460 183 660 254 6 mm² | 2400 − 25π | 2.0e-16 | True |

The old numbers were 2400.0000 (polyline), 78.3333 (polyline) and
2321.6667 (polyline). The bore is now the area of the circle to the last
bit of a double.

**Other cuts, all measured:**

| Cut | Loops | Net area | Expected | `area_exact` |
|---|---|---|---|---|
| box 40×30×20 with a Ø10 hole, XZ at y = 15 | 800.0 outer + 78.5398 inner | 721.460 183 660 255 1 | 800 − 25π | True |
| the same box, YZ at x = 0 (the cut splits the part) | 2 × 149.999 999 999 999 97 | 299.999 999 999 999 94 | 300 | True |
| the L-bracket, XY at z = 5 (four base holes) | 2700.0 outer + 4 × 23.758 294 442 772 81 | 2604.966 822 228 909 | 2700 − 4·π·2.75² | True |
| a box with a Ø20 bore around a Ø8 island, XZ at y = 10 | 800.0 outer + 314.1593 inner + 50.2655 outer (island) | 536.106 217 098 457 3 | 800 − 100π + 16π | True |
| a plane that misses the part | none | 0.0 | 0 | True |

## Tests (35, was 32)

Tightened, as the review asked:

* the net area is 2400 − 25π with `rel=1e-9` (was `rel=1e-3`), and
  `cut.area_exact is True`;
* the inner loop's area is 25π with `rel=1e-9`, and
  `inner.area_exact is True`;
* the outer loop's area is 2400 with `rel=1e-9` (was `abs=0.05`), and
  `outer.area_exact is True`.

Added:

* `test_the_area_of_a_round_loop_is_the_area_of_the_circle` — the area of
  the bore differs from the area of the polygon the loop is drawn with, so
  the number cannot come from the polyline;
* `test_a_tangent_cut_reports_an_approximate_area` — a cut that grazes a
  hole gives a wire that touches itself; the module reports the drawn area
  and says `area_exact is False` rather than passing an approximation off
  as a measurement;
* `test_an_island_inside_a_hole_is_material_again` — a nested cut is
  measured as box − bore + island.

Also tightened: the box-with-hole cut and the cut that splits the part
(`rel=1e-9` instead of `rel=1e-3`, with the flags checked), and the
public-contract test now checks the types of the new fields.

## Gates

| Gate | Command | Result |
|---|---|---|
| Tests | `.venv/bin/python -m pytest tests/unit/studio_design/test_drawing.py -q` | **35 passed** in 0.75 s |
| Lint | `.venv/bin/ruff check src/kala_agent/studio/design/drawing.py tests/unit/studio_design/test_drawing.py` | **All checks passed!** |
| Format | `.venv/bin/ruff format --check <same two paths>` | **2 files already formatted** |
| Types | `.venv/bin/mypy --python-version 3.12 <same two paths>` | **Success: no issues found in 2 source files** |
| Full unit suite | `.venv/bin/python -m pytest tests/unit -q` | **1361 passed, 1 warning** in 6.33 s |

The one warning is the pre-existing `StarletteDeprecationWarning` about
`httpx` vs `httpx2` raised by `fastapi.testclient`; it is not from this
cell. The full-suite count also counts A1's and A3's files, which are
being written in parallel.

The repo-wide mypy gate still fails before it checks any source file, for
the pre-existing reason recorded above (`pyproject.toml` pins
`python_version = "3.11"`, the venv is Python 3.14 and numpy 2.5.4's stubs
use the 3.12 `type` statement). `pyproject.toml` is not this cell's file.

## Gaps and honest notes after the repair

1. **A cut that grazes a hole still reports a number that is not the
   material area.** The wire of such a cut touches itself, so the drawn
   chain walks over both regions and the area of the drawn loop is their
   union. The module reports that number with `area_exact = False`, so the
   UI can label it as an approximation, but it is not the area of the
   material. Splitting a self-touching wire into its simple loops would
   fix it; that is a larger change than this repair and is left open.
2. **The centroid of a loop is still the centroid of the drawn polyline**,
   not the centre of mass of the face. The ADR pins the areas and the
   extents, not the centroid, and the review was about the areas, so the
   centroid was left as it was.
3. **The measured area is the kernel's, to the last bit of a double.** The
   outer rectangle of the L-bracket measures 2399.999 999 999 999 5 mm²,
   not 2400: that is the value `BRepGProp` returns for the face, and the
   module reports what the kernel measured rather than rounding it.
4. **The consistency guards are geometric, not absolute.** A measured area
   is accepted when it agrees with the drawn loop within the loop's length
   times the chord tolerance. A caller who asks for a very coarse chord
   tolerance widens that band, and a caller who asks for a very fine one
   tightens it. The band never rejects the ADR case: the bore's drawn
   polygon is 0.21 mm² away from the circle and the band allows 0.31 mm².

