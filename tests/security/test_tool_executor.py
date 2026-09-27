from __future__ import annotations

import os
import socket
import sys
import threading
import time
from collections.abc import Iterator, Mapping, Sequence
from datetime import UTC, datetime
from pathlib import Path
from typing import cast

import pytest

from kala_agent.contracts import (
    ArtifactClassification,
    ArtifactRef,
    RunBudget,
    SideEffectLevel,
    ToolCall,
    ToolDescriptor,
    ToolResult,
    ToolRunStatus,
)
from kala_agent.kernel.budget import BudgetLedger
from kala_agent.kernel.policy import PolicyEngine
from kala_agent.tools import (
    EVALUATOR_TOOL_ID,
    EVALUATOR_TOOL_VERSION,
    INPUT_SCHEMA,
    MAX_ERROR_MESSAGE_LENGTH,
    MAX_PATTERN_INPUT_LENGTH,
    MAX_PATTERN_LENGTH,
    OUTPUT_SCHEMA,
    SIMULATOR_CAPABILITIES,
    SIMULATOR_TOOL_ID,
    SIMULATOR_TOOL_VERSION,
    STRICT_REQUIRED_CONTROLS,
    TEXTUAL_MEDIA_SUFFIXES,
    DuplicateToolRegistrationError,
    IsolationMode,
    IsolationProfile,
    RegisteredTool,
    SandboxProfile,
    SchemaDigestMismatchError,
    SchemaValidationError,
    SimulatorToolHandler,
    SubprocessToolHandler,
    ToolAuthorityError,
    ToolCapabilityError,
    ToolEnvelopeError,
    ToolErrorCode,
    ToolExecutionReport,
    ToolExecutor,
    ToolRegistrationError,
    ToolRegistry,
    ToolRegistryError,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    ToolVersionNotFoundError,
    ToolWorkspaceError,
    UnknownToolError,
    build_child_environment,
    build_worker_command,
    canonical_json_digest,
    canonical_json_dumps,
    check_object_schema,
    compute_metrics,
    parse_parameters,
    register_reference_evaluator,
    register_simulator,
    resolve_workspace_path,
    sanitize_child_stderr,
    sha256_hex,
    simulator_descriptor,
    simulator_handler,
    strict_json_loads,
    validate_against_schema,
)

RECORDED_AT = datetime(2026, 5, 6, 7, 8, 9, tzinfo=UTC)
TOOLS_ROOT = Path(__file__).parents[2] / "src" / "kala_agent" / "tools"
NETWORK_CAPABILITY = "network.egress"
SLOW_ENTRY_POINT = "kala_agent.tools.simulator:hang_invoke"
CRASH_ENTRY_POINT = "kala_agent.tools.simulator:crash_invoke"
FLOOD_ENTRY_POINT = "kala_agent.tools.simulator:flood_invoke"
LIAR_ENTRY_POINT = "kala_agent.tools.simulator:liar_invoke"
ESCAPE_ENTRY_POINT = "kala_agent.tools.simulator:escape_invoke"
STRAY_CHILD_ENTRY_POINT = "kala_agent.tools.simulator:stray_child_invoke"
LINGERING_CHILD_ENTRY_POINT = "kala_agent.tools.simulator:lingering_child_invoke"
STRAY_CHILD_NAME = "stray-child-effect.txt"
ECHO_ENTRY_POINT = "kala_agent.tools.simulator:invoke"


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("tests must not open network connections")

    monkeypatch.setattr(socket.socket, "connect", blocked)
    monkeypatch.setattr(socket, "create_connection", blocked)
    yield


_ECHO_INPUT: Mapping[str, object] = {
    "additionalProperties": False,
    "properties": {"value": {"maxLength": 32, "type": "string"}},
    "required": ["value"],
    "type": "object",
}
_ECHO_OUTPUT: Mapping[str, object] = {
    "additionalProperties": False,
    "properties": {"value": {"type": "string"}},
    "required": ["value"],
    "type": "object",
}


class _EchoHandler:
    def __init__(
        self,
        value: str = "echo",
        *,
        input_schema: Mapping[str, object] = _ECHO_INPUT,
        side_effect: SideEffectLevel = SideEffectLevel.READ,
        required_capabilities: tuple[str, ...] = (),
        tool_id: str = "test.echo",
    ) -> None:
        self._value = value
        self._descriptor = ToolDescriptor(
            tool_id=tool_id,
            version="1.0.0",
            name="Echo",
            description="Return the supplied value",
            input_schema_digest=canonical_json_digest(input_schema),
            output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
            side_effect_level=side_effect,
            required_capabilities=required_capabilities,
            timeout_seconds=5.0,
        )

    @property
    def descriptor(self) -> ToolDescriptor:
        return self._descriptor

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope:
        return ToolResponseEnvelope.succeeded_envelope(
            call_id=request.call_id,
            tool_id=request.tool_id,
            tool_version=request.tool_version,
            result={"value": self._value},
        )


def _call(
    call_id: str = "call-1",
    *,
    tool_id: str = SIMULATOR_TOOL_ID,
    tool_version: str = SIMULATOR_TOOL_VERSION,
    arguments: str = '{"operation": "beam_volume"}',
    side_effect: SideEffectLevel = SideEffectLevel.LOCAL_WRITE,
    capabilities: Sequence[str] = SIMULATOR_CAPABILITIES,
    timeout_seconds: float = 20.0,
) -> ToolCall:
    return ToolCall(
        call_id=call_id,
        tool_id=tool_id,
        tool_version=tool_version,
        arguments_json=arguments,
        requested_capabilities=tuple(capabilities),
        side_effect_level=side_effect,
        timeout_seconds=timeout_seconds,
        issued_at=RECORDED_AT,
    )


def _executor(
    tmp_path: Path,
    *,
    granted: Sequence[str] = SIMULATOR_CAPABILITIES,
    policy: PolicyEngine | None = None,
    ledger: BudgetLedger | None = None,
    profile: SandboxProfile | None = None,
    registry: ToolRegistry | None = None,
    isolation: IsolationMode = IsolationMode.AUTO,
) -> ToolExecutor:
    selected = registry if registry is not None else ToolRegistry()
    if registry is None:
        register_simulator(selected)
    return ToolExecutor(
        selected,
        run_directory=tmp_path / "run",
        granted_capabilities=granted,
        policy=policy,
        ledger=ledger,
        profile=profile,
        isolation=isolation,
    )


def _approved_executor(tmp_path: Path, **kwargs: object) -> tuple[ToolExecutor, PolicyEngine]:
    policy = PolicyEngine(kwargs.pop("granted", SIMULATOR_CAPABILITIES))  # type: ignore[arg-type]
    executor = _executor(tmp_path, policy=policy, **kwargs)  # type: ignore[arg-type]
    return executor, policy


def _approve(policy: PolicyEngine, executor: ToolExecutor, call: ToolCall) -> None:
    policy.approve(executor.policy_request(call))


