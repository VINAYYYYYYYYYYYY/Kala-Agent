from __future__ import annotations

import errno
import json
import os
import stat
import subprocess
import sys
import time
from collections.abc import Mapping
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.contracts import ToolRunStatus
from kala_agent.tools import simulator as simulator_module
from kala_agent.tools.protocol import (
    PROTOCOL_VERSION,
    JSONValue,
    ToolEnvelopeKind,
    ToolErrorCode,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    ToolWorkspaceError,
    resolve_workspace_path,
)
from kala_agent.tools.simulator import (
    _MAX_REPORTED_MASS_KG,
    _MAX_REPORTED_SLENDERNESS,
    _MAX_REPORTED_VOLUME_MM3,
    EVALUATOR_TOOL_ID,
    SIMULATOR_TOOL_ID,
    ReferenceEvaluatorHandler,
    SimulatorInputError,
    SimulatorParameters,
    SimulatorToolHandler,
    _write_workspace_file,
    compute_metrics,
    escape_invoke,
    evaluator_descriptor,
    flood_invoke,
    liar_invoke,
    parse_parameters,
    register_reference_evaluator,
    register_simulator,
    report_payload,
    simulator_descriptor,
    simulator_handler,
    stray_child_invoke,
    unrecognized_code_invoke,
    write_report,
)


def _request(
    workspace: Path,
    arguments: dict[str, object] | None = None,
    *,
    call_id: str = "call-1",
) -> ToolRequestEnvelope:
    return ToolRequestEnvelope(
        protocol_version=PROTOCOL_VERSION,
        kind=ToolEnvelopeKind.REQUEST,
        call_id=call_id,
        tool_id=SIMULATOR_TOOL_ID,
        tool_version="1.0.0",
        arguments=cast("dict[str, JSONValue]", arguments or {}),
        workspace=workspace.resolve(strict=False).as_posix(),
        deadline_seconds=30.0,
        max_output_bytes=1_000_000,
        max_artifact_bytes=1_000_000,
        granted_capabilities=(),
    )


def _number(metrics: Mapping[str, JSONValue], key: str) -> float:
    return cast(float, metrics[key])


def _result(response: ToolResponseEnvelope) -> Mapping[str, JSONValue]:
    return cast("Mapping[str, JSONValue]", response.result)


def _parameters(**overrides: object) -> SimulatorParameters:
    values: dict[str, object] = {
        "operation": "beam_volume",
        "length_mm": 120.0,
        "width_mm": 20.0,
        "height_mm": 10.0,
        "density_kg_m3": 2_700.0,
        "write_report": False,
    }
    values.update(overrides)
    return SimulatorParameters(
        operation=cast(str, values["operation"]),
        length_mm=cast(float, values["length_mm"]),
        width_mm=cast(float, values["width_mm"]),
        height_mm=cast(float, values["height_mm"]),
        density_kg_m3=cast(float, values["density_kg_m3"]),
        write_report=cast(bool, values["write_report"]),
    )


def test_simulator_parameters_payload_roundtrips() -> None:
    payload = _parameters().to_payload()
    assert payload == {
        "density_kg_m3": 2700.0,
        "height_mm": 10.0,
        "length_mm": 120.0,
        "operation": "beam_volume",
        "width_mm": 20.0,
        "write_report": False,
    }


def test_parse_parameters_rejects_non_mapping() -> None:
    with pytest.raises(SimulatorInputError, match="JSON object"):
        parse_parameters(cast("dict[str, object]", ["operation"]))


def test_parse_parameters_rejects_unknown_operation() -> None:
    with pytest.raises(SimulatorInputError, match="operation must be one of"):
        parse_parameters({"operation": "shear_stress"})
    with pytest.raises(SimulatorInputError, match="operation must be one of"):
        parse_parameters({"operation": 7})


def test_parse_parameters_defaults_every_dimension() -> None:
    parameters = parse_parameters({})
    assert parameters.operation == "beam_volume"
    assert parameters.length_mm == 120.0
    assert parameters.width_mm == 20.0
    assert parameters.height_mm == 10.0
    assert parameters.density_kg_m3 == 2_700.0
    assert parameters.write_report is False


def test_parse_parameters_accepts_integer_dimensions() -> None:
    parameters = parse_parameters({"operation": "bar_summary", "length_mm": 200})
    assert parameters.operation == "bar_summary"
    assert parameters.length_mm == 200.0


def test_parse_parameters_rejects_non_numeric_dimensions() -> None:
    with pytest.raises(SimulatorInputError, match="length_mm must be a number"):
        parse_parameters({"length_mm": "200"})
    with pytest.raises(SimulatorInputError, match="width_mm must be a number"):
        parse_parameters({"width_mm": True})


