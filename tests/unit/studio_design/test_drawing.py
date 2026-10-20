"""Tests for the Studio hidden-line drawing and section geometry.

The shapes are built directly with OCP in this module so the drawing tests
do not depend on the construction compiler: the drawing code is exercised
against geometry whose dimensions the test states up front.

The L-bracket built here is the part of ADR-0009 criterion 2:

* base plate ``x in [-30, 30]``, ``y in [0, 45]``, ``z in [0, 8]``;
* upright ``x in [-30, 30]``, ``y in [0, 8]``, ``z in [0, 40]``;
* a bore of diameter 10 along Y at ``x = 0``, ``z = 24``;
* four base holes of diameter 5.5 along Z at ``x = +-21.75`` and
  ``y in {16.25, 36.75}``.

Criterion 4 is checked on the section of that part by the XZ plane at
``y = 4``: one outer 60 x 40 loop, one inner diameter-10 loop centred at
``(0, 24)``, and an enclosed area of ``2400 - 25*pi = 2321.460 mm^2``.

The areas are *measured* by OpenCascade on the exact planar faces of the
cut, not approximated from the drawn polylines, so the expectations below
are tight to a relative 1e-9: the area of the bore is the area of the
circle and the net area is the area of the face with the hole cut out of
it. Every area carries an ``area_exact`` flag that says whether the number
is a kernel measurement, and the tests check that flag as well.
"""

from __future__ import annotations

import math

import pytest
from OCP.BRepAlgoAPI import (  # type: ignore[import-untyped]
    BRepAlgoAPI_Cut,
    BRepAlgoAPI_Fuse,
)
from OCP.BRepPrimAPI import (  # type: ignore[import-untyped]
    BRepPrimAPI_MakeBox,
    BRepPrimAPI_MakeCylinder,
)
from OCP.gp import (  # type: ignore[import-untyped]
    gp_Ax2,
    gp_Dir,
    gp_Pnt,
)
from OCP.TopoDS import TopoDS_Shape  # type: ignore[import-untyped]

from kala_agent.studio.design.drawing import (
    DEFAULT_CHORD_TOLERANCE_MM,
    DEFAULT_SECTION_CHORD_TOLERANCE_MM,
    Drawing,
    DrawingError,
    Polyline,
    Section,
    SectionLoop,
    ViewDrawing,
    default_section_offset,
    hlr_views,
    section,
)

BRACKET_WIDTH_MM = 60.0
BRACKET_DEPTH_MM = 45.0
BRACKET_HEIGHT_MM = 40.0
BRACKET_WALL_MM = 8.0
BORE_DIAMETER_MM = 10.0
BORE_CENTRE_Z_MM = 24.0
BASE_HOLE_DIAMETER_MM = 5.5
BASE_HOLE_X_MM = 21.75
BASE_HOLE_Y_MM = (16.25, 36.75)
SECTION_OFFSET_MM = 4.0
EXPECTED_SECTION_AREA_MM2 = 2400.0 - 25.0 * math.pi


def _box(
    x0: float,
    y0: float,
    z0: float,
    dx: float,
    dy: float,
    dz: float,
) -> TopoDS_Shape:
    """Return an axis-aligned box with its minimum corner at ``(x0, y0, z0)``."""
    return BRepPrimAPI_MakeBox(gp_Pnt(x0, y0, z0), dx, dy, dz).Shape()


def _cylinder(
    axis: tuple[float, float, float],
    origin: tuple[float, float, float],
    radius: float,
    height: float,
) -> TopoDS_Shape:
    """Return a cylinder with the given axis, origin, radius and height."""
    frame = gp_Ax2(gp_Pnt(*origin), gp_Dir(*axis))
    return BRepPrimAPI_MakeCylinder(frame, radius, height).Shape()


