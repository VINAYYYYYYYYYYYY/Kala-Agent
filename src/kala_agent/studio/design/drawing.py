"""Hidden-line removal and section geometry for the Studio design turns.

The module turns one measured B-rep shape into the two drawings the Studio
sheet needs: an orthographic/axonometric pencil drawing and a real cut
section.

* ``hlr_views`` runs ``HLRBRep_Algo`` + ``HLRBRep_HLRToShape`` and returns
  the visible and hidden edges of the ``front`` (-Y), ``top`` (+Z), ``right``
  (+X) and ``iso`` views. Every polyline is expressed in *view-plane
  millimetres*: the horizontal axis of the view is the first component of
  the pair, the vertical axis the second one, and the view's depth axis is
  projected away.
* ``section`` runs ``BRepAlgoAPI_Section`` against a plane and returns the
  closed loops of the cut, classified as outer boundaries and holes. Without
  an explicit offset the plane is the mid-plane of the shape's bounds normal
  to Y, which is the default position of ADR-0009.

Two conventions matter for the numbers this module reports:

**View bases.** The four views use the third-angle arrangement of the ADR:
``front`` looks from -Y with +X to the right and +Z up, ``top`` looks from
+Z with +X to the right and +Y up, ``right`` looks from +X with +Y to the
right and +Z up, and ``iso`` looks from (+X, -Y, +Z) with +Z up. The
projection direction handed to OpenCascade points *from the part towards the
viewer*, so the visible/hidden classification is the one a drafter expects.

**Chord tolerance.** Curves are sampled with
``GCPnts_QuasiUniformDeflection``, so the polyline never strays further
than the requested tolerance from the true curve. Views use the ADR default
of 0.05 mm; the section uses a tighter 0.01 mm because the section's
loop dimensions are measured quantities that ADR-0009 pins to +-0.05 mm.
The tolerance bounds the *drawing* only: the areas of the section are
measured on the exact faces of the cut, never on the sampled polylines.

**Point caps.** Every polyline starts and ends on the exact end points of
the curve it came from: the first and last samples are the *caps* of the
edge, not approximations of it. A closed curve repeats its first point at
the end so the returned polyline is explicitly closed, and a curve that
collapses to a single point yields a one-point polyline instead of being
dropped.

**Measured areas.** ADR-0009 wants every number on the sheet to be a
measurement, not an approximation, so the areas of a section are measured
by OpenCascade: each loop becomes a planar face
(``BRepBuilderAPI_MakeFace`` on the cut wire) and the net area becomes a
face of the outer wire with the hole wires added. The areas come from
``BRepGProp.SurfaceProperties_s`` on those faces, so the area of a
circular loop is the area of the circle, not the area of the polygon
that approximates it. Every area carries an ``area_exact`` flag: it is
true when the number is a kernel measurement and false when the kernel
could not build the face and the area of the drawn polyline was used
instead. An approximation is never reported as a measurement.

Nothing in the public result is an OpenCascade object: the module returns
frozen dataclasses holding plain floats so the HTTP layer can serialise
them without importing the kernel.
"""

from __future__ import annotations

import itertools
import math
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from enum import StrEnum
from typing import Final, Protocol

from OCP.Bnd import Bnd_Box  # type: ignore[import-untyped]
from OCP.BRep import BRep_Tool  # type: ignore[import-untyped]
from OCP.BRepAdaptor import BRepAdaptor_Curve  # type: ignore[import-untyped]
from OCP.BRepAlgoAPI import BRepAlgoAPI_Section  # type: ignore[import-untyped]
from OCP.BRepBndLib import BRepBndLib  # type: ignore[import-untyped]
from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeFace  # type: ignore[import-untyped]
from OCP.BRepGProp import BRepGProp  # type: ignore[import-untyped]
from OCP.BRepTools import BRepTools_WireExplorer  # type: ignore[import-untyped]
from OCP.collections import HSequence_TopoDS_Shape  # type: ignore[import-untyped]
from OCP.GCPnts import GCPnts_QuasiUniformDeflection  # type: ignore[import-untyped]
from OCP.gp import (  # type: ignore[import-untyped]
    gp_Ax2,
    gp_Dir,
    gp_Pln,
    gp_Pnt,
)
from OCP.GProp import GProp_GProps  # type: ignore[import-untyped]
from OCP.HLRAlgo import HLRAlgo_Projector  # type: ignore[import-untyped]
from OCP.HLRBRep import (  # type: ignore[import-untyped]
    HLRBRep_Algo,
    HLRBRep_HLRToShape,
)
from OCP.ShapeAnalysis import ShapeAnalysis_FreeBounds  # type: ignore[import-untyped]
from OCP.TopAbs import TopAbs_EDGE, TopAbs_REVERSED  # type: ignore[import-untyped]
from OCP.TopExp import TopExp, TopExp_Explorer  # type: ignore[import-untyped]
from OCP.TopoDS import (  # type: ignore[import-untyped]
    TopoDS,
    TopoDS_Face,
    TopoDS_Shape,
    TopoDS_Vertex,
)

