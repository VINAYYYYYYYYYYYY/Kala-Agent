from __future__ import annotations

import math
import os
import stat
import subprocess
import sys
import threading
import time
from collections.abc import Mapping
from dataclasses import dataclass, replace
from pathlib import Path
from types import MappingProxyType
from typing import Final, cast

from kala_agent.contracts import (
    ArtifactClassification,
    ArtifactRef,
    SideEffectLevel,
    ToolDescriptor,
    ToolRunStatus,
)
from kala_agent.tools.protocol import (
    JSONValue,
    SubprocessToolHandler,
    ToolErrorCode,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    ToolWorkspaceError,
    canonical_json_digest,
    canonical_json_dumps,
    resolve_workspace_path,
    sha256_hex,
)
from kala_agent.tools.registry import ToolRegistry

SIMULATOR_TOOL_ID: Final[str] = "mechanical.reference-simulator"
SIMULATOR_TOOL_VERSION: Final[str] = "1.0.0"
SIMULATOR_ENTRY_POINT: Final[str] = "kala_agent.tools.simulator:invoke"
SIMULATOR_REPORT_DIRECTORY: Final[str] = "outputs"
SIMULATOR_REPORT_NAME: Final[str] = "reference-report.json"
SIMULATOR_CAPABILITIES: Final[tuple[str, ...]] = (
    "artifact.read",
    "artifact.write",
    "design.read",
    "tool.execute",
)
EVALUATOR_TOOL_ID: Final[str] = "mechanical.reference-evaluator"
EVALUATOR_TOOL_VERSION: Final[str] = "1.0.0"
EVALUATOR_CAPABILITIES: Final[tuple[str, ...]] = ()
MISBEHAVIOR_ENTRY_POINTS: Final[tuple[str, ...]] = (
    "kala_agent.tools.simulator:crash_invoke",
    "kala_agent.tools.simulator:escape_invoke",
    "kala_agent.tools.simulator:flood_invoke",
    "kala_agent.tools.simulator:hang_invoke",
    "kala_agent.tools.simulator:liar_invoke",
    "kala_agent.tools.simulator:lingering_child_invoke",
    "kala_agent.tools.simulator:stray_child_invoke",
    "kala_agent.tools.simulator:unrecognized_code_invoke",
)
MISBEHAVIOR_SLEEP_SECONDS: Final[float] = 30.0
MISBEHAVIOR_EXIT_CODE: Final[int] = 9
MISBEHAVIOR_FLOOD_CHUNKS: Final[int] = 32
MISBEHAVIOR_ESCAPE_NAME: Final[str] = "escaped-report.json"
MISBEHAVIOR_STRAY_CHILD_NAME: Final[str] = "stray-child-effect.txt"
MISBEHAVIOR_STRAY_CHILD_DELAY_SECONDS: Final[float] = 1.0
STRAY_CHILD_SOURCE: Final[str] = """
import pathlib, sys, time
time.sleep(float(sys.argv[1]))
target = pathlib.Path(sys.argv[2])
try:
    target.write_bytes(b"late side effect")
except OSError:
    pass
"""
_OPERATIONS: Final[tuple[str, ...]] = ("beam_volume", "bar_summary")
_DIMENSION_LIMITS: Final[Mapping[str, tuple[float, float]]] = MappingProxyType(
    {
        "density_kg_m3": (100.0, 20_000.0),
        "height_mm": (1.0, 500.0),
        "length_mm": (10.0, 5_000.0),
        "width_mm": (1.0, 500.0),
    }
)
_DEFAULTS: Final[Mapping[str, float]] = MappingProxyType(
    {
        "density_kg_m3": 2_700.0,
        "height_mm": 10.0,
        "length_mm": 120.0,
        "width_mm": 20.0,
    }
)
_ROUNDING_DIGITS: Final[int] = 6
_MAX_REPORTED_VOLUME_MM3: Final[float] = 1_250_000_000.0
_MAX_REPORTED_MASS_KG: Final[float] = 27_000.0
_MAX_REPORTED_SLENDERNESS: Final[float] = 5_000.0

