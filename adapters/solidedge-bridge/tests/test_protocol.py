"""The versioned Solid Edge bridge contract, checked without Windows or COM."""

from __future__ import annotations

import json
import math
from typing import Any, Final, cast

import pytest
from pydantic import ValidationError

from kala_solidedge_bridge.errors import (
    BridgeError,
    BusyError,
    CalibrationRequiredError,
    ConflictError,
    InputError,
    UnavailableError,
)
from kala_solidedge_bridge.protocol import (
    MAX_ARTIFACTS,
    MAX_OPERATIONS,
    MAX_PARTS,
    PROTOCOL_VERSION,
    Artifact,
    ArtifactFormat,
    AssemblyNew,
    AssemblyPlace,
    BuildAccepted,
    BuildRequest,
    BuildResult,
    BuildStatus,
    ErrorCode,
    ErrorResponse,
    Health,
    JobState,
    PartExportStep,
    PartMeasure,
    PartNew,
    PartResult,
    PartSave,
    Plane,
    RangeBox,
    SketchCircle,
    SketchRectangle,
    SolidCut,
    SolidExtrude,
    SolidRevolve,
    StepTrace,
)

PLATE_OPS: Final[list[dict[str, object]]] = [
    {"op": "part.new", "name": "plate"},
    {"op": "sketch.rectangle", "plane": "XY", "width": 100, "height": 60},
    {"op": "solid.extrude", "length": 10},
    {"op": "sketch.circle", "plane": "XY", "diameter": 20, "x": 30, "y": 20},
    {"op": "solid.cut"},
    {"op": "part.measure"},
    {"op": "part.save"},
    {"op": "part.export_step"},
]
ASSEMBLY_OPS: Final[list[dict[str, object]]] = [
    {"op": "part.new", "name": "base"},
    {"op": "sketch.rectangle", "plane": "XY", "width": 100, "height": 60},
    {"op": "solid.extrude", "length": 10},
    {"op": "part.new", "name": "pin"},
    {"op": "sketch.circle", "plane": "XZ", "diameter": 10},
    {"op": "solid.extrude", "length": 20},
    {"op": "assembly.new", "name": "stack"},
    {"op": "assembly.place", "component": "base"},
    {"op": "assembly.place", "component": "pin", "z": 10},
    {"op": "assembly.save"},
    {"op": "assembly.export_step"},
]

INVALID_PROCEDURES: Final[tuple[tuple[dict[str, object], ...], ...]] = (
    ({"op": "part.new", "name": "plate"},),
    ({"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "part.new", "name": "other"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "sketch.circle", "plane": "XY", "diameter": 5},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "solid.extrude", "length": 10},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.cut"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "part.measure"},
        {"op": "sketch.circle", "plane": "XY", "diameter": 5},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "part.new", "name": "plate"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "assembly.place", "component": "plate"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "assembly.new", "name": "stack"},
        {"op": "part.measure"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "assembly.new", "name": "stack"},
        {"op": "assembly.new", "name": "again"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "assembly.new", "name": "stack"},
        {"op": "assembly.save"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "assembly.new", "name": "stack"},
        {"op": "assembly.place", "component": "ghost"},
    ),
    (
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 10},
        {"op": "assembly.new", "name": "stack"},
        {"op": "assembly.place", "component": "plate"},
        {"op": "assembly.place", "component": "plate"},
    ),
)


def _request(
    ops: list[dict[str, object]],
    **overrides: object,
) -> BuildRequest:
    payload: dict[str, object] = {
        "protocol_version": PROTOCOL_VERSION,
        "request_id": "req-1",
        "summary": "plate with a hole",
        "procedure": ops,
    }
    payload.update(overrides)
    return BuildRequest.model_validate_json(json.dumps(payload))


def _valid_part_ops(name: str) -> list[dict[str, object]]:
    return [
        {"op": "part.new", "name": name},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 2},
    ]


def test_plate_with_hole_request_is_valid() -> None:
    request = _request(PLATE_OPS)

    assert request.protocol_version == PROTOCOL_VERSION
    assert len(request.procedure) == 8
    sketch = request.procedure[1]
    assert isinstance(sketch, SketchRectangle)
    assert sketch.plane is Plane.XY
    assert sketch.width == 100.0
    cut = request.procedure[4]
    assert isinstance(cut, SolidCut)
    assert cut.length is None
    assert len(request.request_digest()) == 64