DEFAULT_CHORD_TOLERANCE_MM: Final[float] = 0.05
DEFAULT_SECTION_CHORD_TOLERANCE_MM: Final[float] = 0.01
MAX_POINTS_PER_VIEW: Final[int] = 20_000
MAX_SECTION_POINTS: Final[int] = 20_000
_SECTION_WIRE_TOLERANCE_MM: Final[float] = 1e-6
_GEOMETRY_EPSILON_MM: Final[float] = 1e-9
_AREA_EPSILON_MM2: Final[float] = 1e-9
# Relative slack allowed when a measured area is compared with the loops it
# is built from: a hole that the kernel added as a second region instead of
# a hole shows up as a gross error, so a tight relative band is enough to
# catch it without rejecting floating point noise.
_AREA_CONSISTENCY_RATIO: Final[float] = 1e-6

Vector3 = tuple[float, float, float]
Point2 = tuple[float, float]


class _OcpPoint(Protocol):
    """Structural type of the OpenCascade point the projector reads.

    The OCP bindings ship without stubs, so the module describes the three
    coordinates it uses instead of spreading ``Any`` over the code.
    """

    def X(self) -> float: ...

    def Y(self) -> float: ...

    def Z(self) -> float: ...


class DrawingError(RuntimeError):
    """Raised when OpenCascade cannot produce a drawing for the given shape."""


class ViewName(StrEnum):
    """The four views the Studio sheet draws."""

    FRONT = "front"
    TOP = "top"
    RIGHT = "right"
    ISO = "iso"


class SectionPlane(StrEnum):
    """The three principal section planes."""

    XY = "XY"
    XZ = "XZ"
    YZ = "YZ"


DEFAULT_VIEW_NAMES: Final[tuple[str, ...]] = (
    ViewName.FRONT.value,
    ViewName.TOP.value,
    ViewName.RIGHT.value,
    ViewName.ISO.value,
)

# view name -> (direction from the part towards the viewer, horizontal axis of
# the view). The vertical axis is the cross product of the two, so every view
# basis is right handed and the local frame of the projection plane matches
# the drawing sheet.
_VIEW_BASES: Final[dict[str, tuple[Vector3, Vector3]]] = {
    ViewName.FRONT.value: ((0.0, -1.0, 0.0), (1.0, 0.0, 0.0)),
    ViewName.TOP.value: ((0.0, 0.0, 1.0), (1.0, 0.0, 0.0)),
    ViewName.RIGHT.value: ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0)),
    ViewName.ISO.value: (
        (
            1.0 / math.sqrt(3.0),
            -1.0 / math.sqrt(3.0),
            1.0 / math.sqrt(3.0),
        ),
        (1.0 / math.sqrt(2.0), 1.0 / math.sqrt(2.0), 0.0),
    ),
}

# section plane -> (plane normal, horizontal axis, vertical axis). The offset
# is measured along the normal, so "XZ" with offset 4 is the plane y = 4.
_SECTION_BASES: Final[dict[str, tuple[Vector3, Vector3, Vector3]]] = {
    SectionPlane.XY.value: ((0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)),
    SectionPlane.XZ.value: ((0.0, 1.0, 0.0), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0)),
    SectionPlane.YZ.value: ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)),
}


@dataclass(frozen=True, slots=True)
class Bounds2D:
    """An axis-aligned bounding box in view-plane millimetres."""

    min_u: float
    min_v: float
    max_u: float
    max_v: float

    @property
    def width(self) -> float:
        """Return the extent along the horizontal axis."""
        return self.max_u - self.min_u

    @property
    def height(self) -> float:
        """Return the extent along the vertical axis."""
        return self.max_v - self.min_v

    @property
    def center_u(self) -> float:
        """Return the horizontal centre."""
        return (self.min_u + self.max_u) / 2.0

    @property
    def center_v(self) -> float:
        """Return the vertical centre."""
        return (self.min_v + self.max_v) / 2.0


@dataclass(frozen=True, slots=True)
class Polyline:
    """One drawn line: a chain of view-plane points.

    ``points`` are ``(u, v)`` pairs in millimetres. ``closed`` is true when
    the chain returns to its first point; the closing point is included in
    ``points`` so a consumer can stroke the loop without a special case.
    """

    points: tuple[Point2, ...]
    closed: bool = False

    @property
    def point_count(self) -> int:
        """Return the number of points in the polyline."""
        return len(self.points)

    @property
    def bounds(self) -> Bounds2D:
        """Return the bounding box of the polyline in view-plane millimetres."""
        return _bounds_of(self.points)

    @property
    def length_mm(self) -> float:
        """Return the length of the polyline chain in millimetres."""
        total = 0.0
        for index in range(1, len(self.points)):
            total += _distance(self.points[index - 1], self.points[index])
        return total


