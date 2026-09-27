"""The shipped tool contracts, checked without a FreeCAD installation."""

from __future__ import annotations

import importlib
from collections.abc import Mapping
from types import MappingProxyType
from typing import Any, Final, cast

import pytest

from kala_agent.contracts import SideEffectLevel, ToolDescriptor
from kala_agent.tools.executor import MAX_DEFAULT_TIMEOUT_SECONDS
from kala_agent.tools.protocol import (
    PROTOCOL_VERSION,
    SchemaValidationError,
    SubprocessToolHandler,
    ToolEnvelopeKind,
    ToolProtocolError,
    ToolRequestEnvelope,
    canonical_json_digest,
    check_object_schema,
    validate_against_schema,
)
from kala_agent.tools.registry import ToolRegistrationError, ToolRegistry
from kala_agent_freecad.contracts import (
    EXPORT_MEDIA_TYPES,
    EXPORT_SUFFIXES,
    FEM_CAPABILITIES,
    FEM_ENTRY_POINT,
    FEM_INPUT_DIGEST,
    FEM_INPUT_SCHEMA,
    FEM_OUTPUT_DIGEST,
    FEM_OUTPUT_SCHEMA,
    FEM_TIMEOUT_SECONDS,
    FEM_TOOL_ID,
    FEM_TOOL_VERSION,
    GEOMETRY_CAPABILITIES,
    GEOMETRY_ENTRY_POINT,
    GEOMETRY_INPUT_DIGEST,
    GEOMETRY_INPUT_SCHEMA,
    GEOMETRY_OUTPUT_DIGEST,
    GEOMETRY_OUTPUT_SCHEMA,
    GEOMETRY_TIMEOUT_SECONDS,
    GEOMETRY_TOOL_ID,
    GEOMETRY_TOOL_VERSION,
    SHAPE_KINDS,
    SOLVER_STATUSES,
    fem_descriptor,
    fem_handler,
    geometry_descriptor,
    geometry_handler,
)

SCHEMAS: Final[Mapping[str, Mapping[str, object]]] = MappingProxyType(
    {
        "GEOMETRY_INPUT_SCHEMA": GEOMETRY_INPUT_SCHEMA,
        "GEOMETRY_OUTPUT_SCHEMA": GEOMETRY_OUTPUT_SCHEMA,
        "FEM_INPUT_SCHEMA": FEM_INPUT_SCHEMA,
        "FEM_OUTPUT_SCHEMA": FEM_OUTPUT_SCHEMA,
    }
)
PRIVILEGED_CAPABILITIES: Final[frozenset[str]] = frozenset(
    {
        "egress",
        "export",
        "network",
        "network.access",
        "network.egress",
        "network.internet",
        "package.install",
        "process.spawn",
        "shell.execute",
        "system.admin",
    }
)


@pytest.mark.parametrize(("label", "schema"), tuple(SCHEMAS.items()))
def test_schema_is_a_frozen_strict_object(label: str, schema: Mapping[str, object]) -> None:
    assert isinstance(schema, MappingProxyType), label
    assert schema["type"] == "object", label
    assert schema["additionalProperties"] is False, label
    properties = schema["properties"]
    assert isinstance(properties, Mapping) and properties, label
    required = schema["required"]
    assert isinstance(required, (list, tuple)) and required, label
    assert set(required) <= set(properties), label
    check_object_schema(schema, label=label)


@pytest.mark.parametrize(("label", "schema"), tuple(SCHEMAS.items()))
def test_nested_schemas_are_strict(label: str, schema: Mapping[str, object]) -> None:
    stack: list[Mapping[str, object]] = [dict(schema)]
    while stack:
        current = stack.pop()
        nested = current.get("properties")
        if isinstance(nested, Mapping):
            for subschema in nested.values():
                if isinstance(subschema, Mapping):
                    if subschema.get("type") == "object":
                        assert subschema.get("additionalProperties") is False, label
                    stack.append(subschema)
        items = current.get("items")
        if isinstance(items, Mapping):
            stack.append(items)


def subschema(schema: Mapping[str, object], name: str) -> Mapping[str, Any]:
    value = schema[name]
    assert isinstance(value, Mapping), name
    return cast("Mapping[str, Any]", value)


def property_schema(properties: Mapping[str, Any], name: str) -> Mapping[str, Any]:
    return subschema(properties, name)