def test_assembly_request_is_valid() -> None:
    request = _request(ASSEMBLY_OPS)

    assert len(request.procedure) == 11
    assert isinstance(request.procedure[6], AssemblyNew)
    assert isinstance(request.procedure[7], AssemblyPlace)
    place_pin = request.procedure[8]
    assert isinstance(place_pin, AssemblyPlace)
    assert place_pin.component == "pin"


def test_additive_features_extend_a_part_that_already_has_a_solid() -> None:
    base: list[dict[str, object]] = [
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 2},
    ]
    # A second profile plus extrude/revolve extends the existing solid instead
    # of being rejected as a duplicate first solid.
    additive_only: list[dict[str, object]] = [
        *base,
        {"op": "sketch.circle", "plane": "XY", "diameter": 4},
        {"op": "solid.extrude", "length": 3},
        {"op": "sketch.rectangle", "plane": "XZ", "width": 2, "height": 2},
        {"op": "solid.revolve", "angle": 90},
    ]
    assert len(_request(additive_only).procedure) == len(additive_only)

    with_cut: list[dict[str, object]] = [
        *additive_only,
        {"op": "sketch.circle", "plane": "XY", "diameter": 2},
        {"op": "solid.cut"},
        {"op": "part.measure"},
    ]
    request = _request(with_cut)
    assert isinstance(request.procedure[-2], SolidCut)
    assert isinstance(request.procedure[-1], PartMeasure)


@pytest.mark.parametrize("op_name", ["part.measure", "part.save", "part.export_step"])
def test_output_ops_are_rejected_while_a_profile_is_pending(op_name: str) -> None:
    # The procedure ends by consuming the profile with a cut, so rejection can
    # only come from the interleaving itself, not from a trailing pending check.
    ops: list[dict[str, object]] = [
        {"op": "part.new", "name": "plate"},
        {"op": "sketch.rectangle", "plane": "XY", "width": 10, "height": 10},
        {"op": "solid.extrude", "length": 2},
        {"op": "sketch.circle", "plane": "XY", "diameter": 4},
        {"op": op_name},
        {"op": "solid.cut"},
    ]
    with pytest.raises(ValidationError):
        _request(ops)


def test_programmatic_models_are_frozen_and_use_fixed_ops() -> None:
    request = BuildRequest(
        request_id="req-2",
        procedure=(
            PartNew(name="plate"),
            SketchRectangle(plane=Plane.XZ, width=10, height=10),
            SolidExtrude(length=2.5, reversed=True),
            PartMeasure(),
            PartSave(),
            PartExportStep(),
        ),
    )
    assert request.procedure[0].op == "part.new"
    with pytest.raises(ValidationError):
        cast(Any, request).request_id = "changed"


def test_json_integer_real_numbers_are_accepted() -> None:
    request = _request(PLATE_OPS)
    sketch = request.procedure[1]
    assert isinstance(sketch, SketchRectangle)
    assert isinstance(sketch.width, float)


@pytest.mark.parametrize("procedure", INVALID_PROCEDURES)
def test_malformed_procedures_are_rejected(procedure: tuple[dict[str, object], ...]) -> None:
    with pytest.raises(ValidationError):
        _request(list(procedure))


def test_unknown_operation_is_rejected() -> None:
    with pytest.raises(ValidationError):
        _request([{"op": "part.delete", "name": "plate"}])


def test_unknown_and_path_fields_are_rejected() -> None:
    ops: list[dict[str, object]] = [
        {"op": "part.new", "name": "plate"},
        {
            "op": "sketch.rectangle",
            "plane": "XY",
            "width": 10,
            "height": 10,
            "artifact_path": "/tmp/plate.par",
        },
        {"op": "solid.extrude", "length": 2},
    ]
    with pytest.raises(ValidationError):
        _request(ops)
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, artifact_path="/tmp/plate.par")


def test_part_count_is_bounded() -> None:
    ops: list[dict[str, object]] = []
    for index in range(MAX_PARTS):
        ops.extend(_valid_part_ops(f"p{index}"))
    _request(ops)

    ops.extend(_valid_part_ops("overflow"))
    with pytest.raises(ValidationError):
        _request(ops)


