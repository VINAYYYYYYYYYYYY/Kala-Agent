"""The frozen tool contracts of the FreeCAD adapter.

Everything an operator, a registry, or the executor needs to trust this adapter
lives here: the two tool identities, their strict input and output schemas,
their canonical digests, the descriptors and subprocess handlers, the
capability and CAD profile constants, and the entry points the executor
dispatches. FreeCAD is a system dependency, so nothing here imports it.
"""

from __future__ import annotations

from collections.abc import Mapping
from types import MappingProxyType
from typing import Final

from kala_agent.contracts import SideEffectLevel, ToolDescriptor
from kala_agent.tools.protocol import (
    SubprocessToolHandler,
    canonical_json_digest,
    check_object_schema,
)

DISTRIBUTION_NAME: Final[str] = "kala-agent-freecad"
GEOMETRY_TOOL_ID: Final[str] = "cad.freecad-geometry"
GEOMETRY_TOOL_VERSION: Final[str] = "1.0.0"
FEM_TOOL_ID: Final[str] = "cad.freecad-fem"
FEM_TOOL_VERSION: Final[str] = "1.0.0"
GEOMETRY_ENTRY_POINT: Final[str] = "kala_agent_freecad.worker:invoke_geometry"
FEM_ENTRY_POINT: Final[str] = "kala_agent_freecad.worker:invoke_fem"
TOOL_CAPABILITIES: Final[tuple[str, ...]] = (
    "artifact.write",
    "design.write",
    "tool.execute",
)
SHAPE_KINDS: Final[tuple[str, ...]] = ("box", "cylinder")
GEOMETRY_EXPORT_FORMATS: Final[tuple[str, ...]] = ("step", "iges", "fcstd")
FEM_EXPORT_FORMATS: Final[tuple[str, ...]] = ("fcstd", "step", "frd")
ARTIFACT_DIRECTORY: Final[str] = "artifacts"
DEFAULT_DOCUMENT_NAME: Final[str] = "kala-freecad"
DEFAULT_ARTIFACT_STEM: Final[str] = "kala-model"
DEFAULT_ANALYSIS_NAME: Final[str] = "KalaAnalysis"
DEFAULT_MATERIAL_NAME: Final[str] = "StructuralSteel"
SOLVER_ENGINE: Final[str] = "calculix_ccx_tools"
SOLVER_STATUSES: Final[tuple[str, ...]] = (
    "not_requested",
    "solver_unavailable",
    "prerequisites_unmet",
    "completed",
    "failed",
)

GEOMETRY_TIMEOUT_SECONDS: Final[float] = 180.0
FEM_TIMEOUT_SECONDS: Final[float] = 900.0

CAD_PROFILE_NAME: Final[str] = "freecad-cad"
CAD_MAX_ARTIFACT_BYTES: Final[int] = 268_435_456
CAD_MAX_OUTPUT_BYTES: Final[int] = 1_048_576
CAD_MAX_ARGUMENT_BYTES: Final[int] = 65_536
CAD_DEFAULT_TIMEOUT_SECONDS: Final[float] = 900.0
CAD_MAX_WORKERS: Final[int] = 2
CAD_MAX_QUEUE_SECONDS: Final[float] = 30.0

EXPORT_SUFFIXES: Final[Mapping[str, str]] = MappingProxyType(
    {
        "fcstd": ".fcstd",
        "frd": ".frd",
        "iges": ".iges",
        "step": ".step",
    }
)
EXPORT_MEDIA_TYPES: Final[Mapping[str, str]] = MappingProxyType(
    {
        "fcstd": "application/x-free-cad",
        "frd": "text/plain",
        "iges": "model/iges",
        "step": "model/step",
    }
)

