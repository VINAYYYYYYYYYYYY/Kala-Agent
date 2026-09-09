"""FreeCAD MCP server — CAD ops as MCP tools (tool-calling, not freeform scripts)."""

from __future__ import annotations

import json

from mcp.server.mcpserver import MCPServer

from kala.cad.factory import create_backend
from kala.cad.protocol import CadBackend, ToolResult

mcp = MCPServer(
    name="kala-freecad",
    instructions=(
        "FreeCAD CAD tools for Kala-Agent. Call these tools to build geometry. "
        "Do not invent FreeCAD Python scripts — use the tools."
    ),
)

_backend: CadBackend | None = None


def get_backend() -> CadBackend:
    global _backend
    if _backend is None:
        _backend = create_backend("freecad")
    return _backend


def _payload(result: ToolResult) -> str:
    return json.dumps(
        {
            "ok": result.ok,
            "message": result.message,
            "data": result.data,
        },
        indent=2,
    )


@mcp.tool()
def create_box(
    length: float,
    width: float,
    height: float,
    label: str = "Box",
) -> str:
    """Create a box solid in FreeCAD (Part.makeBox → Part::Feature)."""
    return _payload(get_backend().create_box(length, width, height, label=label))


@mcp.tool()
def create_cylinder(
    radius: float,
    height: float,
    label: str = "Cylinder",
) -> str:
    """Create a cylinder solid in FreeCAD (Part.makeCylinder → Part::Feature)."""
    return _payload(get_backend().create_cylinder(radius, height, label=label))


@mcp.tool()
def boolean_fuse(body_a: str, body_b: str) -> str:
    """Fuse two FreeCAD bodies (Shape.fuse)."""
    return _payload(get_backend().boolean_fuse(body_a, body_b))


@mcp.tool()
def boolean_cut(body_a: str, body_b: str) -> str:
    """Cut body_b from body_a in FreeCAD (Shape.cut)."""
    return _payload(get_backend().boolean_cut(body_a, body_b))


@mcp.tool()
def translate(body_id: str, x: float, y: float, z: float) -> str:
    """Translate a FreeCAD body (Placement.Base)."""
    return _payload(get_backend().translate(body_id, x, y, z))


@mcp.tool()
def fillet(body_id: str, radius: float) -> str:
    """Fillet edges of a FreeCAD body (Shape.makeFillet)."""
    return _payload(get_backend().fillet(body_id, radius))


@mcp.tool()
def list_bodies() -> str:
    """List FreeCAD bodies in the current Kala document."""
    return _payload(get_backend().list_bodies())


@mcp.tool()
def export(body_id: str, path: str, fmt: str = "step") -> str:
    """Export a FreeCAD body to STEP, STL, or BREP."""
    return _payload(get_backend().export(body_id, path, fmt=fmt))


@mcp.tool()
def live_document() -> str:
    """Return the path of the live FreeCAD document Kala publishes for the GUI."""
    from kala.cad.freecad.gui_sync import default_live_path, default_step_path

    return json.dumps(
        {
            "ok": True,
            "fcstd": str(default_live_path()),
            "step": str(default_step_path()),
            "hint": "Open/restart FreeCAD so Mod/Kala can auto-reload the FCStd.",
        },
        indent=2,
    )


def main() -> None:
    """stdio MCP entrypoint for Cursor / other MCP clients."""
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