def build_l_bracket() -> TopoDS_Shape:
    """Return the ADR L-bracket as one fused solid."""
    base = _box(-30.0, 0.0, 0.0, BRACKET_WIDTH_MM, BRACKET_DEPTH_MM, BRACKET_WALL_MM)
    upright = _box(-30.0, 0.0, 0.0, BRACKET_WIDTH_MM, BRACKET_WALL_MM, BRACKET_HEIGHT_MM)
    shape = BRepAlgoAPI_Fuse(base, upright).Shape()

    bore = _cylinder((0.0, 1.0, 0.0), (0.0, -1.0, BORE_CENTRE_Z_MM), BORE_DIAMETER_MM / 2.0, 10.0)
    shape = BRepAlgoAPI_Cut(shape, bore).Shape()

    hole_radius = BASE_HOLE_DIAMETER_MM / 2.0
    for hole_x in (-BASE_HOLE_X_MM, BASE_HOLE_X_MM):
        for hole_y in BASE_HOLE_Y_MM:
            hole = _cylinder((0.0, 0.0, 1.0), (hole_x, hole_y, -1.0), hole_radius, 10.0)
            shape = BRepAlgoAPI_Cut(shape, hole).Shape()
    return shape


def build_box_with_through_hole() -> TopoDS_Shape:
    """Return a 40 x 30 x 20 box with a diameter-10 hole along Y."""
    box = _box(-20.0, 0.0, 0.0, 40.0, 30.0, 20.0)
    hole = _cylinder((0.0, 1.0, 0.0), (0.0, -1.0, 10.0), 5.0, 40.0)
    return BRepAlgoAPI_Cut(box, hole).Shape()


def build_box_with_an_island_in_a_bore() -> TopoDS_Shape:
    """Return a 40 x 30 x 20 box with a diameter-20 bore around a diameter-8 island.

    The island is a solid cylinder inside the bore, so the cut that crosses
    it has three loops: the outside of the box, the bore and the island
    inside the bore. The island is material again, so the net area is the
    box minus the bore plus the island.
    """
    box = _box(-20.0, 0.0, 0.0, 40.0, 30.0, 20.0)
    bore = _cylinder((0.0, 1.0, 0.0), (0.0, -1.0, 10.0), 10.0, 40.0)
    shape = BRepAlgoAPI_Cut(box, bore).Shape()
    island = _cylinder((0.0, 1.0, 0.0), (0.0, 5.0, 10.0), 4.0, 10.0)
    return BRepAlgoAPI_Fuse(shape, island).Shape()


@pytest.fixture(scope="module")
def l_bracket() -> TopoDS_Shape:
    """Return the ADR L-bracket."""
    return build_l_bracket()


@pytest.fixture(scope="module")
def box_with_hole() -> TopoDS_Shape:
    """Return a box with a through hole."""
    return build_box_with_through_hole()


@pytest.fixture(scope="module")
def box_with_island() -> TopoDS_Shape:
    """Return a box with a bore that carries an island."""
    return build_box_with_an_island_in_a_bore()


@pytest.fixture(scope="module")
def bracket_drawing(l_bracket: TopoDS_Shape) -> Drawing:
    """Return the four views of the L-bracket."""
    return hlr_views(l_bracket)


def _loop_near(loops: tuple[SectionLoop, ...], u: float, v: float) -> SectionLoop:
    """Return the loop whose bounding box centre is closest to ``(u, v)``."""
    return min(
        loops,
        key=lambda loop: math.hypot(
            loop.bounds.center_u - u,
            loop.bounds.center_v - v,
        ),
    )


# --- ADR-0009 criterion 4: the section of the L-bracket ---


def test_section_has_exactly_one_outer_and_one_inner_loop(
    l_bracket: TopoDS_Shape,
) -> None:
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)

    assert len(cut.outer_loops) == 1
    assert len(cut.inner_loops) == 1
    assert len(cut.loops) == 2


def test_outer_loop_is_the_60_by_40_rectangle(l_bracket: TopoDS_Shape) -> None:
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)
    outer = cut.outer_loops[0]

    assert outer.bounds.width == pytest.approx(60.0, abs=0.05)
    assert outer.bounds.height == pytest.approx(40.0, abs=0.05)
    assert outer.bounds.min_u == pytest.approx(-30.0, abs=0.05)
    assert outer.bounds.max_u == pytest.approx(30.0, abs=0.05)
    assert outer.bounds.min_v == pytest.approx(0.0, abs=0.05)
    assert outer.bounds.max_v == pytest.approx(40.0, abs=0.05)
    assert outer.area_mm2 == pytest.approx(2400.0, rel=1e-9)
    assert outer.area_exact is True