def test_parse_parameters_rejects_non_finite_dimensions() -> None:
    with pytest.raises(SimulatorInputError, match="height_mm must be finite"):
        parse_parameters({"height_mm": float("inf")})
    with pytest.raises(SimulatorInputError, match="density_kg_m3 must be finite"):
        parse_parameters({"density_kg_m3": float("nan")})


def test_parse_parameters_enforces_dimension_bounds() -> None:
    with pytest.raises(SimulatorInputError, match="length_mm must be between"):
        parse_parameters({"length_mm": 5_000.1})
    with pytest.raises(SimulatorInputError, match="width_mm must be between"):
        parse_parameters({"width_mm": 0.5})
    with pytest.raises(SimulatorInputError, match="height_mm must be between"):
        parse_parameters({"height_mm": 500.5})
    with pytest.raises(SimulatorInputError, match="density_kg_m3 must be between"):
        parse_parameters({"density_kg_m3": 20_000.5})


def test_parse_parameters_rejects_non_boolean_write_report() -> None:
    with pytest.raises(SimulatorInputError, match="write_report must be a boolean"):
        parse_parameters({"write_report": "yes"})


def test_compute_metrics_matches_closed_form_reference() -> None:
    metrics = compute_metrics(_parameters())
    assert _number(metrics, "volume_mm3") == 24_000.0
    assert _number(metrics, "mass_kg") == 0.0648
    assert _number(metrics, "surface_area_mm2") == 7_600.0
    assert _number(metrics, "slenderness_ratio") == 12.0
    assert _number(metrics, "second_moment_mm4") == 200_000.0
    assert metrics["within_limits"] is True


def test_compute_metrics_floors_slenderness_divisor() -> None:
    metrics = compute_metrics(_parameters(height_mm=0.0))
    assert _number(metrics, "slenderness_ratio") == 120.0


def test_compute_metrics_accepts_the_largest_schema_bounded_body() -> None:
    metrics = compute_metrics(
        parse_parameters(
            {
                "length_mm": 5_000.0,
                "width_mm": 500.0,
                "height_mm": 500.0,
                "density_kg_m3": 20_000.0,
            }
        )
    )
    assert _number(metrics, "volume_mm3") == _MAX_REPORTED_VOLUME_MM3
    assert _number(metrics, "mass_kg") <= _MAX_REPORTED_MASS_KG
    assert _number(metrics, "slenderness_ratio") <= _MAX_REPORTED_SLENDERNESS
    assert metrics["within_limits"] is True


def test_compute_metrics_flags_volume_overflow() -> None:
    metrics = compute_metrics(_parameters(length_mm=5_000.0, width_mm=500.0, height_mm=600.0))
    assert _number(metrics, "volume_mm3") > _MAX_REPORTED_VOLUME_MM3
    assert metrics["within_limits"] is False


def test_compute_metrics_flags_mass_overflow() -> None:
    metrics = compute_metrics(
        _parameters(
            length_mm=5_000.0,
            width_mm=500.0,
            height_mm=500.0,
            density_kg_m3=30_000.0,
        )
    )
    assert _number(metrics, "volume_mm3") <= _MAX_REPORTED_VOLUME_MM3
    assert _number(metrics, "mass_kg") > _MAX_REPORTED_MASS_KG
    assert metrics["within_limits"] is False


def test_compute_metrics_flags_slenderness_overflow() -> None:
    metrics = compute_metrics(_parameters(length_mm=10_000.0, height_mm=0.5))
    assert _number(metrics, "volume_mm3") <= _MAX_REPORTED_VOLUME_MM3
    assert _number(metrics, "mass_kg") <= _MAX_REPORTED_MASS_KG
    assert _number(metrics, "slenderness_ratio") > _MAX_REPORTED_SLENDERNESS
    assert metrics["within_limits"] is False


def test_report_payload_is_canonical_and_self_describing() -> None:
    parameters = _parameters()
    payload = report_payload(parameters, compute_metrics(parameters), call_id="call-7")
    assert payload["call_id"] == "call-7"
    assert payload["tool_id"] == SIMULATOR_TOOL_ID
    assert payload["tool_version"] == "1.0.0"
    assert "not an engineering validation" in cast(str, payload["disclaimer"])
    assert cast("dict[str, JSONValue]", payload["inputs"])["operation"] == "beam_volume"
    assert "volume_mm3" in cast("dict[str, JSONValue]", payload["metrics"])