INPUT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "density_kg_m3": {
                    "default": 2_700.0,
                    "maximum": 20_000.0,
                    "minimum": 100.0,
                    "type": "number",
                },
                "height_mm": {
                    "default": 10.0,
                    "maximum": 500.0,
                    "minimum": 1.0,
                    "type": "number",
                },
                "length_mm": {
                    "default": 120.0,
                    "maximum": 5_000.0,
                    "minimum": 10.0,
                    "type": "number",
                },
                "operation": {
                    "default": "beam_volume",
                    "enum": list(_OPERATIONS),
                    "type": "string",
                },
                "width_mm": {
                    "default": 20.0,
                    "maximum": 500.0,
                    "minimum": 1.0,
                    "type": "number",
                },
                "write_report": {"default": False, "type": "boolean"},
            }
        ),
        "required": ["operation"],
        "title": "mechanical.reference-simulator.input",
        "type": "object",
    }
)

OUTPUT_SCHEMA: Final[Mapping[str, object]] = MappingProxyType(
    {
        "additionalProperties": False,
        "properties": MappingProxyType(
            {
                "mass_kg": {"type": "number"},
                "operation": {"enum": list(_OPERATIONS), "type": "string"},
                "report_path": {"type": ["string", "null"]},
                "second_moment_mm4": {"type": "number"},
                "slenderness_ratio": {"type": "number"},
                "surface_area_mm2": {"type": "number"},
                "volume_mm3": {"type": "number"},
                "within_limits": {"type": "boolean"},
            }
        ),
        "required": [
            "mass_kg",
            "operation",
            "report_path",
            "second_moment_mm4",
            "slenderness_ratio",
            "surface_area_mm2",
            "volume_mm3",
            "within_limits",
        ],
        "title": "mechanical.reference-simulator.output",
        "type": "object",
    }
)


class SimulatorInputError(ValueError):
    pass


@dataclass(frozen=True, slots=True)
class SimulatorParameters:
    operation: str
    length_mm: float
    width_mm: float
    height_mm: float
    density_kg_m3: float
    write_report: bool

    def to_payload(self) -> dict[str, JSONValue]:
        return {
            "density_kg_m3": self.density_kg_m3,
            "height_mm": self.height_mm,
            "length_mm": self.length_mm,
            "operation": self.operation,
            "width_mm": self.width_mm,
            "write_report": self.write_report,
        }


def simulator_descriptor(
    *,
    tool_id: str = SIMULATOR_TOOL_ID,
    version: str = SIMULATOR_TOOL_VERSION,
) -> ToolDescriptor:
    return ToolDescriptor(
        tool_id=tool_id,
        version=version,
        name="Mechanical reference simulator",
        description=(
            "Deterministic closed-form reference calculation for a prismatic bar. "
            "It is an arithmetic reference, not an engineering validation."
        ),
        input_schema_digest=canonical_json_digest(INPUT_SCHEMA),
        output_schema_digest=canonical_json_digest(OUTPUT_SCHEMA),
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        required_capabilities=SIMULATOR_CAPABILITIES,
        timeout_seconds=20.0,
        destructive=False,
        supports_structured_output=True,
        idempotent=True,
    )


def simulator_handler(
    *,
    tool_id: str = SIMULATOR_TOOL_ID,
    version: str = SIMULATOR_TOOL_VERSION,
) -> SubprocessToolHandler:
    return SubprocessToolHandler(
        descriptor=simulator_descriptor(tool_id=tool_id, version=version),
        entry_point=SIMULATOR_ENTRY_POINT,
    )