def test_inner_loop_is_the_diameter_10_bore(l_bracket: TopoDS_Shape) -> None:
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)
    inner = cut.inner_loops[0]

    assert inner.bounds.width == pytest.approx(BORE_DIAMETER_MM, abs=0.05)
    assert inner.bounds.height == pytest.approx(BORE_DIAMETER_MM, abs=0.05)
    assert inner.bounds.center_u == pytest.approx(0.0, abs=0.05)
    assert inner.bounds.center_v == pytest.approx(BORE_CENTRE_Z_MM, abs=0.05)
    assert inner.centroid[0] == pytest.approx(0.0, abs=0.05)
    assert inner.centroid[1] == pytest.approx(BORE_CENTRE_Z_MM, abs=0.05)
    assert inner.area_mm2 == pytest.approx(25.0 * math.pi, rel=1e-9)
    assert inner.area_exact is True


def test_enclosed_area_matches_the_adr_value(l_bracket: TopoDS_Shape) -> None:
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)

    assert cut.area_mm2 == pytest.approx(EXPECTED_SECTION_AREA_MM2, rel=1e-9)
    assert cut.area_exact is True


def test_the_area_of_a_round_loop_is_the_area_of_the_circle(
    l_bracket: TopoDS_Shape,
) -> None:
    """The area is measured on the face, not summed from the drawn polygon."""
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)
    inner = cut.inner_loops[0]

    polygon_area = abs(
        sum(
            inner.points[index][0] * inner.points[(index + 1) % len(inner.points)][1]
            - inner.points[(index + 1) % len(inner.points)][0] * inner.points[index][1]
            for index in range(len(inner.points))
        )
        / 2.0
    )

    assert polygon_area != pytest.approx(inner.area_mm2, rel=1e-3)
    assert inner.area_mm2 == pytest.approx(25.0 * math.pi, rel=1e-9)


def test_section_misses_the_base_holes(l_bracket: TopoDS_Shape) -> None:
    """The base holes sit at y = 16.25 and 36.75, so the cut has no hole for them."""
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)

    for inner in cut.inner_loops:
        assert inner.bounds.center_u == pytest.approx(0.0, abs=0.05)
        assert inner.bounds.center_v == pytest.approx(BORE_CENTRE_Z_MM, abs=0.05)


def test_section_loops_are_closed(l_bracket: TopoDS_Shape) -> None:
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)

    for loop in cut.loops:
        assert len(loop.points) >= 3
        assert loop.points[0] != loop.points[-1]
        # A closed chain is drawn back onto its own first point.
        first = loop.points[0]
        last = loop.points[-1]
        assert math.hypot(last[0] - first[0], last[1] - first[1]) > 1e-6


def test_section_reports_its_plane_and_offset(l_bracket: TopoDS_Shape) -> None:
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)

    assert cut.plane == "XZ"
    assert cut.offset_mm == SECTION_OFFSET_MM
    assert cut.chord_tolerance_mm == DEFAULT_SECTION_CHORD_TOLERANCE_MM
    assert cut.truncated is False


# --- ADR-0009 criterion 4 companion: the hidden lines of the bore ---


def test_front_view_draws_the_bore(bracket_drawing: Drawing) -> None:
    front = bracket_drawing.view("front")

    visible_circles = [poly for poly in front.visible if poly.closed]
    assert visible_circles, "the bore opening must be visible in the front view"
    bore = _loop_near(
        tuple(
            SectionLoop(points=poly.points, area_mm2=0.0, outer=True) for poly in visible_circles
        ),
        0.0,
        BORE_CENTRE_Z_MM,
    )
    assert bore.bounds.width == pytest.approx(BORE_DIAMETER_MM, abs=0.1)
    assert bore.bounds.height == pytest.approx(BORE_DIAMETER_MM, abs=0.1)
    assert bore.bounds.center_v == pytest.approx(BORE_CENTRE_Z_MM, abs=0.1)


