"""Named tool registry shared by planners and the UI."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from kala.cad.protocol import CadBackend, ToolResult, ToolSpec
from kala.parts.catalog import PartsCatalog


ToolHandler = Callable[..., ToolResult]


class ToolRegistry:
    def __init__(self) -> None:
        self._handlers: dict[str, ToolHandler] = {}
        self._specs: dict[str, ToolSpec] = {}

    def register(self, spec: ToolSpec, handler: ToolHandler) -> None:
        self._specs[spec.name] = spec
        self._handlers[spec.name] = handler

    def unregister(self, name: str) -> None:
        self._specs.pop(name, None)
        self._handlers.pop(name, None)

    def has(self, name: str) -> bool:
        return name in self._handlers

    def specs(self) -> list[ToolSpec]:
        return list(self._specs.values())

    def schemas_for_planner(self) -> list[dict[str, Any]]:
        return [
            {
                "name": s.name,
                "description": s.description,
                "parameters": s.parameters,
            }
            for s in self._specs.values()
        ]

    def call(self, name: str, **kwargs: Any) -> ToolResult:
        handler = self._handlers.get(name)
        if handler is None:
            return ToolResult(ok=False, message=f"Unknown tool: {name}")
        try:
            return handler(**kwargs)
        except TypeError as exc:
            return ToolResult(ok=False, message=f"Bad args for {name}: {exc}")
        except Exception as exc:  # noqa: BLE001 — surface tool failures to agent
            return ToolResult(ok=False, message=f"{name} failed: {exc}")


def _cad_specs() -> list[tuple[ToolSpec, str]]:
    """Return (spec, backend method name) pairs."""
    return [
        (
            ToolSpec(
                name="create_box",
                description=(
                    "Create a rectangular box from the origin along +X,+Y,+Z. "
                    "length=X, width=Y, height=Z (mm). For an L-bracket vertical wall "
                    "use a THIN width (thickness) and TALL height — do not stack two "
                    "flat plates."
                ),
                parameters={
                    "type": "object",
                    "properties": {
                        "length": {"type": "number"},
                        "width": {"type": "number"},
                        "height": {"type": "number"},
                        "label": {"type": "string"},
                    },
                    "required": ["length", "width", "height"],
                },
            ),
            "create_box",
        ),
        (
            ToolSpec(
                name="create_cylinder",
                description="Create a cylinder solid (shaft, pin, pipe body).",
                parameters={
                    "type": "object",
                    "properties": {
                        "radius": {"type": "number"},
                        "height": {"type": "number"},
                        "label": {"type": "string"},
                    },
                    "required": ["radius", "height"],
                },
            ),
            "create_cylinder",
        ),
        (
            ToolSpec(
                name="create_sphere",
                description="Create a sphere solid (ball).",
                parameters={
                    "type": "object",
                    "properties": {
                        "radius": {"type": "number"},
                        "label": {"type": "string"},
                    },
                    "required": ["radius"],
                },
            ),
            "create_sphere",
        ),
        (
            ToolSpec(
                name="create_cone",
                description="Create a cone solid.",
                parameters={
                    "type": "object",
                    "properties": {
                        "radius1": {"type": "number"},
                        "radius2": {"type": "number"},
                        "height": {"type": "number"},
                        "label": {"type": "string"},
                    },
                    "required": ["radius1", "radius2", "height"],
                },
            ),
            "create_cone",
        ),
        (
            ToolSpec(
                name="boolean_fuse",
                description="Fuse two bodies into one.",
                parameters={
                    "type": "object",
                    "properties": {
                        "body_a": {"type": "string"},
                        "body_b": {"type": "string"},
                    },
                    "required": ["body_a", "body_b"],
                },
            ),
            "boolean_fuse",
        ),
        (
            ToolSpec(
                name="boolean_cut",
                description="Cut body_b from body_a.",
                parameters={
                    "type": "object",
                    "properties": {
                        "body_a": {"type": "string"},
                        "body_b": {"type": "string"},
                    },
                    "required": ["body_a", "body_b"],
                },
            ),
            "boolean_cut",
        ),
        (
            ToolSpec(
                name="translate",
                description="Translate a body in XYZ (mm).",
                parameters={
                    "type": "object",
                    "properties": {
                        "body_id": {"type": "string"},
                        "x": {"type": "number"},
                        "y": {"type": "number"},
                        "z": {"type": "number"},
                    },
                    "required": ["body_id", "x", "y", "z"],
                },
            ),
            "translate",
        ),
        (
            ToolSpec(
                name="rotate",
                description=(
                    "Rotate a body about local axis x|y|z by angle_deg around center "
                    "(cx,cy,cz). Use 90° about X or Y to stand a plate vertical."
                ),
                parameters={
                    "type": "object",
                    "properties": {
                        "body_id": {"type": "string"},
                        "axis": {"type": "string"},
                        "angle_deg": {"type": "number"},
                        "cx": {"type": "number"},
                        "cy": {"type": "number"},
                        "cz": {"type": "number"},
                    },
                    "required": ["body_id", "axis", "angle_deg"],
                },
            ),
            "rotate",
        ),
        (
            ToolSpec(
                name="fillet",
                description="Fillet edges of a body.",
                parameters={
                    "type": "object",
                    "properties": {
                        "body_id": {"type": "string"},
                        "radius": {"type": "number"},
                    },
                    "required": ["body_id", "radius"],
                },
            ),
            "fillet",
        ),
        (
            ToolSpec(
                name="list_bodies",
                description="List bodies in the current document.",
                parameters={"type": "object", "properties": {}},
            ),
            "list_bodies",
        ),
        (
            ToolSpec(
                name="export",
                description=(
                    "Export a body to STEP/STL/BREP. For multi-part assemblies pass "
                    "body_id='ALL' to export every remaining solid as one STEP compound."
                ),
                parameters={
                    "type": "object",
                    "properties": {
                        "body_id": {"type": "string"},
                        "path": {"type": "string"},
                        "fmt": {"type": "string"},
                    },
                    "required": ["body_id", "path"],
                },
            ),
            "export",
        ),
    ]


def build_registry(
    backend: CadBackend,
    *,
    standard_parts: bool = False,
    catalog: PartsCatalog | None = None,
) -> ToolRegistry:
    registry = ToolRegistry()
    for spec, method_name in _cad_specs():
        method = getattr(backend, method_name)
        registry.register(spec, method)

    if standard_parts:
        cat = catalog or PartsCatalog.default()
        registry.register(
            ToolSpec(
                name="search_parts",
                description="Search the standard parts catalog.",
                parameters={
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"],
                },
            ),
            cat.search,
        )
        registry.register(
            ToolSpec(
                name="insert_part",
                description="Insert a standard part by catalog id.",
                parameters={
                    "type": "object",
                    "properties": {
                        "part_id": {"type": "string"},
                        "x": {"type": "number"},
                        "y": {"type": "number"},
                        "z": {"type": "number"},
                    },
                    "required": ["part_id"],
                },
            ),
            lambda part_id, x=0.0, y=0.0, z=0.0: cat.insert(
                backend, part_id, x=x, y=y, z=z
            ),
        )
    return registry