_DOCUMENT_NAME: Final[Mapping[str, object]] = MappingProxyType(
    {
        "default": "kala-freecad",
        "description": "FreeCAD document name, bounded to identifier characters.",
        "maxLength": 64,
        "pattern": "^[A-Za-z][A-Za-z0-9_-]{0,63}$",
        "type": "string",
    }
)
_ARTIFACT_STEM: Final[Mapping[str, object]] = MappingProxyType(
    {
        "default": "kala-model",
        "description": "Workspace relative file stem for every exported artifact.",
        "maxLength": 64,
        "pattern": "^[a-z0-9][a-z0-9-]{0,63}$",
        "type": "string",
    }
)
_SHAPE: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Primitive solid to build.",
        "enum": list(SHAPE_KINDS),
        "type": "string",
    }
)
_LENGTH_MM: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Box length along X in millimetres.",
        "maximum": 5_000.0,
        "minimum": 0.001,
        "type": "number",
    }
)
_WIDTH_MM: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Box width along Y in millimetres.",
        "maximum": 5_000.0,
        "minimum": 0.001,
        "type": "number",
    }
)
_HEIGHT_MM: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Box height along Z or cylinder height in millimetres.",
        "maximum": 5_000.0,
        "minimum": 0.001,
        "type": "number",
    }
)
_RADIUS_MM: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Cylinder radius in millimetres.",
        "maximum": 2_500.0,
        "minimum": 0.001,
        "type": "number",
    }
)
_WARNINGS: Final[Mapping[str, object]] = MappingProxyType(
    {
        "items": MappingProxyType({"maxLength": 512, "type": "string"}),
        "maxItems": 16,
        "type": "array",
    }
)
_ARTIFACT_DIGEST: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Lowercase SHA-256 digest of the artifact bytes.",
        "maxLength": 64,
        "minLength": 64,
        "pattern": "^[0-9a-f]{64}$",
        "type": "string",
    }
)
_ARTIFACT_RELATIVE_PATH: Final[Mapping[str, object]] = MappingProxyType(
    {
        "description": "Workspace relative POSIX path of the artifact.",
        "maxLength": 256,
        "pattern": "^[a-z0-9][a-z0-9._/-]{0,254}$",
        "type": "string",
    }
)
_OBJECT_NAME: Final[Mapping[str, object]] = MappingProxyType(
    {
        "maxLength": 64,
        "minLength": 1,
        "type": "string",
    }
)
_MEASUREMENT: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "area_mm2": {"type": "number"},
                "bounding_box_mm": MappingProxyType(
                    {
                        "items": {"type": "number"},
                        "maxItems": 3,
                        "minItems": 3,
                        "type": "array",
                    }
                ),
                "edge_count": {"minimum": 0, "type": "integer"},
                "face_count": {"minimum": 0, "type": "integer"},
                "solid": {"type": "boolean"},
                "vertex_count": {"minimum": 0, "type": "integer"},
                "volume_mm3": {"type": "number"},
            }
        ),
        "required": [
            "area_mm2",
            "bounding_box_mm",
            "edge_count",
            "face_count",
            "solid",
            "vertex_count",
            "volume_mm3",
        ],
        "title": "cad.freecad-geometry.measurement",
        "type": "object",
    }
)

GEOMETRY_ARTIFACT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "digest": _ARTIFACT_DIGEST,
                "format": {"enum": list(GEOMETRY_EXPORT_FORMATS), "type": "string"},
                "media_type": {"maxLength": 127, "type": "string"},
                "relative_path": _ARTIFACT_RELATIVE_PATH,
                "size_bytes": {"minimum": 1, "type": "integer"},
            }
        ),
        "required": ["digest", "format", "media_type", "relative_path", "size_bytes"],
        "title": "cad.freecad-geometry.artifact",
        "type": "object",
    }
)

FEM_ARTIFACT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "digest": _ARTIFACT_DIGEST,
                "format": {"enum": list(FEM_EXPORT_FORMATS), "type": "string"},
                "media_type": {"maxLength": 127, "type": "string"},
                "relative_path": _ARTIFACT_RELATIVE_PATH,
                "size_bytes": {"minimum": 1, "type": "integer"},
            }
        ),
        "required": ["digest", "format", "media_type", "relative_path", "size_bytes"],
        "title": "cad.freecad-fem.artifact",
        "type": "object",
    }
)

GEOMETRY_INPUT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "artifact_stem": _ARTIFACT_STEM,
                "document_name": _DOCUMENT_NAME,
                "export_formats": MappingProxyType(
                    {
                        "description": "Formats to write into the worker workspace.",
                        "items": {"enum": list(GEOMETRY_EXPORT_FORMATS), "type": "string"},
                        "maxItems": len(GEOMETRY_EXPORT_FORMATS),
                        "minItems": 1,
                        "type": "array",
                        "uniqueItems": True,
                    }
                ),
                "height_mm": _HEIGHT_MM,
                "length_mm": _LENGTH_MM,
                "measure": {"default": True, "type": "boolean"},
                "radius_mm": _RADIUS_MM,
                "shape": _SHAPE,
                "width_mm": _WIDTH_MM,
            }
        ),
        "required": ["export_formats", "shape"],
        "title": "cad.freecad-geometry.input",
        "type": "object",
    }
)