@dataclass(frozen=True, slots=True)
class ViewDrawing:
    """The visible and hidden pencil lines of one view."""

    name: str
    visible: tuple[Polyline, ...]
    hidden: tuple[Polyline, ...]
    truncated: bool = False

    @property
    def point_count(self) -> int:
        """Return the total number of points in the view."""
        return sum(polyline.point_count for polyline in self.visible) + sum(
            polyline.point_count for polyline in self.hidden
        )

    @property
    def polyline_count(self) -> int:
        """Return the total number of polylines in the view."""
        return len(self.visible) + len(self.hidden)


@dataclass(frozen=True, slots=True)
class Drawing:
    """The views of one shape, in the order they were requested."""

    views: tuple[ViewDrawing, ...]
    chord_tolerance_mm: float
    max_points: int

    def view(self, name: str) -> ViewDrawing:
        """Return one view by name.

        Raises:
            KeyError: the drawing does not contain that view.
        """
        for view in self.views:
            if view.name == name:
                return view
        raise KeyError(name)

    @property
    def view_names(self) -> tuple[str, ...]:
        """Return the names of the views in drawing order."""
        return tuple(view.name for view in self.views)

    @property
    def total_point_count(self) -> int:
        """Return the total number of points across every view."""
        return sum(view.point_count for view in self.views)


@dataclass(frozen=True, slots=True)
class SectionLoop:
    """One closed loop of a section cut.

    ``points`` are the loop in the plane's own coordinates: for the ``XZ``
    plane they are ``(x, z)`` in millimetres. They are the *drawing* of the
    loop, the sampled polyline of the cut's edges, and are used to hatch and
    to draw the section.

    ``area_mm2`` is the area enclosed by the loop, always positive. It is
    measured by OpenCascade on the exact planar face of the loop, so a
    circular loop carries the area of the circle rather than the area of
    the polygon that approximates it. ``area_exact`` says whether that is
    what happened: it is true for a measured area and false when the kernel
    could not build the face and the area of the drawn polyline was used
    instead.

    ``outer`` is true for a boundary of the cut material and false for a
    hole inside it.
    """

    points: tuple[Point2, ...]
    area_mm2: float
    outer: bool
    area_exact: bool = False

    @property
    def bounds(self) -> Bounds2D:
        """Return the bounding box of the loop in plane coordinates."""
        return _bounds_of(self.points)

    @property
    def centroid(self) -> Point2:
        """Return the area centroid of the loop in plane coordinates."""
        return _centroid_of(self.points)

    @property
    def point_count(self) -> int:
        """Return the number of points in the loop."""
        return len(self.points)


@dataclass(frozen=True, slots=True)
class Section:
    """The closed loops of one section cut.

    ``area_mm2`` is the net enclosed area of the cut, holes removed: it is
    the area the hatching is drawn into. It is measured by OpenCascade on
    the exact planar face of the net region, the outer wire with the hole
    wires added, so it is the area of the material and not the area of an
    approximation of it. ``area_exact`` says whether the kernel could
    measure it: it is false when the face could not be built and the area
    had to be summed from the loops, or when the point budget dropped a
    loop and the cut is therefore incomplete.
    """

    plane: str
    offset_mm: float
    loops: tuple[SectionLoop, ...]
    area_mm2: float
    chord_tolerance_mm: float
    truncated: bool = False
    area_exact: bool = False

    @property
    def outer_loops(self) -> tuple[SectionLoop, ...]:
        """Return the boundaries of the cut material."""
        return tuple(loop for loop in self.loops if loop.outer)

    @property
    def inner_loops(self) -> tuple[SectionLoop, ...]:
        """Return the holes inside the cut material."""
        return tuple(loop for loop in self.loops if not loop.outer)

    @property
    def point_count(self) -> int:
        """Return the total number of points in the section."""
        return sum(loop.point_count for loop in self.loops)


def _dot(left: Vector3, right: Vector3) -> float:
    """Return the dot product of two 3D vectors."""
    return left[0] * right[0] + left[1] * right[1] + left[2] * right[2]


def _cross(left: Vector3, right: Vector3) -> Vector3:
    """Return the cross product of two 3D vectors."""
    return (
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    )


def _distance(first: Point2, second: Point2) -> float:
    """Return the distance between two plane points."""
    return math.hypot(second[0] - first[0], second[1] - first[1])


def _bounds_of(points: Sequence[Point2]) -> Bounds2D:
    """Return the bounding box of a chain of plane points."""
    if not points:
        raise ValueError("cannot take the bounds of an empty polyline")
    us = [point[0] for point in points]
    vs = [point[1] for point in points]
    return Bounds2D(min_u=min(us), min_v=min(vs), max_u=max(us), max_v=max(vs))


def _signed_area(points: Sequence[Point2]) -> float:
    """Return the signed area of a closed chain of plane points."""
    total = 0.0
    count = len(points)
    for index in range(count):
        current = points[index]
        following = points[(index + 1) % count]
        total += current[0] * following[1] - following[0] * current[1]
    return total / 2.0


