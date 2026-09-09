"""In-memory CAD backend for agent/UI testing without FreeCAD installed."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from kala.cad.protocol import ToolResult


@dataclass
class _Body:
    body_id: str
    kind: str
    params: dict
    pos: list[float] = field(default_factory=lambda: [0.0, 0.0, 0.0])


class MockBackend:
    """Records solids and writes placeholder export files."""

    name = "mock"

    def __init__(self) -> None:
        self._bodies: dict[str, _Body] = {}
        self._counter = 0

    def _next(self, prefix: str) -> str:
        self._counter += 1
        return f"{prefix}_{self._counter}"

    def create_box(
        self, length: float, width: float, height: float, *, label: str = "Box"
    ) -> ToolResult:
        body_id = self._next(label)
        self._bodies[body_id] = _Body(
            body_id, "box", {"length": length, "width": width, "height": height}
        )
        return ToolResult(
            ok=True,
            message=f"[mock] Created box {body_id}",
            data={"body_id": body_id, "dims": [length, width, height]},
        )

    def create_cylinder(
        self, radius: float, height: float, *, label: str = "Cylinder"
    ) -> ToolResult:
        body_id = self._next(label)
        self._bodies[body_id] = _Body(
            body_id, "cylinder", {"radius": radius, "height": height}
        )
        return ToolResult(
            ok=True,
            message=f"[mock] Created cylinder {body_id}",
            data={"body_id": body_id, "radius": radius, "height": height},
        )

    def create_sphere(self, radius: float, *, label: str = "Sphere") -> ToolResult:
        body_id = self._next(label)
        self._bodies[body_id] = _Body(body_id, "sphere", {"radius": radius})
        return ToolResult(
            ok=True,
            message=f"[mock] Created sphere {body_id}",
            data={"body_id": body_id, "radius": radius},
        )

    def create_cone(
        self, radius1: float, radius2: float, height: float, *, label: str = "Cone"
    ) -> ToolResult:
        body_id = self._next(label)
        self._bodies[body_id] = _Body(
            body_id,
            "cone",
            {"radius1": radius1, "radius2": radius2, "height": height},
        )
        return ToolResult(
            ok=True,
            message=f"[mock] Created cone {body_id}",
            data={
                "body_id": body_id,
                "radius1": radius1,
                "radius2": radius2,
                "height": height,
            },
        )

    def boolean_fuse(self, body_a: str, body_b: str) -> ToolResult:
        if body_a not in self._bodies or body_b not in self._bodies:
            return ToolResult(ok=False, message="Body not found for fuse")
        body_id = self._next("Fuse")
        self._bodies[body_id] = _Body(body_id, "fuse", {"a": body_a, "b": body_b})
        self._bodies.pop(body_a, None)
        self._bodies.pop(body_b, None)
        return ToolResult(
            ok=True,
            message=f"[mock] Fused {body_a}+{body_b} -> {body_id}",
            data={"body_id": body_id, "removed": [body_a, body_b]},
        )

    def boolean_cut(self, body_a: str, body_b: str) -> ToolResult:
        if body_a not in self._bodies or body_b not in self._bodies:
            return ToolResult(ok=False, message="Body not found for cut")
        body_id = self._next("Cut")
        self._bodies[body_id] = _Body(body_id, "cut", {"a": body_a, "b": body_b})
        self._bodies.pop(body_a, None)
        self._bodies.pop(body_b, None)
        return ToolResult(
            ok=True,
            message=f"[mock] Cut {body_b} from {body_a} -> {body_id}",
            data={"body_id": body_id, "removed": [body_a, body_b]},
        )

    def translate(self, body_id: str, x: float, y: float, z: float) -> ToolResult:
        body = self._bodies.get(body_id)
        if body is None:
            return ToolResult(ok=False, message=f"Body not found: {body_id}")
        body.pos = [body.pos[0] + x, body.pos[1] + y, body.pos[2] + z]
        return ToolResult(
            ok=True,
            message=f"[mock] Translated {body_id}",
            data={"body_id": body_id, "pos": body.pos},
        )

    def rotate(
        self,
        body_id: str,
        axis: str,
        angle_deg: float,
        *,
        cx: float = 0.0,
        cy: float = 0.0,
        cz: float = 0.0,
    ) -> ToolResult:
        if body_id not in self._bodies:
            return ToolResult(ok=False, message=f"Body not found: {body_id}")
        return ToolResult(
            ok=True,
            message=f"[mock] Rotated {body_id} {angle_deg} about {axis}",
            data={
                "body_id": body_id,
                "axis": axis,
                "angle_deg": float(angle_deg),
                "center": [cx, cy, cz],
            },
        )

    def fillet(self, body_id: str, radius: float) -> ToolResult:
        if body_id not in self._bodies:
            return ToolResult(ok=False, message=f"Body not found: {body_id}")
        out_id = self._next("Fillet")
        self._bodies[out_id] = _Body(out_id, "fillet", {"src": body_id, "radius": radius})
        self._bodies.pop(body_id, None)
        return ToolResult(
            ok=True,
            message=f"[mock] Filleted {body_id} -> {out_id}",
            data={"body_id": out_id, "radius": radius, "removed": [body_id]},
        )

    def list_bodies(self) -> ToolResult:
        bodies = [
            {"body_id": b.body_id, "kind": b.kind, "params": b.params, "pos": b.pos}
            for b in self._bodies.values()
        ]
        return ToolResult(ok=True, message=f"[mock] {len(bodies)} bodies", data={"bodies": bodies})

    def export(self, body_id: str, path: str, fmt: str = "step") -> ToolResult:
        if str(body_id).upper() in {"*", "ALL", "__ALL__", "ASSEMBLY"}:
            out = Path(path)
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(f"# mock assembly export ({fmt})\nbodies={list(self._bodies)}\n", encoding="utf-8")
            return ToolResult(ok=True, message=f"[mock] Exported assembly -> {out}", data={"path": str(out), "body_id": "ASSEMBLY", "fmt": fmt.lower(), "body_count": len(self._bodies)})
        if body_id not in self._bodies:
            return ToolResult(ok=False, message=f"Body not found: {body_id}")
        out = Path(path)
        out.parent.mkdir(parents=True, exist_ok=True)
        body = self._bodies[body_id]
        out.write_text(
            f"# mock export ({fmt})\nbody={body_id}\nkind={body.kind}\nparams={body.params}\n",
            encoding="utf-8",
        )
        return ToolResult(
            ok=True,
            message=f"[mock] Exported {body_id} -> {out}",
            data={"path": str(out), "body_id": body_id, "fmt": fmt.lower()},
        )