def parse_parameters(arguments: Mapping[str, object]) -> SimulatorParameters:
    if not isinstance(arguments, Mapping):
        raise SimulatorInputError("arguments must be a JSON object")
    operation = arguments.get("operation", "beam_volume")
    if not isinstance(operation, str) or operation not in _OPERATIONS:
        raise SimulatorInputError(f"operation must be one of {', '.join(_OPERATIONS)}")
    values: dict[str, float] = {}
    for name, default in _DEFAULTS.items():
        supplied = arguments.get(name, default)
        if isinstance(supplied, bool) or not isinstance(supplied, (int, float)):
            raise SimulatorInputError(f"{name} must be a number")
        numeric = float(supplied)
        if not math.isfinite(numeric):
            raise SimulatorInputError(f"{name} must be finite")
        low, high = _DIMENSION_LIMITS[name]
        if not low <= numeric <= high:
            raise SimulatorInputError(f"{name} must be between {low} and {high}")
        values[name] = numeric
    write_report = arguments.get("write_report", False)
    if not isinstance(write_report, bool):
        raise SimulatorInputError("write_report must be a boolean")
    return SimulatorParameters(
        operation=operation,
        length_mm=values["length_mm"],
        width_mm=values["width_mm"],
        height_mm=values["height_mm"],
        density_kg_m3=values["density_kg_m3"],
        write_report=write_report,
    )


def compute_metrics(parameters: SimulatorParameters) -> dict[str, JSONValue]:
    volume = parameters.length_mm * parameters.width_mm * parameters.height_mm
    mass = volume * parameters.density_kg_m3 * 1e-9
    second_moment = (parameters.width_mm * parameters.height_mm**3 / 12.0) * (parameters.length_mm)
    surface_area = 2.0 * (
        parameters.length_mm * parameters.width_mm
        + parameters.length_mm * parameters.height_mm
        + parameters.width_mm * parameters.height_mm
    )
    slenderness = parameters.length_mm / max(parameters.height_mm, 1.0)
    return {
        "mass_kg": round(mass, _ROUNDING_DIGITS),
        "second_moment_mm4": round(second_moment, _ROUNDING_DIGITS),
        "slenderness_ratio": round(slenderness, _ROUNDING_DIGITS),
        "surface_area_mm2": round(surface_area, _ROUNDING_DIGITS),
        "volume_mm3": round(volume, _ROUNDING_DIGITS),
        "within_limits": _within_limits(volume, mass, slenderness),
    }


def _within_limits(volume_mm3: float, mass_kg: float, slenderness_ratio: float) -> bool:
    return (
        volume_mm3 <= _MAX_REPORTED_VOLUME_MM3
        and mass_kg <= _MAX_REPORTED_MASS_KG
        and slenderness_ratio <= _MAX_REPORTED_SLENDERNESS
    )


def report_payload(
    parameters: SimulatorParameters,
    metrics: Mapping[str, JSONValue],
    *,
    call_id: str,
) -> dict[str, JSONValue]:
    return {
        "call_id": call_id,
        "disclaimer": "closed-form arithmetic reference; not an engineering validation",
        "inputs": parameters.to_payload(),
        "metrics": dict(metrics),
        "tool_id": SIMULATOR_TOOL_ID,
        "tool_version": SIMULATOR_TOOL_VERSION,
    }


def write_report(
    workspace: Path,
    parameters: SimulatorParameters,
    metrics: Mapping[str, JSONValue],
    *,
    call_id: str,
) -> tuple[ArtifactRef, str]:
    relative = f"{SIMULATOR_REPORT_DIRECTORY}/{SIMULATOR_REPORT_NAME}"
    try:
        target = resolve_workspace_path(workspace, relative)
    except ToolWorkspaceError as error:
        raise SimulatorInputError("the report path is not inside the workspace") from error
    payload = report_payload(parameters, metrics, call_id=call_id)
    encoded = (canonical_json_dumps(payload) + "\n").encode("utf-8")
    _write_workspace_file(target, encoded)
    artifact = ArtifactRef(
        artifact_id=f"{call_id}-report",
        uri=f"file://{target.resolve(strict=False).as_posix()}",
        digest=sha256_hex(encoded),
        media_type="application/json",
        size_bytes=len(encoded),
        classification=ArtifactClassification.TEXT,
    )
    return artifact, relative