def test_simulator_executes_successfully_in_a_subprocess(tmp_path: Path) -> None:
    executor, policy = _approved_executor(tmp_path)
    call = _call(arguments='{"operation": "beam_volume", "write_report": true}')
    _approve(policy, executor, call)
    result = executor.execute(call)
    assert result.status is ToolRunStatus.SUCCEEDED
    assert result.error_code is None
    assert result.result_json is not None
    payload = strict_json_loads(result.result_json)
    assert isinstance(payload, Mapping)
    expected = compute_metrics(parse_parameters({"operation": "beam_volume", "write_report": True}))
    assert payload["volume_mm3"] == expected["volume_mm3"]
    assert payload["mass_kg"] == expected["mass_kg"]
    assert payload["operation"] == "beam_volume"
    assert payload["report_path"] == "outputs/reference-report.json"
    assert len(result.artifacts) == 1
    artifact = result.artifacts[0]
    assert artifact.classification is ArtifactClassification.TEXT
    assert artifact.size_bytes > 0
    assert result.duration_ms is not None
    assert result.duration_ms > 0.0
    reports = executor.reports
    assert len(reports) == 1
    assert reports[0].isolation is IsolationMode.SUBPROCESS
    assert reports[0].process_id is not None
    assert reports[0].status is ToolRunStatus.SUCCEEDED
    assert reports[0].granted_capabilities == tuple(sorted(SIMULATOR_CAPABILITIES))
    workspace = executor.workspace_for("call-1")
    written = workspace / "outputs" / "reference-report.json"
    assert written.is_file()
    assert sha256_hex(written.read_bytes()) == artifact.digest


