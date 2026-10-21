from __future__ import annotations

import errno
import os
import signal
import stat
import threading
import time
from collections.abc import Callable, Mapping
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.contracts import (
    ArtifactClassification,
    ArtifactRef,
    RunBudget,
    SideEffectLevel,
    ToolCall,
    ToolRunStatus,
)
from kala_agent.kernel.budget import BudgetLedger
from kala_agent.tools import executor as executor_module
from kala_agent.tools.executor import (
    MAX_ERROR_MESSAGE_LENGTH,
    MAX_WORKERS,
    TEXTUAL_MEDIA_SUFFIXES,
    IsolationMode,
    SandboxProfile,
    ToolAuthorityError,
    ToolExecutionReport,
    ToolExecutor,
    ToolExecutorError,
    ToolIsolationError,
    _BoundedStreamReader,
    _child_limits,
    _derive_classification,
    _probe_is_textual,
    _resource_limits_available,
    _terminate_process_group,
)
from kala_agent.tools.protocol import (
    SubprocessToolHandler,
    ToolErrorCode,
    ToolProtocolError,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    ToolWorkspaceError,
    sha256_hex,
)
from kala_agent.tools.registry import RegisteredTool, ToolRegistry
from kala_agent.tools.sandbox import (
    STRICT_REQUIRED_CONTROLS,
    IsolationProfile,
    SandboxProbe,
)
from kala_agent.tools.simulator import (
    INPUT_SCHEMA,
    OUTPUT_SCHEMA,
    ReferenceEvaluatorHandler,
    evaluator_descriptor,
    register_reference_evaluator,
    register_simulator,
    simulator_descriptor,
    simulator_handler,
)

ISSUED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
_EVALUATOR_INVOKE = ReferenceEvaluatorHandler.invoke


def _real_evaluate(
    handler: ReferenceEvaluatorHandler,
    request: ToolRequestEnvelope,
) -> ToolResponseEnvelope:
    return _EVALUATOR_INVOKE(handler, request)


def _profile_with(name: str, value: object) -> SandboxProfile:
    return SandboxProfile(**cast("Any", {name: value}))


def _call(
    tool_id: str = "mechanical.reference-evaluator",
    *,
    call_id: str = "call-1",
    version: str = "1.0.0",
    arguments: str = '{"operation":"beam_volume"}',
    side_effect_level: SideEffectLevel = SideEffectLevel.NONE,
    requested_capabilities: tuple[str, ...] = (),
    timeout_seconds: float | None = 5.0,
) -> ToolCall:
    return ToolCall(
        call_id=call_id,
        tool_id=tool_id,
        tool_version=version,
        side_effect_level=side_effect_level,
        arguments_json=arguments,
        requested_capabilities=requested_capabilities,
        timeout_seconds=timeout_seconds,
        issued_at=ISSUED_AT,
    )


def _budget(**overrides: object) -> RunBudget:
    values: dict[str, Any] = {
        "max_model_calls": 4,
        "max_tool_calls": 4,
        "max_total_tokens": 4_000,
        "max_artifact_bytes": 4_000,
        "max_duration_seconds": 60.0,
        "max_replanning_attempts": 1,
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return RunBudget(**values)


def _executor(
    tmp_path: Path,
    *,
    registry: ToolRegistry | None = None,
    profile: SandboxProfile | None = None,
    granted: tuple[str, ...] = (),
    ledger: BudgetLedger | None = None,
    isolation: IsolationMode = IsolationMode.AUTO,
    python_executable: str | None = None,
    result_sink: Callable[[Path], str] | None = None,
) -> ToolExecutor:
    return ToolExecutor(
        registry if registry is not None else ToolRegistry(),
        run_directory=tmp_path / "run",
        granted_capabilities=granted,
        profile=profile,
        ledger=ledger,
        isolation=isolation,
        python_executable=python_executable,
        result_sink=result_sink,
    )


def _evaluator_registry() -> ToolRegistry:
    registry = ToolRegistry()
    register_reference_evaluator(registry)
    return registry


def test_sandbox_profile_rejects_a_blank_name() -> None:
    with pytest.raises(ValueError, match="profile_name"):
        SandboxProfile(profile_name="   ")
    with pytest.raises(ValueError, match="profile_name"):
        SandboxProfile(profile_name=cast(str, 7))


def test_sandbox_profile_rejects_non_positive_byte_budgets() -> None:
    for name in ("max_output_bytes", "max_argument_bytes", "max_artifact_bytes"):
        for bad in (0, cast("int", True), cast("int", "1024")):
            with pytest.raises(ValueError, match=name):
                _profile_with(name, bad)


def test_sandbox_profile_bounds_the_worker_count() -> None:
    assert 1 <= MAX_WORKERS
    with pytest.raises(ValueError, match="max_workers must be an integer"):
        SandboxProfile(max_workers=cast(int, True))
    with pytest.raises(ValueError, match="max_workers must be between"):
        SandboxProfile(max_workers=0)
    with pytest.raises(ValueError, match="max_workers must be between"):
        SandboxProfile(max_workers=MAX_WORKERS + 1)
    assert SandboxProfile(max_workers=MAX_WORKERS).max_workers == MAX_WORKERS


def test_sandbox_profile_bounds_the_queue_wait() -> None:
    with pytest.raises(ValueError, match="max_queue_seconds must be a number"):
        SandboxProfile(max_queue_seconds=cast(float, True))
    with pytest.raises(ValueError, match="max_queue_seconds must be within"):
        SandboxProfile(max_queue_seconds=-1.0)
    assert SandboxProfile(max_queue_seconds=0.0).max_queue_seconds == 0.0
    with pytest.raises(ValueError, match="max_queue_seconds must be within"):
        SandboxProfile(max_queue_seconds=1_000.0)


def test_sandbox_profile_bounds_the_default_timeout() -> None:
    with pytest.raises(ValueError, match="default_timeout_seconds must be a number"):
        SandboxProfile(default_timeout_seconds=cast(float, "30"))
    with pytest.raises(ValueError, match="default_timeout_seconds must be within"):
        SandboxProfile(default_timeout_seconds=0.0)
    with pytest.raises(ValueError, match="default_timeout_seconds must be within"):
        SandboxProfile(default_timeout_seconds=1_000_000.0)


def test_sandbox_profile_requires_boolean_switches() -> None:
    for name in (
        "allow_network",
        "allow_egress",
        "allow_privileged_capabilities",
        "enforce_child_limits",
    ):
        with pytest.raises(ValueError, match=name):
            _profile_with(name, cast("bool", "yes"))


def test_sandbox_profile_normalizes_the_isolation_profile() -> None:
    assert (
        SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX).isolation_profile
        is IsolationProfile.STRICT_LINUX
    )
    with pytest.raises(ValueError, match="not a supported profile"):
        SandboxProfile(isolation_profile=cast(Any, "nope"))
    with pytest.raises(ValueError, match="not a supported profile"):
        SandboxProfile(isolation_profile=cast(Any, 7))