def _write_workspace_file(target: Path, payload: bytes) -> None:
    """Publish a report atomically and without following a planted symlink.

    The parent directory is created and re-checked with a single ``lstat`` so a
    concurrent writer, or a symlink swapped in between the check and the write,
    cannot redirect the report. The payload is written to a fresh exclusive
    file and renamed into place, so a reader never observes a partial artifact.
    """
    parent = target.parent
    try:
        os.makedirs(parent, mode=0o700, exist_ok=True)
    except OSError as error:
        raise SimulatorInputError(
            f"the report directory could not be created: {error.strerror}"
        ) from error
    try:
        status = os.lstat(parent)
    except OSError as error:
        raise SimulatorInputError("the report directory is not accessible") from error
    if stat.S_ISLNK(status.st_mode) or not stat.S_ISDIR(status.st_mode):
        raise SimulatorInputError("the report directory must be a real directory")
    temporary = parent / f".{target.name}.{os.getpid()}.{threading.get_ident()}.partial"
    descriptor: int | None = None
    try:
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
        )
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            descriptor = None
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, target)
    except OSError as error:
        if descriptor is not None:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)
        raise SimulatorInputError(f"the report could not be written: {error.strerror}") from error


class SimulatorToolHandler:
    def __init__(
        self,
        *,
        tool_id: str = SIMULATOR_TOOL_ID,
        version: str = SIMULATOR_TOOL_VERSION,
    ) -> None:
        self._descriptor = simulator_descriptor(tool_id=tool_id, version=version)

    @property
    def descriptor(self) -> ToolDescriptor:
        return self._descriptor

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope:
        return _simulate(request)


def evaluator_descriptor(
    *,
    tool_id: str = EVALUATOR_TOOL_ID,
    version: str = EVALUATOR_TOOL_VERSION,
) -> ToolDescriptor:
    """Describe the same arithmetic as a pure, side-effect free evaluator.

    The evaluator is the one registration the registry accepts as a trusted
    in-process evaluator: it is shipped with the kernel, it declares no side
    effects, and it writes nothing. It is the only way a tool may be invoked
    inside the kernel process, and only under the explicit trusted mode.
    """
    return ToolDescriptor(
        tool_id=tool_id,
        version=version,
        name="Mechanical reference evaluator",
        description=(
            "Deterministic closed-form arithmetic with no filesystem, network, or "
            "process authority. It is a reference calculation, not a validation."
        ),
        input_schema_digest=canonical_json_digest(INPUT_SCHEMA),
        output_schema_digest=canonical_json_digest(OUTPUT_SCHEMA),
        side_effect_level=SideEffectLevel.NONE,
        required_capabilities=EVALUATOR_CAPABILITIES,
        timeout_seconds=10.0,
        destructive=False,
        supports_structured_output=True,
        idempotent=True,
    )


class ReferenceEvaluatorHandler:
    """A first-party handler that performs arithmetic and returns a result."""

    def __init__(
        self,
        *,
        tool_id: str = EVALUATOR_TOOL_ID,
        version: str = EVALUATOR_TOOL_VERSION,
    ) -> None:
        self._descriptor = evaluator_descriptor(tool_id=tool_id, version=version)

    @property
    def descriptor(self) -> ToolDescriptor:
        return self._descriptor

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope:
        return _evaluate(request)