def test_write_report_publishes_artifact(tmp_path: Path) -> None:
    parameters = _parameters()
    artifact, relative = write_report(
        tmp_path, parameters, compute_metrics(parameters), call_id="call-1"
    )
    assert relative == "outputs/reference-report.json"
    target = tmp_path / relative
    assert target.is_file()
    assert artifact.artifact_id == "call-1-report"
    assert artifact.uri == f"file://{target.resolve().as_posix()}"
    assert artifact.size_bytes == target.stat().st_size
    assert artifact.media_type == "application/json"
    document = json.loads(target.read_text(encoding="utf-8"))
    assert document["call_id"] == "call-1"
    assert not list(target.parent.glob(".*.partial"))


def test_write_report_overwrites_previous_report(tmp_path: Path) -> None:
    parameters = _parameters()
    first, relative = write_report(tmp_path, parameters, compute_metrics(parameters), call_id="a")
    second, _ = write_report(tmp_path, parameters, compute_metrics(parameters), call_id="b")
    assert first.digest != second.digest
    assert json.loads((tmp_path / relative).read_text(encoding="utf-8"))["call_id"] == "b"


def test_write_report_reports_workspace_violation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(_workspace: Path, _relative: str) -> Path:
        raise ToolWorkspaceError("outside the workspace")

    monkeypatch.setattr(simulator_module, "resolve_workspace_path", explode)
    parameters = _parameters()
    with pytest.raises(SimulatorInputError, match="not inside the workspace"):
        write_report(tmp_path, parameters, compute_metrics(parameters), call_id="call-1")


def test_write_workspace_file_reports_uncreatable_directory(tmp_path: Path) -> None:
    blocker = tmp_path / "blocked"
    blocker.write_text("x")
    with pytest.raises(SimulatorInputError, match="report directory could not be created"):
        _write_workspace_file(blocker / "nested" / "report.json", b"{}")


def test_write_workspace_file_rejects_symlinked_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    real = tmp_path / "real"
    real.mkdir()
    linked = tmp_path / "linked"
    linked.symlink_to(real, target_is_directory=True)
    with pytest.raises(SimulatorInputError, match="must be a real directory"):
        _write_workspace_file(linked / "report.json", b"{}")


def test_write_workspace_file_rejects_non_directory_parent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    plain = tmp_path / "plain"
    plain.write_text("x")
    monkeypatch.setattr(os, "lstat", lambda _path: plain.stat())
    with pytest.raises(SimulatorInputError, match="must be a real directory"):
        _write_workspace_file(tmp_path / "outputs" / "report.json", b"{}")


def test_write_workspace_file_reports_unreadable_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(self: Path) -> os.stat_result:
        raise OSError(errno.EACCES, "denied")

    monkeypatch.setattr(os, "lstat", explode)
    with pytest.raises(SimulatorInputError, match="not accessible"):
        _write_workspace_file(tmp_path / "outputs" / "report.json", b"{}")