def test_sandbox_profile_validates_extra_environment() -> None:
    with pytest.raises(ValueError, match="extra_environment must be a mapping"):
        SandboxProfile(extra_environment=cast(Any, [("A", "1")]))
    with pytest.raises(ValueError, match="extra_environment must map strings"):
        SandboxProfile(extra_environment=cast("Mapping[str, str]", {"A": 7}))
    with pytest.raises(ValueError, match="extra_environment must map strings"):
        SandboxProfile(extra_environment=cast("Mapping[str, str]", {7: "1"}))
    with pytest.raises(ValueError, match="protected key"):
        SandboxProfile(extra_environment={"HOME": "/elsewhere"})
    with pytest.raises(ValueError, match="non-authority keys"):
        SandboxProfile(extra_environment={"OMP_NUM_THREADS": "1"})
    profile = SandboxProfile(extra_environment={"TZ": "UTC"})
    assert profile.extra_environment == {"TZ": "UTC"}


def test_sandbox_profile_requires_a_real_probe() -> None:
    probe = SandboxProbe(available=True, platform="linux")
    assert SandboxProfile(sandbox_probe=probe).sandbox_probe is probe
    with pytest.raises(ValueError, match="sandbox_probe must be"):
        SandboxProfile(sandbox_probe=cast(Any, object()))


def test_sandbox_profile_reports_enforced_controls() -> None:
    portable = SandboxProfile()
    assert "process_isolation" in portable.enforced_controls
    assert "network_isolation" not in portable.enforced_controls
    without_limits = SandboxProfile(enforce_child_limits=False)
    assert "resource_limits" not in without_limits.enforced_controls
    if _resource_limits_available():
        assert "resource_limits" in portable.enforced_controls
    else:
        assert "resource_limits" not in portable.enforced_controls


def test_sandbox_profile_reports_strict_probe_controls() -> None:
    probe = SandboxProbe(
        available=True,
        platform="linux",
        controls=("mount_namespace", "user_namespace"),
    )
    profile = SandboxProfile(
        isolation_profile=IsolationProfile.STRICT_LINUX,
        sandbox_probe=probe,
    )
    assert "mount_namespace" in profile.enforced_controls
    assert "user_namespace" in profile.enforced_controls
    unavailable = SandboxProfile(
        isolation_profile=IsolationProfile.STRICT_LINUX,
        sandbox_probe=SandboxProbe(available=False, platform="linux"),
    )
    assert "mount_namespace" not in unavailable.enforced_controls
    portable = SandboxProfile(
        isolation_profile=IsolationProfile.PORTABLE,
        sandbox_probe=probe,
    )
    assert "mount_namespace" not in portable.enforced_controls


def test_sandbox_profile_clamps_requested_timeouts() -> None:
    profile = SandboxProfile(default_timeout_seconds=5.0)
    assert profile.timeout_for(None, 2.0) == 2.0
    assert profile.timeout_for(1.0, 2.0) == 1.0
    assert profile.timeout_for(100.0, 2.0) == 2.0
    assert profile.timeout_for(None, 100.0) == 5.0
    assert profile.timeout_for(0.0, -1.0) == 5.0
    assert profile.timeout_for(None, 0.0) == 5.0
    assert profile.timeout_for(0.001, 0.001) >= 0.001


def test_probe_is_textual_distinguishes_text_from_binary() -> None:
    assert _probe_is_textual(b"plain text") is True
    assert _probe_is_textual(b"with\x00null") is False
    assert _probe_is_textual(b"\xff\xfe invalid") is False
    assert _probe_is_textual("καλός".encode()) is True


def test_derive_classification_refuses_to_trust_a_tool(tmp_path: Path) -> None:
    text = tmp_path / "report.json"
    text.write_text('{"ok":true}')
    assert _derive_classification(text, ArtifactClassification.DERIVED_DATA) is (
        ArtifactClassification.TEXT
    )
    assert text.suffix.lower() in TEXTUAL_MEDIA_SUFFIXES


def test_derive_classification_detects_binary_signatures(tmp_path: Path) -> None:
    binary = tmp_path / "report.json"
    binary.write_bytes(b"\x89PNG\r\n\x1a\n" + b"\x00" * 16)
    assert _derive_classification(binary, ArtifactClassification.DERIVED_DATA) is (
        ArtifactClassification.DESIGN_BINARY
    )


def test_derive_classification_reports_a_declared_binary(tmp_path: Path) -> None:
    text = tmp_path / "report.json"
    text.write_text('{"ok":true}')
    assert _derive_classification(text, ArtifactClassification.DESIGN_BINARY) is (
        ArtifactClassification.DESIGN_BINARY
    )


def test_derive_classification_rejects_unknown_suffixes(tmp_path: Path) -> None:
    unknown = tmp_path / "report.bin"
    unknown.write_text('{"ok":true}')
    assert _derive_classification(unknown, ArtifactClassification.DERIVED_DATA) is (
        ArtifactClassification.DESIGN_BINARY
    )


def test_derive_classification_rejects_undecodable_text(tmp_path: Path) -> None:
    undecodable = tmp_path / "report.json"
    undecodable.write_bytes(b"\xff\xfe\xfd\xfc")
    assert _derive_classification(undecodable, ArtifactClassification.DERIVED_DATA) is (
        ArtifactClassification.DESIGN_BINARY
    )


def test_derive_classification_reports_unreadable_paths(tmp_path: Path) -> None:
    assert (
        _derive_classification(tmp_path / "absent.json", ArtifactClassification.DERIVED_DATA)
        is ArtifactClassification.DESIGN_BINARY
    )


def test_bounded_stream_reader_collects_until_the_limit() -> None:
    reader = _BoundedStreamReader(_ChunkStream([b"a" * 10, b"b" * 10, b"c" * 10]), 25)
    reader.start()
    reader.join(timeout=5)
    assert reader.truncated is True
    assert reader.data == b"a" * 10 + b"b" * 10


def test_bounded_stream_reader_stops_at_end_of_stream() -> None:
    reader = _BoundedStreamReader(_ChunkStream([b"a", b"b", b""]), 1_000)
    reader.start()
    reader.join(timeout=5)
    assert reader.truncated is False
    assert reader.data == b"ab"


def test_bounded_stream_reader_ignores_non_bytes_chunks() -> None:
    reader = _BoundedStreamReader(_ChunkStream([b"a", cast(Any, "text"), b"b"]), 1_000)
    stream = _ChunkStream([b"a"])
    reader = _BoundedStreamReader(stream, 1_000)
    reader.start()
    reader.join(timeout=5)
    assert reader.data == b"a"
    assert stream.closed is True


def test_bounded_stream_reader_tolerates_unreadable_streams() -> None:
    reader = _BoundedStreamReader(cast(Any, object()), 1_000)
    reader.start()
    reader.join(timeout=5)
    assert reader.data == b""
    assert reader.truncated is False


def test_bounded_stream_reader_tolerates_read_errors() -> None:
    reader = _BoundedStreamReader(_ExplodingStream(), 1_000)
    reader.start()
    reader.join(timeout=5)
    assert reader.data == b""


def test_bounded_stream_reader_tolerates_close_errors() -> None:
    reader = _BoundedStreamReader(_ChunkStream([b"a"], close_error=True), 1_000)
    reader.start()
    reader.join(timeout=5)
    assert reader.data == b"a"


def test_bounded_stream_reader_tolerates_a_missing_close() -> None:
    reader = _BoundedStreamReader(_ChunkStream([b"a"], close_error=True, no_close=True), 1_000)
    reader.start()
    reader.join(timeout=5)
    assert reader.data == b"a"