def _centroid_of(points: Sequence[Point2]) -> Point2:
    """Return the area centroid of a closed chain of plane points."""
    area = _signed_area(points)
    if abs(area) <= _AREA_EPSILON_MM2:
        bounds = _bounds_of(points)
        return (bounds.center_u, bounds.center_v)
    count = len(points)
    sum_u = 0.0
    sum_v = 0.0
    for index in range(count):
        current = points[index]
        following = points[(index + 1) % count]
        cross = current[0] * following[1] - following[0] * current[1]
        sum_u += (current[0] + following[0]) * cross
        sum_v += (current[1] + following[1]) * cross
    return (sum_u / (6.0 * area), sum_v / (6.0 * area))


def _point_in_polygon(point: Point2, polygon: Sequence[Point2]) -> bool:
    """Return true when ``point`` lies inside the closed ``polygon``."""
    x, y = point
    inside = False
    count = len(polygon)
    if count < 3:
        return False
    previous = count - 1
    for index in range(count):
        x_i, y_i = polygon[index]
        x_j, y_j = polygon[previous]
        if (y_i > y) != (y_j > y):
            crossing = (x_j - x_i) * (y - y_i) / (y_j - y_i) + x_i
            if x < crossing:
                inside = not inside
        previous = index
    return inside


def _validate_chord_tolerance(value: float) -> float:
    """Return a usable chord tolerance in millimetres."""
    tolerance = float(value)
    if not math.isfinite(tolerance) or tolerance <= 0.0:
        raise ValueError(f"the chord tolerance must be a positive finite number: {value!r}")
    return tolerance


def _require_shape(shape: TopoDS_Shape) -> None:
    """Raise when the caller handed in a null shape."""
    if shape is None or shape.IsNull():
        raise DrawingError("the drawing needs a non-null shape")


def _edge_polylines(
    edges: Sequence[TopoDS_Shape],
    project: Callable[[_OcpPoint], Point2],
    chord_tolerance_mm: float,
) -> list[Polyline]:
    """Return one polyline per edge, sampled along the view plane."""
    polylines: list[Polyline] = []
    for edge in edges:
        polyline = _discretise_edge(edge, project, chord_tolerance_mm)
        if polyline is not None:
            polylines.append(polyline)
    return polylines


def _discretise_edge(
    edge: TopoDS_Shape,
    project: Callable[[_OcpPoint], Point2],
    chord_tolerance_mm: float,
) -> Polyline | None:
    """Sample one edge into a polyline in view-plane millimetres.

    The first and last samples are the exact end points of the edge, so the
    polyline carries the true *point caps* of the curve. A closed curve
    repeats its first point at the end.
    """
    adaptor = BRepAdaptor_Curve(edge)
    first = float(adaptor.FirstParameter())
    last = float(adaptor.LastParameter())
    if last < first:
        first, last = last, first
    if last - first <= _GEOMETRY_EPSILON_MM:
        # A curve that collapsed to a single point still has to be drawn.
        point = project(adaptor.Value(first))
        return Polyline(points=(point,), closed=False)

    sampler = GCPnts_QuasiUniformDeflection()
    sampler.Initialize(adaptor, chord_tolerance_mm, first, last)
    count = int(sampler.NbPoints())
    if count < 2:
        start = project(adaptor.Value(first))
        end = project(adaptor.Value(last))
        return Polyline(points=(start, end), closed=False)

    points: list[Point2] = []
    for index in range(1, count + 1):
        parameter = float(sampler.Parameter(index))
        points.append(project(adaptor.Value(parameter)))
    # Pin the caps to the exact end points of the edge.
    points[0] = project(adaptor.Value(first))
    points[-1] = project(adaptor.Value(last))

    closed = bool(adaptor.IsClosed())
    if closed and _distance(points[0], points[-1]) > _GEOMETRY_EPSILON_MM:
        points.append(points[0])
    return Polyline(points=tuple(points), closed=closed)


def _plane_projector(u_axis: Vector3, v_axis: Vector3) -> Callable[[_OcpPoint], Point2]:
    """Return a projector from 3D space onto the ``(u, v)`` plane axes."""

    def project(point: _OcpPoint) -> Point2:
        x = float(point.X())
        y = float(point.Y())
        z = float(point.Z())
        return (
            x * u_axis[0] + y * u_axis[1] + z * u_axis[2],
            x * v_axis[0] + y * v_axis[1] + z * v_axis[2],
        )

    return project


def _local_projector() -> Callable[[_OcpPoint], Point2]:
    """Return the projector of an already flattened hidden-line result.

    ``HLRBRep_HLRToShape`` returns its edges in the local frame of the
    projection plane, so the first two coordinates of every point already
    are the view-plane millimetres.
    """

    def project(point: _OcpPoint) -> Point2:
        return (float(point.X()), float(point.Y()))

    return project


def _dedupe(polylines: Sequence[Polyline]) -> tuple[Polyline, ...]:
    """Drop polylines that repeat an earlier one exactly.

    The hidden-line algorithm reports the same edge more than once when the
    edge is split across several statuses. The duplicates carry no extra
    geometry, so the drawing keeps one copy of each.
    """
    seen: set[tuple[Point2, ...]] = set()
    unique: list[Polyline] = []
    for polyline in polylines:
        key = polyline.points
        if key in seen:
            continue
        seen.add(key)
        unique.append(polyline)
    return tuple(unique)