def test_maximum_eight_part_artifacts_fit_the_cap() -> None:
    assert MAX_ARTIFACTS == 2 * MAX_PARTS + 2 == 18
    parts = tuple(
        PartResult(
            name=f"p{index}",
            valid=True,
            volume_mm3=1,
            surface_area_mm2=1,
            range_box=RangeBox(min_mm=(0, 0, 0), max_mm=(1, 1, 1)),
        )
        for index in range(MAX_PARTS)
    )
    part_artifacts = tuple(
        Artifact(
            name=f"p{index}.{fmt.value}",
            format=fmt,
            media_type="application/octet-stream",
            size_bytes=index,
            sha256="a" * 64,
        )
        for index in range(MAX_PARTS)
        for fmt in (ArtifactFormat.PAR, ArtifactFormat.STEP)
    )
    assembly_artifacts = (
        Artifact(
            name="stack.asm",
            format=ArtifactFormat.ASM,
            media_type="application/octet-stream",
            size_bytes=0,
            sha256="b" * 64,
        ),
        Artifact(
            name="stack.step",
            format=ArtifactFormat.STEP,
            media_type="application/octet-stream",
            size_bytes=0,
            sha256="c" * 64,
        ),
    )
    artifacts = part_artifacts + assembly_artifacts
    assert len(artifacts) == MAX_ARTIFACTS

    result = BuildResult(
        build_id="build-1",
        state=JobState.SUCCEEDED,
        parts=parts,
        artifacts=artifacts,
    )
    assert len(result.parts) == MAX_PARTS
    assert len(result.artifacts) == MAX_ARTIFACTS

    with pytest.raises(ValidationError):
        BuildResult(
            build_id="build-1",
            state=JobState.SUCCEEDED,
            parts=parts,
            artifacts=(*artifacts, artifacts[0]),
        )


def test_operation_cap_is_independent_of_the_core_ir() -> None:
    ops = list(PLATE_OPS)
    while len(ops) < MAX_OPERATIONS:
        ops.extend(
            (
                {"op": "sketch.circle", "plane": "XY", "diameter": 5},
                {"op": "solid.cut"},
            )
        )
    request = _request(ops)
    assert len(request.procedure) == MAX_OPERATIONS

    with pytest.raises(ValidationError):
        _request([*ops, {"op": "solid.cut"}])


@pytest.mark.parametrize("value", [True, "10", float("nan"), float("inf")])
def test_real_numbers_reject_bools_strings_and_non_finite(value: object) -> None:
    with pytest.raises(ValidationError):
        SketchRectangle(plane=Plane.XY, width=cast(Any, value), height=10)
    with pytest.raises(ValidationError):
        SolidExtrude(length=cast(Any, value))


@pytest.mark.parametrize("value", [0.001, 5_000.0])
def test_size_bounds_are_inclusive(value: float) -> None:
    sketch = SketchRectangle(plane=Plane.XY, width=value, height=value)
    assert sketch.width == value


@pytest.mark.parametrize("value", [0.0009, 5_000.001])
def test_size_bounds_reject_outside_values(value: float) -> None:
    with pytest.raises(ValidationError):
        SketchRectangle(plane=Plane.XY, width=value, height=10)


@pytest.mark.parametrize("value", [-5_000.0, 5_000.0])
def test_offset_bounds_are_inclusive(value: float) -> None:
    assert AssemblyPlace(component="base", x=value).x == value


@pytest.mark.parametrize("value", [-5_000.001, 5_000.001])
def test_offset_bounds_reject_outside_values(value: float) -> None:
    with pytest.raises(ValidationError):
        SketchCircle(plane=Plane.YZ, diameter=10, x=value)


@pytest.mark.parametrize("value", [360.0, 0.5])
def test_angle_bounds_are_open_at_zero(value: float) -> None:
    assert SolidRevolve(angle=value).angle == value


@pytest.mark.parametrize("value", [0.0, 360.001, -1.0])
def test_angle_bounds_reject_outside_values(value: float) -> None:
    with pytest.raises(ValidationError):
        SolidRevolve(angle=value)


def test_revolve_axis_and_cut_direction_are_fixed() -> None:
    assert SolidRevolve().axis == "Y"
    with pytest.raises(ValidationError):
        SolidRevolve(axis=cast(Any, "Z"))
    with pytest.raises(ValidationError):
        cast(Any, SolidCut)(reversed=False)


@pytest.mark.parametrize("value", [1.0, 900.0])
def test_timeout_bounds_are_inclusive(value: float) -> None:
    assert _request(PLATE_OPS, timeout_seconds=value).timeout_seconds == value


@pytest.mark.parametrize("value", [0.5, 900.5, True, "120"])
def test_timeout_rejects_outside_or_non_numeric_values(value: object) -> None:
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, timeout_seconds=value)


@pytest.mark.parametrize("value", [0.001, 22_000.0, 7_850])
def test_density_bounds_are_positive_and_capped(value: float) -> None:
    assert _request(PLATE_OPS, density_kg_m3=value).density_kg_m3 == float(value)


