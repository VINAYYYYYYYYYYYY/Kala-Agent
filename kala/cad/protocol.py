"""Backend-agnostic CAD tool contracts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable


@dataclass(frozen=True)
class ToolSpec:
    """Description of a callable CAD/catalog tool."""

    name: str
    description: str
    parameters: dict[str, Any] = field(default_factory=dict)


@dataclass
class ToolResult:
    """Observation returned after a tool call."""

    ok: bool
    message: str
    data: dict[str, Any] = field(default_factory=dict)


@runtime_checkable
class CadBackend(Protocol):
    """Minimal CAD kernel surface used by the shared tool registry."""

    name: str

    def create_box(
        self, length: float, width: float, height: float, *, label: str = "Box"
    ) -> ToolResult: ...

    def create_cylinder(
        self, radius: float, height: float, *, label: str = "Cylinder"
    ) -> ToolResult: ...

    def boolean_fuse(self, body_a: str, body_b: str) -> ToolResult: ...

    def boolean_cut(self, body_a: str, body_b: str) -> ToolResult: ...

    def translate(self, body_id: str, x: float, y: float, z: float) -> ToolResult: ...

    def rotate(
        self,
        body_id: str,
        axis: str,
        angle_deg: float,
        *,
        cx: float = 0.0,
        cy: float = 0.0,
        cz: float = 0.0,
    ) -> ToolResult: ...

    def fillet(self, body_id: str, radius: float) -> ToolResult: ...

    def list_bodies(self) -> ToolResult: ...

    def export(self, body_id: str, path: str, fmt: str = "step") -> ToolResult: ...