def _edges_of(compound: object) -> list[TopoDS_Shape]:
    """Return the edges of an OpenCascade compound."""
    edges: list[TopoDS_Shape] = []
    explorer = TopExp_Explorer(compound, TopAbs_EDGE)
    while explorer.More():
        edges.append(TopoDS.Edge(explorer.Current()))
        explorer.Next()
    return edges


def hlr_views(
    shape: TopoDS_Shape,
    *,
    views: Sequence[str] = DEFAULT_VIEW_NAMES,
    chord_tolerance_mm: float = DEFAULT_CHORD_TOLERANCE_MM,
    max_points: int = MAX_POINTS_PER_VIEW,
) -> Drawing:
    """Return the hidden-line drawing of ``shape`` for the requested views.

    Args:
        shape: the B-rep shape to draw.
        views: the view names to compute; every name must be one of
            :class:`ViewName`.
        chord_tolerance_mm: the maximum distance between a polyline and the
            curve it approximates.
        max_points: the maximum number of points kept per view.

    Returns:
        A :class:`Drawing` holding one :class:`ViewDrawing` per requested
        view, in the requested order.

    Raises:
        DrawingError: the kernel could not draw the shape.
        ValueError: an argument is out of range or a view name is unknown.
    """
    _require_shape(shape)
    tolerance = _validate_chord_tolerance(chord_tolerance_mm)
    limit = int(max_points)
    if limit < 1:
        raise ValueError(f"the point limit must be positive: {max_points!r}")

    drawn: list[ViewDrawing] = []
    for name in views:
        key = str(name)
        if key not in _VIEW_BASES:
            raise ValueError(f"unknown view {key!r}; expected one of {sorted(_VIEW_BASES)}")
        drawn.append(_draw_view(shape, key, tolerance, limit))
    return Drawing(
        views=tuple(drawn),
        chord_tolerance_mm=tolerance,
        max_points=limit,
    )


def _draw_view(
    shape: TopoDS_Shape,
    name: str,
    chord_tolerance_mm: float,
    max_points: int,
) -> ViewDrawing:
    """Return the visible and hidden lines of one view."""
    direction, u_axis = _VIEW_BASES[name]

    # The local frame of the projection plane is (u_axis, direction x u_axis),
    # so the kernel already returns the edges in view-plane millimetres.
    axis = gp_Ax2(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(*direction), gp_Dir(*u_axis))
    algorithm = HLRBRep_Algo()
    algorithm.Add(shape)
    algorithm.Projector(HLRAlgo_Projector(axis))
    algorithm.Update()
    algorithm.Hide()
    shapes = HLRBRep_HLRToShape(algorithm)

    visible = _dedupe(
        _edge_polylines(
            _edges_of(shapes.VCompound()) + _edges_of(shapes.OutLineVCompound()),
            _local_projector(),
            chord_tolerance_mm,
        )
    )
    hidden = _dedupe(
        _edge_polylines(
            _edges_of(shapes.HCompound()) + _edges_of(shapes.OutLineHCompound()),
            _local_projector(),
            chord_tolerance_mm,
        )
    )

    kept_visible: list[Polyline] = []
    kept_hidden: list[Polyline] = []
    used = 0
    truncated = False
    for polyline in visible:
        if used + polyline.point_count > max_points:
            truncated = True
            break
        used += polyline.point_count
        kept_visible.append(polyline)
    for polyline in hidden:
        if used + polyline.point_count > max_points:
            truncated = True
            break
        used += polyline.point_count
        kept_hidden.append(polyline)

    return ViewDrawing(
        name=name,
        visible=tuple(kept_visible),
        hidden=tuple(kept_hidden),
        truncated=truncated,
    )