GEOMETRY_OUTPUT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "artifacts": MappingProxyType(
                    {
                        "items": GEOMETRY_ARTIFACT_SCHEMA,
                        "maxItems": len(GEOMETRY_EXPORT_FORMATS),
                        "type": "array",
                    }
                ),
                "document_name": _OBJECT_NAME,
                "freecad_version": {"maxLength": 64, "type": "string"},
                "import_root": {"maxLength": 1_024, "type": "string"},
                "measurement": _MEASUREMENT,
                "shape": {"enum": list(SHAPE_KINDS), "type": "string"},
                "shape_name": {"maxLength": 64, "type": "string"},
                "valid": {"type": "boolean"},
                "warnings": _WARNINGS,
            }
        ),
        "required": [
            "artifacts",
            "document_name",
            "freecad_version",
            "import_root",
            "measurement",
            "shape",
            "shape_name",
            "valid",
            "warnings",
        ],
        "title": "cad.freecad-geometry.output",
        "type": "object",
    }
)

FEM_INPUT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "analysis_name": MappingProxyType(
                    {
                        "default": "KalaAnalysis",
                        "description": "FreeCAD analysis object name.",
                        "maxLength": 64,
                        "pattern": "^[A-Za-z][A-Za-z0-9_-]{0,63}$",
                        "type": "string",
                    }
                ),
                "artifact_stem": _ARTIFACT_STEM,
                "density_kg_m3": {"maximum": 22_000.0, "minimum": 1.0, "type": "number"},
                "document_name": _DOCUMENT_NAME,
                "export_solid": {"default": True, "type": "boolean"},
                "fixed_face": {
                    "default": 1,
                    "description": "One based index of the face held fixed.",
                    "maximum": 6,
                    "minimum": 1,
                    "type": "integer",
                },
                "height_mm": _HEIGHT_MM,
                "length_mm": _LENGTH_MM,
                "material_name": MappingProxyType(
                    {
                        "default": "StructuralSteel",
                        "maxLength": 64,
                        "pattern": "^[A-Za-z0-9][A-Za-z0-9 ._-]{0,63}$",
                        "type": "string",
                    }
                ),
                "poisson_ratio": {
                    "exclusiveMaximum": 0.5,
                    "exclusiveMinimum": 0.0,
                    "type": "number",
                },
                "radius_mm": _RADIUS_MM,
                "run_solver": {
                    "default": False,
                    "description": (
                        "Only a safely discovered CalculiX executable may ever run; "
                        "without one the call reports solver_unavailable."
                    ),
                    "type": "boolean",
                },
                "shape": _SHAPE,
                "width_mm": _WIDTH_MM,
                "youngs_modulus_mpa": {"maximum": 1_000_000.0, "minimum": 1.0, "type": "number"},
            }
        ),
        "required": ["shape"],
        "title": "cad.freecad-fem.input",
        "type": "object",
    }
)

