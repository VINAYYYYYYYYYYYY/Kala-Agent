"""The studio integration registry.

The registry is the catalog of CAD and analysis tools the studio advertises.
It distinguishes neutral-file handoff capability from native API status so
that the UI can never claim an integration supports something the studio
cannot back up. Each row carries an explicit ``status`` and a ``notes``
field that documents platform and licence requirements. The data here is
static and never reaches the network.

Categories:

- ``cad`` covers the parametric CAD systems the studio can hand off to
  through STEP / IGES / 3D-PDF neutral files.
- ``analysis`` covers representative FEA / CAE products. None of them are
  wired through a native solver; the studio never runs a solver itself and
  never labels an analytic preview as a simulation result.

The five analysis entries (ANSYS Mechanical, Abaqus, COMSOL, Simcenter
Nastran, Altair OptiStruct) are documented as a representative set rather
than a ranked top-five.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Final

from kala_agent.studio.schemas import (
    IntegrationCategory,
    IntegrationRecord,
    IntegrationStatus,
)

_SUPPORTED_FORMATS: Final[frozenset[str]] = frozenset({"step", "stl"})

# Capabilities are restricted to neutral-file formats the studio can
# produce and that the integration is known to import. Listing an
# integration without a matching capability would invite a CAD user to
# attempt something the studio cannot deliver.
_CAPABILITIES_BY_INTEGRATION: Final[dict[str, tuple[str, ...]]] = {
    "solidworks": ("step",),
    "solid_edge": ("step",),
    "autocad": ("step",),
    "fusion360": ("step",),
    "freecad": ("step", "stl"),
    "openscad": ("stl",),
    "ansys_mechanical": ("step",),
    "abaqus": ("step",),
    "comsol": ("step",),
    "simcenter_nastran": ("step",),
    "altair_optistruct": ("step",),
}


@dataclass(frozen=True, slots=True)
class RegistryEntry:
    """Internal row used to construct :class:`IntegrationRecord`."""

    id: str
    name: str
    category: IntegrationCategory
    platforms: tuple[str, ...]
    status: IntegrationStatus
    notes: str


_REGISTRY: Final[tuple[RegistryEntry, ...]] = (
    # CAD: SOLIDWORKS, Solid Edge, AutoCAD, Fusion 360, FreeCAD, OpenSCAD.
    RegistryEntry(
        id="solidworks",
        name="SOLIDWORKS",
        category=IntegrationCategory.CAD,
        platforms=("windows", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "Native automation requires a SOLIDWORKS licence and Windows; the "
            "studio does not drive SOLIDWORKS through its COM automation and "
            "is limited to neutral-file imports. Hand off the STEP file the "
            "studio exports into the user's local SOLIDWORKS session."
        ),
    ),
    RegistryEntry(
        id="solid_edge",
        name="Solid Edge",
        category=IntegrationCategory.CAD,
        platforms=("windows", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "Native automation requires Solid Edge and a Windows host; the "
            "studio does not call into Solid Edge from this process and "
            "exposes STEP neutral-file handoff only."
        ),
    ),
    RegistryEntry(
        id="autocad",
        name="AutoCAD",
        category=IntegrationCategory.CAD,
        platforms=("windows", "macos", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "STEP support varies by AutoCAD release and product edition; "
            "operators should verify their local installation imports "
            "STEP before relying on a handoff. The studio does not drive "
            "AutoCAD through its automation API and exports the STEP file "
            "for the operator to import locally."
        ),
    ),
    RegistryEntry(
        id="fusion360",
        name="Fusion 360",
        category=IntegrationCategory.CAD,
        platforms=("windows", "macos", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "Fusion 360 imports STEP through its data panel; native REST "
            "automation requires a Fusion 360 licence and an authenticated "
            "client. The studio is limited to neutral-file imports and "
            "does not attempt to call the Fusion REST API."
        ),
    ),
    RegistryEntry(
        id="freecad",
        name="FreeCAD",
        category=IntegrationCategory.CAD,
        platforms=("windows", "macos", "linux", "open-source"),
        status=IntegrationStatus.AVAILABLE_VIA_NEUTRAL_FILE,
        notes=(
            "FreeCAD imports STEP and STL natively. The studio does not "
            "automate FreeCAD; the operator imports the exported STEP or "
            "STL in a local FreeCAD session."
        ),
    ),
    RegistryEntry(
        id="openscad",
        name="OpenSCAD",
        category=IntegrationCategory.CAD,
        platforms=("windows", "macos", "linux", "open-source"),
        status=IntegrationStatus.NOT_CONFIGURED,
        notes=(
            "The studio does not transpile geometry into OpenSCAD script; "
            "OpenSCAD's CSG language is not a parametric sweep of size, "
            "radius, and height. STL export is the supported handoff for "
            "this integration status; install OpenSCAD locally to view the "
            "studio STL output."
        ),
    ),
    # Analysis: ANSYS Mechanical, Abaqus, COMSOL, Simcenter Nastran, Altair
    # OptiStruct. The five entries are a representative set chosen to span
    # the breadth of downstream ecosystems; the studio does not rank them
    # and never substitutes one for another.
    RegistryEntry(
        id="ansys_mechanical",
        name="ANSYS Mechanical",
        category=IntegrationCategory.ANALYSIS,
        platforms=("windows", "linux", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "ANSYS Mechanical requires a Windows host with ANSYS installed "
            "and a paid licence. Geometry is imported through SpaceClaim or "
            "ANSYS Workbench, not directly by the Mechanical solver. The "
            "studio does not automate ANSYS Mechanical."
        ),
    ),
    RegistryEntry(
        id="abaqus",
        name="Abaqus",
        category=IntegrationCategory.ANALYSIS,
        platforms=("windows", "linux", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "Abaqus/CAE ingests STEP geometry through its part module and "
            "requires a paid licence. The studio exports the STEP file and "
            "the operator drives Abaqus/CAE locally; the studio does not "
            "run the Abaqus analysis."
        ),
    ),
    RegistryEntry(
        id="comsol",
        name="COMSOL Multiphysics",
        category=IntegrationCategory.ANALYSIS,
        platforms=("windows", "macos", "linux", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "COMSOL Multiphysics imports STEP through its geometry import "
            "node and requires a paid licence. The studio does not invoke "
            "the COMSOL server API; the operator completes meshing, "
            "physics, and the local solve in their COMSOL session."
        ),
    ),
    RegistryEntry(
        id="simcenter_nastran",
        name="Simcenter Nastran",
        category=IntegrationCategory.ANALYSIS,
        platforms=("windows", "linux", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "Simcenter Nastran does not ingest STEP directly; the geometry "
            "passes through Femap or Siemens NX as a preprocessor. The "
            "studio exports STEP for the geometry phase only; Nastran is "
            "a separate analysis product and is not driven by this process."
        ),
    ),
    RegistryEntry(
        id="altair_optistruct",
        name="Altair OptiStruct",
        category=IntegrationCategory.ANALYSIS,
        platforms=("windows", "linux", "commercial"),
        status=IntegrationStatus.NATIVE_API_UNAVAILABLE,
        notes=(
            "OptiStruct reads solver decks rather than STEP geometry; "
            "Altair HyperMesh is the mesh and property preprocessor. The "
            "studio exports STEP for the geometry phase only; OptiStruct "
            "is a separate analysis product and is not driven by this "
            "process."
        ),
    ),
)


def list_capabilities(integration_id: str) -> tuple[str, ...]:
    """Return the supported neutral-file formats for ``integration_id``."""
    return _CAPABILITIES_BY_INTEGRATION.get(integration_id, ())


def build_registry() -> tuple[IntegrationRecord, ...]:
    """Return the static, sanitized integration registry."""
    records = []
    for entry in _REGISTRY:
        capabilities = list_capabilities(entry.id)
        records.append(
            IntegrationRecord(
                id=entry.id,
                name=entry.name,
                category=entry.category,
                platforms=entry.platforms,
                status=entry.status,
                capabilities=capabilities,
                notes=entry.notes,
            )
        )
    return tuple(records)


def integration_by_id(integration_id: str) -> IntegrationRecord | None:
    """Return one registry row by id, or ``None`` when absent."""
    for row in _REGISTRY:
        if row.id == integration_id:
            return IntegrationRecord(
                id=row.id,
                name=row.name,
                category=row.category,
                platforms=row.platforms,
                status=row.status,
                capabilities=list_capabilities(row.id),
                notes=row.notes,
            )
    return None


__all__ = (
    "RegistryEntry",
    "build_registry",
    "integration_by_id",
    "list_capabilities",
)