def section(
    shape: TopoDS_Shape,
    plane: str = SectionPlane.XZ.value,
    offset_mm: float | None = None,
    *,
    chord_tolerance_mm: float = DEFAULT_SECTION_CHORD_TOLERANCE_MM,
) -> Section:
    """Return the closed loops of a section cut through ``shape``.

    Args:
        shape: the B-rep shape to cut.
        plane: ``"XY"``, ``"XZ"`` or ``"YZ"``. The plane is normal to the
            third axis of the name: ``"XZ"`` is normal to Y.
        offset_mm: the position of the plane along its normal, measured from
            the global origin. ``"XZ"`` with an offset of 4 is the plane
            y = 4. When it is left out the mid-plane of the shape's bounds
            is used, which is the default position of ADR-0009.
        chord_tolerance_mm: the maximum distance between a loop and the
            curve it approximates. The tolerance bounds the drawn polyline
            only; the areas are measured on the exact faces of the cut.

    Returns:
        A :class:`Section` holding the closed loops of the cut, each
        classified as an outer boundary or a hole, and the net enclosed area
        of the cut as measured by OpenCascade.

    Raises:
        DrawingError: the kernel could not cut the shape.
        ValueError: the plane name is unknown or an argument is out of range.
    """
    _require_shape(shape)
    key = str(plane).upper()
    if key not in _SECTION_BASES:
        raise ValueError(
            f"unknown section plane {plane!r}; expected one of {sorted(_SECTION_BASES)}"
        )
    if offset_mm is None:
        offset = default_section_offset(shape)
    else:
        offset = float(offset_mm)
    if not math.isfinite(offset):
        raise ValueError(f"the section offset must be a finite number: {offset_mm!r}")
    tolerance = _validate_chord_tolerance(chord_tolerance_mm)

    normal, u_axis, v_axis = _SECTION_BASES[key]
    origin = _plane_origin(normal, offset)
    cutting_plane = gp_Pln(gp_Pnt(*origin), gp_Dir(*normal))

    operation = BRepAlgoAPI_Section(shape, cutting_plane)
    operation.Build()
    if not operation.IsDone():
        raise DrawingError(f"the section at {key} offset {offset} mm did not complete")

    loops, truncated, area_mm2, area_exact = _section_loops(
        operation.Shape(),
        u_axis,
        v_axis,
        tolerance,
        MAX_SECTION_POINTS,
    )
    return Section(
        plane=key,
        offset_mm=offset,
        loops=loops,
        area_mm2=area_mm2,
        chord_tolerance_mm=tolerance,
        truncated=truncated,
        area_exact=area_exact,
    )


def _plane_origin(normal: Vector3, offset: float) -> Vector3:
    """Return a point of the plane at ``offset`` along its normal."""
    return (normal[0] * offset, normal[1] * offset, normal[2] * offset)


@dataclass(slots=True)
class _LoopRecord:
    """One closed wire of a section cut with the numbers measured from it.

    The record keeps the cut wire next to the drawn polyline: the polyline
    is what the caller draws, the wire is what the kernel measures the area
    on. ``depth`` is the nesting depth of the loop, zero for a boundary of
    the material, and ``parent`` the index of the loop it is cut out of.
    ``simple`` says whether the wire walks its boundary once, which is what
    a loop of a cut has to be for a face of it to mean anything.
    """

    wire: TopoDS_Shape
    points: list[Point2]
    bounds: Bounds2D
    length_mm: float = 0.0
    area_mm2: float = 0.0
    area_exact: bool = False
    depth: int = 0
    parent: int = -1
    simple: bool = True


def _section_loops(
    section_shape: object,
    u_axis: Vector3,
    v_axis: Vector3,
    chord_tolerance_mm: float,
    max_points: int,
) -> tuple[tuple[SectionLoop, ...], bool, float, bool]:
    """Return the closed loops of a section shape, in plane coordinates.

    The result holds the loops, whether the point limit forced the module to
    drop a loop, the net enclosed area of the cut and whether that area is a
    kernel measurement. The caller can therefore say the section is
    incomplete instead of quietly drawing a wrong area.
    """
    edges = _edges_of(section_shape)
    if not edges:
        return (), False, 0.0, True

    project = _plane_projector(u_axis, v_axis)
    records: list[_LoopRecord] = []
    for wire in _connect_edges(edges):
        points = _wire_points(wire, project, chord_tolerance_mm)
        if len(points) < 3:
            continue
        record = _LoopRecord(
            wire=wire,
            points=points,
            bounds=_bounds_of(points),
            length_mm=_wire_length(wire),
            simple=_wire_is_simple(wire),
        )
        record.area_mm2, record.area_exact = _measure_loop_area(record, chord_tolerance_mm)
        records.append(record)

    records.sort(key=lambda record: (-record.area_mm2, record.bounds.min_u, record.bounds.min_v))

    kept: list[_LoopRecord] = []
    used = 0
    truncated = False
    for record in records:
        if used + len(record.points) > max_points:
            truncated = True
            continue
        used += len(record.points)
        kept.append(record)

    _classify_loops(kept)
    loops = tuple(
        SectionLoop(
            points=tuple(record.points),
            area_mm2=record.area_mm2,
            outer=record.depth % 2 == 0,
            area_exact=record.area_exact,
        )
        for record in kept
    )
    area_mm2, area_exact = _net_area(kept)
    if truncated:
        # A dropped loop leaves a hole in the cut, so the area no longer
        # covers the whole section and cannot be called exact.
        area_exact = False
    return loops, truncated, area_mm2, area_exact


def _measure_loop_area(record: _LoopRecord, chord_tolerance_mm: float) -> tuple[float, bool]:
    """Return the area of a loop and whether the kernel measured it.

    The area is measured on the exact planar face of the loop, so a circular
    loop carries the area of the circle and not the area of the polygon that
    approximates it. The measurement is only used when the face describes the
    loop it was built from: a wire that touches itself, a tangent cut for
    instance, can make the kernel build a face of a part of the loop, and
    that face is not the area of the loop. Such a face is rejected and the
    area of the drawn polyline is used instead, reported as not exact.
    """
    if record.simple:
        face = _face_from_wire(record.wire)
        if face is not None:
            area = _measured_face_area(face)
            if area is not None and _face_describes_loop(area, record, chord_tolerance_mm):
                return area, True
    return abs(_signed_area(record.points)), False