FEM_OUTPUT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "analysis_name": _OBJECT_NAME,
                "artifacts": MappingProxyType(
                    {
                        "items": FEM_ARTIFACT_SCHEMA,
                        "maxItems": len(FEM_EXPORT_FORMATS),
                        "type": "array",
                    }
                ),
                "document_name": _OBJECT_NAME,
                "freecad_version": {"maxLength": 64, "type": "string"},
                "import_root": {"maxLength": 1_024, "type": "string"},
                "material": MappingProxyType(
                    {
                        "additionalProperties": False,
                        "properties": MappingProxyType(
                            {
                                "density_kg_m3": {"type": "number"},
                                "name": {"maxLength": 64, "type": "string"},
                                "object_name": _OBJECT_NAME,
                                "poisson_ratio": {"type": "number"},
                                "youngs_modulus_mpa": {"type": "number"},
                            }
                        ),
                        "required": [
                            "density_kg_m3",
                            "name",
                            "object_name",
                            "poisson_ratio",
                            "youngs_modulus_mpa",
                        ],
                        "title": "cad.freecad-fem.material",
                        "type": "object",
                    }
                ),
                "mesh": MappingProxyType(
                    {
                        "additionalProperties": False,
                        "properties": MappingProxyType(
                            {
                                "element_count": {"minimum": 0, "type": "integer"},
                                "generated": {"type": "boolean"},
                                "node_count": {"minimum": 0, "type": "integer"},
                                "object_name": _OBJECT_NAME,
                            }
                        ),
                        "required": ["element_count", "generated", "node_count", "object_name"],
                        "title": "cad.freecad-fem.mesh",
                        "type": "object",
                    }
                ),
                "objects": MappingProxyType(
                    {
                        "additionalProperties": False,
                        "properties": MappingProxyType(
                            {
                                "analysis": _OBJECT_NAME,
                                "constraint": _OBJECT_NAME,
                                "material": _OBJECT_NAME,
                                "mesh": _OBJECT_NAME,
                                "solver": _OBJECT_NAME,
                            }
                        ),
                        "required": ["analysis", "constraint", "material", "mesh", "solver"],
                        "title": "cad.freecad-fem.objects",
                        "type": "object",
                    }
                ),
                "results": MappingProxyType(
                    {
                        "additionalProperties": False,
                        "properties": MappingProxyType(
                            {
                                "available": {"type": "boolean"},
                                "element_count": {"minimum": 0, "type": "integer"},
                                "max_displacement_mm": {"type": ["number", "null"]},
                                "max_stress_mpa": {"type": ["number", "null"]},
                                "message": {"maxLength": 512, "type": ["string", "null"]},
                                "node_count": {"minimum": 0, "type": "integer"},
                            }
                        ),
                        "required": [
                            "available",
                            "element_count",
                            "max_displacement_mm",
                            "max_stress_mpa",
                            "message",
                            "node_count",
                        ],
                        "title": "cad.freecad-fem.results",
                        "type": "object",
                    }
                ),
                "shape": {"enum": list(SHAPE_KINDS), "type": "string"},
                "solver": MappingProxyType(
                    {
                        "additionalProperties": False,
                        "properties": MappingProxyType(
                            {
                                "available": {"type": "boolean"},
                                "engine": {"const": SOLVER_ENGINE, "type": "string"},
                                "executable": {"maxLength": 1_024, "type": ["string", "null"]},
                                "executed": {"type": "boolean"},
                                "message": {"maxLength": 512, "type": ["string", "null"]},
                                "object_name": _OBJECT_NAME,
                                "status": {"enum": list(SOLVER_STATUSES), "type": "string"},
                            }
                        ),
                        "required": [
                            "available",
                            "engine",
                            "executable",
                            "executed",
                            "message",
                            "object_name",
                            "status",
                        ],
                        "title": "cad.freecad-fem.solver",
                        "type": "object",
                    }
                ),
                "warnings": _WARNINGS,
            }
        ),
        "required": [
            "analysis_name",
            "artifacts",
            "document_name",
            "freecad_version",
            "import_root",
            "material",
            "mesh",
            "objects",
            "results",
            "shape",
            "solver",
            "warnings",
        ],
        "title": "cad.freecad-fem.output",
        "type": "object",
    }
)

GEOMETRY_INPUT_DIGEST: Final[str] = canonical_json_digest(GEOMETRY_INPUT_SCHEMA)
GEOMETRY_OUTPUT_DIGEST: Final[str] = canonical_json_digest(GEOMETRY_OUTPUT_SCHEMA)
FEM_INPUT_DIGEST: Final[str] = canonical_json_digest(FEM_INPUT_SCHEMA)
FEM_OUTPUT_DIGEST: Final[str] = canonical_json_digest(FEM_OUTPUT_SCHEMA)

GEOMETRY_CAPABILITIES: Final[tuple[str, ...]] = TOOL_CAPABILITIES
FEM_CAPABILITIES: Final[tuple[str, ...]] = TOOL_CAPABILITIES


def geometry_descriptor(
    *,
    tool_id: str = GEOMETRY_TOOL_ID,
    version: str = GEOMETRY_TOOL_VERSION,
) -> ToolDescriptor:
    return ToolDescriptor(
        tool_id=tool_id,
        version=version,
        name="FreeCAD geometry builder",
        description=(
            "Builds one primitive solid in a FreeCAD worker, measures it, and "
            "writes the requested STEP, IGES, or FCStd exports into the call "
            "workspace. FreeCAD is an optional system dependency, so the call "
            "fails closed when the interpreter cannot be imported."
        ),
        input_schema_digest=GEOMETRY_INPUT_DIGEST,
        output_schema_digest=GEOMETRY_OUTPUT_DIGEST,
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        required_capabilities=GEOMETRY_CAPABILITIES,
        timeout_seconds=GEOMETRY_TIMEOUT_SECONDS,
        destructive=False,
        supports_structured_output=True,
        idempotent=True,
    )


