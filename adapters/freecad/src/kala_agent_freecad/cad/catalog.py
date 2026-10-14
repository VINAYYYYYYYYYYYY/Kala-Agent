"""The FreeCAD modeling tools an agent is allowed to call.

Names follow the Sketcher, PartDesign, Part, and FEM toolbars. A view toggle
is not a tool: it does not change the model. The procedure, not this list,
decides whether a call is legal at the current stage.
"""

from __future__ import annotations

from typing import Final

SKETCH_GEOMETRY: Final[tuple[str, ...]] = (
    "sketch.point",
    "sketch.line",
    "sketch.polyline",
    "sketch.arc",
    "sketch.arc_3point",
    "sketch.arc_ellipse",
    "sketch.arc_hyperbola",
    "sketch.arc_parabola",
    "sketch.circle",
    "sketch.circle_3point",
    "sketch.ellipse",
    "sketch.ellipse_3point",
    "sketch.rectangle",
    "sketch.rectangle_centered",
    "sketch.rectangle_rounded",
    "sketch.polygon",
    "sketch.slot",
    "sketch.slot_arc",
    "sketch.bspline",
    "sketch.bspline_periodic",
    "sketch.construction",
)
SKETCH_CONSTRAINTS: Final[tuple[str, ...]] = (
    "sketch.coincident",
    "sketch.point_on_object",
    "sketch.horizontal",
    "sketch.vertical",
    "sketch.parallel",
    "sketch.perpendicular",
    "sketch.tangent",
    "sketch.equal",
    "sketch.symmetric",
    "sketch.block",
    "sketch.distance_x",
    "sketch.distance_y",
    "sketch.distance",
    "sketch.radius",
    "sketch.diameter",
    "sketch.angle",
    "sketch.lock",
    "sketch.driving",
)
SKETCH_EDIT: Final[tuple[str, ...]] = (
    "sketch.trim",
    "sketch.split",
    "sketch.extend",
    "sketch.fillet",
    "sketch.chamfer",
    "sketch.offset",
    "sketch.mirror",
    "sketch.array",
    "sketch.rotate",
    "sketch.scale",
    "sketch.external",
    "sketch.external_intersection",
    "sketch.carbon_copy",
    "sketch.delete_geometry",
    "sketch.delete_constraints",
    "sketch.bspline_convert",
    "sketch.bspline_degree",
    "sketch.bspline_knot",
    "sketch.join",
)
SKETCH_LIFECYCLE: Final[tuple[str, ...]] = (
    "sketch.new",
    "sketch.edit",
    "sketch.attach",
    "sketch.reorient",
    "sketch.validate",
    "sketch.merge",
    "sketch.mirror_sketch",
    "sketch.leave",
)
PARTDESIGN_SETUP: Final[tuple[str, ...]] = (
    "partdesign.body",
    "partdesign.datum_point",
    "partdesign.datum_line",
    "partdesign.datum_plane",
    "partdesign.coordinate_system",
    "partdesign.shape_binder",
    "partdesign.sub_shape_binder",
    "partdesign.clone",
    "partdesign.placement",
)
PARTDESIGN_ADDITIVE: Final[tuple[str, ...]] = (
    "partdesign.pad",
    "partdesign.revolution",
    "partdesign.loft",
    "partdesign.pipe",
    "partdesign.helix",
    "partdesign.additive_box",
    "partdesign.additive_cylinder",
    "partdesign.additive_sphere",
    "partdesign.additive_cone",
    "partdesign.additive_ellipsoid",
    "partdesign.additive_torus",
    "partdesign.additive_prism",
    "partdesign.additive_wedge",
)
PARTDESIGN_SUBTRACTIVE: Final[tuple[str, ...]] = (
    "partdesign.pocket",
    "partdesign.hole",
    "partdesign.groove",
    "partdesign.subtractive_loft",
    "partdesign.subtractive_pipe",
    "partdesign.subtractive_helix",
    "partdesign.subtractive_box",
    "partdesign.subtractive_cylinder",
    "partdesign.subtractive_sphere",
    "partdesign.subtractive_cone",
    "partdesign.subtractive_ellipsoid",
    "partdesign.subtractive_torus",
    "partdesign.subtractive_prism",
    "partdesign.subtractive_wedge",
)
PARTDESIGN_DRESSUP: Final[tuple[str, ...]] = (
    "partdesign.fillet",
    "partdesign.chamfer",
    "partdesign.draft",
    "partdesign.thickness",
)
PARTDESIGN_PATTERN: Final[tuple[str, ...]] = (
    "partdesign.mirror",
    "partdesign.linear_pattern",
    "partdesign.polar_pattern",
    "partdesign.multitransform",
)
PART_PRIMITIVES: Final[tuple[str, ...]] = (
    "part.box",
    "part.cylinder",
    "part.sphere",
    "part.cone",
    "part.torus",
    "part.tube",
)
PART_PROFILES: Final[tuple[str, ...]] = (
    "part.extrude",
    "part.revolve",
    "part.loft",
    "part.sweep",
)
PART_SOLID: Final[tuple[str, ...]] = (
    "part.mirror",
    "part.scale",
    "part.fillet",
    "part.chamfer",
    "part.thickness",
    "part.offset",
    "part.face",
    "part.ruled",
    "part.offset3d",
    "part.section",
    "part.cross_section",
    "part.projection",
)
PART_BOOLEANS: Final[tuple[str, ...]] = (
    "part.cut",
    "part.fuse",
    "part.common",
    "part.compound",
    "part.explode",
    "part.fragments",
    "part.slice",
    "part.xor",
    "part.defeaturing",
    "part.check",
)
FEM_TOOLS: Final[tuple[str, ...]] = (
    "fem.analysis",
    "fem.material",
    "fem.fixed",
    "fem.force",
    "fem.pressure",
    "fem.displacement",
    "fem.mesh",
    "fem.solve",
    "fem.result",
)

CATALOG: Final[frozenset[str]] = frozenset(
    (
        *SKETCH_GEOMETRY,
        *SKETCH_CONSTRAINTS,
        *SKETCH_EDIT,
        *SKETCH_LIFECYCLE,
        *PARTDESIGN_SETUP,
        *PARTDESIGN_ADDITIVE,
        *PARTDESIGN_SUBTRACTIVE,
        *PARTDESIGN_DRESSUP,
        *PARTDESIGN_PATTERN,
        "partdesign.boolean",
        "partdesign.check",
        *PART_PRIMITIVES,
        *PART_PROFILES,
        *PART_SOLID,
        *PART_BOOLEANS,
        *FEM_TOOLS,
    )
)

__all__ = (
    "CATALOG",
    "FEM_TOOLS",
    "PARTDESIGN_ADDITIVE",
    "PARTDESIGN_DRESSUP",
    "PARTDESIGN_PATTERN",
    "PARTDESIGN_SETUP",
    "PARTDESIGN_SUBTRACTIVE",
    "PART_BOOLEANS",
    "PART_PRIMITIVES",
    "PART_PROFILES",
    "PART_SOLID",
    "SKETCH_CONSTRAINTS",
    "SKETCH_EDIT",
    "SKETCH_GEOMETRY",
    "SKETCH_LIFECYCLE",
)