def _wire_is_simple(wire: TopoDS_Shape) -> bool:
    """Return true when the wire walks its boundary once.

    A wire that touches itself, a cut that grazes the edge of a hole for
    instance, is not a simple loop: the face the kernel builds from it
    covers the regions the wire walks over instead of the region the wire
    bounds, so its area is not the area of the loop. The wire starts every
    one of its edges at a point of its boundary, and a point that starts two
    edges means the wire touches itself.
    """
    starts: list[tuple[float, float, float]] = []
    explorer = BRepTools_WireExplorer(wire)
    while explorer.More():
        starts.append(_edge_start_point(explorer.Current()))
        explorer.Next()
    ordered = sorted(starts)
    for first, second in itertools.pairwise(ordered):
        if _coordinates_are_close(first, second):
            return False
    return True


def _edge_start_point(edge: TopoDS_Shape) -> tuple[float, float, float]:
    """Return the point where the wire enters an edge."""
    if edge.Orientation() == TopAbs_REVERSED:
        vertex = TopExp.LastVertex_s(edge)
    else:
        vertex = TopExp.FirstVertex_s(edge)
    return _vertex_coordinates(vertex)


def _vertex_coordinates(vertex: TopoDS_Vertex) -> tuple[float, float, float]:
    """Return the coordinates of a vertex."""
    point = BRep_Tool.Pnt_s(vertex)
    return (float(point.X()), float(point.Y()), float(point.Z()))


def _coordinates_are_close(
    first: tuple[float, float, float],
    second: tuple[float, float, float],
) -> bool:
    """Return true when two points are the same point to the kernel's liking."""
    return (
        abs(first[0] - second[0]) <= _GEOMETRY_EPSILON_MM
        and abs(first[1] - second[1]) <= _GEOMETRY_EPSILON_MM
        and abs(first[2] - second[2]) <= _GEOMETRY_EPSILON_MM
    )


def _face_describes_loop(
    area: float,
    record: _LoopRecord,
    chord_tolerance_mm: float,
) -> bool:
    """Return true when a measured area agrees with the loop it came from.

    The drawn polyline of a loop differs from the loop by at most the area
    between the chords and the curves, which is bounded by the length of the
    loop times the chord tolerance. A measured area further away than that
    does not describe the loop and is not used.
    """
    slack = _AREA_CONSISTENCY_RATIO * max(1.0, abs(area))
    allowance = record.length_mm * chord_tolerance_mm + slack
    return abs(area - abs(_signed_area(record.points))) <= allowance


def _wire_length(wire: TopoDS_Shape) -> float:
    """Return the exact length of the edges of a wire."""
    total = 0.0
    explorer = BRepTools_WireExplorer(wire)
    while explorer.More():
        properties = GProp_GProps()
        BRepGProp.LinearProperties_s(explorer.Current(), properties)
        total += abs(float(properties.Mass()))
        explorer.Next()
    return total


def _face_from_wire(wire: TopoDS_Shape) -> TopoDS_Face | None:
    """Return the planar face bounded by a closed wire, or None."""
    builder = BRepBuilderAPI_MakeFace(wire, True)
    if not builder.IsDone():
        return None
    face = builder.Face()
    if face.IsNull():
        return None
    return face


def _measured_face_area(face: TopoDS_Face) -> float | None:
    """Return the area of a face as measured by OpenCascade."""
    properties = GProp_GProps()
    BRepGProp.SurfaceProperties_s(face, properties)
    area = abs(float(properties.Mass()))
    if not math.isfinite(area) or area <= 0.0:
        return None
    return area


def _net_area(records: Sequence[_LoopRecord]) -> tuple[float, bool]:
    """Return the net enclosed area of the cut and whether it is exact.

    The net area is measured on the face of every region of material: the
    face of its outer wire with the wires of its holes cut out of it. When
    a face cannot be built, or when the measured face does not agree with
    the loops it is built from, the area falls back to the signed sum of
    the loop areas, which is exact as long as every loop area was measured.
    """
    total = 0.0
    for index, region in enumerate(records):
        if region.depth % 2 != 0:
            continue
        children = [child for child in records if child.parent == index]
        if not region.simple or any(not child.simple for child in children):
            return _signed_area_sum(records)
        face = _net_face(region, children)
        if face is None:
            return _signed_area_sum(records)
        area = _measured_face_area(face)
        if area is None or not _net_area_is_consistent(area, region, children):
            return _signed_area_sum(records)
        total += area
    return total, True


def _net_face(region: _LoopRecord, children: Sequence[_LoopRecord]) -> TopoDS_Face | None:
    """Return the face of a region of material: its outer wire, holes cut out."""
    builder = BRepBuilderAPI_MakeFace(region.wire, True)
    if not builder.IsDone():
        return None
    for child in children:
        builder.Add(_hole_wire(child.wire, region.wire))
    face = builder.Face()
    if face.IsNull():
        return None
    return face