@pytest.mark.parametrize(
    ("schema", "digest"),
    (
        (GEOMETRY_INPUT_SCHEMA, GEOMETRY_INPUT_DIGEST),
        (GEOMETRY_OUTPUT_SCHEMA, GEOMETRY_OUTPUT_DIGEST),
        (FEM_INPUT_SCHEMA, FEM_INPUT_DIGEST),
        (FEM_OUTPUT_SCHEMA, FEM_OUTPUT_DIGEST),
    ),
)
def test_digest_is_canonical(
    schema: Mapping[str, object],
    digest: str,
) -> None:
    assert digest == canonical_json_digest(schema)
    assert len(digest) == 64
    assert set(digest) <= set("0123456789abcdef")


def test_digests_bind_different_schemas() -> None:
    assert GEOMETRY_INPUT_DIGEST != GEOMETRY_OUTPUT_DIGEST
    assert FEM_INPUT_DIGEST != FEM_OUTPUT_DIGEST
    assert GEOMETRY_INPUT_DIGEST != FEM_INPUT_DIGEST


@pytest.mark.parametrize(
    ("descriptor", "input_digest", "output_digest", "timeout"),
    (
        (
            geometry_descriptor(),
            GEOMETRY_INPUT_DIGEST,
            GEOMETRY_OUTPUT_DIGEST,
            GEOMETRY_TIMEOUT_SECONDS,
        ),
        (fem_descriptor(), FEM_INPUT_DIGEST, FEM_OUTPUT_DIGEST, FEM_TIMEOUT_SECONDS),
    ),
)
def test_descriptor_declares_local_work_only(
    descriptor: ToolDescriptor,
    input_digest: str,
    output_digest: str,
    timeout: float,
) -> None:
    assert descriptor.input_schema_digest == input_digest
    assert descriptor.output_schema_digest == output_digest
    assert descriptor.side_effect_level is SideEffectLevel.LOCAL_WRITE
    assert descriptor.required_capabilities
    assert set(descriptor.required_capabilities) & PRIVILEGED_CAPABILITIES == set()
    assert descriptor.destructive is False
    assert descriptor.supports_structured_output is True
    assert descriptor.idempotent is True
    assert 0.0 < descriptor.timeout_seconds == timeout <= MAX_DEFAULT_TIMEOUT_SECONDS


def test_tool_identities_are_distinct() -> None:
    assert geometry_descriptor().tool_id == GEOMETRY_TOOL_ID
    assert geometry_descriptor().version == GEOMETRY_TOOL_VERSION
    assert fem_descriptor().tool_id == FEM_TOOL_ID
    assert fem_descriptor().version == FEM_TOOL_VERSION
    assert GEOMETRY_CAPABILITIES == FEM_CAPABILITIES
    assert GEOMETRY_CAPABILITIES == ("artifact.write", "design.write", "tool.execute")


@pytest.mark.parametrize(
    ("handler", "entry_point"),
    (
        (geometry_handler(), GEOMETRY_ENTRY_POINT),
        (fem_handler(), FEM_ENTRY_POINT),
    ),
)
def test_entry_point_resolves_to_a_callable(
    handler: SubprocessToolHandler,
    entry_point: str,
) -> None:
    module_name, _, attribute = entry_point.partition(":")
    module = importlib.import_module(module_name)
    assert callable(getattr(module, attribute))
    assert handler.entry_point == entry_point
    with pytest.raises(ToolProtocolError):
        handler.invoke(_minimal_request())


def test_export_table_covers_every_format() -> None:
    assert set(EXPORT_SUFFIXES) == set(EXPORT_MEDIA_TYPES)
    assert set(EXPORT_SUFFIXES) == {"step", "iges", "fcstd", "frd"}
    for suffix in EXPORT_SUFFIXES.values():
        assert suffix.startswith(".")
        assert suffix == suffix.lower()
    for media_type in EXPORT_MEDIA_TYPES.values():
        assert media_type == media_type.lower()
        assert "/" in media_type


def _minimal_request() -> ToolRequestEnvelope:
    return ToolRequestEnvelope(
        protocol_version=PROTOCOL_VERSION,
        kind=ToolEnvelopeKind.REQUEST,
        call_id="kala-contract-1",
        tool_id=GEOMETRY_TOOL_ID,
        tool_version=GEOMETRY_TOOL_VERSION,
        arguments={},
        workspace="/tmp",
        deadline_seconds=1.0,
        max_output_bytes=4_096,
        max_artifact_bytes=4_096,
        granted_capabilities=(),
    )


def test_geometry_input_accepts_a_plain_box_request() -> None:
    validate_against_schema(
        {
            "export_formats": ["step"],
            "height_mm": 20.0,
            "length_mm": 120.0,
            "shape": "box",
            "width_mm": 40.0,
        },
        GEOMETRY_INPUT_SCHEMA,
        path="$",
    )