def test_front_view_has_hidden_lines_for_the_bore(bracket_drawing: Drawing) -> None:
    front = bracket_drawing.view("front")

    assert front.hidden, "the front view must carry hidden lines"
    hidden_loops = tuple(
        SectionLoop(points=poly.points, area_mm2=0.0, outer=True)
        for poly in front.hidden
        if poly.closed
    )
    assert hidden_loops, "the bore must be drawn as a hidden loop in the front view"

    bore = _loop_near(hidden_loops, 0.0, BORE_CENTRE_Z_MM)
    assert bore.bounds.width == pytest.approx(BORE_DIAMETER_MM, abs=0.1)
    assert bore.bounds.height == pytest.approx(BORE_DIAMETER_MM, abs=0.1)
    assert bore.bounds.center_u == pytest.approx(0.0, abs=0.1)
    assert bore.bounds.center_v == pytest.approx(BORE_CENTRE_Z_MM, abs=0.1)


def test_front_view_outline_is_the_part_envelope(bracket_drawing: Drawing) -> None:
    front = bracket_drawing.view("front")

    u_values = [point[0] for poly in front.visible for point in poly.points]
    v_values = [point[1] for poly in front.visible for point in poly.points]
    assert min(u_values) == pytest.approx(-30.0, abs=0.05)
    assert max(u_values) == pytest.approx(30.0, abs=0.05)
    assert min(v_values) == pytest.approx(0.0, abs=0.05)
    assert max(v_values) == pytest.approx(40.0, abs=0.05)


# --- the four views ---


def test_drawing_has_the_four_views(bracket_drawing: Drawing) -> None:
    assert bracket_drawing.view_names == ("front", "top", "right", "iso")
    for name in bracket_drawing.view_names:
        view = bracket_drawing.view(name)
        assert view.visible or view.hidden
        assert view.truncated is False


def test_top_view_is_measured_in_x_and_y(bracket_drawing: Drawing) -> None:
    top = bracket_drawing.view("top")

    u_values = [point[0] for poly in top.visible for point in poly.points]
    v_values = [point[1] for poly in top.visible for point in poly.points]
    assert min(u_values) == pytest.approx(-30.0, abs=0.05)
    assert max(u_values) == pytest.approx(30.0, abs=0.05)
    assert min(v_values) == pytest.approx(0.0, abs=0.05)
    assert max(v_values) == pytest.approx(45.0, abs=0.05)


def test_right_view_is_measured_in_y_and_z(bracket_drawing: Drawing) -> None:
    right = bracket_drawing.view("right")

    u_values = [point[0] for poly in right.visible for point in poly.points]
    v_values = [point[1] for poly in right.visible for point in poly.points]
    assert min(u_values) == pytest.approx(0.0, abs=0.05)
    assert max(u_values) == pytest.approx(45.0, abs=0.05)
    assert min(v_values) == pytest.approx(0.0, abs=0.05)
    assert max(v_values) == pytest.approx(40.0, abs=0.05)


def test_iso_view_is_drawn(bracket_drawing: Drawing) -> None:
    iso = bracket_drawing.view("iso")

    assert iso.visible
    u_values = [point[0] for poly in iso.visible for point in poly.points]
    v_values = [point[1] for poly in iso.visible for point in poly.points]
    # The isometric projection of the part is wider than it is tall.
    assert max(u_values) - min(u_values) > 40.0
    assert max(v_values) - min(v_values) > 20.0


def test_only_the_requested_views_are_drawn(l_bracket: TopoDS_Shape) -> None:
    drawing = hlr_views(l_bracket, views=["front"])

    assert drawing.view_names == ("front",)
    assert drawing.total_point_count > 0


def test_unknown_view_is_refused(l_bracket: TopoDS_Shape) -> None:
    with pytest.raises(ValueError, match="unknown view"):
        hlr_views(l_bracket, views=["side"])


# --- the section of a simpler part ---