def test_default_auto_never_runs_a_tool_inside_the_kernel(tmp_path: Path) -> None:
    registry = ToolRegistry()
    registry.register(
        _EchoHandler().descriptor,
        _EchoHandler(),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    call = _call(
        tool_id="test.echo",
        arguments='{"value": "hello"}',
        side_effect=SideEffectLevel.READ,
        capabilities=(),
    )
    default = _executor(tmp_path, registry=registry, granted=("design.read",))
    denied = default.execute(call)
    assert denied.status is ToolRunStatus.DENIED
    assert denied.error_code == str(ToolErrorCode.ISOLATION_REQUIRED)
    assert default.reports == ()
    assert not (default.run_directory / "calls").exists()

    forced = _executor(
        tmp_path / "forced",
        registry=registry,
        granted=("design.read",),
        isolation=IsolationMode.SUBPROCESS,
    )
    forced_result = forced.execute(call)
    assert forced_result.status is ToolRunStatus.DENIED
    assert forced_result.error_code == str(ToolErrorCode.ISOLATION_REQUIRED)
    assert forced.reports == ()


def test_trusted_in_process_mode_serves_only_a_registered_pure_evaluator(tmp_path: Path) -> None:
    registry = ToolRegistry()
    register_reference_evaluator(registry)
    call = _call(
        call_id="call-evaluator",
        tool_id=EVALUATOR_TOOL_ID,
        tool_version=EVALUATOR_TOOL_VERSION,
        arguments='{"operation": "beam_volume"}',
        side_effect=SideEffectLevel.NONE,
        capabilities=(),
    )
    auto = _executor(tmp_path, registry=registry, granted=())
    refused = auto.execute(call)
    assert refused.status is ToolRunStatus.DENIED
    assert refused.error_code == str(ToolErrorCode.ISOLATION_REQUIRED)
    assert auto.reports == ()

    trusted = _executor(
        tmp_path / "trusted",
        registry=registry,
        granted=(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = trusted.execute(call)
    assert result.status is ToolRunStatus.SUCCEEDED
    assert result.result_json is not None
    payload = strict_json_loads(result.result_json)
    assert isinstance(payload, Mapping)
    assert payload["report_path"] is None
    assert payload["volume_mm3"] == 24_000.0
    assert trusted.reports[0].isolation is IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR
    assert trusted.reports[0].process_id is None


def test_untrusted_handlers_cannot_reach_the_trusted_in_process_mode(tmp_path: Path) -> None:
    dynamic = ToolRegistry()
    dynamic.register(
        _EchoHandler().descriptor,
        _EchoHandler(),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(
        tmp_path,
        registry=dynamic,
        granted=("design.read",),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(
        _call(
            tool_id="test.echo",
            arguments='{"value": "hello"}',
            side_effect=SideEffectLevel.READ,
            capabilities=(),
        )
    )
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == str(ToolErrorCode.ISOLATION_REQUIRED)
    assert executor.reports == ()

    subprocess_registry = ToolRegistry()
    subprocess_registry.register(
        _EchoHandler().descriptor,
        SubprocessToolHandler(descriptor=_EchoHandler().descriptor, entry_point=ECHO_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    subprocess_executor = _executor(
        tmp_path / "subprocess",
        registry=subprocess_registry,
        granted=("design.read",),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    escalated = subprocess_executor.execute(
        _call(
            call_id="call-subprocess",
            tool_id="test.echo",
            arguments='{"value": "hello"}',
            side_effect=SideEffectLevel.READ,
            capabilities=(),
        )
    )
    assert escalated.status is ToolRunStatus.DENIED
    assert escalated.error_code == str(ToolErrorCode.ISOLATION_REQUIRED)


def test_only_a_first_party_pure_evaluator_may_be_registered_as_trusted() -> None:
    registry = ToolRegistry()
    with pytest.raises(ToolRegistrationError, match="non-destructive"):
        registry.register_trusted_evaluator(
            simulator_descriptor(),
            SimulatorToolHandler(),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )
    pure = _EchoHandler(side_effect=SideEffectLevel.NONE)
    with pytest.raises(ToolRegistrationError, match="kernel package"):
        registry.register_trusted_evaluator(
            pure.descriptor,
            pure,
            input_schema=_ECHO_INPUT,
            output_schema=_ECHO_OUTPUT,
        )
    with pytest.raises(ToolRegistrationError, match="subprocess handler"):
        registry.register_trusted_evaluator(
            pure.descriptor,
            SubprocessToolHandler(descriptor=pure.descriptor, entry_point=ECHO_ENTRY_POINT),
            input_schema=_ECHO_INPUT,
            output_schema=_ECHO_OUTPUT,
        )
    assert len(registry) == 0
    descriptor = register_reference_evaluator(registry)
    registered = registry.require(descriptor.tool_id)
    assert registered.trusted_in_process is True
    assert registered.side_effect_level is SideEffectLevel.NONE


def test_missing_capabilities_are_denied_before_dispatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("design.read",))
    call = _call()
    result = executor.execute(call)
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == str(ToolErrorCode.CAPABILITY_DENIED)
    assert "artifact.write" in str(result.error_message)
    assert executor.reports == ()
    assert not (executor.run_directory / "calls").exists()


def test_registration_refuses_tools_that_claim_ungranted_authority(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("design.read",), registry=ToolRegistry())
    with pytest.raises(ToolAuthorityError):
        executor.register(
            simulator_descriptor(),
            simulator_handler(),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )
    assert executor.registry.tool_ids() == ()
    assert len(executor.registry) == 0
    for capability in SIMULATOR_CAPABILITIES:
        executor.grant_capability(capability)
    registered = executor.register(
        simulator_descriptor(),
        simulator_handler(),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    assert isinstance(registered, RegisteredTool)
    assert executor.registry.tool_ids() == (SIMULATOR_TOOL_ID,)


@pytest.mark.parametrize(
    ("capability", "side_effect", "expected"),
    [
        ("network", SideEffectLevel.NETWORK, ToolErrorCode.NETWORK_DENIED),
        ("network.egress", SideEffectLevel.EGRESS, ToolErrorCode.EGRESS_DENIED),
    ],
)
def test_network_and_egress_are_denied_by_default(
    tmp_path: Path,
    capability: str,
    side_effect: SideEffectLevel,
    expected: ToolErrorCode,
) -> None:
    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id=f"test.{capability.replace('.', '-')}",
        version="1.0.0",
        name="Remote",
        description="Declares a side effect the profile denies",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=side_effect,
        required_capabilities=(capability,),
        timeout_seconds=5.0,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=SLOW_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(
        tmp_path,
        granted=(capability,),
        registry=registry,
        policy=PolicyEngine([capability]),
    )
    call = _call(
        tool_id=descriptor.tool_id,
        arguments='{"value": "hello"}',
        side_effect=side_effect,
        capabilities=(capability,),
    )
    result = executor.execute(call)
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == str(expected)
    assert executor.reports == ()
    assert not (executor.run_directory / "calls").exists()


def test_unknown_tools_and_versions_are_denied(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    unknown = executor.execute(_call(tool_id="test.absent"))
    assert unknown.status is ToolRunStatus.DENIED
    assert unknown.error_code == str(ToolErrorCode.UNKNOWN_TOOL)
    missing_version = executor.execute(_call(tool_version="9.9.9"))
    assert missing_version.status is ToolRunStatus.DENIED
    assert missing_version.error_code == str(ToolErrorCode.VERSION_NOT_FOUND)
    assert executor.reports == ()


def test_side_effect_mismatch_is_denied(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    call = _call(side_effect=SideEffectLevel.READ)
    result = executor.execute(call)
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == str(ToolErrorCode.SIDE_EFFECT_MISMATCH)


def test_undeclared_capability_requests_are_denied(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=(*SIMULATOR_CAPABILITIES, "artifact.archive"))
    call = _call(capabilities=(*SIMULATOR_CAPABILITIES, "artifact.archive"))
    result = executor.execute(call)
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == str(ToolErrorCode.CAPABILITY_DENIED)


def test_policy_approval_is_required_and_recorded(tmp_path: Path) -> None:
    executor, policy = _approved_executor(tmp_path)
    call = _call()
    pending = executor.execute(call)
    assert pending.status is ToolRunStatus.DENIED
    assert pending.error_code == str(ToolErrorCode.APPROVAL_REQUIRED)
    assert executor.reports == ()
    _approve(policy, executor, call)
    approved = executor.execute(call)
    assert approved.status is ToolRunStatus.SUCCEEDED


def test_approval_cannot_be_replayed_with_changed_arguments(tmp_path: Path) -> None:
    executor, policy = _approved_executor(tmp_path)
    approved_call = _call(call_id="call-replay", arguments='{"operation": "beam_volume"}')
    approved_digest = executor.policy_request(approved_call).action_digest
    assert (
        executor.policy_request(
            _call(call_id="call-replay", arguments='{"operation": "bar_summary"}')
        ).action_digest
        != approved_digest
    )
    _approve(policy, executor, approved_call)
    first = executor.execute(approved_call)
    assert first.status is ToolRunStatus.SUCCEEDED

    changed = _call(
        call_id="call-replay",
        arguments='{"operation": "bar_summary", "write_report": true}',
    )
    replayed = executor.execute(changed)
    assert replayed.status is ToolRunStatus.DENIED
    assert replayed.error_code == str(ToolErrorCode.APPROVAL_REQUIRED)
    assert "stale_approval" in str(replayed.error_message)
    assert not (executor.workspace_for("call-replay") / "outputs").exists()
    assert executor.reports[0].status is ToolRunStatus.SUCCEEDED

    benign = _call(
        call_id="call-replay",
        arguments='{"operation": "beam_volume", "write_report": true}',
    )
    still_rejected = executor.execute(benign)
    assert still_rejected.status is ToolRunStatus.DENIED
    assert still_rejected.error_code == str(ToolErrorCode.APPROVAL_REQUIRED)
    assert executor.policy_request(approved_call).action_digest == approved_digest
    assert executor.policy_request(changed).action_digest != approved_digest


def test_approval_is_bound_to_the_resolved_workspace_and_tool_identity(tmp_path: Path) -> None:
    executor, _ = _approved_executor(tmp_path / "left")
    other_run = _executor(tmp_path / "right")
    call = _call(call_id="call-binding")
    request = executor.policy_request(call)
    elsewhere = other_run.policy_request(call)
    assert elsewhere.action_digest != request.action_digest
    assert elsewhere.state_digest == request.state_digest
    assert request.requested_capabilities == tuple(sorted(SIMULATOR_CAPABILITIES))
    assert len(request.action_digest) == 64
    assert len(request.state_digest) == 64
    assert request.checkpoint_type is not None
    executor.grant_capability("artifact.archive")
    assert executor.policy_request(call).state_digest != request.state_digest


def test_sandbox_controls_are_reported_only_when_probed(tmp_path: Path) -> None:
    portable = _executor(tmp_path / "portable")
    controls = portable.enforced_controls
    assert controls == tuple(sorted(controls))
    assert "process_isolation" in controls
    assert "process_group_kill" in controls
    assert "worker_concurrency_cap" in controls
    for confinement in (
        "chroot_workspace_only",
        "filesystem_confinement",
        "network_isolation",
        "user_namespace",
        "mount_namespace",
        "workspace_read_write",
    ):
        assert confinement not in controls
    assert portable.sandbox_probe.available is False
    unprobed = SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX)
    assert "chroot_workspace_only" not in unprobed.enforced_controls
    assert "chroot_workspace_only" in STRICT_REQUIRED_CONTROLS

    strict = _executor(
        tmp_path / "strict",
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
    )
    probe = strict.sandbox_probe
    verified = strict.enforced_controls
    assert verified == tuple(sorted(verified))
    call = _call(call_id="call-strict", arguments='{"operation": "beam_volume"}')
    if not probe.available:
        denied = strict.execute(call)
        assert denied.status is ToolRunStatus.DENIED
        assert denied.error_code == str(ToolErrorCode.SANDBOX_UNAVAILABLE)
        assert strict.reports == ()
        return
    for control in probe.controls:
        assert control in verified
    assert "filesystem_confinement" not in verified
    policy = PolicyEngine(SIMULATOR_CAPABILITIES)
    policy.approve(strict.policy_request(call))
    result = strict.execute(call)
    assert result.status is ToolRunStatus.SUCCEEDED
    assert strict.reports[0].enforced_controls == verified
    assert strict.reports[0].isolation is IsolationMode.SUBPROCESS
    workspace = strict.workspace_for("call-strict")
    assert not (workspace.parent.parent / "sandbox").exists() or not any(
        (workspace.parent.parent / "sandbox").rglob("root")
    )


def test_policy_denial_for_a_missing_capability_is_reported(tmp_path: Path) -> None:
    policy = PolicyEngine(("design.read",))
    executor = _executor(tmp_path, granted=SIMULATOR_CAPABILITIES, policy=policy)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == str(ToolErrorCode.CAPABILITY_DENIED)
    assert "capability_not_granted" in str(result.error_message)


def test_timeout_terminates_the_worker(tmp_path: Path) -> None:
    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id="test.slow",
        version="1.0.0",
        name="Slow",
        description="Sleeps past its deadline",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("design.read",),
        timeout_seconds=0.05,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=SLOW_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(tmp_path, granted=("design.read",), registry=registry)
    call = _call(
        call_id="call-slow",
        tool_id="test.slow",
        arguments='{"value": "hello"}',
        side_effect=SideEffectLevel.READ,
        capabilities=("design.read",),
        timeout_seconds=0.2,
    )
    started = time.monotonic()
    result = executor.execute(call)
    elapsed = time.monotonic() - started
    assert result.status is ToolRunStatus.TIMED_OUT
    assert result.error_code == str(ToolErrorCode.TIMEOUT)
    assert result.result_json is None
    assert elapsed < 20.0
    assert executor.reports[0].status is ToolRunStatus.TIMED_OUT


def _report_ids(executor: ToolExecutor) -> tuple[str, ...]:
    return tuple(report.call_id for report in executor.reports)


def _subprocess_executor(
    tmp_path: Path,
    *,
    tool_id: str,
    entry_point: str,
    side_effect: SideEffectLevel = SideEffectLevel.READ,
    capabilities: tuple[str, ...] = ("design.read",),
    timeout_seconds: float = 30.0,
    profile: SandboxProfile | None = None,
) -> ToolExecutor:
    descriptor = ToolDescriptor(
        tool_id=tool_id,
        version="1.0.0",
        name=tool_id,
        description="A tool used to exercise process supervision",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=side_effect,
        required_capabilities=capabilities,
        timeout_seconds=timeout_seconds,
    )
    registry = ToolRegistry()
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=entry_point),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    return _executor(
        tmp_path,
        granted=capabilities,
        registry=registry,
        profile=profile,
    )


def _call_for(
    tool_id: str,
    call_id: str,
    *,
    side_effect: SideEffectLevel = SideEffectLevel.READ,
    capabilities: tuple[str, ...] = ("design.read",),
    timeout_seconds: float = 20.0,
    arguments: str = '{"value": "hello"}',
) -> ToolCall:
    return _call(
        call_id=call_id,
        tool_id=tool_id,
        arguments=arguments,
        side_effect=side_effect,
        capabilities=capabilities,
        timeout_seconds=timeout_seconds,
    )


def test_a_timed_out_tool_leaves_no_running_descendant(tmp_path: Path) -> None:
    executor = _subprocess_executor(
        tmp_path,
        tool_id="test.stray",
        entry_point=STRAY_CHILD_ENTRY_POINT,
        timeout_seconds=20.0,
    )
    result = executor.execute(_call_for("test.stray", "call-stray", timeout_seconds=0.3))
    assert result.status is ToolRunStatus.TIMED_OUT
    assert result.error_code == str(ToolErrorCode.TIMEOUT)
    workspace = executor.workspace_for("call-stray")
    stray = workspace / STRAY_CHILD_NAME
    deadline = time.monotonic() + 3.0
    while time.monotonic() < deadline:
        assert not stray.exists(), "a timed out tool left a descendant that still writes"
        time.sleep(0.1)
    assert not stray.exists()
    assert not list(workspace.glob("*.partial"))
    report = executor.reports[0]
    assert report.status is ToolRunStatus.TIMED_OUT
    assert report.process_id is not None
    assert _process_is_gone(report.process_id)


def test_a_successful_tool_leaves_no_running_descendant(tmp_path: Path) -> None:
    executor = _subprocess_executor(
        tmp_path,
        tool_id="test.lingering",
        entry_point=LINGERING_CHILD_ENTRY_POINT,
        timeout_seconds=20.0,
    )
    result = executor.execute(_call_for("test.lingering", "call-lingering", timeout_seconds=10.0))
    assert result.status is ToolRunStatus.SUCCEEDED
    stray = executor.workspace_for("call-lingering") / STRAY_CHILD_NAME
    deadline = time.monotonic() + 3.0
    while time.monotonic() < deadline:
        assert not stray.exists(), "a successful tool left a descendant that still writes"
        time.sleep(0.1)
    assert not stray.exists()


def _process_is_gone(pid: int) -> bool:
    try:
        with open(f"/proc/{pid}/stat", encoding="utf-8") as handle:
            state = handle.read().rsplit(")", 1)[-1].split()[0]
    except OSError:
        return True
    return state == "Z"


def test_the_worker_cap_bounds_concurrent_tool_processes(tmp_path: Path) -> None:
    deadline_seconds = 0.25
    executor = _subprocess_executor(
        tmp_path,
        tool_id="test.slow",
        entry_point=SLOW_ENTRY_POINT,
        profile=SandboxProfile(max_workers=1, default_timeout_seconds=10.0),
    )
    results: list[ToolResult] = []
    started = time.monotonic()
    threads = [
        threading.Thread(
            target=lambda index=index: results.append(
                executor.execute(
                    _call_for("test.slow", f"call-capped-{index}", timeout_seconds=deadline_seconds)
                )
            )
        )
        for index in range(3)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=60.0)
    elapsed = time.monotonic() - started
    assert len(results) == 3
    assert all(result.status is ToolRunStatus.TIMED_OUT for result in results)
    assert elapsed >= 3 * deadline_seconds
    assert executor.profile.max_workers == 1
    assert "worker_concurrency_cap" in executor.enforced_controls
    assert len(executor.reports) == 3


def test_a_saturated_worker_pool_denies_instead_of_queueing_forever(tmp_path: Path) -> None:
    executor = _subprocess_executor(
        tmp_path,
        tool_id="test.slow",
        entry_point=SLOW_ENTRY_POINT,
        profile=SandboxProfile(
            max_workers=1,
            max_queue_seconds=0.05,
            default_timeout_seconds=10.0,
        ),
    )
    occupied = threading.Event()
    finish = threading.Event()
    holding: list[ToolResult] = []

    def hold() -> None:
        call = _call_for("test.slow", "call-holder", timeout_seconds=1.5)
        timer = threading.Timer(0.3, occupied.set)
        timer.start()
        try:
            holding.append(executor.execute(call))
        finally:
            timer.cancel()
            finish.set()

    holder = threading.Thread(target=hold)
    holder.start()
    try:
        assert occupied.wait(timeout=10.0)
        started = time.monotonic()
        result = executor.execute(_call_for("test.slow", "call-saturated", timeout_seconds=1.0))
        waited = time.monotonic() - started
        assert result.status is ToolRunStatus.DENIED
        assert result.error_code == str(ToolErrorCode.WORKER_CAPACITY_EXHAUSTED)
        assert waited < 1.0
        assert _report_ids(executor) == ()
    finally:
        finish.set()
        holder.join(timeout=30.0)
    assert holding and holding[0].status is ToolRunStatus.TIMED_OUT
    assert _report_ids(executor) == ("call-holder",)


def test_crashing_worker_is_reported_as_a_structured_failure(tmp_path: Path) -> None:
    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id="test.crash",
        version="1.0.0",
        name="Crash",
        description="Terminates abruptly",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("design.read",),
        timeout_seconds=10.0,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=CRASH_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(tmp_path, granted=("design.read",), registry=registry)
    result = executor.execute(
        _call(
            call_id="call-crash",
            tool_id="test.crash",
            arguments='{"value": "hello"}',
            side_effect=SideEffectLevel.READ,
            capabilities=("design.read",),
        )
    )
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code in {
        str(ToolErrorCode.CRASHED),
        str(ToolErrorCode.INTERNAL_ERROR),
    }
    assert result.result_json is None
    assert result.artifacts == ()


def test_output_flooding_is_bounded(tmp_path: Path) -> None:
    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id="test.flood",
        version="1.0.0",
        name="Flood",
        description="Writes far more than the output limit",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("design.read",),
        timeout_seconds=20.0,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=FLOOD_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(tmp_path, granted=("design.read",), registry=registry)
    result = executor.execute(
        _call(
            call_id="call-flood",
            tool_id="test.flood",
            arguments='{"value": "hello"}',
            side_effect=SideEffectLevel.READ,
            capabilities=("design.read",),
        )
    )
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.OUTPUT_TOO_LARGE)
    assert result.result_json is None


def test_oversized_arguments_are_rejected(tmp_path: Path) -> None:
    registry = ToolRegistry()
    long_input: Mapping[str, object] = {
        "additionalProperties": False,
        "properties": {"value": {"type": "string"}},
        "required": ["value"],
        "type": "object",
    }
    handler = _EchoHandler(input_schema=long_input)
    registry.register(
        handler.descriptor,
        SubprocessToolHandler(descriptor=handler.descriptor, entry_point=ECHO_ENTRY_POINT),
        input_schema=long_input,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(
        tmp_path,
        granted=(),
        registry=registry,
        profile=SandboxProfile(max_argument_bytes=128),
    )
    oversized = executor.execute(
        _call(
            tool_id="test.echo",
            arguments=canonical_json_dumps({"value": "x" * 4_096}),
            side_effect=SideEffectLevel.READ,
            capabilities=(),
        )
    )
    assert oversized.status is ToolRunStatus.FAILED
    assert oversized.error_code == str(ToolErrorCode.INVALID_ARGUMENTS)
    assert oversized.result_json is None
    assert executor.reports == ()


def test_cancellation_is_observed_during_dispatch(tmp_path: Path) -> None:
    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id="test.slow",
        version="1.0.0",
        name="Slow",
        description="Sleeps until cancelled",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("design.read",),
        timeout_seconds=30.0,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=SLOW_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(tmp_path, granted=("design.read",), registry=registry)
    cancel = threading.Event()
    timer = threading.Timer(0.2, cancel.set)
    timer.start()
    try:
        result = executor.execute(
            _call(
                call_id="call-cancel",
                tool_id="test.slow",
                arguments='{"value": "hello"}',
                side_effect=SideEffectLevel.READ,
                capabilities=("design.read",),
                timeout_seconds=25.0,
            ),
            cancel=cancel,
        )
    finally:
        timer.cancel()
    assert result.status is ToolRunStatus.CANCELLED
    assert result.error_code is None
    assert result.result_json is None


def test_pre_cancelled_calls_never_dispatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    cancel = threading.Event()
    cancel.set()
    result = executor.execute(_call(call_id="call-cancelled"), cancel=cancel)
    assert result.status is ToolRunStatus.CANCELLED
    assert result.result_json is None
    assert executor.reports == ()


def test_workspace_paths_reject_traversal_and_symlinks(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    workspace = executor.workspace_for("call-paths")
    (workspace / "real").mkdir()
    outside = tmp_path / "outside.txt"
    outside.write_text("secret", encoding="utf-8")
    (workspace / "link-outside").symlink_to(outside)
    (workspace / "real" / "nested.txt").write_text("value", encoding="utf-8")
    (workspace / "real" / "link-outside").symlink_to(outside)
    (workspace / "real" / "link-inside").symlink_to(workspace / "real")

    assert resolve_workspace_path(workspace, "real/nested.txt") == workspace / "real" / "nested.txt"
    for candidate in (
        "../outside.txt",
        "real/../../outside.txt",
        "/etc/passwd",
        "link-outside",
        "real/link-inside",
        "..",
        ".",
        "",
        "back\\slash.txt",
        "null\x00byte.txt",
        "x" * 2_048,
    ):
        with pytest.raises(ToolWorkspaceError):
            resolve_workspace_path(workspace, candidate)
    for candidate in ("a" * 100,):
        with pytest.raises(ToolWorkspaceError):
            resolve_workspace_path(workspace, "/".join([candidate] * 40))


def test_artifacts_outside_the_workspace_are_rejected(tmp_path: Path) -> None:
    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id="test.escape",
        version="1.0.0",
        name="Escape",
        description="Reports an artifact outside its workspace",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(OUTPUT_SCHEMA),
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        required_capabilities=("design.read",),
        timeout_seconds=10.0,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(descriptor=descriptor, entry_point=ESCAPE_ENTRY_POINT),
        input_schema=_ECHO_INPUT,
        output_schema=OUTPUT_SCHEMA,
    )
    executor, policy = _approved_executor(tmp_path, granted=("design.read",), registry=registry)
    call = _call(
        call_id="call-escape",
        tool_id="test.escape",
        arguments='{"value": "hello"}',
        capabilities=("design.read",),
    )
    _approve(policy, executor, call)
    result = executor.execute(call)
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.ARTIFACT_REJECTED)
    assert result.artifacts == ()
    assert result.result_json is None
    escaped = executor.run_directory / "calls" / "call-escape" / "escaped-report.json"
    assert escaped.is_file()
    assert not (executor.workspace_for("call-escape") / "escaped-report.json").exists()


def test_symlinked_artifacts_are_rejected(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "payload.txt"
    target.write_text("payload", encoding="utf-8")
    link = workspace / "link.txt"
    link.symlink_to(target)
    artifact = ArtifactRef(
        artifact_id="artifact-1",
        uri=f"file://{link.as_posix()}",
        digest=sha256_hex(b"payload"),
        media_type="text/plain",
        size_bytes=len("payload"),
        classification=ArtifactClassification.TEXT,
    )
    executor = _executor(tmp_path)
    with pytest.raises(ToolWorkspaceError):
        executor._verify_artifacts((artifact,), workspace)
    escaping = workspace / "escape.txt"
    escaping.write_text("payload", encoding="utf-8")
    outside_artifact = ArtifactRef(
        artifact_id="artifact-2",
        uri=f"file://{(tmp_path / 'outside.txt').as_posix()}",
        digest=sha256_hex(b"payload"),
        media_type="text/plain",
        size_bytes=len("payload"),
    )
    (tmp_path / "outside.txt").write_text("payload", encoding="utf-8")
    with pytest.raises(ToolWorkspaceError):
        executor._verify_artifacts((outside_artifact,), workspace)
    assert escaping.is_file()


def test_artifact_digest_and_size_are_verified(tmp_path: Path) -> None:
    workspace = tmp_path / "verify"
    workspace.mkdir()
    payload = b"verified"
    (workspace / "report.json").write_bytes(payload)
    good = ArtifactRef(
        artifact_id="artifact-1",
        uri=f"file://{(workspace / 'report.json').as_posix()}",
        digest=sha256_hex(payload),
        media_type="application/json",
        size_bytes=len(payload),
    )
    executor = _executor(tmp_path)
    verified = executor._verify_artifacts((good,), workspace)
    assert verified[0].size_bytes == len(payload)

    wrong_digest = good.model_copy(update={"digest": sha256_hex(b"different")})
    with pytest.raises(ToolWorkspaceError):
        executor._verify_artifacts((wrong_digest,), workspace)
    wrong_size = good.model_copy(update={"size_bytes": len(payload) + 1})
    with pytest.raises(ToolWorkspaceError):
        executor._verify_artifacts((wrong_size,), workspace)
    remote = good.model_copy(update={"uri": "https://example.com/report.json"})
    with pytest.raises(ToolWorkspaceError):
        executor._verify_artifacts((remote,), workspace)
    duplicate = good.model_copy(update={"artifact_id": "artifact-1"})
    with pytest.raises(ToolWorkspaceError):
        executor._verify_artifacts((duplicate, duplicate), workspace)


def test_unknown_artifact_formats_are_classified_as_design_binaries(tmp_path: Path) -> None:
    workspace = tmp_path / "classification"
    workspace.mkdir()
    executor = _executor(tmp_path)
    cases: tuple[tuple[str, bytes, ArtifactClassification, ArtifactClassification], ...] = (
        (
            "report.json",
            b'{"ok": true}',
            ArtifactClassification.METADATA,
            ArtifactClassification.TEXT,
        ),
        ("notes.md", b"plain text", ArtifactClassification.TEXT, ArtifactClassification.TEXT),
        (
            "unknown.payload",
            b"plain text",
            ArtifactClassification.TEXT,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "unknown.payload",
            b"plain text",
            ArtifactClassification.METADATA,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "archive.bin",
            b"PK\x03\x04rest",
            ArtifactClassification.TEXT,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "mesh.stl",
            b"solid x\nendsolid x\n",
            ArtifactClassification.TEXT,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "part.step",
            b"ISO-10303-21;\nHEADER;\n",
            ArtifactClassification.TEXT,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "part.step",
            b"ISO-10303-21;\n",
            ArtifactClassification.DESIGN_BINARY,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "report.json",
            b'{"ok": tru\x00e}',
            ArtifactClassification.TEXT,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "report.json",
            b"\xff\xfe\x00bad",
            ArtifactClassification.TEXT,
            ArtifactClassification.DESIGN_BINARY,
        ),
        (
            "report.json",
            b"{}",
            ArtifactClassification.DESIGN_BINARY,
            ArtifactClassification.DESIGN_BINARY,
        ),
        ("report.json", b"{}", ArtifactClassification.DERIVED_DATA, ArtifactClassification.TEXT),
    )
    for name, payload, declared, expected in cases:
        path = workspace / name
        path.write_bytes(payload)
        artifact = ArtifactRef(
            artifact_id=f"artifact-{name}-{declared.value}",
            uri=f"file://{path.as_posix()}",
            digest=sha256_hex(payload),
            media_type="application/octet-stream",
            size_bytes=len(payload),
            classification=declared,
        )
        verified = executor._verify_artifacts((artifact,), workspace)
        assert verified[0].classification is expected, (name, declared)
    for name in ("mesh.stl", "part.step"):
        assert name not in TEXTUAL_MEDIA_SUFFIXES


def test_schema_patterns_are_bounded_before_they_are_executed() -> None:
    schema: dict[str, object] = {
        "type": "string",
        "maxLength": MAX_PATTERN_INPUT_LENGTH,
        "pattern": "^bar-[0-9]{2}$",
    }
    validate_against_schema("bar-42", schema)
    with pytest.raises(SchemaValidationError, match="does not match"):
        validate_against_schema("nope", schema)

    for pattern in (
        "(a+)+$",
        r"(\w+\s?)*$",
        "(x+x+)+y",
        r"(.+)*z",
        "^(a*)*$",
        r"^(\s+)+x",
        r"^(?:foo|bar)+$",
    ):
        hostile: dict[str, object] = {
            "type": "string",
            "maxLength": MAX_PATTERN_INPUT_LENGTH,
            "pattern": pattern,
        }
        with pytest.raises(
            SchemaValidationError,
            match=r"unsafe construct|quantifier|alternatives",
        ):
            validate_against_schema("aaa", hostile)

    for pattern, sample in (
        ("(?:abc)+", "abc"),
        ("^(a+)$", "aaa"),
        ("^(ab)+$", "abab"),
        (r"^/[\w./-]{0,200}$", "/a/b-c.json"),
    ):
        safe: dict[str, object] = {
            "type": "string",
            "maxLength": MAX_PATTERN_INPUT_LENGTH,
            "pattern": pattern,
        }
        validate_against_schema(sample, safe)

    for pattern in (r"(a)\1", r"(?=x)y", r"(?<=x)y", r"(a)?(?(1)b|c)"):
        hostile = {"type": "string", "maxLength": 64, "pattern": pattern}
        with pytest.raises(SchemaValidationError, match="unsafe construct"):
            validate_against_schema("a", hostile)

    unbounded = {"type": "string", "pattern": "^a+$"}
    with pytest.raises(SchemaValidationError, match="maxLength"):
        validate_against_schema("aaa", unbounded)
    too_wide = {"type": "string", "maxLength": 100_000, "pattern": "^a+$"}
    with pytest.raises(SchemaValidationError, match="maxLength"):
        validate_against_schema("aaa", too_wide)
    over_long = {"type": "string", "maxLength": 32, "pattern": "a" * (MAX_PATTERN_LENGTH + 1)}
    with pytest.raises(SchemaValidationError, match="bounded length"):
        validate_against_schema("a", over_long)
    excessive_repeat = {"type": "string", "maxLength": 32, "pattern": "^a{100000}$"}
    with pytest.raises(SchemaValidationError, match="repetition"):
        validate_against_schema("a", excessive_repeat)
    with pytest.raises(SchemaValidationError, match="unbalanced group"):
        validate_against_schema("a", {"type": "string", "maxLength": 32, "pattern": "([a-"})
    with pytest.raises(SchemaValidationError, match="not a valid regular expression"):
        validate_against_schema("a", {"type": "string", "maxLength": 32, "pattern": "a{2,1}"})

    with pytest.raises(SchemaValidationError, match="maxLength between"):
        validate_against_schema("a", {"type": "string", "maxLength": 4_096, "pattern": "^a$"})
    wide_but_bounded = {"type": "string", "maxLength": 4_096, "pattern": "^a+$"}
    with pytest.raises(SchemaValidationError, match="maxLength between"):
        validate_against_schema("a" * (MAX_PATTERN_INPUT_LENGTH + 1), wide_but_bounded)
    narrow = {"type": "string", "maxLength": 8, "pattern": "^a+$"}
    with pytest.raises(SchemaValidationError, match="longer than the permitted length"):
        validate_against_schema("a" * 9, narrow)

    with pytest.raises(SchemaValidationError, match="maxLength"):
        check_object_schema(
            {
                "additionalProperties": False,
                "properties": {"name": {"type": "string", "pattern": "^(a+)+$"}},
                "required": ["name"],
                "type": "object",
            }
        )
    registry = ToolRegistry()
    with pytest.raises(ToolRegistrationError, match=r"unsafe construct|quantifier"):
        registry.register(
            _EchoHandler().descriptor,
            _EchoHandler(),
            input_schema={
                "additionalProperties": False,
                "properties": {"value": {"type": "string", "maxLength": 32, "pattern": "^(a+)+$"}},
                "required": ["value"],
                "type": "object",
            },
            output_schema=_ECHO_OUTPUT,
        )
    with pytest.raises(SchemaValidationError, match="maxLength"):
        check_object_schema(
            {
                "additionalProperties": False,
                "properties": {"value": {"type": "string", "pattern": "^a$"}},
                "required": ["value"],
                "type": "object",
            }
        )


def test_an_unrecognised_tool_error_code_is_normalized(tmp_path: Path) -> None:
    executor = _subprocess_executor(
        tmp_path,
        tool_id="test.code",
        entry_point="kala_agent.tools.simulator:unrecognized_code_invoke",
    )
    result = executor.execute(_call_for("test.code", "call-code"))
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.INTERNAL_ERROR)
    assert "totally_fine_code" not in str(result.error_code)
    assert executor.reports[0].error_code == str(ToolErrorCode.INTERNAL_ERROR)
    assert executor._normalize_error_code(None) is None
    assert executor._normalize_error_code(ToolErrorCode.TIMEOUT) is ToolErrorCode.TIMEOUT
    assert executor._normalize_error_code("nonsense") is ToolErrorCode.INTERNAL_ERROR


def test_child_stderr_is_surfaced_as_a_sanitized_diagnostic(tmp_path: Path) -> None:
    workspace = tmp_path / "stderr"
    workspace.mkdir()
    summary = sanitize_child_stderr(
        f"ImportError: no module named definitely_missing at {workspace}/mod.py\n".encode(),
        replacement=str(workspace),
    )
    assert "definitely_missing" in summary
    assert str(workspace) not in summary
    assert "<workspace>" in summary
    assert sanitize_child_stderr(b"") == ""
    assert sanitize_child_stderr(b"\x00\x01binary\xff") == "binary�"
    assert len(sanitize_child_stderr(b"x" * 8_192)) <= MAX_ERROR_MESSAGE_LENGTH

    registry = ToolRegistry()
    descriptor = ToolDescriptor(
        tool_id="test.import",
        version="1.0.0",
        name="Import",
        description="Fails to import its module",
        input_schema_digest=canonical_json_digest(_ECHO_INPUT),
        output_schema_digest=canonical_json_digest(_ECHO_OUTPUT),
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("design.read",),
        timeout_seconds=20.0,
    )
    registry.register(
        descriptor,
        SubprocessToolHandler(
            descriptor=descriptor, entry_point="kala_agent.tools.simulator:absent_invoke"
        ),
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    executor = _executor(tmp_path, granted=("design.read",), registry=registry)
    result = executor.execute(_call_for("test.import", "call-import"))
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.CRASHED)
    assert "absent_invoke" in str(result.error_message)
    assert "Traceback" in str(result.error_message)


def test_child_environments_are_minimal_and_workspace_scoped(tmp_path: Path) -> None:
    workspace = tmp_path / "child"
    workspace.mkdir()
    environment = build_child_environment(
        workspace=workspace,
        import_roots=[Path(__file__).parents[2] / "src"],
    )
    assert set(environment) == {
        "HOME",
        "KALA_TOOL_PROTOCOL",
        "LANG",
        "PATH",
        "PYTHONDONTWRITEBYTECODE",
        "PYTHONHASHSEED",
        "PYTHONIOENCODING",
        "PYTHONNOUSERSITE",
        "PYTHONPATH",
        "PYTHONSAFEPATH",
        "TMPDIR",
    }
    assert environment["HOME"] == environment["TMPDIR"] == workspace.resolve().as_posix()
    assert "SECRET" not in environment
    command = build_worker_command(
        SubprocessToolHandler(
            descriptor=register_simulator(ToolRegistry()),
            entry_point="kala_agent.tools.simulator:invoke",
        )
    )
    assert command[0] == sys.executable
    assert command[-1] == "kala_agent.tools.simulator:invoke"
    assert "-I" not in command
    assert "-c" in command
    assert os.path.isabs(command[0])


def test_child_environment_keys_owned_by_the_executor_cannot_be_overridden(tmp_path: Path) -> None:
    workspace = tmp_path / "child"
    workspace.mkdir()
    for key in (
        "HOME",
        "PATH",
        "PYTHONPATH",
        "PYTHONNOUSERSITE",
        "PYTHONSAFEPATH",
        "TMPDIR",
        "KALA_TOOL_PROTOCOL",
    ):
        with pytest.raises(ToolEnvelopeError, match="protected"):
            build_child_environment(workspace=workspace, extra={key: "/etc"})
        with pytest.raises(ValueError, match="protected"):
            SandboxProfile(extra_environment={key: "/etc"})
    with pytest.raises(ToolEnvelopeError, match="simple names"):
        build_child_environment(workspace=workspace, extra={"BAD KEY": "/etc"})
    with pytest.raises(ToolEnvelopeError, match="null-free"):
        build_child_environment(workspace=workspace, extra={"TZ": "Europe/\x00"})
    with pytest.raises(ValueError, match="non-authority keys"):
        SandboxProfile(extra_environment={"SECRET_TOKEN": "value"})
    with pytest.raises(ValueError, match="non-authority keys"):
        SandboxProfile(extra_environment={"LD_PRELOAD": "/tmp/lib.so"})
    allowed = SandboxProfile(extra_environment={"TZ": "UTC"})
    assert allowed.extra_environment == {"TZ": "UTC"}
    assert (
        build_child_environment(workspace=workspace, extra=allowed.extra_environment)["TZ"] == "UTC"
    )


def test_registry_is_immutable_and_version_aware() -> None:
    registry = ToolRegistry()
    register_simulator(registry)
    assert registry.tool_ids() == (SIMULATOR_TOOL_ID,)
    assert registry.versions(SIMULATOR_TOOL_ID) == (SIMULATOR_TOOL_VERSION,)
    assert len(registry) == 1
    tool = registry.require(SIMULATOR_TOOL_ID)
    assert tool.version == SIMULATOR_TOOL_VERSION
    assert tool.required_capabilities == tuple(sorted(SIMULATOR_CAPABILITIES))
    assert tool.verify_immutable() is tool
    with pytest.raises(DuplicateToolRegistrationError):
        register_simulator(registry)
    with pytest.raises(UnknownToolError):
        registry.require("test.absent")
    with pytest.raises(ToolVersionNotFoundError):
        registry.require(SIMULATOR_TOOL_ID, "0.0.1")
    with pytest.raises(ToolRegistryError):
        registry.require("bad id")
    with pytest.raises(ToolCapabilityError):
        registry.require_capabilities(SIMULATOR_TOOL_ID, None, ("design.read",))
    assert registry.require_capabilities(SIMULATOR_TOOL_ID, None, SIMULATOR_CAPABILITIES) is tool
    assert registry.tools_requiring("artifact.write") == (tool,)
    assert registry.required_capabilities() == tuple(sorted(SIMULATOR_CAPABILITIES))
    assert (SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION) in registry
    assert (SIMULATOR_TOOL_ID, "0.0.0") not in registry


def test_registry_rejects_mutated_or_mismatched_schemas() -> None:
    registry = ToolRegistry()
    schema = {
        "additionalProperties": False,
        "properties": {"value": {"type": "string"}},
        "required": ["value"],
        "type": "object",
    }
    handler = _EchoHandler()
    registered = registry.register(
        handler.descriptor,
        handler,
        input_schema=_ECHO_INPUT,
        output_schema=_ECHO_OUTPUT,
    )
    assert registered.input_schema_digest == canonical_json_digest(_ECHO_INPUT)
    with pytest.raises(TypeError):
        registered.input_schema["properties"]["injected"] = {"type": "string"}  # type: ignore[index]
    with pytest.raises(TypeError):
        registered.input_schema["properties"] = {}  # type: ignore[index]
    with pytest.raises(SchemaDigestMismatchError):
        RegisteredTool(
            descriptor=handler.descriptor,
            handler=handler,
            input_schema=schema,
            output_schema=_ECHO_OUTPUT,
            input_schema_digest=sha256_hex(b"wrong"),
            output_schema_digest=handler.descriptor.output_schema_digest,
        )
    with pytest.raises(ToolRegistrationError):
        RegisteredTool(
            descriptor=handler.descriptor,
            handler="not a handler",  # type: ignore[arg-type]
            input_schema=_ECHO_INPUT,
            output_schema=_ECHO_OUTPUT,
            input_schema_digest=handler.descriptor.input_schema_digest,
            output_schema_digest=handler.descriptor.output_schema_digest,
        )
    with pytest.raises(ToolRegistrationError):
        RegisteredTool(
            descriptor=handler.descriptor,
            handler=handler,
            input_schema=_ECHO_INPUT,
            output_schema=_ECHO_OUTPUT,
            input_schema_digest=handler.descriptor.input_schema_digest,
            output_schema_digest=handler.descriptor.output_schema_digest,
        ).__class__(
            descriptor=handler.descriptor,
            handler=handler,
            input_schema="not a mapping",  # type: ignore[arg-type]
            output_schema=_ECHO_OUTPUT,
            input_schema_digest=handler.descriptor.input_schema_digest,
            output_schema_digest=handler.descriptor.output_schema_digest,
        )
    with pytest.raises(ToolRegistrationError):
        registry.register(
            handler.descriptor,
            object(),  # type: ignore[arg-type]
            input_schema=_ECHO_INPUT,
            output_schema=_ECHO_OUTPUT,
        )
    with pytest.raises(ToolRegistrationError):
        registry.register(
            handler.descriptor,
            handler,
            input_schema={"type": "object"},
            output_schema=_ECHO_OUTPUT,
        )
    with pytest.raises(ToolRegistrationError):
        registry.register(
            handler.descriptor,
            handler,
            input_schema={"additionalProperties": True, "properties": {"a": {}}, "type": "object"},
            output_schema=_ECHO_OUTPUT,
        )
    with pytest.raises(ToolRegistrationError):
        registry.build(
            handler.descriptor.model_copy(update={"side_effect_level": SideEffectLevel.NETWORK}),
            SubprocessToolHandler(
                descriptor=handler.descriptor.model_copy(
                    update={"side_effect_level": SideEffectLevel.NETWORK}
                ),
                entry_point=SLOW_ENTRY_POINT,
            ),
            input_schema=_ECHO_INPUT,
            output_schema=_ECHO_OUTPUT,
        )
    assert len(registry) == 1


def test_schema_validation_rejects_unsafe_documents() -> None:
    with pytest.raises(Exception, match="not permitted"):
        strict_json_loads('{"a": 1, "a": 2}')
    with pytest.raises(Exception, match="not permitted"):
        strict_json_loads('{"a": NaN}')
    with pytest.raises(Exception, match="bounded size"):
        strict_json_loads("x" * 2_000_000)
    with pytest.raises(Exception, match="exceeds the supported depth"):
        strict_json_loads("[" * 200 + "]" * 200)
    with pytest.raises(Exception, match="unsupported schema keyword"):
        validate_against_schema({"a": 1}, {"type": "object", "patternProperties": {}})
    with pytest.raises(Exception, match="unpermitted properties"):
        validate_against_schema(
            {"a": 1, "b": 2},
            {
                "additionalProperties": False,
                "properties": {"a": {"type": "integer"}},
                "type": "object",
            },
        )
    with pytest.raises(Exception, match="does not match any anyOf"):
        validate_against_schema("text", {"anyOf": [{"type": "integer"}]})
    with pytest.raises(Exception, match="minimum"):
        validate_against_schema(1, {"minimum": 2})


def test_isolation_reports_expose_no_secrets(tmp_path: Path) -> None:
    executor, policy = _approved_executor(tmp_path)
    call = _call()
    _approve(policy, executor, call)
    executor.execute(call)
    report = executor.reports[0]
    assert isinstance(report, ToolExecutionReport)
    payload = report.to_payload()
    assert payload["status"] == "succeeded"
    assert payload["sandbox_profile"] == "portable-minimum"
    assert "SECRET" not in str(payload)
    assert set(payload) == {
        "artifact_bytes",
        "call_id",
        "enforced_controls",
        "envelope_digest",
        "error_code",
        "granted_capabilities",
        "isolation",
        "process_id",
        "sandbox_profile",
        "status",
        "tool_id",
        "tool_version",
    }
    assert len(cast(str, payload["envelope_digest"])) == 64
    assert "chroot_workspace_only" not in cast("list[str]", payload["enforced_controls"])


def test_ledger_accounting_bounds_tool_calls(tmp_path: Path) -> None:
    ledger = BudgetLedger(RunBudget(max_tool_calls=1))
    executor, policy = _approved_executor(tmp_path, ledger=ledger)
    call = _call(call_id="call-budget-1")
    _approve(policy, executor, call)
    first = executor.execute(call)
    assert first.status is ToolRunStatus.SUCCEEDED
    assert ledger.usage.tool_calls == 1
    assert ledger.usage.duration_seconds > 0.0
    second_call = _call(call_id="call-budget-2")
    _approve(policy, executor, second_call)
    second = executor.execute(second_call)
    assert second.status is ToolRunStatus.DENIED
    assert second.error_code == str(ToolErrorCode.BUDGET_EXHAUSTED)


def test_simulator_handler_and_contracts_agree() -> None:
    descriptor = register_simulator(ToolRegistry())
    handler = SimulatorToolHandler()
    assert handler.descriptor == descriptor
    assert descriptor.input_schema_digest == canonical_json_digest(INPUT_SCHEMA)
    assert descriptor.output_schema_digest == canonical_json_digest(OUTPUT_SCHEMA)
    assert descriptor.side_effect_level is SideEffectLevel.LOCAL_WRITE
    assert descriptor.destructive is False
    assert descriptor.supports_structured_output is True
    assert descriptor.timeout_seconds > 0.0
    assert "mechanical" in descriptor.tool_id
    assert compute_metrics(parse_parameters({}))["within_limits"] is True
    assert compute_metrics(parse_parameters({}))["slenderness_ratio"] == 12.0
    with pytest.raises(Exception, match="operation must be one of"):
        parse_parameters({"operation": "explode"})
    with pytest.raises(Exception, match="length_mm must be"):
        parse_parameters({"length_mm": 0.5})


def test_tools_package_does_not_import_the_network() -> None:
    forbidden = ("socket", "http.client", "urllib.request", "requests", "httpx")
    for path in sorted(TOOLS_ROOT.glob("*.py")):
        source = path.read_text(encoding="utf-8")
        for token in forbidden:
            assert f"import {token}" not in source, f"{path.name} imports {token}"


def test_tool_result_and_result_envelope_stay_consistent(tmp_path: Path) -> None:
    executor, policy = _approved_executor(tmp_path)
    call = _call()
    _approve(policy, executor, call)
    result: ToolResult = executor.execute(call)
    assert result.call_id == call.call_id
    assert result.tool_id == call.tool_id
    assert result.started_at is not None
    assert result.finished_at is not None
    assert result.finished_at >= result.started_at
    report = executor.reports[0]
    assert report.call_id == result.call_id
    assert report.tool_version == SIMULATOR_TOOL_VERSION
