"""Registration of the FreeCAD tools into a kernel tool registry.

Importing this module never imports FreeCAD. Availability is discovered from
the host filesystem, and a host without FreeCAD simply registers nothing
rather than registering a tool that cannot run.
"""

from __future__ import annotations

from pathlib import Path
from typing import Final

from kala_agent.contracts import ToolDescriptor
from kala_agent.tools.registry import ToolRegistry
from kala_agent_freecad.availability import FreeCADAvailability, discover
from kala_agent_freecad.contracts import (
    DESIGN_INPUT_SCHEMA,
    DESIGN_OUTPUT_SCHEMA,
    DESIGN_TOOL_ID,
    DESIGN_TOOL_VERSION,
    FEM_INPUT_SCHEMA,
    FEM_OUTPUT_SCHEMA,
    FEM_TOOL_ID,
    FEM_TOOL_VERSION,
    GEOMETRY_INPUT_SCHEMA,
    GEOMETRY_OUTPUT_SCHEMA,
    GEOMETRY_TOOL_ID,
    GEOMETRY_TOOL_VERSION,
    design_descriptor,
    design_handler,
    fem_descriptor,
    fem_handler,
    geometry_descriptor,
    geometry_handler,
)

REGISTRATIONS: Final[tuple[tuple[str, str], ...]] = (
    (GEOMETRY_TOOL_ID, GEOMETRY_TOOL_VERSION),
    (FEM_TOOL_ID, FEM_TOOL_VERSION),
    (DESIGN_TOOL_ID, DESIGN_TOOL_VERSION),
)


def register(
    registry: ToolRegistry,
    availability: FreeCADAvailability | None = None,
) -> tuple[ToolDescriptor, ...]:
    """Register the FreeCAD tools and return the descriptors that were registered."""
    if not isinstance(registry, ToolRegistry):
        raise TypeError("registry must be a ToolRegistry")
    resolved = availability if availability is not None else discover()
    if not isinstance(resolved, FreeCADAvailability):
        raise TypeError("availability must be a FreeCADAvailability")
    if not resolved.available:
        return ()
    registry.register(
        geometry_descriptor(),
        geometry_handler(),
        input_schema=GEOMETRY_INPUT_SCHEMA,
        output_schema=GEOMETRY_OUTPUT_SCHEMA,
    )
    registry.register(
        fem_descriptor(),
        fem_handler(),
        input_schema=FEM_INPUT_SCHEMA,
        output_schema=FEM_OUTPUT_SCHEMA,
    )
    registry.register(
        design_descriptor(),
        design_handler(),
        input_schema=DESIGN_INPUT_SCHEMA,
        output_schema=DESIGN_OUTPUT_SCHEMA,
    )
    return (
        geometry_descriptor(),
        fem_descriptor(),
        design_descriptor(),
    )


def register_if_available(
    registry: ToolRegistry,
    availability: FreeCADAvailability | None = None,
) -> tuple[ToolDescriptor, ...]:
    """Register the tools only on a host that really has FreeCAD."""
    resolved = availability if availability is not None else discover()
    if not resolved.available:
        return ()
    return register(registry, resolved)


def freecad_import_roots(
    availability: FreeCADAvailability | None = None,
) -> tuple[Path, ...]:
    """Return the loader paths a worker needs in order to import FreeCAD.

    An operator passes these to the executor as ``import_roots`` so the child
    interpreter can import the native module without inheriting a host path.
    """
    resolved = availability if availability is not None else discover()
    if not resolved.available:
        return ()
    return resolved.import_root_paths


def worker_import_roots(
    availability: FreeCADAvailability | None = None,
) -> tuple[Path, ...]:
    """Return every import root a FreeCAD worker needs, adapter included.

    A worker has to import this adapter and FreeCAD, and nothing else that the
    host happens to have on its path. The kernel itself is expected to be
    installed in the worker interpreter, which is what the distribution
    dependency guarantees.
    """
    resolved = availability if availability is not None else discover()
    roots: list[Path] = []
    seen: set[str] = set()
    candidates = (
        *freecad_import_roots(resolved),
        Path(__file__).resolve().parent.parent,
    )
    for candidate in candidates:
        if not candidate.is_dir():
            continue
        text = candidate.as_posix()
        if text in seen:
            continue
        seen.add(text)
        roots.append(candidate)
    return tuple(roots)


def tool_identifiers() -> tuple[tuple[str, str], ...]:
    """Return the ``(tool_id, version)`` pairs this adapter publishes."""
    return REGISTRATIONS


__all__ = (
    "REGISTRATIONS",
    "freecad_import_roots",
    "register",
    "register_if_available",
    "tool_identifiers",
    "worker_import_roots",
)