@pytest.mark.parametrize("value", [0.0, -1.0, 22_000.5, True, "7850"])
def test_density_rejects_outside_or_non_numeric_values(value: object) -> None:
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, density_kg_m3=value)


@pytest.mark.parametrize("version", [2, 0, "1", True, 1.0])
def test_protocol_version_is_exactly_one(version: object) -> None:
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, protocol_version=version)


def test_protocol_version_defaults_to_one() -> None:
    payload = {
        "request_id": "req-1",
        "summary": "plate",
        "procedure": PLATE_OPS,
    }
    assert BuildRequest.model_validate_json(json.dumps(payload)).protocol_version == 1


def test_request_id_and_calibration_id_are_bounded_and_safe() -> None:
    _request(PLATE_OPS, calibration_id="cal-2026_10.19:1")
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, request_id="bad id")
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, request_id="x" * 65)
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, calibration_id="x" * 129)


def test_summary_is_bounded() -> None:
    _request(PLATE_OPS, summary="x" * 500)
    with pytest.raises(ValidationError):
        _request(PLATE_OPS, summary="x" * 501)


def test_canonical_digest_is_deterministic_and_ignores_request_id() -> None:
    first = _request(PLATE_OPS, request_id="req-a")
    second = _request(PLATE_OPS, request_id="req-b")

    assert first.canonical_request_bytes() == second.canonical_request_bytes()
    assert first.request_digest() == second.request_digest()
    assert first.canonical_request_bytes() == first.canonical_request_bytes()
    assert b"req-a" not in first.canonical_request_bytes()
    assert b"request_id" not in first.canonical_request_bytes()
    assert first.canonical_request_bytes() == json.dumps(
        first.model_dump(mode="json", exclude={"request_id"}),
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def test_canonical_digest_covers_the_semantic_payload() -> None:
    base = _request(PLATE_OPS)
    assert base.request_digest() != _request(PLATE_OPS, summary="other").request_digest()
    assert base.request_digest() != _request(PLATE_OPS, timeout_seconds=30).request_digest()
    assert base.request_digest() != _request(PLATE_OPS, calibration_id="cal-1").request_digest()
    assert base.request_digest() != _request(PLATE_OPS, density_kg_m3=7_850).request_digest()
    changed = [dict(op) for op in PLATE_OPS]
    changed[1]["width"] = 101
    assert base.request_digest() != _request(changed).request_digest()


def test_json_round_trip_preserves_the_digest() -> None:
    request = _request(ASSEMBLY_OPS)
    encoded = request.model_dump_json()
    restored = BuildRequest.model_validate_json(encoded)
    assert restored == request
    assert restored.request_digest() == request.request_digest()


def test_artifact_names_cannot_be_paths() -> None:
    Artifact(
        name="plate.par",
        format=ArtifactFormat.PAR,
        media_type="application/octet-stream",
        size_bytes=0,
        sha256="a" * 64,
    )
    for name in ("../plate.par", "C:\\plate.par", "/plate.par", "part/plate.par", "."):
        with pytest.raises(ValidationError):
            Artifact(
                name=name,
                format=ArtifactFormat.PAR,
                media_type="application/octet-stream",
                size_bytes=0,
                sha256="a" * 64,
            )
    with pytest.raises(ValidationError):
        Artifact(
            name="plate.par",
            format=cast(Any, "iges"),
            media_type="application/octet-stream",
            size_bytes=0,
            sha256="a" * 64,
        )


def test_range_box_requires_ordered_finite_triples() -> None:
    box = RangeBox(min_mm=(-10, -20, 0), max_mm=(10, 20, 5))
    assert box.max_mm == (10.0, 20.0, 5.0)
    with pytest.raises(ValidationError):
        RangeBox(min_mm=(1, 0, 0), max_mm=(0, 1, 1))
    with pytest.raises(ValidationError):
        RangeBox(min_mm=(0, 0, 0), max_mm=(math.nan, 1, 1))


def test_trace_and_status_are_bounded() -> None:
    trace = StepTrace(seq=0, op="part.new", status="ok", message="created", duration_ms=1.5)
    assert trace.seq == 0
    with pytest.raises(ValidationError):
        StepTrace(seq=-1, op="part.new", status="ok", duration_ms=1)
    with pytest.raises(ValidationError):
        StepTrace(seq=0, op="part.new", status="ok", duration_ms=-0.1)
    with pytest.raises(ValidationError):
        StepTrace(seq=0, op="part.new", status="ok", message="x" * 501, duration_ms=1)

    status = BuildStatus(
        build_id="build-1",
        state=JobState.RUNNING,
        step_index=2,
        total=8,
        trace=(trace,),
        message="working",
    )
    assert status.step_index == 2
    with pytest.raises(ValidationError):
        BuildStatus(build_id="build-1", state=JobState.RUNNING, step_index=0, total=0)
    with pytest.raises(ValidationError):
        BuildStatus(build_id="build-1", state=JobState.RUNNING, step_index=601, total=600)


def test_result_models_report_local_metrics_and_metadata_only() -> None:
    part = PartResult(
        name="plate",
        valid=True,
        volume_mm3=1_000,
        surface_area_mm2=500,
        range_box=RangeBox(min_mm=(0, 0, 0), max_mm=(100, 60, 10)),
    )
    artifact = Artifact(
        name="plate.par",
        format=ArtifactFormat.PAR,
        media_type="application/octet-stream",
        size_bytes=12,
        sha256="b" * 64,
    )
    result = BuildResult(
        build_id="build-1",
        state=JobState.SUCCEEDED,
        parts=(part,),
        artifacts=(artifact,),
        trace=(StepTrace(seq=0, op="part.new", status="ok", duration_ms=1),),
        warnings=("mass not requested",),
    )
    assert result.parts[0].mass_kg is None
    assert result.artifacts[0].format is ArtifactFormat.PAR
    with pytest.raises(ValidationError):
        PartResult(
            name="plate",
            valid=True,
            volume_mm3=-1,
            surface_area_mm2=500,
            range_box=RangeBox(min_mm=(0, 0, 0), max_mm=(1, 1, 1)),
        )


def test_health_is_honest_not_a_magic_ready_flag() -> None:
    health = Health(
        bridge_version="0.1.0",
        platform="win32",
        supported=True,
        available=False,
        busy=True,
        se_version="Solid Edge 2024",
        calibration_id="cal-1",
        capabilities=("part.build", "assembly.position"),
        detail="Solid Edge is not attached",
    )
    assert health.available is False
    assert health.capabilities == ("part.build", "assembly.position")
    with pytest.raises(ValidationError):
        Health(
            bridge_version="0.1.0",
            platform="win32",
            supported="yes",
            available=False,
            busy=False,
        )
    with pytest.raises(ValidationError):
        Health(
            bridge_version="0.1.0",
            platform="win32",
            supported=True,
            available=False,
            busy=False,
            capabilities=("READY",),
        )


def test_error_and_acceptance_models_reject_unknown_codes() -> None:
    error = ErrorResponse(code=ErrorCode.CALIBRATION_REQUIRED, message="calibrate first")
    assert error.code is ErrorCode.CALIBRATION_REQUIRED
    with pytest.raises(ValidationError):
        ErrorResponse(code=cast(Any, "magic_ready"), message="no")
    with pytest.raises(ValidationError):
        ErrorResponse(code=ErrorCode.INVALID_REQUEST, message="")

    accepted = BuildAccepted(build_id="build-1")
    assert accepted.state is JobState.QUEUED


def test_bridge_errors_map_to_stable_wire_codes() -> None:
    cases = (
        (BridgeError(ErrorCode.CONFLICT, "conflict"), ErrorCode.CONFLICT),
        (InputError("bad input"), ErrorCode.INVALID_REQUEST),
        (UnavailableError("no Solid Edge"), ErrorCode.UNAVAILABLE),
        (CalibrationRequiredError("calibrate"), ErrorCode.CALIBRATION_REQUIRED),
        (BusyError("busy"), ErrorCode.BUSY),
        (ConflictError("conflict"), ErrorCode.CONFLICT),
    )
    for error, expected in cases:
        response = error.to_response()
        assert response.code is expected
        assert len(response.message) <= 500
    assert BridgeError(ErrorCode.INTERNAL_ERROR, "x" * 900).to_response().message == "x" * 500
    assert BridgeError(ErrorCode.INTERNAL_ERROR, "   ").to_response().message == "bridge failure"


def test_enum_values_are_stable_strings() -> None:
    assert tuple(state.value for state in JobState) == (
        "queued",
        "running",
        "succeeded",
        "failed",
        "cancelled",
        "indeterminate",
        "interrupted",
    )
    assert tuple(plane.value for plane in Plane) == ("XY", "XZ", "YZ")
    assert tuple(fmt.value for fmt in ArtifactFormat) == ("par", "asm", "step")
    assert set(item.value for item in ErrorCode) >= {
        "invalid_request",
        "unavailable",
        "calibration_required",
        "busy",
        "conflict",
    }
