"""Registration of the adapter tools, checked without a FreeCAD installation."""

from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path
from typing import Any, cast

import pytest

import kala_agent_freecad
from kala_agent.contracts import SideEffectLevel, ToolCall, ToolDescriptor, ToolRunStatus
from kala_agent.tools.executor import IsolationMode, ToolExecutor
from kala_agent.tools.protocol import (
    SubprocessToolHandler,
    ToolErrorCode,
    canonical_json_digest,
    canonical_json_dumps,
)
from kala_agent.tools.registry import ToolRegistrationError, ToolRegistry
from kala_agent_freecad.availability import FreeCADAvailability, unavailable
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
    TOOL_CAPABILITIES,
    design_handler,
    fem_handler,
    geometry_handler,
)
from kala_agent_freecad.registration import (
    freecad_import_roots,
    register,
    register_if_available,
    tool_identifiers,
    worker_import_roots,
)

AVAILABLE = FreeCADAvailability(
    available=True,
    interpreter_path="/usr/bin/freecadcmd",
    executable_path="/usr/bin/freecad",
    import_roots=("/opt/freecad/lib",),
    version="1.1.3",
    version_parts=(1, 1, 3),
    calculix_path="",
    detail="FreeCAD 1.1.3 is available for import",
)


def _call(tool_id: str, version: str, call_id: str) -> ToolCall:
    return ToolCall(
        call_id=call_id,
        tool_id=tool_id,
        tool_version=version,
        arguments_json=canonical_json_dumps({"shape": "box", "export_formats": ["step"]}),
        requested_capabilities=TOOL_CAPABILITIES,
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        timeout_seconds=60.0,
        issued_at=datetime.now(UTC),
    )


def test_register_publishes_the_freecad_tools() -> None:
    registry = ToolRegistry()
    descriptors = register(registry, AVAILABLE)
    assert [descriptor.tool_id for descriptor in descriptors] == [
        GEOMETRY_TOOL_ID,
        FEM_TOOL_ID,
        DESIGN_TOOL_ID,
    ]
    assert registry.tool_ids() == (
        DESIGN_TOOL_ID,
        FEM_TOOL_ID,
        GEOMETRY_TOOL_ID,
    )
    assert registry.required_capabilities() == TOOL_CAPABILITIES
    geometry = registry.require(GEOMETRY_TOOL_ID, GEOMETRY_TOOL_VERSION)
    assert isinstance(geometry.handler, SubprocessToolHandler)
    assert geometry.input_schema_digest == geometry.handler.descriptor.input_schema_digest


def test_register_refuses_anything_but_a_registry() -> None:
    with pytest.raises(TypeError):
        register(cast("Any", object()), AVAILABLE)


def test_register_refuses_a_foreign_availability() -> None:
    with pytest.raises(TypeError):
        register(ToolRegistry(), cast("Any", "available"))


def test_register_is_a_no_op_without_freecad() -> None:
    registry = ToolRegistry()
    assert register(registry, unavailable("no freecadcmd here")) == ()
    assert len(registry) == 0
    assert registry.tool_ids() == ()


def test_register_if_available_is_a_no_op_without_freecad() -> None:
    registry = ToolRegistry()
    assert register_if_available(registry, unavailable("no freecadcmd here")) == ()
    assert len(registry) == 0


def test_register_if_available_registers_a_real_host() -> None:
    registry = ToolRegistry()
    descriptors = register_if_available(registry, AVAILABLE)
    assert len(descriptors) == 3
    assert len(registry) == 3


def test_registered_tool_cannot_run_in_the_kernel_process() -> None:
    registry = ToolRegistry()
    register(registry, AVAILABLE)
    with pytest.raises(ToolRegistrationError) as refusal:
        registry.register_trusted_evaluator(
            registry.require(GEOMETRY_TOOL_ID).descriptor,
            geometry_handler(),
            input_schema=GEOMETRY_INPUT_SCHEMA,
            output_schema=GEOMETRY_OUTPUT_SCHEMA,
        )
    assert "subprocess handler cannot run inside the kernel process" in str(refusal.value)