def _evaluate(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    arguments = cast(Mapping[str, object], dict(request.arguments))
    if arguments.get("write_report") is True:
        return ToolResponseEnvelope.failed_envelope(
            call_id=request.call_id,
            tool_id=request.tool_id,
            tool_version=request.tool_version,
            status=ToolRunStatus.FAILED,
            error_code=ToolErrorCode.WORKSPACE_VIOLATION,
            error_message="the evaluator cannot write a report",
        )
    try:
        parameters = parse_parameters(arguments)
        metrics = compute_metrics(parameters)
    except SimulatorInputError as error:
        return ToolResponseEnvelope.failed_envelope(
            call_id=request.call_id,
            tool_id=request.tool_id,
            tool_version=request.tool_version,
            status=ToolRunStatus.FAILED,
            error_code=ToolErrorCode.INVALID_ARGUMENTS,
            error_message=str(error)[:1_024],
        )
    result: dict[str, JSONValue] = {"operation": parameters.operation, "report_path": None}
    result.update(metrics)
    return ToolResponseEnvelope.succeeded_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        result=result,
    )


def _simulate(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    arguments = cast(Mapping[str, object], dict(request.arguments))
    try:
        parameters = parse_parameters(arguments)
        metrics = compute_metrics(parameters)
    except SimulatorInputError as error:
        return ToolResponseEnvelope.failed_envelope(
            call_id=request.call_id,
            tool_id=request.tool_id,
            tool_version=request.tool_version,
            status=ToolRunStatus.FAILED,
            error_code=ToolErrorCode.INVALID_ARGUMENTS,
            error_message=str(error)[:1_024],
        )
    artifacts: tuple[ArtifactRef, ...] = ()
    report_path: str | None = None
    if parameters.write_report:
        try:
            artifact, relative = write_report(
                Path(request.workspace),
                parameters,
                metrics,
                call_id=request.call_id,
            )
        except (OSError, SimulatorInputError) as error:
            return ToolResponseEnvelope.failed_envelope(
                call_id=request.call_id,
                tool_id=request.tool_id,
                tool_version=request.tool_version,
                status=ToolRunStatus.FAILED,
                error_code=ToolErrorCode.WORKSPACE_VIOLATION,
                error_message=f"{type(error).__name__}: {error}"[:1_024],
            )
        artifacts = (artifact,)
        report_path = relative
    result: dict[str, JSONValue] = {
        "operation": parameters.operation,
        "report_path": report_path,
        **metrics,
    }
    return ToolResponseEnvelope.succeeded_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        result=result,
        artifacts=artifacts,
    )


def invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    return _simulate(request)


def hang_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    time.sleep(MISBEHAVIOR_SLEEP_SECONDS)
    return _simulate(request)


def crash_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(MISBEHAVIOR_EXIT_CODE)


def flood_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    channel = sys.__stdout__
    if channel is None:
        return _simulate(request)
    chunk = "x" * 65_536
    for _ in range(MISBEHAVIOR_FLOOD_CHUNKS):
        channel.write(chunk)
    channel.flush()
    return _simulate(request)


def liar_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    return ToolResponseEnvelope.succeeded_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        result=cast("Mapping[str, JSONValue]", {"within_limits": "not-a-boolean"}),
    )


def unrecognized_code_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    """Report a failure with an error code the kernel does not recognise."""
    envelope = ToolResponseEnvelope.failed_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        status=ToolRunStatus.FAILED,
        error_code=ToolErrorCode.INTERNAL_ERROR,
        error_message="the tool failed",
    )
    return replace(
        envelope,
        error_code="totally_fine_code",
        error_message="reported under a code the kernel does not recognise",
    )


def stray_child_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    """Leave a grandchild behind that writes a side effect long after the call.

    The grandchild stays in the worker's own process group, so a supervisor
    that only kills its direct child, or that leaves the group running, lets
    the side effect land after the call has already been reported as timed out.
    """
    _spawn_stray_child(Path(request.workspace))
    time.sleep(MISBEHAVIOR_SLEEP_SECONDS)
    return _simulate(request)