def test_write_workspace_file_reports_failed_publication(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(*_args: object, **_kwargs: object) -> None:
        raise OSError(errno.ENOSPC, "full")

    monkeypatch.setattr(os, "replace", explode)
    with pytest.raises(SimulatorInputError, match="could not be written"):
        _write_workspace_file(tmp_path / "outputs" / "report.json", b"{}")
    leftovers = list((tmp_path / "outputs").glob(".*.partial"))
    assert leftovers == []


def test_write_workspace_file_reports_replace_failure_with_open_descriptor(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    real_open = os.open
    descriptors: list[int] = []

    def tracking_open(path: object, flags: int, mode: int = 0o600) -> int:
        descriptor = real_open(cast("str | Path", path), flags, mode)
        descriptors.append(descriptor)
        return descriptor

    def explode(*_args: object, **_kwargs: object) -> None:
        raise OSError(errno.EIO, "io error")

    monkeypatch.setattr(os, "open", tracking_open)
    monkeypatch.setattr(os, "replace", explode)
    monkeypatch.setattr(os, "fdopen", explode)
    with pytest.raises(SimulatorInputError, match="could not be written"):
        _write_workspace_file(tmp_path / "outputs" / "report.json", b"{}")
    assert len(descriptors) == 1


def test_write_workspace_file_overwrites_existing_target(tmp_path: Path) -> None:
    target = tmp_path / "outputs" / "report.json"
    _write_workspace_file(target, b"first")
    _write_workspace_file(target, b"second")
    assert target.read_bytes() == b"second"


def test_simulator_descriptor_declares_side_effects() -> None:
    descriptor = simulator_descriptor(tool_id="custom.tool", version="9.9.9")
    assert descriptor.tool_id == "custom.tool"
    assert descriptor.version == "9.9.9"
    assert descriptor.side_effect_level.value == "local_write"
    assert descriptor.timeout_seconds == 20.0
    assert descriptor.destructive is False
    assert descriptor.supports_structured_output is True
    assert descriptor.idempotent is True
    assert "artifact.write" in descriptor.required_capabilities


def test_evaluator_descriptor_declares_no_side_effects() -> None:
    descriptor = evaluator_descriptor(tool_id="custom.evaluator", version="2.0.0")
    assert descriptor.tool_id == "custom.evaluator"
    assert descriptor.version == "2.0.0"
    assert descriptor.side_effect_level.value == "none"
    assert descriptor.required_capabilities == ()
    assert descriptor.timeout_seconds == 10.0
    assert "filesystem" in descriptor.description


def test_simulator_handler_wraps_descriptor() -> None:
    handler = simulator_handler(tool_id="custom.tool", version="3.0.0")
    assert handler.descriptor.tool_id == "custom.tool"
    assert handler.descriptor.version == "3.0.0"


def test_simulator_tool_handler_invocates(tmp_path: Path) -> None:
    handler = SimulatorToolHandler()
    response = handler.invoke(_request(tmp_path, {"operation": "beam_volume"}))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert handler.descriptor.tool_id == SIMULATOR_TOOL_ID
    assert (
        handler.invoke(
            _request(tmp_path, {"operation": "unknown"}),
        ).status
        is ToolRunStatus.FAILED
    )


def test_simulator_tool_handler_honours_custom_identity(tmp_path: Path) -> None:
    handler = SimulatorToolHandler(tool_id="custom.tool", version="4.0.0")
    assert handler.descriptor.tool_id == "custom.tool"
    assert (
        handler.invoke(
            _request(tmp_path, {"operation": "beam_volume"}),
        ).status
        is ToolRunStatus.SUCCEEDED
    )


def test_evaluator_handler_returns_metrics(tmp_path: Path) -> None:
    handler = ReferenceEvaluatorHandler()
    response = handler.invoke(_request(tmp_path, {"operation": "beam_volume"}))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert _result(response)["operation"] == "beam_volume"
    assert _result(response)["report_path"] is None
    assert _result(response)["within_limits"] is True


def test_evaluator_handler_honours_custom_identity(tmp_path: Path) -> None:
    handler = ReferenceEvaluatorHandler(tool_id="custom.evaluator", version="5.0.0")
    assert handler.descriptor.tool_id == "custom.evaluator"
    assert (
        handler.invoke(
            _request(tmp_path, {"operation": "beam_volume"}),
        ).status
        is ToolRunStatus.SUCCEEDED
    )


def test_evaluator_refuses_to_write_reports(tmp_path: Path) -> None:
    handler = ReferenceEvaluatorHandler()
    response = handler.invoke(
        _request(tmp_path, {"operation": "beam_volume", "write_report": True})
    )
    assert response.status is ToolRunStatus.FAILED
    assert ToolErrorCode(response.error_code or "") is ToolErrorCode.WORKSPACE_VIOLATION
    assert "cannot write a report" in str(response.error_message)


def test_evaluator_reports_invalid_arguments(tmp_path: Path) -> None:
    handler = ReferenceEvaluatorHandler()
    response = handler.invoke(_request(tmp_path, {"operation": "nope"}))
    assert response.status is ToolRunStatus.FAILED
    assert ToolErrorCode(response.error_code or "") is ToolErrorCode.INVALID_ARGUMENTS
    assert "operation must be one of" in str(response.error_message)


def test_simulator_reports_invalid_arguments(tmp_path: Path) -> None:
    handler = SimulatorToolHandler()
    response = handler.invoke(_request(tmp_path, {"length_mm": 0.5}))
    assert response.status is ToolRunStatus.FAILED
    assert ToolErrorCode(response.error_code or "") is ToolErrorCode.INVALID_ARGUMENTS
    assert "length_mm must be between" in str(response.error_message)


def test_simulator_writes_report_on_request(tmp_path: Path) -> None:
    handler = SimulatorToolHandler()
    response = handler.invoke(
        _request(tmp_path, {"operation": "beam_volume", "write_report": True})
    )
    assert response.status is ToolRunStatus.SUCCEEDED
    assert _result(response)["report_path"] == "outputs/reference-report.json"
    assert len(response.artifacts) == 1
    assert response.artifacts[0].artifact_id == "call-1-report"
    assert (tmp_path / "outputs" / "reference-report.json").is_file()


def test_simulator_reports_workspace_violation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    handler = SimulatorToolHandler()

    def refusing(workspace: Path, relative: str) -> Path:
        raise ToolWorkspaceError("refused")

    monkeypatch.setattr(simulator_module, "resolve_workspace_path", refusing)
    response = handler.invoke(
        _request(tmp_path, {"operation": "beam_volume", "write_report": True})
    )
    assert response.status is ToolRunStatus.FAILED
    assert ToolErrorCode(response.error_code or "") is ToolErrorCode.WORKSPACE_VIOLATION
    assert "the report path is not inside the workspace" in str(response.error_message)


def test_liar_invoke_reports_a_non_boolean_claim(tmp_path: Path) -> None:
    response = liar_invoke(_request(tmp_path))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert _result(response)["within_limits"] == "not-a-boolean"


def test_unrecognized_code_invoke_reports_an_unknown_code(tmp_path: Path) -> None:
    response = unrecognized_code_invoke(_request(tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == "totally_fine_code"
    assert "does not recognise" in str(response.error_message)


def test_escape_invoke_writes_outside_the_workspace(tmp_path: Path) -> None:
    workspace = tmp_path / "ws"
    workspace.mkdir()
    response = escape_invoke(_request(workspace))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert _result(response)["report_path"] == "../escaped-report.json"
    assert len(response.artifacts) == 1
    assert (tmp_path / "escaped-report.json").is_file()


def test_stray_child_invoke_is_available_as_a_misbehavior_entry_point(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(time, "sleep", lambda _seconds: None)
    workspace = tmp_path / "ws"
    workspace.mkdir()
    monkeypatch.setattr(
        subprocess,
        "Popen",
        lambda *_a, **_k: None,
    )
    response = stray_child_invoke(_request(workspace))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert not (workspace / "stray-child-effect.txt").exists()


def test_flood_invoke_writes_a_bounded_flood(tmp_path: Path) -> None:
    response = flood_invoke(_request(tmp_path))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert _result(response)["operation"] == "beam_volume"


def test_flood_invoke_falls_back_when_stdout_is_missing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(sys, "__stdout__", None)
    response = flood_invoke(_request(tmp_path))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert _number(_result(response), "volume_mm3") == 24_000.0


def test_register_simulator_requires_a_registry() -> None:
    with pytest.raises(TypeError, match="ToolRegistry"):
        register_simulator(cast("Any", object()))


def test_register_reference_evaluator_requires_a_registry() -> None:
    with pytest.raises(TypeError, match="ToolRegistry"):
        register_reference_evaluator(cast("Any", object()))


def test_registration_uses_the_shared_schema_digests() -> None:
    from kala_agent.tools.registry import ToolRegistry

    registry = ToolRegistry()
    descriptor = register_simulator(registry)
    assert descriptor.tool_id == SIMULATOR_TOOL_ID
    evaluator = register_reference_evaluator(registry)
    assert evaluator.tool_id == EVALUATOR_TOOL_ID
    assert descriptor.input_schema_digest == evaluator.input_schema_digest
    assert descriptor.output_schema_digest == evaluator.output_schema_digest


def test_registration_honours_custom_identity() -> None:
    from kala_agent.tools.registry import ToolRegistry

    registry = ToolRegistry()
    assert register_simulator(registry, tool_id="a.tool", version="1.2.3").version == "1.2.3"
    assert (
        register_reference_evaluator(registry, tool_id="b.tool", version="4.5.6").version == "4.5.6"
    )


def test_response_envelope_reports_success_without_artifacts(tmp_path: Path) -> None:
    response = ToolResponseEnvelope.succeeded_envelope(
        call_id="call-1",
        tool_id=SIMULATOR_TOOL_ID,
        tool_version="1.0.0",
        result=cast("dict[str, JSONValue]", {"ok": True}),
    )
    assert response.status is ToolRunStatus.SUCCEEDED
    assert response.artifacts == ()
    assert response.call_id == "call-1"


def test_workspace_path_resolution_keeps_reports_inside(tmp_path: Path) -> None:
    resolved = resolve_workspace_path(tmp_path, "outputs/reference-report.json")
    assert resolved.parent == tmp_path / "outputs"
    with pytest.raises(ToolWorkspaceError):
        resolve_workspace_path(tmp_path, "../escape.json")


def test_report_directory_is_not_a_symlink_after_writes(tmp_path: Path) -> None:
    parameters = _parameters()
    write_report(tmp_path, parameters, compute_metrics(parameters), call_id="call-1")
    outputs = tmp_path / "outputs"
    assert not stat.S_ISLNK(os.lstat(outputs).st_mode)