def _hole_wire(wire: TopoDS_Shape, outer: TopoDS_Shape) -> TopoDS_Shape:
    """Return a wire oriented against the outer wire, the way a hole needs it.

    The kernel reads a wire in a face as a hole when its orientation is
    against the orientation of the outer wire; a wire with the same
    orientation would be read as a second region of the face.
    """
    if wire.Orientation() == outer.Orientation():
        return TopoDS.Wire(wire.Reversed())
    return wire


def _net_area_is_consistent(
    area: float,
    region: _LoopRecord,
    children: Sequence[_LoopRecord],
) -> bool:
    """Return true when the measured net area really has the holes removed.

    A hole the kernel added as a second region instead of a hole makes the
    face larger than the region, and a hole it dropped makes it too small.
    Both are caught here, so a face the kernel did not build the way it was
    asked is never reported as a measurement.
    """
    slack = _AREA_CONSISTENCY_RATIO * max(1.0, region.area_mm2)
    lowest = region.area_mm2 - sum(child.area_mm2 for child in children) - slack
    highest = region.area_mm2 + slack
    return lowest <= area <= highest


def _signed_area_sum(records: Sequence[_LoopRecord]) -> tuple[float, bool]:
    """Return the net area summed from the loops, and whether it is exact."""
    total = sum(
        record.area_mm2 if record.depth % 2 == 0 else -record.area_mm2 for record in records
    )
    return total, all(record.area_exact for record in records)


def _connect_edges(edges: Sequence[TopoDS_Shape]) -> list[TopoDS_Shape]:
    """Return the wires built from the section edges."""
    sequence = HSequence_TopoDS_Shape()
    for edge in edges:
        sequence.Append(edge)
    wires = ShapeAnalysis_FreeBounds.ConnectEdgesToWires_s(
        sequence,
        _SECTION_WIRE_TOLERANCE_MM,
        False,
    )
    return [TopoDS.Wire(wires.Value(index)) for index in range(1, wires.Size() + 1)]


def _wire_points(
    wire: TopoDS_Shape,
    project: Callable[[_OcpPoint], Point2],
    chord_tolerance_mm: float,
) -> list[Point2]:
    """Return the points of a closed wire in plane coordinates."""
    points: list[Point2] = []
    explorer = BRepTools_WireExplorer(wire)
    while explorer.More():
        edge = TopoDS.Edge(explorer.Current())
        polyline = _discretise_edge(edge, project, chord_tolerance_mm)
        if polyline is not None:
            segment = list(polyline.points)
            if explorer.Orientation() == TopAbs_REVERSED:
                segment.reverse()
            if points and segment and _distance(points[-1], segment[0]) <= _GEOMETRY_EPSILON_MM:
                segment = segment[1:]
            points.extend(segment)
        explorer.Next()
    if len(points) >= 2 and _distance(points[0], points[-1]) <= _GEOMETRY_EPSILON_MM:
        points.pop()
    return points


def _classify_loops(records: Sequence[_LoopRecord]) -> None:
    """Classify every loop as material or hole, and record its nesting.

    A loop that sits inside a larger loop is a hole, a loop that is not
    inside any larger loop bounds the material. Nesting is handled by
    counting how many larger loops contain a loop, so a loop inside a hole
    counts as material again. The parent of a loop is the smallest loop
    that contains it: the region whose face the loop is cut out of.
    """
    for index, record in enumerate(records):
        depth = 0
        parent = -1
        parent_area = math.inf
        centroid = _centroid_of(record.points)
        for other_index, other in enumerate(records):
            if other_index == index:
                continue
            if other.area_mm2 <= record.area_mm2 + _AREA_EPSILON_MM2:
                continue
            if not _point_in_polygon(centroid, other.points):
                continue
            depth += 1
            if other.area_mm2 < parent_area:
                parent_area = other.area_mm2
                parent = other_index
        record.depth = depth
        record.parent = parent


def default_section_offset(shape: TopoDS_Shape) -> float:
    """Return the mid-plane offset of the shape's bounds, normal to Y.

    This is the default section position of ADR-0009: the plane that cuts
    the part through the middle of its depth.
    """
    _require_shape(shape)
    bounds = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, bounds)
    if not bounds.HasFinitePart():
        raise DrawingError("the shape has no finite bounds")
    minimum = bounds.CornerMin()
    maximum = bounds.CornerMax()
    return (float(minimum.Y()) + float(maximum.Y())) / 2.0


__all__ = (
    "DEFAULT_CHORD_TOLERANCE_MM",
    "DEFAULT_SECTION_CHORD_TOLERANCE_MM",
    "DEFAULT_VIEW_NAMES",
    "MAX_POINTS_PER_VIEW",
    "MAX_SECTION_POINTS",
    "Bounds2D",
    "Drawing",
    "DrawingError",
    "Polyline",
    "Section",
    "SectionLoop",
    "SectionPlane",
    "ViewDrawing",
    "ViewName",
    "default_section_offset",
    "hlr_views",
    "section",
)