def lingering_child_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    """Echo the arguments at once while a grandchild in the worker's group keeps running."""
    _spawn_stray_child(Path(request.workspace))
    return ToolResponseEnvelope.succeeded_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        result=request.arguments,
    )


def _spawn_stray_child(workspace: Path) -> None:
    subprocess.Popen(
        [
            sys.executable,
            "-c",
            STRAY_CHILD_SOURCE,
            str(MISBEHAVIOR_STRAY_CHILD_DELAY_SECONDS),
            str(workspace / MISBEHAVIOR_STRAY_CHILD_NAME),
        ],
        cwd=workspace,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def escape_invoke(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    workspace = Path(request.workspace)
    target = workspace.parent / MISBEHAVIOR_ESCAPE_NAME
    payload = b'{"escaped": true}\n'
    target.write_bytes(payload)
    artifact = ArtifactRef(
        artifact_id=f"{request.call_id}-escape",
        uri=f"file://{target.as_posix()}",
        digest=sha256_hex(payload),
        media_type="application/json",
        size_bytes=len(payload),
        classification=ArtifactClassification.DERIVED_DATA,
    )
    result: dict[str, JSONValue] = {
        "operation": "beam_volume",
        "report_path": f"../{MISBEHAVIOR_ESCAPE_NAME}",
        "mass_kg": 0.0,
        "second_moment_mm4": 0.0,
        "slenderness_ratio": 0.0,
        "surface_area_mm2": 0.0,
        "volume_mm3": 0.0,
        "within_limits": True,
    }
    return ToolResponseEnvelope.succeeded_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        result=result,
        artifacts=(artifact,),
    )


def register_simulator(
    registry: ToolRegistry,
    *,
    tool_id: str = SIMULATOR_TOOL_ID,
    version: str = SIMULATOR_TOOL_VERSION,
) -> ToolDescriptor:
    if not isinstance(registry, ToolRegistry):
        raise TypeError("registry must be a ToolRegistry")
    registry.register(
        simulator_descriptor(tool_id=tool_id, version=version),
        simulator_handler(tool_id=tool_id, version=version),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    return simulator_descriptor(tool_id=tool_id, version=version)


def register_reference_evaluator(
    registry: ToolRegistry,
    *,
    tool_id: str = EVALUATOR_TOOL_ID,
    version: str = EVALUATOR_TOOL_VERSION,
) -> ToolDescriptor:
    """Register the pure evaluator that may run inside the kernel process."""
    if not isinstance(registry, ToolRegistry):
        raise TypeError("registry must be a ToolRegistry")
    registry.register_trusted_evaluator(
        evaluator_descriptor(tool_id=tool_id, version=version),
        ReferenceEvaluatorHandler(tool_id=tool_id, version=version),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    return evaluator_descriptor(tool_id=tool_id, version=version)


__all__ = (
    "EVALUATOR_CAPABILITIES",
    "EVALUATOR_TOOL_ID",
    "EVALUATOR_TOOL_VERSION",
    "INPUT_SCHEMA",
    "MISBEHAVIOR_ENTRY_POINTS",
    "OUTPUT_SCHEMA",
    "SIMULATOR_CAPABILITIES",
    "SIMULATOR_ENTRY_POINT",
    "SIMULATOR_TOOL_ID",
    "SIMULATOR_TOOL_VERSION",
    "ReferenceEvaluatorHandler",
    "SimulatorInputError",
    "SimulatorParameters",
    "SimulatorToolHandler",
    "compute_metrics",
    "crash_invoke",
    "escape_invoke",
    "evaluator_descriptor",
    "flood_invoke",
    "hang_invoke",
    "invoke",
    "liar_invoke",
    "lingering_child_invoke",
    "parse_parameters",
    "register_reference_evaluator",
    "register_simulator",
    "report_payload",
    "simulator_descriptor",
    "simulator_handler",
    "stray_child_invoke",
    "unrecognized_code_invoke",
    "write_report",
)