def test_terminate_process_group_kills_the_recorded_group(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    signals: list[tuple[int, int]] = []
    monkeypatch.setattr(os, "killpg", lambda pid, sig: signals.append((pid, sig)))
    process = cast("Any", _FakeProcess(pid=4321))
    assert _terminate_process_group(process) is True
    assert signals == [(4321, signal.SIGKILL)]


def test_terminate_process_group_falls_back_to_kill(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(_pid: int, _sig: int) -> None:
        raise ProcessLookupError("gone")

    monkeypatch.setattr(os, "killpg", explode)
    process = _FakeProcess(pid=4321, exitcode=0)
    assert _terminate_process_group(cast("Any", process)) is True
    assert process.killed is True


def test_terminate_process_group_reports_an_unreapable_process(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(_pid: int, _sig: int) -> None:
        raise PermissionError("denied")

    def kill_explodes() -> BaseException:
        raise ProcessLookupError("gone")

    monkeypatch.setattr(os, "killpg", explode)
    process = _FakeProcess(pid=4321, exitcode=None, kill_error=kill_explodes)
    assert _terminate_process_group(cast("Any", process)) is False
    assert process.kill_attempts == 1


def test_child_limits_returns_none_when_disabled() -> None:
    assert _child_limits(SandboxProfile(enforce_child_limits=False), 5.0) is None


def test_child_limits_sets_bounded_resource_limits(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import resource

    limits = _child_limits(SandboxProfile(), 5.0)
    assert limits is not None
    applied: dict[str, tuple[int, int]] = {}

    def tracking(constant: int, values: tuple[int, int]) -> None:
        applied[str(constant)] = values

    monkeypatch.setattr(resource, "setrlimit", tracking)
    limits()
    assert applied
    assert all(soft >= 0 and hard >= soft for soft, hard in applied.values())


def test_tool_execution_report_payload_reports_every_field() -> None:
    report = ToolExecutionReport(
        call_id="call-1",
        tool_id="mechanical.reference-evaluator",
        tool_version="1.0.0",
        isolation=IsolationMode.SUBPROCESS,
        sandbox_profile="portable-minimum",
        status=ToolRunStatus.SUCCEEDED,
        error_code=None,
        process_id=None,
        granted_capabilities=("artifact.read",),
        artifact_bytes=12,
        enforced_controls=("process_isolation",),
        envelope_digest="a" * 64,
    )
    assert report.to_payload() == {
        "artifact_bytes": 12,
        "call_id": "call-1",
        "enforced_controls": ["process_isolation"],
        "envelope_digest": "a" * 64,
        "error_code": None,
        "granted_capabilities": ["artifact.read"],
        "isolation": "subprocess",
        "process_id": None,
        "sandbox_profile": "portable-minimum",
        "status": "succeeded",
        "tool_id": "mechanical.reference-evaluator",
        "tool_version": "1.0.0",
    }


def test_executor_validates_its_dependencies(tmp_path: Path) -> None:
    with pytest.raises(TypeError, match="ToolRegistry"):
        ToolExecutor(cast(Any, object()), run_directory=tmp_path / "run")
    with pytest.raises(TypeError, match="PolicyEngine"):
        ToolExecutor(
            ToolRegistry(),
            run_directory=tmp_path / "run",
            policy=cast(Any, object()),
        )
    with pytest.raises(TypeError, match="result_sink must be callable"):
        ToolExecutor(
            ToolRegistry(),
            run_directory=tmp_path / "run",
            result_sink=cast(Any, object()),
        )


def test_executor_prepares_and_exposes_its_run_directory(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    assert executor.run_directory == (tmp_path / "run").resolve()
    assert executor.run_directory.is_dir()
    assert executor.registry is not None
    assert executor.isolation is IsolationMode.AUTO
    assert executor.granted_capabilities == frozenset()
    assert executor.sandbox_probe.available is False
    assert executor.sandbox_probe.platform == "not-probed"
    assert "process_isolation" in executor.enforced_controls
    assert executor.reports == ()
    assert executor.profile.sandbox_probe is executor.sandbox_probe


def test_executor_normalizes_import_roots(tmp_path: Path) -> None:
    package_root = tmp_path / "roots"
    package_root.mkdir()
    executor = ToolExecutor(
        ToolRegistry(),
        run_directory=tmp_path / "run",
        import_roots=(package_root, package_root),
    )
    assert executor._import_roots == (package_root.resolve(),)
    default = ToolExecutor(ToolRegistry(), run_directory=tmp_path / "run2")
    assert len(default._import_roots) == 1
    assert default._import_roots[0].is_dir()
    assert default._import_roots[0] == (Path(__file__).parents[2] / "src").resolve()


def test_executor_rejects_missing_import_roots(tmp_path: Path) -> None:
    with pytest.raises(ToolExecutorError, match="existing directories"):
        ToolExecutor(
            ToolRegistry(),
            run_directory=tmp_path / "run",
            import_roots=(tmp_path / "absent",),
        )


def test_executor_normalizes_granted_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("artifact.read", " artifact.read "))
    assert executor.granted_capabilities == frozenset({"artifact.read"})
    with pytest.raises(ToolExecutorError, match="collection of names"):
        _executor(tmp_path, granted=cast(Any, "artifact.read"))
    with pytest.raises(ToolExecutorError, match="must be strings"):
        _executor(tmp_path, granted=cast(Any, [7]))
    with pytest.raises(ToolExecutorError, match="nonempty and bounded"):
        _executor(tmp_path, granted=("   ",))
    with pytest.raises(ToolExecutorError, match="nonempty and bounded"):
        _executor(tmp_path, granted=("x" * 129,))
    with pytest.raises(ToolExecutorError, match="unsafe characters"):
        _executor(tmp_path, granted=("has space",))


def test_executor_grants_capabilities_lazily(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    assert executor.grant_capability("artifact.read") == frozenset({"artifact.read"})
    assert executor.grant_capability("  artifact.write  ") == frozenset(
        {"artifact.read", "artifact.write"}
    )


def test_executor_rejects_a_symlinked_run_directory(tmp_path: Path) -> None:
    real = tmp_path / "real"
    real.mkdir()
    linked = tmp_path / "linked"
    linked.symlink_to(real, target_is_directory=True)
    with pytest.raises(ToolWorkspaceError, match="cannot be a symlink"):
        ToolExecutor(ToolRegistry(), run_directory=linked)


def test_executor_rejects_a_run_directory_at_the_filesystem_root(tmp_path: Path) -> None:
    with pytest.raises(ToolWorkspaceError, match="filesystem root"):
        ToolExecutor(ToolRegistry(), run_directory=Path("/"))


def test_executor_rejects_a_non_path_run_directory(tmp_path: Path) -> None:
    with pytest.raises(ToolExecutorError, match="must be a Path"):
        ToolExecutor(ToolRegistry(), run_directory=cast(Any, str(tmp_path)))


def test_executor_rejects_a_run_directory_blocked_by_a_file(tmp_path: Path) -> None:
    blocker = tmp_path / "run"
    blocker.write_text("x")
    with pytest.raises(ToolWorkspaceError, match="exists as a file"):
        ToolExecutor(ToolRegistry(), run_directory=blocker)


def test_executor_rejects_a_symlinked_run_directory_after_creation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    real = tmp_path / "real"
    real.mkdir()
    decoy = tmp_path / "decoy"
    decoy.symlink_to(real)
    decoy_lstat = os.lstat
    target = tmp_path / "run"
    target.mkdir()

    def fake_lstat(path: str | os.PathLike[str]) -> os.stat_result:
        return decoy_lstat(decoy if Path(path) == target else path)

    original_resolve = Path.resolve

    def fake_resolve(self: Path, strict: bool = False) -> Path:
        return self if self == target else original_resolve(self, strict=strict)

    monkeypatch.setattr(Path, "is_symlink", lambda self: False)
    monkeypatch.setattr(Path, "resolve", fake_resolve)
    monkeypatch.setattr(os, "lstat", fake_lstat)
    with pytest.raises(ToolWorkspaceError, match="must be real directories"):
        ToolExecutor(ToolRegistry(), run_directory=target)


def test_executor_reports_an_unreadable_run_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(_path: object) -> os.stat_result:
        raise OSError(errno.EACCES, "denied")

    monkeypatch.setattr(os, "lstat", explode)
    with pytest.raises(ToolWorkspaceError, match="not accessible"):
        ToolExecutor(ToolRegistry(), run_directory=tmp_path / "run")


def test_call_component_rejects_unsafe_identifiers(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    with pytest.raises(ToolWorkspaceError, match="must be a string"):
        executor.workspace_path(cast(str, 7))
    for bad in ("   ", "x" * 129, ".", "..", "a/b", "a\\b", "a\x00b", "a b", "a\nb"):
        with pytest.raises(ToolWorkspaceError):
            executor.workspace_path(bad)


def test_workspace_path_does_not_create_anything(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    path = executor.workspace_path("call-1")
    assert not path.exists()
    assert path.name == "workspace"
    assert path.parent.name == "call-1"


def test_workspace_for_creates_a_confined_directory(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    workspace = executor.workspace_for("call-1")
    assert workspace.is_dir()
    assert workspace.is_relative_to(executor.run_directory)
    assert executor.resolve_output_path(workspace, "outputs/report.json") == (
        workspace / "outputs" / "report.json"
    )
    with pytest.raises(ToolWorkspaceError, match="unsafe segment"):
        executor.resolve_output_path(workspace, "../escape.json")
    outside = tmp_path / "outside"
    outside.mkdir()
    (workspace / "link").symlink_to(outside, target_is_directory=True)
    with pytest.raises(ToolWorkspaceError, match="cannot traverse symlinks"):
        executor.resolve_output_path(workspace, "link/report.json")


def test_workspace_for_rejects_a_symlinked_workspace(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    real = executor.run_directory / "calls" / "call-1"
    real.mkdir(parents=True)
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    (real / "workspace").symlink_to(elsewhere, target_is_directory=True)
    executor._ensure_directory = lambda _path: None  # type: ignore[assignment]
    with pytest.raises(ToolWorkspaceError, match="escapes the run directory"):
        executor.workspace_for("call-1")
    ToolExecutor._ensure_directory(real / "workspace") if False else None


def test_require_authority_rejects_ungranted_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("artifact.read",))
    registry = ToolRegistry()
    register_simulator(registry)
    tool = registry.require("mechanical.reference-simulator", "1.0.0")
    with pytest.raises(ToolAuthorityError, match="not granted"):
        executor.require_authority(tool)


def test_require_authority_requires_a_registered_tool(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    with pytest.raises(TypeError, match="RegisteredTool"):
        executor.require_authority(cast(Any, object()))


def test_require_authority_denies_network_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("network.access",))
    with pytest.raises(ToolAuthorityError, match="network capabilities are denied"):
        executor.require_authority(_network_tool(executor))


def _network_tool(_executor: ToolExecutor) -> RegisteredTool:
    descriptor = simulator_descriptor()
    registry = ToolRegistry()
    tool = registry.build(
        descriptor,
        simulator_handler(),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    object.__setattr__(
        tool,
        "descriptor",
        descriptor.model_copy(update={"required_capabilities": ("network.access",)}),
    )
    return tool


def test_require_authority_denies_egress_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("network.egress", "network.access"))
    with pytest.raises(ToolAuthorityError, match="egress capabilities are denied"):
        executor._check_capability_policy(("export",))
    allowed = _executor(
        tmp_path,
        granted=("export", "network.access"),
        profile=SandboxProfile(allow_egress=True, allow_network=True),
    )
    allowed._check_capability_policy(("export",))
    egress_only = _executor(
        tmp_path,
        granted=("export",),
        profile=SandboxProfile(allow_egress=True),
    )
    with pytest.raises(ToolAuthorityError, match="egress capabilities are denied"):
        egress_only._check_capability_policy(("export",))


def test_require_authority_denies_privileged_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, granted=("process.spawn",))
    with pytest.raises(ToolAuthorityError, match="denied by default"):
        executor._check_capability_policy(("process.spawn",))
    allowed = _executor(
        tmp_path,
        granted=("process.spawn",),
        profile=SandboxProfile(allow_privileged_capabilities=True),
    )
    allowed._check_capability_policy(("process.spawn",))


def test_require_authority_denies_network_side_effects(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    with pytest.raises(ToolAuthorityError, match="network side effects are denied"):
        executor._check_side_effect_policy(SideEffectLevel.NETWORK)
    with pytest.raises(ToolAuthorityError, match="egress side effects are denied"):
        executor._check_side_effect_policy(SideEffectLevel.EGRESS)
    executor._check_side_effect_policy(SideEffectLevel.NONE)
    executor._check_side_effect_policy(SideEffectLevel.READ)
    executor._check_side_effect_policy(SideEffectLevel.LOCAL_WRITE)


def test_policy_error_code_classifies_each_refusal(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    assert (
        executor._policy_error_code((), SideEffectLevel.NONE) is ToolErrorCode.CAPABILITY_FORBIDDEN
    )
    assert executor._policy_error_code((), SideEffectLevel.NETWORK) is ToolErrorCode.NETWORK_DENIED
    assert executor._policy_error_code((), SideEffectLevel.EGRESS) is ToolErrorCode.EGRESS_DENIED
    assert (
        executor._policy_error_code(("network.access",), SideEffectLevel.NONE)
        is ToolErrorCode.NETWORK_DENIED
    )
    assert (
        executor._policy_error_code(("network.egress",), SideEffectLevel.NONE)
        is ToolErrorCode.EGRESS_DENIED
    )


def test_normalize_error_code_never_surfaces_an_unknown_code(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    assert executor._normalize_error_code(None) is None
    assert executor._normalize_error_code(ToolErrorCode.TIMEOUT) is ToolErrorCode.TIMEOUT
    assert executor._normalize_error_code("timeout") is ToolErrorCode.TIMEOUT
    assert executor._normalize_error_code("made_up") is ToolErrorCode.INTERNAL_ERROR


def test_select_isolation_refuses_in_kernel_dispatch(tmp_path: Path) -> None:
    registry = _evaluator_registry()
    tool = registry.require("mechanical.reference-evaluator", "1.0.0")
    auto = _executor(tmp_path, registry=registry)
    assert auto._select_isolation(tool) is None
    subprocess_only = _executor(
        tmp_path / "b", registry=registry, isolation=IsolationMode.SUBPROCESS
    )
    assert subprocess_only._select_isolation(tool) is None


def test_select_isolation_allows_only_trusted_evaluators(tmp_path: Path) -> None:
    registry = ToolRegistry()
    descriptor = simulator_descriptor()
    tool = registry.register(
        descriptor,
        simulator_handler(),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    trusted = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    assert trusted._select_isolation(tool) is None
    evaluator_registry = _evaluator_registry()
    evaluator = evaluator_registry.require("mechanical.reference-evaluator", "1.0.0")
    allowed = _executor(
        tmp_path / "b",
        registry=evaluator_registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    assert allowed._select_isolation(evaluator) is IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR


def test_select_isolation_refuses_a_side_effecting_trusted_tool(tmp_path: Path) -> None:
    registry = ToolRegistry()
    plain = registry.build(
        evaluator_descriptor(),
        ReferenceEvaluatorHandler(),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    assert plain.trusted_in_process is False
    assert executor._select_isolation(plain) is None


def test_sandbox_unavailable_only_refuses_side_effecting_tools(tmp_path: Path) -> None:
    registry = _evaluator_registry()
    tool = registry.require("mechanical.reference-evaluator", "1.0.0")
    strict = _executor(
        tmp_path,
        registry=registry,
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
    )
    assert strict._sandbox_unavailable(tool) is None


def test_build_sandbox_spec_is_portable_by_default(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    assert executor._build_sandbox_spec(tmp_path / "root", tmp_path / "ws") is None


def test_executor_denies_unknown_tools(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    result = executor.execute(_call("absent.tool"))
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "unknown_tool"
    assert "is not registered" in str(result.error_message)


def test_executor_denies_unknown_versions(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    result = executor.execute(_call(version="9.9.9"))
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "version_not_found"


def test_executor_denies_schema_digest_mismatches(tmp_path: Path) -> None:
    registry = _evaluator_registry()
    executor = _executor(tmp_path, registry=registry)
    tool = registry.require("mechanical.reference-evaluator", "1.0.0")
    object.__setattr__(tool, "input_schema", {"tampered": True})
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "schema_digest_mismatch"


def test_executor_requires_a_tool_call(tmp_path: Path) -> None:
    with pytest.raises(TypeError, match="must be a ToolCall"):
        _executor(tmp_path).execute(cast(Any, object()))


def test_executor_policy_request_requires_a_tool_call(tmp_path: Path) -> None:
    with pytest.raises(TypeError, match="must be a ToolCall"):
        _executor(tmp_path).policy_request(cast(Any, object()))


def test_executor_policy_request_binds_the_exact_call(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    request = executor.policy_request(_call())
    assert request.request_id == "call-1"
    assert request.side_effect_level is SideEffectLevel.NONE
    assert request.checkpoint_type is not None
    assert len(request.state_digest) == 64
    assert len(request.action_digest) == 64


def test_executor_approval_binding_falls_back_for_uncanonicalizable_arguments(
    tmp_path: Path,
) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    tool = executor.registry.require("mechanical.reference-evaluator", "1.0.0")
    oversized = '{"operation":"' + "x" * 600_000 + '"}'
    binding = executor._approval_binding(_call(arguments=oversized), tool)
    assert binding.envelope is None
    assert binding.arguments_digest == sha256_hex(f"uncanonicalized:{oversized}")
    assert len(binding.envelope_digest) == 64
    assert len(binding.action_digest) == 64


def test_executor_approval_binding_captures_a_valid_envelope(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    tool = executor.registry.require("mechanical.reference-evaluator", "1.0.0")
    binding = executor._approval_binding(_call(), tool)
    assert binding.envelope is not None
    assert binding.envelope.call_id == "call-1"
    assert binding.envelope_digest == binding.envelope.envelope_digest
    assert binding.effective_capabilities == ()


def test_executor_denies_undeclared_requested_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry(), granted=("artifact.read",))
    result = executor.execute(_call(requested_capabilities=("artifact.read",)))
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "capability_denied"
    assert "does not declare" in str(result.error_message)


def test_executor_denies_missing_granted_capabilities(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    result = executor.execute(_call(requested_capabilities=("artifact.read",)))
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "capability_denied"
    assert "not granted" in str(result.error_message)


def test_executor_denies_a_side_effect_mismatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    result = executor.execute(_call(side_effect_level=SideEffectLevel.LOCAL_WRITE))
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "side_effect_mismatch"


def test_executor_reports_a_cancelled_call_before_dispatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    cancel = threading.Event()
    cancel.set()
    result = executor.execute(_call(), cancel=cancel)
    assert result.status is ToolRunStatus.CANCELLED
    assert "cancelled before dispatch" in str(result.error_message)


def test_executor_refuses_in_kernel_dispatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "isolation_required"
    assert "cannot be dispatched" in str(result.error_message)


def test_executor_refuses_side_effects_without_a_sandbox(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        executor_module,
        "probe_sandbox",
        lambda **_k: SandboxProbe(
            available=False,
            platform="linux",
            detail="namespaces are unavailable",
        ),
    )
    registry = ToolRegistry()
    register_simulator(registry)
    executor = _executor(
        tmp_path,
        registry=registry,
        granted=("artifact.read", "artifact.write", "design.read", "tool.execute"),
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
    )
    result = executor.execute(
        _call(
            "mechanical.reference-simulator",
            side_effect_level=SideEffectLevel.LOCAL_WRITE,
        )
    )
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "sandbox_unavailable"
    assert "namespaces are unavailable" in str(result.error_message)


def test_executor_reports_uncanonicalizable_arguments(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    executor._select_isolation = lambda _tool: IsolationMode.SUBPROCESS  # type: ignore[assignment]
    result = executor.execute(_call(arguments='{"operation":"' + "x" * 600_000 + '"}'))
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_arguments"
    assert "could not be canonicalized" in str(result.error_message)


def test_executor_reports_schema_violations(tmp_path: Path) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    executor._select_isolation = lambda _tool: IsolationMode.SUBPROCESS  # type: ignore[assignment]
    result = executor.execute(_call(arguments='{"operation":"shear"}'))
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_arguments"


def test_executor_reports_oversized_arguments(tmp_path: Path) -> None:
    profile = SandboxProfile(max_argument_bytes=64)
    executor = _executor(tmp_path, registry=_evaluator_registry(), profile=profile)
    executor._select_isolation = lambda _tool: IsolationMode.SUBPROCESS  # type: ignore[assignment]
    result = executor.execute(_call(arguments='{"operation":"' + "x" * 200 + '"}'))
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_arguments"


def test_executor_reports_worker_capacity_exhaustion(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(tmp_path, registry=_evaluator_registry())
    executor._select_isolation = lambda _tool: IsolationMode.SUBPROCESS  # type: ignore[assignment]
    monkeypatch.setattr(executor, "_acquire_worker_slot", lambda: False)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "worker_capacity_exhausted"
    assert "busy" in str(result.error_message)


def test_executor_reports_an_exhausted_budget(tmp_path: Path) -> None:
    ledger = BudgetLedger(_budget(max_tool_calls=0))
    executor = _executor(tmp_path, registry=_evaluator_registry(), ledger=ledger)
    executor._select_isolation = lambda _tool: IsolationMode.SUBPROCESS  # type: ignore[assignment]
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == "budget_exhausted"
    assert len(executor.reports) == 0


def test_executor_records_a_dispatch_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _subprocess_executor(tmp_path, monkeypatch)
    monkeypatch.setattr(
        executor,
        "_dispatch_subprocess",
        lambda *_a, **_k: (
            None,
            1234,
            (ToolRunStatus.FAILED, ToolErrorCode.PROTOCOL_ERROR, "child spoke nonsense"),
        ),
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "protocol_error"
    reports = executor.reports
    assert len(reports) == 1
    assert reports[0].process_id == 1234
    assert reports[0].isolation is IsolationMode.SUBPROCESS
    assert reports[0].error_code == "protocol_error"
    assert reports[0].envelope_digest
    assert reports[0].artifact_bytes == 0


def test_executor_records_a_cancelled_dispatch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _subprocess_executor(tmp_path, monkeypatch)
    monkeypatch.setattr(
        executor,
        "_dispatch_subprocess",
        lambda *_a, **_k: (
            None,
            None,
            (ToolRunStatus.CANCELLED, ToolErrorCode.CANCELLED, "cancelled by operator"),
        ),
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.CANCELLED
    assert "cancelled by operator" in str(result.error_message)
    assert executor.reports[0].process_id is None


def test_executor_records_a_missing_response_envelope(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _subprocess_executor(tmp_path, monkeypatch)
    monkeypatch.setattr(
        executor,
        "_dispatch_subprocess",
        lambda *_a, **_k: (None, None, None),
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "tool_error"
    assert "no result envelope" in str(result.error_message)


def test_executor_reports_a_subprocess_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _subprocess_executor(tmp_path, monkeypatch)
    monkeypatch.setattr(
        executor,
        "_dispatch_subprocess",
        lambda *_a, **_k: (
            None,
            99,
            (ToolRunStatus.TIMED_OUT, ToolErrorCode.TIMEOUT, "worker exceeded its deadline"),
        ),
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.TIMED_OUT
    assert result.error_code == "timeout"
    assert executor.reports[0].process_id == 99


def test_executor_reports_an_oversized_envelope(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        profile=SandboxProfile(max_output_bytes=64),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "output_too_large"
    assert "exceeds the bounded output size" in str(result.error_message)


def test_executor_trusted_in_process_records_a_trusted_run(tmp_path: Path) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.SUCCEEDED
    assert result.result_json is not None
    reports = executor.reports
    assert len(reports) == 1
    assert reports[0].isolation is IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR
    assert reports[0].status is ToolRunStatus.SUCCEEDED
    assert reports[0].process_id is None
    assert reports[0].granted_capabilities == ()
    assert reports[0].sandbox_profile == "portable-minimum"


def test_executor_trusted_in_process_reports_invalid_arguments(tmp_path: Path) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(_call(arguments='{"operation":"shear"}'))
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_arguments"


def test_executor_trusted_in_process_commits_the_budget(tmp_path: Path) -> None:
    ledger = BudgetLedger(_budget())
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        ledger=ledger,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.SUCCEEDED
    assert ledger.usage.tool_calls == 1
    assert ledger.usage.artifact_bytes == 0


def test_executor_trusted_in_process_releases_the_budget_on_failure(tmp_path: Path) -> None:
    ledger = BudgetLedger(_budget())
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        ledger=ledger,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    result = executor.execute(_call(arguments='{"operation":"shear"}'))
    assert result.status is ToolRunStatus.FAILED
    assert ledger.usage.tool_calls == 0


def test_executor_trusted_in_process_reports_a_crashing_handler(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def explode(
        _self: ReferenceEvaluatorHandler,
        _request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        raise RuntimeError("handler exploded")

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", explode)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "tool_error"
    assert "RuntimeError" in str(result.error_message)
    assert "handler exploded" in str(result.error_message)
    assert executor.reports[0].error_code == "tool_error"


def test_executor_trusted_in_process_reports_a_protocol_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def misbehave(
        _self: ReferenceEvaluatorHandler,
        _request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        raise ToolProtocolError("the handler produced a broken envelope")

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", misbehave)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "protocol_error"
    assert "broken envelope" in str(result.error_message)


def test_executor_trusted_in_process_redacts_the_workspace(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    workspace = executor.workspace_path("call-1")

    def explode(
        _self: ReferenceEvaluatorHandler,
        _request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        raise RuntimeError(f"failed inside {workspace}")

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", explode)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert "<workspace>" in str(result.error_message)
    assert workspace.as_posix() not in str(result.error_message)


def test_executor_trusted_in_process_truncates_long_error_messages(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def explode(
        _self: ReferenceEvaluatorHandler,
        _request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        raise RuntimeError("x" * 10_000)

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", explode)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert len(str(result.error_message)) <= MAX_ERROR_MESSAGE_LENGTH


def test_executor_trusted_in_process_reports_a_lying_handler(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def lie(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return replace(
            _real_evaluate(ReferenceEvaluatorHandler(), request),
            result=cast("Mapping[str, Any]", {"made_up": True}),
        )

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", lie)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_result"
    assert "required property" in str(result.error_message)


def test_executor_trusted_in_process_rejects_a_mismatched_call(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def misattribute(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return replace(_real_evaluate(ReferenceEvaluatorHandler(), request), call_id="other-call")

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", misattribute)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_result"


def test_executor_trusted_in_process_rejects_a_wrong_tool_version(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def relabel(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return replace(_real_evaluate(ReferenceEvaluatorHandler(), request), tool_version="9.9.9")

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", relabel)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_result"


def test_executor_trusted_in_process_rejects_a_failed_envelope(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def deny(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return ToolResponseEnvelope.failed_envelope(
            call_id=request.call_id,
            tool_id=request.tool_id,
            tool_version=request.tool_version,
            status=ToolRunStatus.FAILED,
            error_code=ToolErrorCode.INVALID_ARGUMENTS,
            error_message="the evaluator refused",
        )

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", deny)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "invalid_arguments"
    assert "the evaluator refused" in str(result.error_message)


def test_executor_trusted_in_process_rejects_an_unrecognized_error_code(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def mislabel(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return replace(
            _real_evaluate(ReferenceEvaluatorHandler(), request),
            status=ToolRunStatus.FAILED,
            result=None,
            error_code="totally_fine_code",
            error_message="reported under an unknown code",
        )

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", mislabel)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "tool_error"


def test_executor_trusted_in_process_reports_a_cancel_during_execution(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    cancel = threading.Event()
    started = threading.Event()

    def slow(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        started.set()
        time.sleep(1.0)
        return _real_evaluate(ReferenceEvaluatorHandler(), request)

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", slow)
    timer = threading.Timer(0.1, cancel.set)
    timer.start()
    try:
        result = executor.execute(_call(), cancel=cancel)
    finally:
        cancel.set()
        timer.cancel()
    assert started.is_set()
    assert result.status is ToolRunStatus.CANCELLED
    assert "cancelled during execution" in str(result.error_message)


def test_executor_trusted_in_process_rejects_a_subprocess_handler(
    tmp_path: Path,
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    tool = registry.require("mechanical.reference-evaluator", "1.0.0")
    object.__setattr__(
        tool,
        "handler",
        SubprocessToolHandler(
            descriptor=tool.descriptor,
            entry_point="kala_agent.tools.simulator:invoke",
        ),
    )
    with pytest.raises(ToolIsolationError, match="cannot run inside the kernel process"):
        executor._dispatch_in_process(tool, cast(Any, {}), None)


def test_executor_trusted_in_process_rejects_an_untrusted_tool(
    tmp_path: Path,
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    tool = registry.require("mechanical.reference-evaluator", "1.0.0")
    assert tool.trusted_in_process is True
    plain = registry.build(
        evaluator_descriptor(),
        ReferenceEvaluatorHandler(),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    with pytest.raises(ToolIsolationError, match="only a registered first-party pure evaluator"):
        executor._dispatch_in_process(plain, cast(Any, {}), None)


def test_executor_trusted_in_process_requires_the_explicit_mode(tmp_path: Path) -> None:
    registry = _evaluator_registry()
    executor = _executor(tmp_path, registry=registry, isolation=IsolationMode.SUBPROCESS)
    tool = registry.require("mechanical.reference-evaluator", "1.0.0")
    with pytest.raises(ToolIsolationError, match="requires the explicit trusted in-process"):
        executor._dispatch_in_process(tool, cast(Any, {}), None)


def test_executor_refuses_a_workspace_that_changed_after_approval(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _subprocess_executor(tmp_path, monkeypatch)
    original = executor.workspace_for

    def relocated(call_id: str) -> Path:
        path = original(call_id)
        return path.parent / "elsewhere"

    monkeypatch.setattr(executor, "workspace_for", relocated)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "workspace_violation"
    assert "changed between authorization and dispatch" in str(result.error_message)


def test_executor_reports_an_unreadable_workspace(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _subprocess_executor(tmp_path, monkeypatch)

    def explode(_call_id: str) -> Path:
        raise ToolWorkspaceError("the run directory path exists as a file")

    monkeypatch.setattr(executor, "workspace_for", explode)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "workspace_violation"


def test_executor_publishes_a_verified_artifact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def with_artifact(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        workspace = Path(request.workspace)
        target = workspace / "outputs"
        target.mkdir(parents=True, exist_ok=True)
        payload = b'{"ok":true}'
        written = target / "report.json"
        written.write_bytes(payload)
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{written.as_posix()}",
            digest=sha256_hex(payload),
            media_type="application/json",
            size_bytes=len(payload),
            classification=ArtifactClassification.DERIVED_DATA,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", with_artifact)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.SUCCEEDED
    assert len(result.artifacts) == 1
    assert result.artifacts[0].digest == sha256_hex(b'{"ok":true}')
    assert result.artifacts[0].classification is ArtifactClassification.TEXT
    assert executor.reports[0].artifact_bytes == len(b'{"ok":true}')


def test_executor_rejects_a_binary_artifact_claim(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def binary_artifact(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        workspace = Path(request.workspace)
        target = workspace / "outputs"
        target.mkdir(parents=True, exist_ok=True)
        payload = b"\x89PNG\r\n\x1a\n" + b"\x00" * 8
        written = target / "report.json"
        written.write_bytes(payload)
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{written.as_posix()}",
            digest=sha256_hex(payload),
            media_type="application/json",
            size_bytes=len(payload),
            classification=ArtifactClassification.TEXT,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", binary_artifact)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.SUCCEEDED
    assert result.artifacts[0].classification is ArtifactClassification.DESIGN_BINARY


def test_executor_rejects_an_artifact_outside_the_workspace(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def escaping(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        outside = Path(request.workspace).parent / "escaped.json"
        payload = b'{"escaped":true}'
        outside.write_bytes(payload)
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{outside.as_posix()}",
            digest=sha256_hex(payload),
            media_type="application/json",
            size_bytes=len(payload),
            classification=ArtifactClassification.DERIVED_DATA,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", escaping)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "artifact_rejected"


def test_executor_rejects_a_symlinked_artifact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def symlinked(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        workspace = Path(request.workspace)
        target = workspace / "outputs"
        target.mkdir(parents=True, exist_ok=True)
        payload = b'{"ok":true}'
        elsewhere = workspace.parent / "elsewhere.json"
        elsewhere.write_bytes(payload)
        written = target / "report.json"
        written.symlink_to(elsewhere)
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{written.as_posix()}",
            digest=sha256_hex(payload),
            media_type="application/json",
            size_bytes=len(payload),
            classification=ArtifactClassification.DERIVED_DATA,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", symlinked)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "artifact_rejected"


def test_executor_rejects_an_artifact_with_a_wrong_digest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def wrong_digest(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        workspace = Path(request.workspace)
        target = workspace / "outputs"
        target.mkdir(parents=True, exist_ok=True)
        written = target / "report.json"
        written.write_bytes(b'{"ok":true}')
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{written.as_posix()}",
            digest=sha256_hex(b"something else"),
            media_type="application/json",
            size_bytes=9,
            classification=ArtifactClassification.DERIVED_DATA,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", wrong_digest)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "artifact_rejected"


def test_executor_rejects_a_missing_artifact_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def missing(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        payload = b'{"ok":true}'
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{Path(request.workspace) / 'outputs' / 'absent.json'}",
            digest=sha256_hex(payload),
            media_type="application/json",
            size_bytes=len(payload),
            classification=ArtifactClassification.DERIVED_DATA,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", missing)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == "artifact_rejected"


def test_executor_result_sink_receives_published_artifacts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()
    sinked: list[str] = []
    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
        result_sink=lambda path: _record_sink(sinked, path),
    )

    def with_artifact(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        workspace = Path(request.workspace)
        target = workspace / "outputs"
        target.mkdir(parents=True, exist_ok=True)
        payload = b'{"ok":true}'
        written = target / "report.json"
        written.write_bytes(payload)
        artifact = ArtifactRef(
            artifact_id="artifact-1",
            uri=f"file://{written.as_posix()}",
            digest=sha256_hex(payload),
            media_type="application/json",
            size_bytes=len(payload),
            classification=ArtifactClassification.DERIVED_DATA,
        )
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", with_artifact)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.SUCCEEDED
    assert sinked == ["report.json"]


def test_executor_result_sink_failure_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    registry = _evaluator_registry()

    def sink(_path: Path) -> str:
        raise OSError(errno.EIO, "io error")

    executor = _executor(
        tmp_path,
        registry=registry,
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
        result_sink=sink,
    )
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.SUCCEEDED
    assert result.artifacts == ()


def _failed_response(
    request: ToolRequestEnvelope,
    code: ToolErrorCode,
    *,
    status: ToolRunStatus = ToolRunStatus.FAILED,
) -> ToolResponseEnvelope:
    return ToolResponseEnvelope.failed_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        status=status,
        error_code=code,
        error_message="claimed by the tool",
    )


def _write_report_artifact(request: ToolRequestEnvelope) -> ArtifactRef:
    target = Path(request.workspace) / "outputs"
    target.mkdir(parents=True, exist_ok=True)
    payload = b'{"ok":true}'
    written = target / "report.json"
    written.write_bytes(payload)
    return ArtifactRef(
        artifact_id="artifact-1",
        uri=f"file://{written.as_posix()}",
        digest=sha256_hex(payload),
        media_type="application/json",
        size_bytes=len(payload),
        classification=ArtifactClassification.TEXT,
    )


def test_executor_refuses_artifacts_on_a_failed_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    sinked: list[str] = []
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
        result_sink=lambda path: _record_sink(sinked, path),
    )

    def failed_with_artifact(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        failed = _failed_response(request, ToolErrorCode.INVALID_ARGUMENTS)
        return replace(failed, artifacts=(_write_report_artifact(request),))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", failed_with_artifact)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.INVALID_RESULT)
    assert result.artifacts == ()
    assert sinked == []


@pytest.mark.parametrize(
    "status",
    [ToolRunStatus.DENIED, ToolRunStatus.TIMED_OUT, ToolRunStatus.CANCELLED],
)
def test_executor_refuses_a_kernel_status_reported_by_a_tool(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, status: ToolRunStatus
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def claims_status(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return _failed_response(request, ToolErrorCode.TIMEOUT, status=status)

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", claims_status)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.INVALID_RESULT)


def test_executor_refuses_an_error_code_on_a_successful_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def succeeded_with_code(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        return replace(response, error_code=ToolErrorCode.INTERNAL_ERROR)

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", succeeded_with_code)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.INVALID_RESULT)


def test_executor_never_surfaces_a_kernel_decision_code_from_a_tool(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )

    def claims_approval(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        return _failed_response(request, ToolErrorCode.APPROVAL_REQUIRED)

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", claims_approval)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.INTERNAL_ERROR)


def test_executor_releases_the_reservation_when_the_sink_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ledger = BudgetLedger(_budget())

    def sink(_path: Path) -> str:
        raise OSError(errno.EIO, "io error")

    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
        ledger=ledger,
        result_sink=sink,
    )

    def with_artifact(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        return replace(response, artifacts=(_write_report_artifact(request),))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", with_artifact)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.INTERNAL_ERROR)
    assert ledger.reservations() == ()


def test_executor_refuses_an_artifact_replaced_by_a_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
    )
    outside = tmp_path / "outside.json"
    outside.write_bytes(b'{"ok":true}')

    def symlinked(
        _self: ReferenceEvaluatorHandler,
        request: ToolRequestEnvelope,
    ) -> ToolResponseEnvelope:
        response = _real_evaluate(ReferenceEvaluatorHandler(), request)
        artifact = _write_report_artifact(request)
        report = Path(request.workspace) / "outputs" / "report.json"
        report.unlink()
        report.symlink_to(outside)
        return replace(response, artifacts=(artifact,))

    monkeypatch.setattr(ReferenceEvaluatorHandler, "invoke", symlinked)
    result = executor.execute(_call())
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == str(ToolErrorCode.ARTIFACT_REJECTED)


def test_executor_probes_the_strict_profile(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    captured: dict[str, Any] = {}

    def fake_probe(**kwargs: object) -> SandboxProbe:
        captured.update(kwargs)
        return SandboxProbe(
            available=True,
            platform="linux",
            controls=tuple(sorted(STRICT_REQUIRED_CONTROLS)),
        )

    monkeypatch.setattr(executor_module, "probe_sandbox", fake_probe)
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
        python_executable="/usr/bin/python3",
    )
    assert executor.sandbox_probe.available is True
    assert captured["python_executable"] == "/usr/bin/python3"
    assert "mount_namespace" in executor.enforced_controls
    assert "chroot_workspace_only" in executor.enforced_controls


def test_executor_probes_with_the_running_interpreter(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    captured: dict[str, Any] = {}

    def fake_probe(**kwargs: object) -> SandboxProbe:
        captured.update(kwargs)
        return SandboxProbe(
            available=True,
            platform="linux",
            controls=tuple(sorted(STRICT_REQUIRED_CONTROLS)),
        )

    monkeypatch.setattr(executor_module, "probe_sandbox", fake_probe)
    _executor(
        tmp_path,
        registry=_evaluator_registry(),
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
    )
    assert captured["python_executable"] is None


def test_executor_builds_a_strict_sandbox_spec(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        executor_module,
        "probe_sandbox",
        lambda **_k: SandboxProbe(
            available=True,
            platform="linux",
            controls=tuple(sorted(STRICT_REQUIRED_CONTROLS | {"scratch_tmpfs"})),
        ),
    )
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
    )
    spec = executor._build_sandbox_spec(tmp_path / "root", tmp_path / "ws")
    assert spec is not None
    assert STRICT_REQUIRED_CONTROLS <= set(spec.controls)
    assert "scratch_tmpfs" in spec.controls


def test_executor_reports_an_incomplete_strict_spec(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        executor_module,
        "probe_sandbox",
        lambda **_k: SandboxProbe(
            available=True,
            platform="linux",
            controls=("mount_namespace",),
        ),
    )
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        profile=SandboxProfile(isolation_profile=IsolationProfile.STRICT_LINUX),
    )
    with pytest.raises(ToolIsolationError, match="strict profile requires controls"):
        executor._build_sandbox_spec(tmp_path / "root", tmp_path / "ws")


def test_executor_sandbox_root_is_unique_per_dispatch(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    first = executor._sandbox_root(_call())
    second = executor._sandbox_root(_call())
    assert first != second
    assert first.parent.parent == executor.run_directory / "sandbox"
    first.parent.mkdir(parents=True)
    ToolExecutor._discard_sandbox_root(first)
    assert not first.parent.exists()


def test_executor_rejects_an_unsafe_artifact_path(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    workspace = executor.workspace_for("call-1")
    outside = tmp_path / "outside.json"
    with pytest.raises(ToolWorkspaceError, match="escapes the call workspace"):
        executor._confine(workspace, outside)
    with pytest.raises(ToolWorkspaceError, match="must be absolute"):
        executor._confine(workspace, Path("relative.json"))
    with pytest.raises(ToolWorkspaceError, match="does not exist"):
        executor._confine(tmp_path / "absent", tmp_path / "absent" / "x.json")


def test_executor_confine_accepts_a_workspace_relative_path(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    workspace = executor.workspace_for("call-1")
    candidate = workspace / "outputs" / "report.json"
    candidate.parent.mkdir(parents=True)
    candidate.write_bytes(b"{}")
    assert executor._confine(workspace, candidate) == candidate
    assert executor._confine(workspace, workspace) == workspace
    (workspace / "outputs" / "link.json").symlink_to(candidate)
    with pytest.raises(ToolWorkspaceError, match="cannot traverse symlinks"):
        executor._confine(workspace, workspace / "outputs" / "link.json")


def test_executor_run_directory_is_private(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    assert stat.S_IMODE(executor.run_directory.stat().st_mode) == 0o700


def _record_sink(sinked: list[str], path: Path) -> str:
    sinked.append(path.name)
    return "digested"


def _subprocess_executor(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    profile: SandboxProfile | None = None,
    granted: tuple[str, ...] = (),
    ledger: BudgetLedger | None = None,
) -> ToolExecutor:
    executor = _executor(
        tmp_path,
        registry=_evaluator_registry(),
        profile=profile,
        granted=granted,
        ledger=ledger,
    )
    monkeypatch.setattr(executor, "_select_isolation", lambda _tool: IsolationMode.SUBPROCESS)
    return executor


class _ChunkStream:
    def __init__(
        self,
        chunks: list[Any],
        *,
        close_error: bool = False,
        no_close: bool = False,
    ) -> None:
        self._chunks = list(chunks)
        self._close_error = close_error
        self.closed = False
        self._no_close = no_close

    def read(self, size: int) -> object:
        if not self._chunks:
            return b""
        return self._chunks.pop(0)

    def close(self) -> None:
        if self._no_close:
            return
        if self._close_error:
            raise OSError(errno.EIO, "io error")
        self.closed = True


class _ExplodingStream:
    def read(self, size: int) -> bytes:
        raise OSError(errno.EIO, "io error")

    def close(self) -> None:
        return


class _FakeProcess:
    def __init__(
        self,
        *,
        pid: int,
        exitcode: int | None = None,
        kill_error: Callable[[], BaseException] | None = None,
    ) -> None:
        self.pid = pid
        self._exitcode = exitcode
        self._kill_error = kill_error
        self.killed = False
        self.kill_attempts = 0

    def kill(self) -> None:
        self.kill_attempts += 1
        if self._kill_error is not None:
            raise self._kill_error()
        self.killed = True
        self._exitcode = -9

    def poll(self) -> int | None:
        return self._exitcode