def fem_descriptor(
    *,
    tool_id: str = FEM_TOOL_ID,
    version: str = FEM_TOOL_VERSION,
) -> ToolDescriptor:
    return ToolDescriptor(
        tool_id=tool_id,
        version=version,
        name="FreeCAD FEM analysis builder",
        description=(
            "Builds a FreeCAD Fem analysis with a solid, material, fixed "
            "constraint, mesh object, and CalculiX solver object, and writes "
            "the analysis document into the call workspace. The solver runs "
            "only when a safely discovered CalculiX executable exists; "
            "without one the call reports solver_unavailable and no result is "
            "invented."
        ),
        input_schema_digest=FEM_INPUT_DIGEST,
        output_schema_digest=FEM_OUTPUT_DIGEST,
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        required_capabilities=FEM_CAPABILITIES,
        timeout_seconds=FEM_TIMEOUT_SECONDS,
        destructive=False,
        supports_structured_output=True,
        idempotent=True,
    )


def geometry_handler(
    *,
    tool_id: str = GEOMETRY_TOOL_ID,
    version: str = GEOMETRY_TOOL_VERSION,
) -> SubprocessToolHandler:
    return SubprocessToolHandler(
        descriptor=geometry_descriptor(tool_id=tool_id, version=version),
        entry_point=GEOMETRY_ENTRY_POINT,
    )


def fem_handler(
    *,
    tool_id: str = FEM_TOOL_ID,
    version: str = FEM_TOOL_VERSION,
) -> SubprocessToolHandler:
    return SubprocessToolHandler(
        descriptor=fem_descriptor(tool_id=tool_id, version=version),
        entry_point=FEM_ENTRY_POINT,
    )


def validate_contract_schemas() -> None:
    """Fail loudly at import time if a shipped schema is not admissible."""
    for label, schema in (
        ("GEOMETRY_INPUT_SCHEMA", GEOMETRY_INPUT_SCHEMA),
        ("GEOMETRY_OUTPUT_SCHEMA", GEOMETRY_OUTPUT_SCHEMA),
        ("FEM_INPUT_SCHEMA", FEM_INPUT_SCHEMA),
        ("FEM_OUTPUT_SCHEMA", FEM_OUTPUT_SCHEMA),
    ):
        check_object_schema(schema, label=label)


validate_contract_schemas()

__all__ = (
    "ARTIFACT_DIRECTORY",
    "CAD_DEFAULT_TIMEOUT_SECONDS",
    "CAD_MAX_ARGUMENT_BYTES",
    "CAD_MAX_ARTIFACT_BYTES",
    "CAD_MAX_OUTPUT_BYTES",
    "CAD_MAX_QUEUE_SECONDS",
    "CAD_MAX_WORKERS",
    "CAD_PROFILE_NAME",
    "DEFAULT_ANALYSIS_NAME",
    "DEFAULT_ARTIFACT_STEM",
    "DEFAULT_DOCUMENT_NAME",
    "DEFAULT_MATERIAL_NAME",
    "DISTRIBUTION_NAME",
    "EXPORT_MEDIA_TYPES",
    "EXPORT_SUFFIXES",
    "FEM_ARTIFACT_SCHEMA",
    "FEM_CAPABILITIES",
    "FEM_ENTRY_POINT",
    "FEM_EXPORT_FORMATS",
    "FEM_INPUT_DIGEST",
    "FEM_INPUT_SCHEMA",
    "FEM_OUTPUT_DIGEST",
    "FEM_OUTPUT_SCHEMA",
    "FEM_TIMEOUT_SECONDS",
    "FEM_TOOL_ID",
    "FEM_TOOL_VERSION",
    "GEOMETRY_ARTIFACT_SCHEMA",
    "GEOMETRY_CAPABILITIES",
    "GEOMETRY_ENTRY_POINT",
    "GEOMETRY_EXPORT_FORMATS",
    "GEOMETRY_INPUT_DIGEST",
    "GEOMETRY_INPUT_SCHEMA",
    "GEOMETRY_OUTPUT_DIGEST",
    "GEOMETRY_OUTPUT_SCHEMA",
    "GEOMETRY_TIMEOUT_SECONDS",
    "GEOMETRY_TOOL_ID",
    "GEOMETRY_TOOL_VERSION",
    "SHAPE_KINDS",
    "SOLVER_ENGINE",
    "SOLVER_STATUSES",
    "TOOL_CAPABILITIES",
    "fem_descriptor",
    "fem_handler",
    "geometry_descriptor",
    "geometry_handler",
    "validate_contract_schemas",
)