@pytest.mark.parametrize(
    "arguments",
    (
        {"shape": "sphere", "export_formats": ["step"]},
        {"shape": "box"},
        {"shape": "box", "export_formats": []},
        {"shape": "box", "export_formats": ["step", "step"]},
        {"shape": "box", "export_formats": ["step"], "length_mm": -1.0},
        {"shape": "box", "export_formats": ["step"], "length_mm": 0.0},
        {"shape": "box", "export_formats": ["step"], "measure": "yes"},
        {"shape": "box", "export_formats": ["step"], "unknown": 1},
        {"shape": "box", "export_formats": ["step"], "document_name": "../escape"},
        {"shape": "box", "export_formats": ["step"], "artifact_stem": "../escape"},
        {"shape": "box", "export_formats": ["step"], "artifact_stem": "UPPER"},
    ),
)
def test_geometry_input_rejects_bad_requests(arguments: dict[str, Any]) -> None:
    with pytest.raises(SchemaValidationError):
        validate_against_schema(arguments, GEOMETRY_INPUT_SCHEMA, path="$")


@pytest.mark.parametrize(
    "arguments",
    (
        {"shape": "box", "poisson_ratio": 0.5},
        {"shape": "box", "poisson_ratio": 0.0},
        {"shape": "box", "fixed_face": 0},
        {"shape": "box", "fixed_face": 7},
        {"shape": "box", "fixed_face": 1.5},
        {"shape": "box", "run_solver": "true"},
    ),
)
def test_fem_input_rejects_bad_requests(arguments: dict[str, Any]) -> None:
    with pytest.raises(SchemaValidationError):
        validate_against_schema(arguments, FEM_INPUT_SCHEMA, path="$")


def test_fem_input_leaves_shape_specific_arguments_to_the_worker() -> None:
    validate_against_schema(
        {"shape": "cylinder", "length_mm": 10.0},
        FEM_INPUT_SCHEMA,
        path="$",
    )


def test_fem_input_accepts_a_plain_box_request() -> None:
    validate_against_schema(
        {"shape": "box", "length_mm": 120.0, "run_solver": True},
        FEM_INPUT_SCHEMA,
        path="$",
    )


def test_fem_output_declares_the_honest_solver_surface() -> None:
    properties = subschema(FEM_OUTPUT_SCHEMA, "properties")
    solver = property_schema(properties, "solver")
    assert subschema(solver, "properties") is not None
    status = property_schema(subschema(solver, "properties"), "status")
    assert status["enum"] == list(SOLVER_STATUSES)
    assert "solver_unavailable" in SOLVER_STATUSES
    results = property_schema(properties, "results")
    assert results["required"] == [
        "available",
        "element_count",
        "max_displacement_mm",
        "max_stress_mpa",
        "message",
        "node_count",
    ]


def test_geometry_output_declares_every_measurement() -> None:
    measurement = property_schema(
        subschema(GEOMETRY_OUTPUT_SCHEMA, "properties"),
        "measurement",
    )
    assert measurement["required"] == [
        "area_mm2",
        "bounding_box_mm",
        "edge_count",
        "face_count",
        "solid",
        "vertex_count",
        "volume_mm3",
    ]


def test_shape_enumeration_is_closed() -> None:
    shape = property_schema(subschema(GEOMETRY_INPUT_SCHEMA, "properties"), "shape")
    assert shape["enum"] == list(SHAPE_KINDS)
    assert SHAPE_KINDS == ("box", "cylinder")


def test_registry_binds_the_descriptor_to_the_schemas() -> None:
    registry = ToolRegistry()
    registry.register(
        geometry_descriptor(),
        geometry_handler(),
        input_schema=GEOMETRY_INPUT_SCHEMA,
        output_schema=GEOMETRY_OUTPUT_SCHEMA,
    )
    registered = registry.require(GEOMETRY_TOOL_ID, GEOMETRY_TOOL_VERSION)
    assert registered.input_schema_digest == GEOMETRY_INPUT_DIGEST
    assert registered.output_schema_digest == GEOMETRY_OUTPUT_DIGEST
    assert registered.trusted_in_process is False


def test_registry_refuses_a_digest_that_does_not_match() -> None:
    registry = ToolRegistry()
    descriptor = geometry_descriptor().model_copy(update={"input_schema_digest": FEM_INPUT_DIGEST})
    with pytest.raises(ToolRegistrationError):
        registry.register(
            descriptor,
            geometry_handler(),
            input_schema=GEOMETRY_INPUT_SCHEMA,
            output_schema=GEOMETRY_OUTPUT_SCHEMA,
        )