def test_trusted_mode_refuses_a_registered_adapter_tool(tmp_path: Path) -> None:
    registry = ToolRegistry()
    register(registry, AVAILABLE)
    executor = ToolExecutor(
        registry,
        run_directory=tmp_path,
        granted_capabilities=TOOL_CAPABILITIES,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(_call(GEOMETRY_TOOL_ID, GEOMETRY_TOOL_VERSION, "trusted-1"))
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == ToolErrorCode.ISOLATION_REQUIRED.value
    assert result.result_json is None


def test_default_mode_dispatches_the_tool_out_of_process(tmp_path: Path) -> None:
    registry = ToolRegistry()
    register(registry, AVAILABLE)
    executor = ToolExecutor(
        registry,
        run_directory=tmp_path,
        granted_capabilities=TOOL_CAPABILITIES,
    )
    tool = registry.require(GEOMETRY_TOOL_ID)
    assert tool.trusted_in_process is False
    assert isinstance(tool.handler, SubprocessToolHandler)
    assert executor.isolation is IsolationMode.AUTO


def test_registration_does_not_mutate_kernel_state() -> None:
    registry = ToolRegistry()
    before = set(registry.tool_ids())
    register(registry, AVAILABLE)
    assert set(ToolRegistry().tool_ids()) == before == set()
    assert not any(isinstance(value, ToolRegistry) for value in vars(kala_agent_freecad).values())


def test_import_roots_are_empty_without_freecad() -> None:
    absent = unavailable("no freecadcmd here")
    assert freecad_import_roots(absent) == ()
    assert worker_import_roots(absent) == (
        Path(kala_agent_freecad.__file__).resolve().parent.parent,
    )


def test_import_roots_include_the_freecad_root(tmp_path: Path) -> None:
    available = _available_with_root(tmp_path)
    assert freecad_import_roots(available) == (available.import_root_paths[0],)


def test_worker_import_roots_include_the_adapter(tmp_path: Path) -> None:
    available = _available_with_root(tmp_path)
    roots = worker_import_roots(available)
    assert available.import_root_paths[0] in roots
    package_parent = Path(kala_agent_freecad.__file__).resolve().parent.parent
    assert package_parent in roots
    assert len(roots) == len(set(roots))
    assert all(root.is_dir() for root in roots)


def _available_with_root(tmp_path: Path) -> FreeCADAvailability:
    lib_root = tmp_path / "opt" / "freecad" / "lib"
    lib_root.mkdir(parents=True, exist_ok=True)
    return FreeCADAvailability(
        available=True,
        interpreter_path="/usr/bin/freecadcmd",
        executable_path="/usr/bin/freecad",
        import_roots=(lib_root.as_posix(),),
        version="1.1.3",
        version_parts=(1, 1, 3),
        calculix_path="",
        detail="FreeCAD 1.1.3 is available for import",
    )


def test_tool_identifiers_are_closed() -> None:
    assert tool_identifiers() == (
        (GEOMETRY_TOOL_ID, GEOMETRY_TOOL_VERSION),
        (FEM_TOOL_ID, FEM_TOOL_VERSION),
        (DESIGN_TOOL_ID, DESIGN_TOOL_VERSION),
    )


def test_handlers_declare_the_entry_points_of_the_worker() -> None:
    assert geometry_handler().entry_point == "kala_agent_freecad.worker:invoke_geometry"
    assert fem_handler().entry_point == "kala_agent_freecad.worker:invoke_fem"
    assert design_handler().entry_point == "kala_agent_freecad.worker:invoke_design"
    assert geometry_handler().descriptor.tool_id == GEOMETRY_TOOL_ID
    assert fem_handler().descriptor.tool_id == FEM_TOOL_ID


def test_registered_descriptors_round_trip_through_the_registry() -> None:
    registry = ToolRegistry()
    descriptors: tuple[ToolDescriptor, ...] = register(registry, AVAILABLE)
    for descriptor in descriptors:
        registered = registry.require(descriptor.tool_id, descriptor.version)
        assert registered.descriptor == descriptor
        assert registered.verify_immutable() is registered


def test_schemas_are_the_ones_the_registry_bound() -> None:
    registry = ToolRegistry()
    register(registry, AVAILABLE)
    geometry = registry.require(GEOMETRY_TOOL_ID)
    assert geometry.input_schema_digest == canonical_json_digest(GEOMETRY_INPUT_SCHEMA)
    assert geometry.output_schema_digest == canonical_json_digest(GEOMETRY_OUTPUT_SCHEMA)
    fem = registry.require(FEM_TOOL_ID)
    assert fem.input_schema_digest == canonical_json_digest(FEM_INPUT_SCHEMA)
    assert fem.output_schema_digest == canonical_json_digest(FEM_OUTPUT_SCHEMA)
    design = registry.require(DESIGN_TOOL_ID)
    assert design.input_schema_digest == canonical_json_digest(DESIGN_INPUT_SCHEMA)
    assert design.output_schema_digest == canonical_json_digest(DESIGN_OUTPUT_SCHEMA)