def test_section_of_a_box_with_a_through_hole(box_with_hole: TopoDS_Shape) -> None:
    cut = section(box_with_hole, "XZ", 15.0)

    assert len(cut.outer_loops) == 1
    assert len(cut.inner_loops) == 1
    outer = cut.outer_loops[0]
    inner = cut.inner_loops[0]
    assert outer.bounds.width == pytest.approx(40.0, abs=0.05)
    assert outer.bounds.height == pytest.approx(20.0, abs=0.05)
    assert outer.area_mm2 == pytest.approx(800.0, rel=1e-9)
    assert outer.area_exact is True
    assert inner.bounds.width == pytest.approx(10.0, abs=0.05)
    assert inner.bounds.center_u == pytest.approx(0.0, abs=0.05)
    assert inner.bounds.center_v == pytest.approx(10.0, abs=0.05)
    assert inner.area_mm2 == pytest.approx(25.0 * math.pi, rel=1e-9)
    assert inner.area_exact is True
    assert cut.area_mm2 == pytest.approx(800.0 - 25.0 * math.pi, rel=1e-9)
    assert cut.area_exact is True


def test_a_tangent_cut_reports_an_approximate_area(
    box_with_hole: TopoDS_Shape,
) -> None:
    """A cut that grazes a hole is reported as an approximation, never as exact.

    The plane z = 5 touches the bore of the box, so the cut wire touches
    itself and the kernel cannot build a face of the whole loop. The module
    then uses the area of the drawn loop and says so with the flag.
    """
    cut = section(box_with_hole, "XY", 5.0)

    assert len(cut.loops) == 1
    assert cut.loops[0].area_mm2 == pytest.approx(1200.0, rel=1e-9)
    assert cut.loops[0].area_exact is False
    assert cut.area_mm2 == pytest.approx(1200.0, rel=1e-9)
    assert cut.area_exact is False


def test_an_island_inside_a_hole_is_material_again(
    box_with_island: TopoDS_Shape,
) -> None:
    """The net area of a nested cut is the material, holes and islands included."""
    cut = section(box_with_island, "XZ", 10.0)

    assert len(cut.outer_loops) == 2
    assert len(cut.inner_loops) == 1
    assert cut.area_mm2 == pytest.approx(800.0 - 100.0 * math.pi + 16.0 * math.pi, rel=1e-9)
    assert cut.area_exact is True
    for loop in cut.loops:
        assert loop.area_exact is True


def test_section_of_a_plane_that_misses_the_part_is_empty(
    box_with_hole: TopoDS_Shape,
) -> None:
    cut = section(box_with_hole, "XZ", 100.0)

    assert cut.loops == ()
    assert cut.area_mm2 == pytest.approx(0.0)
    assert cut.area_exact is True


def test_section_on_the_other_planes(box_with_hole: TopoDS_Shape) -> None:
    xy_cut = section(box_with_hole, "XY", 5.0)
    yz_cut = section(box_with_hole, "YZ", 10.0)

    assert len(xy_cut.outer_loops) == 1
    assert xy_cut.outer_loops[0].bounds.width == pytest.approx(40.0, abs=0.05)
    assert xy_cut.outer_loops[0].bounds.height == pytest.approx(30.0, abs=0.05)
    assert len(yz_cut.outer_loops) == 1
    assert yz_cut.outer_loops[0].bounds.width == pytest.approx(30.0, abs=0.05)
    assert yz_cut.outer_loops[0].bounds.height == pytest.approx(20.0, abs=0.05)


def test_a_section_through_the_hole_splits_the_part(
    box_with_hole: TopoDS_Shape,
) -> None:
    """The YZ plane at x = 0 runs along the axis of the through hole."""
    cut = section(box_with_hole, "YZ", 0.0)

    assert len(cut.outer_loops) == 2
    assert cut.inner_loops == ()
    assert cut.area_mm2 == pytest.approx(2 * 30.0 * 5.0, rel=1e-9)
    assert cut.area_exact is True
    for loop in cut.outer_loops:
        assert loop.area_mm2 == pytest.approx(150.0, rel=1e-9)
        assert loop.area_exact is True


def test_unknown_section_plane_is_refused(l_bracket: TopoDS_Shape) -> None:
    with pytest.raises(ValueError, match="unknown section plane"):
        section(l_bracket, "AB", 0.0)


def test_default_section_offset_is_the_mid_plane(l_bracket: TopoDS_Shape) -> None:
    assert default_section_offset(l_bracket) == pytest.approx(22.5, abs=1e-6)


def test_a_section_without_an_offset_uses_the_mid_plane(
    l_bracket: TopoDS_Shape,
) -> None:
    cut = section(l_bracket, "XZ")

    assert cut.offset_mm == pytest.approx(22.5, abs=1e-6)
    assert cut.plane == "XZ"


# --- the public contract of the module ---


def test_results_are_plain_frozen_dataclasses(
    l_bracket: TopoDS_Shape,
) -> None:
    drawing = hlr_views(l_bracket, views=["front"])
    cut = section(l_bracket, "XZ", SECTION_OFFSET_MM)

    assert isinstance(drawing, Drawing)
    assert isinstance(drawing.views[0], ViewDrawing)
    assert isinstance(drawing.views[0].visible[0], Polyline)
    assert isinstance(cut, Section)
    assert isinstance(cut.loops[0], SectionLoop)

    assert isinstance(cut.area_mm2, float)
    assert isinstance(cut.area_exact, bool)
    for loop in cut.loops:
        assert isinstance(loop.area_mm2, float)
        assert isinstance(loop.area_exact, bool)
        assert isinstance(loop.outer, bool)

    for polyline in drawing.views[0].visible:
        assert isinstance(polyline.points, tuple)
        for point in polyline.points:
            assert isinstance(point, tuple)
            assert len(point) == 2
            assert all(isinstance(value, float) for value in point)


def test_a_frozen_result_cannot_be_edited(l_bracket: TopoDS_Shape) -> None:
    drawing = hlr_views(l_bracket, views=["front"])

    with pytest.raises(AttributeError):
        drawing.views[0].name = "side"  # type: ignore[misc]


def test_a_null_shape_is_refused() -> None:
    with pytest.raises(DrawingError):
        hlr_views(TopoDS_Shape())
    with pytest.raises(DrawingError):
        section(TopoDS_Shape(), "XZ", 0.0)


def test_the_point_cap_is_the_exact_end_of_the_curve(
    box_with_hole: TopoDS_Shape,
) -> None:
    """Every drawn line starts and ends on the true end points of its curve."""
    drawing = hlr_views(box_with_hole, views=["front"])

    for polyline in drawing.view("front").visible:
        if polyline.closed:
            assert math.dist(polyline.points[0], polyline.points[-1]) <= 1e-6
            continue
        assert len(polyline.points) >= 2
        assert polyline.points[0] != polyline.points[-1]


def test_a_tight_chord_tolerance_is_closer_to_the_curve(
    l_bracket: TopoDS_Shape,
) -> None:
    coarse = hlr_views(l_bracket, views=["front"], chord_tolerance_mm=0.5)
    fine = hlr_views(l_bracket, views=["front"], chord_tolerance_mm=0.001)

    assert fine.total_point_count > coarse.total_point_count


def test_an_invalid_chord_tolerance_is_refused(l_bracket: TopoDS_Shape) -> None:
    with pytest.raises(ValueError, match="chord tolerance"):
        hlr_views(l_bracket, chord_tolerance_mm=0.0)
    with pytest.raises(ValueError, match="chord tolerance"):
        section(l_bracket, "XZ", 0.0, chord_tolerance_mm=-1.0)


def test_the_point_limit_truncates_the_view(l_bracket: TopoDS_Shape) -> None:
    drawing = hlr_views(l_bracket, views=["front"], max_points=5)

    view = drawing.view("front")
    assert view.truncated is True
    assert view.point_count <= 5


def test_the_default_chord_tolerance_is_the_adr_value() -> None:
    assert DEFAULT_CHORD_TOLERANCE_MM == 0.05
    assert DEFAULT_SECTION_CHORD_TOLERANCE_MM == 0.01


def test_the_drawing_is_deterministic(l_bracket: TopoDS_Shape) -> None:
    first = hlr_views(l_bracket, views=["front", "top"])
    second = hlr_views(l_bracket, views=["front", "top"])

    assert first.views[0].visible == second.views[0].visible
    assert first.views[1].hidden == second.views[1].hidden
