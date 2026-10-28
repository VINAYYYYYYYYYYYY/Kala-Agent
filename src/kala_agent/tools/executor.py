from __future__ import annotations

import importlib.util
import os
import shutil
import signal
import stat
import subprocess
import sys
import sysconfig
import threading
import time
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path
from threading import BoundedSemaphore, RLock
from typing import Any, Final, cast
from urllib.parse import unquote, urlsplit

from kala_agent.contracts import (
    ApprovalKind,
    ArtifactClassification,
    ArtifactRef,
    SideEffectLevel,
    ToolCall,
    ToolDescriptor,
    ToolResult,
    ToolRunStatus,
)
from kala_agent.kernel.budget import (
    BudgetError,
    BudgetExhaustedError,
    BudgetLedger,
    BudgetReservation,
    BudgetReservationUnknownError,
)
from kala_agent.kernel.policy import (
    PolicyDecision,
    PolicyDecisionStatus,
    PolicyEngine,
    PolicyReason,
    PolicyRequest,
)
from kala_agent.tools.protocol import (
    MAX_ARGUMENT_BYTES,
    PROTECTED_CHILD_ENVIRONMENT_KEYS,
    JsonEnvelopeError,
    SchemaValidationError,
    SubprocessToolHandler,
    ToolErrorCode,
    ToolHandler,
    ToolProtocolError,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    ToolWorkspaceError,
    build_child_environment,
    build_worker_command,
    canonical_json_digest,
    canonical_json_dumps,
    resolve_workspace_path,
    sanitize_child_stderr,
    sha256_hex,
    strict_json_loads,
    summarize_error,
)
from kala_agent.tools.registry import (
    RegisteredTool,
    ToolRegistry,
    ToolRegistryError,
    ToolVersionNotFoundError,
    UnknownToolError,
)
from kala_agent.tools.sandbox import (
    ISOLATION_FAILURE_EXIT,
    IsolationProfile,
    SandboxProbe,
    SandboxSetupError,
    SandboxSpec,
    build_sandbox_spec,
    child_pre_exec,
    judge_sandbox,
    probe_sandbox,
)

MAX_WORKERS: Final[int] = 8
MAX_DEFAULT_TIMEOUT_SECONDS: Final[float] = 3_600.0
MINIMUM_TIMEOUT_SECONDS: Final[float] = 0.01
MAX_ERROR_MESSAGE_LENGTH: Final[int] = 1_024
MAX_QUEUE_SECONDS: Final[float] = 300.0
MINIMUM_QUEUE_SECONDS: Final[float] = 0.0
_PACKAGE_ROOT: Final[Path] = Path(__file__).resolve().parents[2]
_POLL_INTERVAL_SECONDS: Final[float] = 0.02
_STDERR_SAMPLE_BYTES: Final[int] = 65_536
_WORKER_BOOTSTRAP_TIMEOUT_SECONDS: Final[float] = 5.0
_APPROVAL_BINDING_VERSION: Final[str] = "tool-approval/2"
_CLASSIFICATION_HEADER_BYTES: Final[int] = 4_096
_EXTRA_ENVIRONMENT_KEYS: Final[frozenset[str]] = frozenset({"TZ"})
_NETWORK_CAPABILITIES: Final[frozenset[str]] = frozenset(
    {"egress", "network", "network.access", "network.egress", "network.internet"}
)
_EGRESS_CAPABILITIES: Final[frozenset[str]] = frozenset({"egress", "export", "network.egress"})
_FORBIDDEN_CAPABILITIES: Final[frozenset[str]] = frozenset(
    {
        "package.install",
        "process.spawn",
        "shell.execute",
        "system.admin",
    }
)
_PORTABLE_CONTROLS: Final[tuple[str, ...]] = (
    "artifact_classification_kernel_verified",
    "artifact_digest_verification",
    "cwd_confined",
    "environment_minimal",
    "file_descriptor_close",
    "output_size_limit",
    "process_group_kill",
    "process_isolation",
    "wall_clock_timeout",
    "worker_concurrency_cap",
)
TEXTUAL_MEDIA_SUFFIXES: Final[frozenset[str]] = frozenset(
    {
        ".csv",
        ".json",
        ".md",
        ".text",
        ".toml",
        ".txt",
        ".yaml",
        ".yml",
    }
)
_BINARY_SIGNATURES: Final[tuple[tuple[bytes, str], ...]] = (
    (b"AC10", "image/vnd.dwg"),
    (b"ISO-10303", "model/step"),
    (b"PK\x03\x04", "application/zip"),
    (b"%PDF", "application/pdf"),
    (b"\x89PNG\r\n\x1a\n", "image/png"),
    (b"solid ", "model/stl"),
    (b"\x7fELF", "application/x-elf"),
)
_FAILURE_STATUS_BY_CODE: Final[Mapping[ToolErrorCode, ToolRunStatus]] = {
    ToolErrorCode.APPROVAL_REQUIRED: ToolRunStatus.DENIED,
    ToolErrorCode.CAPABILITY_DENIED: ToolRunStatus.DENIED,
    ToolErrorCode.CAPABILITY_FORBIDDEN: ToolRunStatus.DENIED,
    ToolErrorCode.DESCRIPTOR_MISMATCH: ToolRunStatus.DENIED,
    ToolErrorCode.EGRESS_DENIED: ToolRunStatus.DENIED,
    ToolErrorCode.ISOLATION_REQUIRED: ToolRunStatus.DENIED,
    ToolErrorCode.ISOLATION_UNAVAILABLE: ToolRunStatus.DENIED,
    ToolErrorCode.NETWORK_DENIED: ToolRunStatus.DENIED,
    ToolErrorCode.SANDBOX_UNAVAILABLE: ToolRunStatus.DENIED,
    ToolErrorCode.SCHEMA_DIGEST_MISMATCH: ToolRunStatus.DENIED,
    ToolErrorCode.SIDE_EFFECT_MISMATCH: ToolRunStatus.DENIED,
    ToolErrorCode.WORKER_CAPACITY_EXHAUSTED: ToolRunStatus.DENIED,
    ToolErrorCode.TIMEOUT: ToolRunStatus.TIMED_OUT,
    ToolErrorCode.UNKNOWN_TOOL: ToolRunStatus.DENIED,
    ToolErrorCode.VERSION_NOT_FOUND: ToolRunStatus.DENIED,
}
_SIDE_EFFECTING_LEVELS: Final[frozenset[SideEffectLevel]] = frozenset(
    {
        SideEffectLevel.LOCAL_WRITE,
        SideEffectLevel.NETWORK,
        SideEffectLevel.EGRESS,
    }
)
_CAPABILITY_POLICY_REASONS: Final[frozenset[PolicyReason]] = frozenset(
    {
        PolicyReason.CAPABILITY_NOT_GRANTED,
        PolicyReason.CAPABILITY_EXPIRED,
        PolicyReason.SCOPE_MISMATCH,
    }
)
_TOOL_REPORTABLE_STATUSES: Final[frozenset[ToolRunStatus]] = frozenset(
    {ToolRunStatus.SUCCEEDED, ToolRunStatus.FAILED}
)
_KERNEL_DECISION_CODES: Final[frozenset[ToolErrorCode]] = frozenset(
    {
        ToolErrorCode.APPROVAL_REQUIRED,
        ToolErrorCode.BUDGET_EXHAUSTED,
        ToolErrorCode.CANCELLED,
        ToolErrorCode.CAPABILITY_DENIED,
        ToolErrorCode.CAPABILITY_FORBIDDEN,
        ToolErrorCode.DESCRIPTOR_MISMATCH,
        ToolErrorCode.EGRESS_DENIED,
        ToolErrorCode.ISOLATION_REQUIRED,
        ToolErrorCode.NETWORK_DENIED,
        ToolErrorCode.SCHEMA_DIGEST_MISMATCH,
        ToolErrorCode.SIDE_EFFECT_MISMATCH,
        ToolErrorCode.TIMEOUT,
        ToolErrorCode.UNKNOWN_TOOL,
        ToolErrorCode.VERSION_NOT_FOUND,
        ToolErrorCode.WORKER_CAPACITY_EXHAUSTED,
    }
)


class ToolExecutorError(RuntimeError):
    pass


class ToolAuthorityError(ToolExecutorError):
    pass


class ToolIsolationError(ToolExecutorError):
    pass


class IsolationMode(StrEnum):
    """How the executor is allowed to run a tool.

    ``AUTO`` is the default and never runs tool code inside the kernel: an
    in-kernel handler is refused rather than silently invoked. Running inside
    the kernel is only possible through the deliberately loud
    ``TRUSTED_IN_PROCESS_EVALUATOR`` mode, which the registry restricts to
    registered first-party pure evaluators.
    """

    AUTO = "auto"
    SUBPROCESS = "subprocess"
    TRUSTED_IN_PROCESS_EVALUATOR = "trusted_in_process_evaluator"


DispatchFailure = tuple[ToolRunStatus, ToolErrorCode, str]
DispatchOutcome = tuple[ToolResponseEnvelope | None, int | None, DispatchFailure | None]


@dataclass(frozen=True, slots=True)
class SandboxProfile:
    profile_name: str = "portable-minimum"
    max_output_bytes: int = 262_144
    max_argument_bytes: int = MAX_ARGUMENT_BYTES
    max_artifact_bytes: int = 8_388_608
    default_timeout_seconds: float = 30.0
    max_workers: int = 4
    max_queue_seconds: float = 5.0
    allow_network: bool = False
    allow_egress: bool = False
    allow_privileged_capabilities: bool = False
    enforce_child_limits: bool = True
    isolation_profile: IsolationProfile = IsolationProfile.PORTABLE
    extra_environment: Mapping[str, str] = field(default_factory=dict)
    sandbox_probe: SandboxProbe | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.profile_name, str) or not self.profile_name.strip():
            raise ValueError("profile_name must be a nonempty string")
        for name in ("max_output_bytes", "max_argument_bytes", "max_artifact_bytes"):
            value = getattr(self, name)
            if isinstance(value, bool) or not isinstance(value, int) or value < 1:
                raise ValueError(f"{name} must be a positive integer")
        if isinstance(self.max_workers, bool) or not isinstance(self.max_workers, int):
            raise ValueError("max_workers must be an integer")
        if not 1 <= self.max_workers <= MAX_WORKERS:
            raise ValueError(f"max_workers must be between 1 and {MAX_WORKERS}")
        queue = self.max_queue_seconds
        if isinstance(queue, bool) or not isinstance(queue, (int, float)):
            raise ValueError("max_queue_seconds must be a number")
        if not MINIMUM_QUEUE_SECONDS <= float(queue) <= MAX_QUEUE_SECONDS:
            raise ValueError("max_queue_seconds must be within the supported bounds")
        timeout = self.default_timeout_seconds
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)):
            raise ValueError("default_timeout_seconds must be a number")
        if not 0.0 < float(timeout) <= MAX_DEFAULT_TIMEOUT_SECONDS:
            raise ValueError("default_timeout_seconds must be within the supported bounds")
        for name in (
            "allow_network",
            "allow_egress",
            "allow_privileged_capabilities",
            "enforce_child_limits",
        ):
            if not isinstance(getattr(self, name), bool):
                raise ValueError(f"{name} must be a boolean")
        try:
            object.__setattr__(self, "isolation_profile", IsolationProfile(self.isolation_profile))
        except (TypeError, ValueError) as error:
            raise ValueError("isolation_profile is not a supported profile") from error
        if not isinstance(self.extra_environment, Mapping):
            raise ValueError("extra_environment must be a mapping")
        for key, value in self.extra_environment.items():
            if not isinstance(key, str) or not isinstance(value, str):
                raise ValueError("extra_environment must map strings to strings")
            if key in PROTECTED_CHILD_ENVIRONMENT_KEYS:
                raise ValueError(f"extra_environment cannot override the protected key {key}")
            if key not in _EXTRA_ENVIRONMENT_KEYS:
                raise ValueError(
                    "extra_environment only accepts non-authority keys: "
                    + ", ".join(sorted(_EXTRA_ENVIRONMENT_KEYS))
                )
        object.__setattr__(self, "extra_environment", dict(self.extra_environment))
        probe = self.sandbox_probe
        if probe is not None and not isinstance(probe, SandboxProbe):
            raise ValueError("sandbox_probe must be a SandboxProbe")

    @property
    def enforced_controls(self) -> tuple[str, ...]:
        """The controls this profile actually enforces on this host.

        Every entry is a control the executor implements unconditionally, or a
        control a preflight probe established and verified inside a real child
        process. A control that was not established is never reported, so the
        portable profile never claims filesystem confinement.
        """
        controls = set(_PORTABLE_CONTROLS)
        if self.enforce_child_limits and _resource_limits_available():
            controls.add("resource_limits")
        probe = self.sandbox_probe
        if (
            self.isolation_profile is IsolationProfile.STRICT_LINUX
            and probe is not None
            and probe.available
        ):
            controls.update(probe.controls)
        return tuple(sorted(controls))

    def timeout_for(self, requested: float | None, descriptor_timeout: float) -> float:
        candidates = [
            float(value)
            for value in (requested, descriptor_timeout)
            if value is not None and float(value) > 0.0
        ]
        selected = min(candidates) if candidates else float(self.default_timeout_seconds)
        return max(MINIMUM_TIMEOUT_SECONDS, min(selected, float(self.default_timeout_seconds)))


def _resource_limits_available() -> bool:
    try:
        return importlib.util.find_spec("resource") is not None
    except (ImportError, ValueError):  # pragma: no cover - non POSIX platforms
        return False


@dataclass(frozen=True, slots=True)
class _ApprovalBinding:
    """The canonical description a policy approval is bound to."""

    envelope: ToolRequestEnvelope | None
    arguments_digest: str
    workspace: str
    envelope_digest: str
    state_digest: str
    action_digest: str
    effective_capabilities: tuple[str, ...]


class _CallRejected(Exception):
    """A call that cannot be dispatched, with the code the caller must see."""

    def __init__(self, code: ToolErrorCode, message: str) -> None:
        self.code = code
        self.message = message
        super().__init__(message)


class _BoundedStreamReader(threading.Thread):
    def __init__(self, stream: object, limit: int) -> None:
        super().__init__(daemon=True)
        self._stream = stream
        self._limit = limit
        self._chunks: list[bytes] = []
        self._total = 0
        self.truncated = False

    def run(self) -> None:
        read = getattr(self._stream, "read", None)
        if not callable(read):
            return
        try:
            while True:
                chunk = read(65_536)
                if not isinstance(chunk, bytes) or not chunk:
                    break
                if self._total + len(chunk) > self._limit:
                    self.truncated = True
                    break
                self._chunks.append(chunk)
                self._total += len(chunk)
        except (OSError, ValueError):
            pass
        self._close()

    def _close(self) -> None:
        close = getattr(self._stream, "close", None)
        if not callable(close):
            return
        try:
            close()
        except (OSError, ValueError):
            return

    @property
    def data(self) -> bytes:
        return b"".join(self._chunks)


def _terminate_process_group(process: subprocess.Popen[bytes]) -> bool:
    """Kill the whole process group of a worker and report whether it went away.

    The worker is started in its own session, so its process group id is its
    pid. Signalling the recorded group id rather than looking the group up with
    ``getpgid`` avoids signalling this process's own group when the worker has
    already been reaped. A platform without process groups (Windows) kills the
    one child instead of reporting a group kill that never happened.
    """
    killpg = getattr(os, "killpg", None)
    if killpg is None:
        try:
            process.kill()
        except (OSError, ProcessLookupError):
            return process.poll() is not None
        return True
    try:
        killpg(process.pid, getattr(signal, "SIGKILL", signal.SIGTERM))
    except (OSError, ProcessLookupError, PermissionError):
        try:
            process.kill()
        except (OSError, ProcessLookupError):
            return process.poll() is not None
    return True


def _has_exited(process: subprocess.Popen[bytes]) -> bool:
    """Report whether the worker exited, leaving it unreaped.

    An unreaped worker keeps its pid, and therefore its process group id,
    reserved, so the group can still be killed afterwards without any risk of
    signalling an unrelated process that reused the id. A platform without
    ``waitid`` (Windows) cannot observe an unreaped exit, so it polls instead.
    """
    if not hasattr(os, "waitid"):
        return process.poll() is not None
    try:
        status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    except ChildProcessError:
        return True
    return status is not None


def _child_limits(profile: SandboxProfile, timeout_seconds: float) -> Callable[[], None] | None:
    if not profile.enforce_child_limits:
        return None
    try:
        import resource
    except ImportError:
        return None
    cpu_seconds = max(1, int(timeout_seconds) + 1)
    file_size = profile.max_artifact_bytes

    def apply_limits() -> None:
        os.umask(0o077)
        for limit_name, soft, hard in (
            ("RLIMIT_CORE", 0, 0),
            ("RLIMIT_CPU", cpu_seconds, cpu_seconds + 1),
            ("RLIMIT_FSIZE", file_size, file_size),
            ("RLIMIT_NOFILE", 256, 256),
        ):
            constant = getattr(resource, limit_name, None)
            if constant is None:
                continue
            try:
                resource.setrlimit(constant, (soft, hard))
            except (OSError, ValueError):
                continue

    return apply_limits


def _probe_is_textual(header: bytes) -> bool:
    if b"\x00" in header:
        return False
    try:
        header.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def _derive_classification(
    path: Path,
    declared: ArtifactClassification,
) -> ArtifactClassification:
    """Classify an artifact, refusing to trust a tool's own claim.

    A recognised binary signature, a payload that is not decodable text, or a
    format the kernel does not recognise all classify as ``DESIGN_BINARY``. The
    weaker claims a tool can make are never allowed to downgrade that verdict,
    so an unrecognised file can never be published as reviewable text.
    """
    try:
        with path.open("rb") as artifact_file:
            header = artifact_file.read(_CLASSIFICATION_HEADER_BYTES)
    except OSError:
        return ArtifactClassification.DESIGN_BINARY
    return _classify_header(header, path.suffix, declared)


def _classify_header(
    header: bytes,
    suffix: str,
    declared: ArtifactClassification,
) -> ArtifactClassification:
    header = header[:_CLASSIFICATION_HEADER_BYTES]
    for signature, _ in _BINARY_SIGNATURES:
        if header.startswith(signature):
            return ArtifactClassification.DESIGN_BINARY
    if declared is ArtifactClassification.DESIGN_BINARY:
        return ArtifactClassification.DESIGN_BINARY
    if suffix.lower() not in TEXTUAL_MEDIA_SUFFIXES:
        return ArtifactClassification.DESIGN_BINARY
    if not _probe_is_textual(header):
        return ArtifactClassification.DESIGN_BINARY
    return ArtifactClassification.TEXT


@dataclass(frozen=True, slots=True)
class ToolExecutionReport:
    call_id: str
    tool_id: str
    tool_version: str
    isolation: IsolationMode
    sandbox_profile: str
    status: ToolRunStatus
    error_code: str | None
    process_id: int | None
    granted_capabilities: tuple[str, ...]
    artifact_bytes: int
    enforced_controls: tuple[str, ...] = ()
    envelope_digest: str = ""

    def to_payload(self) -> dict[str, object]:
        return {
            "artifact_bytes": self.artifact_bytes,
            "call_id": self.call_id,
            "enforced_controls": list(self.enforced_controls),
            "envelope_digest": self.envelope_digest,
            "error_code": self.error_code,
            "granted_capabilities": list(self.granted_capabilities),
            "isolation": self.isolation.value,
            "process_id": self.process_id,
            "sandbox_profile": self.sandbox_profile,
            "status": self.status.value,
            "tool_id": self.tool_id,
            "tool_version": self.tool_version,
        }


class ToolExecutor:
    def __init__(
        self,
        registry: ToolRegistry,
        *,
        run_directory: Path,
        granted_capabilities: Iterable[str] = (),
        profile: SandboxProfile | None = None,
        ledger: BudgetLedger | None = None,
        policy: PolicyEngine | None = None,
        isolation: IsolationMode = IsolationMode.AUTO,
        import_roots: Sequence[Path] = (),
        python_executable: str | None = None,
        result_sink: Callable[[Path], str] | None = None,
    ) -> None:
        if not isinstance(registry, ToolRegistry):
            raise TypeError("registry must be a ToolRegistry")
        if policy is not None and not isinstance(policy, PolicyEngine):
            raise TypeError("policy must be a PolicyEngine")
        if result_sink is not None and not callable(result_sink):
            raise TypeError("result_sink must be callable")
        self._registry = registry
        self._run_directory = self._prepare_run_directory(run_directory)
        self._granted = self._normalize_capabilities(granted_capabilities)
        self._profile = profile if profile is not None else SandboxProfile()
        self._ledger = ledger
        self._policy = policy
        self._isolation = IsolationMode(isolation)
        self._import_roots = self._normalize_import_roots(import_roots)
        self._python_executable = python_executable
        self._result_sink = result_sink
        self._probe = self._probe_sandbox()
        self._profile = replace(self._profile, sandbox_probe=self._probe)
        self._worker_slots = BoundedSemaphore(self._profile.max_workers)
        self._reports: list[ToolExecutionReport] = []
        self._dispatch_sequence = 0
        self._lock = RLock()

    @property
    def registry(self) -> ToolRegistry:
        return self._registry

    @property
    def run_directory(self) -> Path:
        return self._run_directory

    @property
    def granted_capabilities(self) -> frozenset[str]:
        return self._granted

    @property
    def profile(self) -> SandboxProfile:
        return self._profile

    @property
    def isolation(self) -> IsolationMode:
        return self._isolation

    @property
    def sandbox_probe(self) -> SandboxProbe:
        return self._probe

    @property
    def enforced_controls(self) -> tuple[str, ...]:
        return self._profile.enforced_controls

    @property
    def reports(self) -> tuple[ToolExecutionReport, ...]:
        with self._lock:
            return tuple(self._reports)

    def register(
        self,
        descriptor: ToolDescriptor,
        handler: ToolHandler,
        *,
        input_schema: Mapping[str, object],
        output_schema: Mapping[str, object],
    ) -> RegisteredTool:
        tool = self._registry.build(
            descriptor,
            handler,
            input_schema=input_schema,
            output_schema=output_schema,
        )
        self.require_authority(tool)
        return self._registry.register_tool(tool)

    def require_authority(self, tool: RegisteredTool) -> RegisteredTool:
        if not isinstance(tool, RegisteredTool):
            raise TypeError("tool must be a RegisteredTool")
        missing = sorted(set(tool.required_capabilities) - self._granted)
        if missing:
            raise ToolAuthorityError(
                f"tool {tool.tool_id} claims capabilities that are not granted: "
                f"{', '.join(missing)}"
            )
        self._check_capability_policy(tool.descriptor.required_capabilities)
        self._check_side_effect_policy(tool.side_effect_level)
        return tool

    def grant_capability(self, capability: str) -> frozenset[str]:
        normalized = self._normalize_capabilities([capability])
        with self._lock:
            self._granted = frozenset(self._granted | normalized)
            return self._granted

    def workspace_path(self, call_id: str) -> Path:
        """Resolve a call workspace without creating anything on disk."""
        return (
            self._run_directory / "calls" / self._call_component(call_id) / "workspace"
        ).resolve(strict=False)

    def workspace_for(self, call_id: str) -> Path:
        workspace = self.workspace_path(call_id)
        self._ensure_directory(workspace)
        if not workspace.is_relative_to(self._run_directory):
            raise ToolWorkspaceError("workspace escapes the run directory")
        return workspace

    def resolve_output_path(self, workspace: Path, relative: str) -> Path:
        return resolve_workspace_path(workspace, relative)

    def policy_request(self, call: ToolCall) -> PolicyRequest:
        if not isinstance(call, ToolCall):
            raise TypeError("call must be a ToolCall")
        tool = self._registry.require(call.tool_id, call.tool_version)
        return self._request_for(call, tool, self._approval_binding(call, tool))

    def _approval_binding(
        self,
        call: ToolCall,
        tool: RegisteredTool,
    ) -> _ApprovalBinding:
        """Bind an approval to the exact canonical request it authorizes.

        The binding covers the argument digest, the resolved workspace, the
        effective capabilities, and the tool, version, descriptor, and schema
        digests, so a request id cannot be replayed with different arguments, a
        different workspace, or a retargeted tool.
        """
        workspace = self.workspace_path(call.call_id)
        deadline = self._profile.timeout_for(call.timeout_seconds, tool.descriptor.timeout_seconds)
        capabilities = self._effective_capabilities(call, tool)
        arguments_digest: str
        envelope_digest: str
        envelope: ToolRequestEnvelope | None
        try:
            envelope = ToolRequestEnvelope.from_call(
                call,
                workspace=workspace,
                deadline_seconds=deadline,
                max_output_bytes=self._profile.max_output_bytes,
                max_artifact_bytes=self._profile.max_artifact_bytes,
                granted_capabilities=capabilities,
            )
        except (JsonEnvelopeError, ToolProtocolError, ValueError):
            envelope = None
            arguments_digest = sha256_hex(f"uncanonicalized:{call.arguments_json}")
            envelope_digest = canonical_json_digest(
                {
                    "arguments_digest": arguments_digest,
                    "call_id": call.call_id,
                    "effective_capabilities": list(capabilities),
                    "tool_id": tool.tool_id,
                    "tool_version": tool.version,
                    "workspace": workspace.as_posix(),
                }
            )
        else:
            arguments_digest = envelope.arguments_digest
            envelope_digest = envelope.envelope_digest
        state_digest = canonical_json_digest(
            {
                "binding_version": _APPROVAL_BINDING_VERSION,
                "call_id": call.call_id,
                "enforced_controls": list(self._profile.enforced_controls),
                "granted_capabilities": sorted(self._granted),
                "isolation_mode": self._isolation.value,
                "isolation_profile": self._profile.isolation_profile.value,
                "profile_name": self._profile.profile_name,
                "tool_descriptor_digest": tool.descriptor_digest,
            }
        )
        action_digest = canonical_json_digest(
            {
                "arguments_digest": arguments_digest,
                "destructive": tool.descriptor.destructive,
                "effective_capabilities": list(capabilities),
                "envelope_digest": envelope_digest,
                "idempotent": tool.descriptor.idempotent,
                "input_schema_digest": tool.input_schema_digest,
                "output_schema_digest": tool.output_schema_digest,
                "side_effect_level": tool.side_effect_level.value,
                "tool_descriptor_digest": tool.descriptor_digest,
                "tool_id": tool.tool_id,
                "tool_version": tool.version,
                "workspace": workspace.as_posix(),
            }
        )
        return _ApprovalBinding(
            envelope=envelope,
            arguments_digest=arguments_digest,
            workspace=workspace.as_posix(),
            envelope_digest=envelope_digest,
            state_digest=state_digest,
            action_digest=action_digest,
            effective_capabilities=capabilities,
        )

    def execute(
        self,
        call: ToolCall,
        *,
        timeout_seconds: float | None = None,
        cancel: threading.Event | None = None,
    ) -> ToolResult:
        if not isinstance(call, ToolCall):
            raise TypeError("call must be a ToolCall")
        started_at = datetime.now(UTC)
        try:
            tool = self._resolve(call)
            binding = self._approval_binding(call, tool)
        except ToolVersionNotFoundError as error:
            return self._denied(call, ToolErrorCode.VERSION_NOT_FOUND, str(error), started_at)
        except UnknownToolError as error:
            return self._denied(call, ToolErrorCode.UNKNOWN_TOOL, str(error), started_at)
        except ToolRegistryError as error:
            return self._denied(call, ToolErrorCode.SCHEMA_DIGEST_MISMATCH, str(error), started_at)
        denial = self._authorize(call, tool, binding)
        if denial is not None:
            return self._denied(call, denial[0], denial[1], started_at)
        if cancel is not None and cancel.is_set():
            return self._cancelled(call, "the tool call was cancelled before dispatch", started_at)
        isolation = self._select_isolation(tool)
        if isolation is None:
            return self._denied(
                call,
                ToolErrorCode.ISOLATION_REQUIRED,
                f"tool {tool.tool_id} cannot be dispatched under the configured isolation policy",
                started_at,
            )
        unavailable = self._sandbox_unavailable(tool)
        if unavailable is not None:
            return self._denied(call, ToolErrorCode.SANDBOX_UNAVAILABLE, unavailable, started_at)
        try:
            workspace, envelope = self._prepare_dispatch(call, tool, binding)
        except _CallRejected as rejected:
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                rejected.code,
                rejected.message,
                started_at,
            )
        if not self._acquire_worker_slot():
            return self._denied(
                call,
                ToolErrorCode.WORKER_CAPACITY_EXHAUSTED,
                f"all {self._profile.max_workers} tool workers are busy",
                started_at,
            )
        try:
            return self._dispatch_call(
                call,
                tool,
                binding,
                envelope,
                workspace,
                isolation=isolation,
                cancel=cancel,
                started_at=started_at,
            )
        finally:
            self._worker_slots.release()

    def _prepare_dispatch(
        self,
        call: ToolCall,
        tool: RegisteredTool,
        binding: _ApprovalBinding,
    ) -> tuple[Path, ToolRequestEnvelope]:
        """Materialize the workspace and re-check the arguments after approval."""
        envelope = binding.envelope
        if envelope is None:
            raise _CallRejected(
                ToolErrorCode.INVALID_ARGUMENTS,
                "the call arguments could not be canonicalized into a request envelope",
            )
        try:
            workspace = self.workspace_for(call.call_id)
            arguments = cast(Mapping[str, object], strict_json_loads(call.arguments_json))
            tool.validate_arguments(arguments)
            canonical_json_dumps(arguments, max_bytes=self._profile.max_argument_bytes)
        except SchemaValidationError as error:
            raise _CallRejected(ToolErrorCode.INVALID_ARGUMENTS, str(error)) from error
        except JsonEnvelopeError as error:
            raise _CallRejected(ToolErrorCode.INVALID_ARGUMENTS, summarize_error(error)) from error
        except ToolWorkspaceError as error:
            raise _CallRejected(
                ToolErrorCode.WORKSPACE_VIOLATION, summarize_error(error)
            ) from error
        except (ToolProtocolError, ValueError) as error:
            raise _CallRejected(
                ToolErrorCode.WORKSPACE_VIOLATION, summarize_error(error)
            ) from error
        if workspace.as_posix() != binding.workspace:
            raise _CallRejected(
                ToolErrorCode.WORKSPACE_VIOLATION,
                "the resolved workspace changed between authorization and dispatch",
            )
        return workspace, envelope

    def _dispatch_call(
        self,
        call: ToolCall,
        tool: RegisteredTool,
        binding: _ApprovalBinding,
        envelope: ToolRequestEnvelope,
        workspace: Path,
        *,
        isolation: IsolationMode,
        cancel: threading.Event | None,
        started_at: datetime,
    ) -> ToolResult:
        reservation: BudgetReservation | None = None
        if self._ledger is not None:
            try:
                reservation = self._ledger.reserve(
                    tool_calls=1,
                    duration_seconds=envelope.deadline_seconds,
                )
            except BudgetExhaustedError as error:
                return self._denied(call, ToolErrorCode.BUDGET_EXHAUSTED, str(error), started_at)
        try:
            return self._dispatch_reserved(
                call,
                tool,
                binding,
                envelope,
                workspace,
                reservation,
                isolation=isolation,
                cancel=cancel,
                started_at=started_at,
            )
        except Exception as error:
            self._release_if_held(reservation)
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                ToolErrorCode.INTERNAL_ERROR,
                summarize_error(error, replacement=str(workspace)),
                started_at,
            )
        except BaseException:
            self._release_if_held(reservation)
            raise

    def _dispatch_reserved(
        self,
        call: ToolCall,
        tool: RegisteredTool,
        binding: _ApprovalBinding,
        envelope: ToolRequestEnvelope,
        workspace: Path,
        reservation: BudgetReservation | None,
        *,
        isolation: IsolationMode,
        cancel: threading.Event | None,
        started_at: datetime,
    ) -> ToolResult:
        dispatch_started = time.monotonic()
        sandbox_root: Path | None = None
        try:
            if isolation is IsolationMode.SUBPROCESS:
                sandbox_root = self._sandbox_root(call)
                outcome = self._dispatch_subprocess(
                    tool,
                    envelope,
                    workspace,
                    sandbox_root=sandbox_root,
                    cancel=cancel,
                )
            else:
                outcome = self._dispatch_in_process(tool, envelope, cancel)
        except (ToolProtocolError, ToolIsolationError) as error:
            self._release(reservation)
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                ToolErrorCode.INTERNAL_ERROR,
                summarize_error(error, replacement=str(workspace)),
                started_at,
            )
        finally:
            if sandbox_root is not None:
                self._discard_sandbox_root(sandbox_root)
        elapsed = time.monotonic() - dispatch_started
        response, process_id, failure = outcome
        if failure is not None:
            status, code, message = failure
            self._release(reservation)
            self._record(
                tool,
                call,
                binding,
                isolation,
                status=status,
                error_code=code,
                process_id=process_id,
                artifact_bytes=0,
            )
            if status is ToolRunStatus.CANCELLED:
                return self._cancelled(call, message, started_at, duration_seconds=elapsed)
            return self._failure(call, status, code, message, started_at, duration_seconds=elapsed)
        if response is None:
            self._release(reservation)
            self._record(
                tool,
                call,
                binding,
                isolation,
                status=ToolRunStatus.FAILED,
                error_code=ToolErrorCode.INTERNAL_ERROR,
                process_id=process_id,
                artifact_bytes=0,
            )
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                ToolErrorCode.INTERNAL_ERROR,
                "tool dispatch produced no result envelope",
                started_at,
                duration_seconds=elapsed,
            )
        try:
            verified = self._verify_response(
                response, tool, call, workspace, elapsed_seconds=elapsed
            )
        except SchemaValidationError as error:
            self._release(reservation)
            self._record(
                tool,
                call,
                binding,
                isolation,
                status=ToolRunStatus.FAILED,
                error_code=ToolErrorCode.INVALID_RESULT,
                process_id=process_id,
                artifact_bytes=0,
            )
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                ToolErrorCode.INVALID_RESULT,
                str(error),
                started_at,
                duration_seconds=elapsed,
            )
        except (ToolProtocolError, ToolWorkspaceError) as error:
            self._release(reservation)
            code = (
                ToolErrorCode.ARTIFACT_REJECTED
                if isinstance(error, ToolWorkspaceError)
                else ToolErrorCode.INVALID_RESULT
            )
            self._record(
                tool,
                call,
                binding,
                isolation,
                status=ToolRunStatus.FAILED,
                error_code=code,
                process_id=process_id,
                artifact_bytes=0,
            )
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                code,
                summarize_error(error, replacement=str(workspace)),
                started_at,
                duration_seconds=elapsed,
            )
        artifact_bytes = sum(artifact.size_bytes for artifact in verified.artifacts)
        budget_failure = self._commit(reservation, elapsed, artifact_bytes)
        error_code = self._normalize_error_code(verified.error_code)
        self._record(
            tool,
            call,
            binding,
            isolation,
            status=verified.status,
            error_code=error_code,
            process_id=process_id,
            artifact_bytes=artifact_bytes,
        )
        if budget_failure is not None:
            return self._failure(
                call,
                ToolRunStatus.FAILED,
                ToolErrorCode.BUDGET_EXHAUSTED,
                budget_failure,
                started_at,
                duration_seconds=elapsed,
            )
        return verified.to_result(started_at=started_at, finished_at=datetime.now(UTC))

    def _resolve(self, call: ToolCall) -> RegisteredTool:
        return self._registry.require(call.tool_id, call.tool_version)

    def _denied(
        self,
        call: ToolCall,
        code: ToolErrorCode,
        message: str,
        started_at: datetime,
    ) -> ToolResult:
        status = _FAILURE_STATUS_BY_CODE.get(code, ToolRunStatus.DENIED)
        return self._failure(call, status, code, message, started_at)

    def _cancelled(
        self,
        call: ToolCall,
        message: str,
        started_at: datetime,
        *,
        duration_seconds: float = 0.0,
    ) -> ToolResult:
        envelope = ToolResponseEnvelope.failed_envelope(
            call_id=call.call_id,
            tool_id=call.tool_id,
            tool_version=call.tool_version,
            status=ToolRunStatus.CANCELLED,
            error_code=None,
            error_message=message,
            duration_ms=round(duration_seconds * 1_000.0, 3),
        )
        return envelope.to_result(started_at=started_at, finished_at=datetime.now(UTC))

    def _failure(
        self,
        call: ToolCall,
        status: ToolRunStatus,
        code: ToolErrorCode,
        message: str,
        started_at: datetime,
        *,
        duration_seconds: float = 0.0,
    ) -> ToolResult:
        envelope = ToolResponseEnvelope.failed_envelope(
            call_id=call.call_id,
            tool_id=call.tool_id,
            tool_version=call.tool_version,
            status=status,
            error_code=code,
            error_message=message[:MAX_ERROR_MESSAGE_LENGTH] or str(code),
            duration_ms=round(duration_seconds * 1_000.0, 3),
        )
        return envelope.to_result(started_at=started_at, finished_at=datetime.now(UTC))

    @staticmethod
    def _normalize_error_code(value: object) -> ToolErrorCode | None:
        """Never surface a code the kernel does not recognise as authoritative."""
        if value is None:
            return None
        candidate = value.value if isinstance(value, ToolErrorCode) else value
        try:
            return ToolErrorCode(cast(str, candidate))
        except ValueError:
            return ToolErrorCode.INTERNAL_ERROR

    @classmethod
    def _normalize_tool_error_code(cls, value: object) -> ToolErrorCode:
        """Never let a tool report a decision only the kernel can make."""
        code = cls._normalize_error_code(value)
        if code is None or code in _KERNEL_DECISION_CODES:
            return ToolErrorCode.INTERNAL_ERROR
        return code

    def _effective_capabilities(self, call: ToolCall, tool: RegisteredTool) -> tuple[str, ...]:
        return tuple(
            sorted(
                self._granted & (set(tool.required_capabilities) | set(call.requested_capabilities))
            )
        )

    def _check_capability_policy(self, capabilities: Iterable[str]) -> None:
        names = frozenset(capabilities)
        if not self._profile.allow_privileged_capabilities:
            forbidden = sorted(names & _FORBIDDEN_CAPABILITIES)
            if forbidden:
                raise ToolAuthorityError(
                    f"capabilities are denied by default: {', '.join(forbidden)}"
                )
        network = sorted(names & _NETWORK_CAPABILITIES)
        if network and not self._profile.allow_network:
            raise ToolAuthorityError(
                f"network capabilities are denied by default: {', '.join(network)}"
            )
        egress = sorted(names & _EGRESS_CAPABILITIES)
        if egress and not (self._profile.allow_egress and self._profile.allow_network):
            raise ToolAuthorityError(
                f"egress capabilities are denied by default: {', '.join(egress)}"
            )

    def _check_side_effect_policy(self, side_effect_level: SideEffectLevel) -> None:
        if side_effect_level is SideEffectLevel.NETWORK and not self._profile.allow_network:
            raise ToolAuthorityError("network side effects are denied by default")
        if side_effect_level is SideEffectLevel.EGRESS and not (
            self._profile.allow_egress and self._profile.allow_network
        ):
            raise ToolAuthorityError("egress side effects are denied by default")

    def _authorize(
        self,
        call: ToolCall,
        tool: RegisteredTool,
        binding: _ApprovalBinding,
    ) -> tuple[ToolErrorCode, str] | None:
        requested = set(call.requested_capabilities)
        required = set(tool.required_capabilities)
        effective = requested | required
        missing = sorted(effective - self._granted)
        if missing:
            return (
                ToolErrorCode.CAPABILITY_DENIED,
                f"capabilities are not granted: {', '.join(missing)}",
            )
        undeclared = sorted(requested - required)
        if undeclared:
            return (
                ToolErrorCode.CAPABILITY_DENIED,
                "call requests capabilities the descriptor does not declare: "
                f"{', '.join(undeclared)}",
            )
        try:
            self._check_capability_policy(effective)
            self._check_side_effect_policy(tool.side_effect_level)
        except ToolAuthorityError as error:
            return self._policy_error_code(effective, tool.side_effect_level), str(error)
        if call.side_effect_level is not tool.side_effect_level:
            return (
                ToolErrorCode.SIDE_EFFECT_MISMATCH,
                "the declared call side effect does not match the registered descriptor",
            )
        if self._policy is not None:
            decision: PolicyDecision = self._policy.decide(self._request_for(call, tool, binding))
            if not decision.allowed:
                return (
                    self._policy_decision_code(decision),
                    f"policy decision {decision.reason.value} for request {decision.request_id}",
                )
        return None

    def _request_for(
        self,
        call: ToolCall,
        tool: RegisteredTool,
        binding: _ApprovalBinding,
    ) -> PolicyRequest:
        return PolicyRequest(
            request_id=call.call_id,
            state_digest=binding.state_digest,
            action_digest=binding.action_digest,
            requested_capabilities=binding.effective_capabilities,
            side_effect_level=tool.side_effect_level,
            destructive=tool.descriptor.destructive,
            egress=tool.side_effect_level is SideEffectLevel.EGRESS,
            checkpoint_type=(
                ApprovalKind.EGRESS
                if tool.side_effect_level is SideEffectLevel.EGRESS
                else ApprovalKind.TOOL_CLASS_CHANGE
            ),
        )

    @staticmethod
    def _policy_decision_code(decision: PolicyDecision) -> ToolErrorCode:
        if decision.reason in _CAPABILITY_POLICY_REASONS:
            return ToolErrorCode.CAPABILITY_DENIED
        if decision.reason is PolicyReason.INVALID_REQUEST:
            return ToolErrorCode.CAPABILITY_FORBIDDEN
        if decision.status is PolicyDecisionStatus.PENDING:
            return ToolErrorCode.APPROVAL_REQUIRED
        return ToolErrorCode.APPROVAL_REQUIRED

    def _policy_error_code(
        self,
        capabilities: Iterable[str],
        side_effect_level: SideEffectLevel,
    ) -> ToolErrorCode:
        names = frozenset(capabilities)
        if side_effect_level is SideEffectLevel.EGRESS or names & _EGRESS_CAPABILITIES:
            return ToolErrorCode.EGRESS_DENIED
        if side_effect_level is SideEffectLevel.NETWORK or names & _NETWORK_CAPABILITIES:
            return ToolErrorCode.NETWORK_DENIED
        return ToolErrorCode.CAPABILITY_FORBIDDEN

    def _select_isolation(self, tool: RegisteredTool) -> IsolationMode | None:
        """Choose how a tool may run, or refuse it.

        ``AUTO`` never runs tool code inside the kernel. A subprocess handler is
        always dispatched out of process. An in-kernel handler is only eligible
        for the explicit trusted mode and only when the registry marked it as a
        first-party pure evaluator.
        """
        handler = tool.handler
        if isinstance(handler, SubprocessToolHandler):
            if self._isolation is IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR:
                return None
            return IsolationMode.SUBPROCESS
        if self._isolation is IsolationMode.SUBPROCESS:
            return None
        if self._isolation is IsolationMode.AUTO:
            return None
        if tool.trusted_in_process and tool.side_effect_level is SideEffectLevel.NONE:
            return IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR
        return None

    def _sandbox_unavailable(self, tool: RegisteredTool) -> str | None:
        """Refuse a side effect that the configured profile cannot back."""
        verdict = judge_sandbox(self._profile.isolation_profile, self._probe)
        if verdict.allowed:
            return None
        if tool.side_effect_level in _SIDE_EFFECTING_LEVELS or tool.descriptor.destructive:
            return f"tool {tool.tool_id} is refused: {verdict.reason}"
        return None

    def _acquire_worker_slot(self) -> bool:
        return self._worker_slots.acquire(timeout=self._profile.max_queue_seconds)

    def _sandbox_root(self, call: ToolCall) -> Path:
        with self._lock:
            self._dispatch_sequence += 1
            sequence = self._dispatch_sequence
        return (
            self._run_directory
            / "sandbox"
            / f"{self._call_component(call.call_id)}-{sequence}"
            / "root"
        )

    @staticmethod
    def _discard_sandbox_root(root: Path) -> None:
        shutil.rmtree(root.parent, ignore_errors=True)

    def _child_import_roots(self) -> tuple[Path, ...]:
        candidates = [*self._import_roots, _PACKAGE_ROOT]
        for name in ("stdlib", "platstdlib", "purelib", "platlib"):
            value = sysconfig.get_paths().get(name, "")
            if value:
                candidates.append(Path(value))
        selected: list[Path] = []
        for root in candidates:
            candidate = Path(root).resolve(strict=False)
            if candidate.is_dir() and candidate not in selected:
                selected.append(candidate)
        return tuple(selected)

    def _build_sandbox_spec(
        self,
        root: Path,
        workspace: Path,
    ) -> SandboxSpec | None:
        if self._profile.isolation_profile is not IsolationProfile.STRICT_LINUX:
            return None
        try:
            return build_sandbox_spec(
                controls=frozenset(self._probe.controls),
                root=root,
                workspace=workspace,
                import_roots=self._child_import_roots(),
                scratch_bytes=min(self._profile.max_artifact_bytes, 16 * 1_048_576),
            )
        except SandboxSetupError as error:
            raise ToolIsolationError(str(error)) from error

    def _probe_sandbox(self) -> SandboxProbe:
        if self._profile.isolation_profile is not IsolationProfile.STRICT_LINUX:
            return SandboxProbe(
                available=False,
                platform="not-probed",
                detail="the portable profile confines a process, not a filesystem",
            )
        return probe_sandbox(
            import_roots=self._child_import_roots(),
            scratch_bytes=min(self._profile.max_artifact_bytes, 16 * 1_048_576),
            python_executable=self._python_executable,
        )

    def _dispatch_in_process(
        self,
        tool: RegisteredTool,
        envelope: ToolRequestEnvelope,
        cancel: threading.Event | None,
    ) -> DispatchOutcome:
        handler = tool.handler
        if isinstance(handler, SubprocessToolHandler):
            raise ToolIsolationError("subprocess handlers cannot run inside the kernel process")
        if not tool.trusted_in_process or tool.side_effect_level is not SideEffectLevel.NONE:
            raise ToolIsolationError(
                "only a registered first-party pure evaluator may run inside the kernel process"
            )
        if self._isolation is not IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR:
            raise ToolIsolationError(
                "in-process execution requires the explicit trusted in-process evaluator mode"
            )
        outcome: list[DispatchOutcome] = []
        finished = threading.Event()

        def run() -> None:
            try:
                response = handler.invoke(envelope)
            except ToolProtocolError as error:
                outcome.append(
                    (
                        None,
                        None,
                        (
                            ToolRunStatus.FAILED,
                            ToolErrorCode.PROTOCOL_ERROR,
                            summarize_error(error, replacement=envelope.workspace),
                        ),
                    )
                )
            except Exception as error:
                outcome.append(
                    (
                        None,
                        None,
                        (
                            ToolRunStatus.FAILED,
                            ToolErrorCode.INTERNAL_ERROR,
                            summarize_error(error, replacement=envelope.workspace),
                        ),
                    )
                )
            else:
                outcome.append((response, None, None))
            finally:
                finished.set()

        worker = threading.Thread(
            target=run, name=f"tool-invocation-{envelope.call_id}", daemon=True
        )
        worker.start()
        deadline = time.monotonic() + envelope.deadline_seconds
        while not finished.wait(_POLL_INTERVAL_SECONDS):
            if cancel is not None and cancel.is_set():
                return (
                    None,
                    None,
                    (
                        ToolRunStatus.CANCELLED,
                        ToolErrorCode.CANCELLED,
                        "the tool call was cancelled during execution",
                    ),
                )
            if time.monotonic() >= deadline:
                return (
                    None,
                    None,
                    (
                        ToolRunStatus.TIMED_OUT,
                        ToolErrorCode.TIMEOUT,
                        f"tool exceeded the {envelope.deadline_seconds} second deadline",
                    ),
                )
        if not outcome:
            return (
                None,
                None,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.INTERNAL_ERROR,
                    "tool dispatch produced no result envelope",
                ),
            )
        response, process_id, failure = outcome[0]
        if failure is not None or response is None:
            return outcome[0]
        if cancel is not None and cancel.is_set():
            return (
                None,
                None,
                (
                    ToolRunStatus.CANCELLED,
                    ToolErrorCode.CANCELLED,
                    "the tool call was cancelled during execution",
                ),
            )
        payload_size = len(response.to_json().encode("utf-8"))
        if payload_size > self._profile.max_output_bytes:
            return (
                None,
                None,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.OUTPUT_TOO_LARGE,
                    f"result envelope of {payload_size} bytes exceeds the bounded output size",
                ),
            )
        return response, process_id, None

    def _dispatch_subprocess(
        self,
        tool: RegisteredTool,
        envelope: ToolRequestEnvelope,
        workspace: Path,
        *,
        sandbox_root: Path | None,
        cancel: threading.Event | None,
    ) -> DispatchOutcome:
        handler = cast(SubprocessToolHandler, tool.handler)
        limits = _child_limits(self._profile, envelope.deadline_seconds)
        spec: SandboxSpec | None = None
        if sandbox_root is not None:
            if os.name != "posix":
                return (
                    None,
                    None,
                    (
                        ToolRunStatus.FAILED,
                        ToolErrorCode.ISOLATION_UNAVAILABLE,
                        "the strict sandbox requires a POSIX host and is unsupported on this "
                        "platform",
                    ),
                )
            self._ensure_directory(sandbox_root)
            spec = self._build_sandbox_spec(sandbox_root, workspace)
        python_executable = self._python_executable
        if spec is not None and python_executable is None:
            python_executable = os.path.realpath(sys.executable)
        command = build_worker_command(handler, python_executable=python_executable)
        environment = build_child_environment(
            workspace=workspace,
            import_roots=self._child_import_roots(),
            extra=self._profile.extra_environment,
        )
        # ``start_new_session`` and ``preexec_fn`` are POSIX-only; on Windows
        # ``subprocess`` rejects both, which used to turn every dispatch into a
        # generic isolation failure instead of running the worker.
        spawn_options: dict[str, Any] = {}
        if os.name == "posix":
            spawn_options["start_new_session"] = True
            spawn_options["preexec_fn"] = lambda: _child_pre_exec(limits, spec)
        try:
            process = subprocess.Popen(
                command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=workspace,
                env=environment,
                close_fds=True,
                **spawn_options,
            )
        except (OSError, ValueError) as error:
            return (
                None,
                None,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.ISOLATION_UNAVAILABLE,
                    summarize_error(error),
                ),
            )
        try:
            return self._supervise(process, envelope, workspace, cancel)
        except BaseException:
            _terminate_process_group(process)
            self._reap(process)
            raise

    def _supervise(
        self,
        process: subprocess.Popen[bytes],
        envelope: ToolRequestEnvelope,
        workspace: Path,
        cancel: threading.Event | None,
    ) -> DispatchOutcome:
        process_id = process.pid
        stdout_reader = _BoundedStreamReader(process.stdout, self._profile.max_output_bytes)
        stderr_reader = _BoundedStreamReader(process.stderr, _STDERR_SAMPLE_BYTES)
        stdout_reader.start()
        stderr_reader.start()
        self._send_request(process, (envelope.to_json() + "\n").encode("utf-8"))
        deadline = time.monotonic() + envelope.deadline_seconds
        failure: DispatchFailure | None = None
        while True:
            if _has_exited(process):
                break
            time.sleep(_POLL_INTERVAL_SECONDS)
            if stdout_reader.truncated:
                failure = (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.OUTPUT_TOO_LARGE,
                    "tool output exceeds the bounded output size",
                )
                break
            if cancel is not None and cancel.is_set():
                failure = (
                    ToolRunStatus.CANCELLED,
                    ToolErrorCode.CANCELLED,
                    "the tool call was cancelled during execution",
                )
                break
            if time.monotonic() >= deadline:
                failure = (
                    ToolRunStatus.TIMED_OUT,
                    ToolErrorCode.TIMEOUT,
                    f"tool exceeded the {envelope.deadline_seconds} second deadline",
                )
                break
        _terminate_process_group(process)
        if not self._reap(process):
            return (
                None,
                process_id,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.CRASHED,
                    "the tool process could not be terminated",
                ),
            )
        stdout_reader.join(timeout=_WORKER_BOOTSTRAP_TIMEOUT_SECONDS)
        stderr_reader.join(timeout=_WORKER_BOOTSTRAP_TIMEOUT_SECONDS)
        stderr_text = sanitize_child_stderr(stderr_reader.data, replacement=str(workspace))
        if failure is not None:
            return None, process_id, failure
        stdout_data = stdout_reader.data
        if stdout_reader.truncated:
            return (
                None,
                process_id,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.OUTPUT_TOO_LARGE,
                    "tool output exceeds the bounded output size",
                ),
            )
        exit_code = -1 if process.returncode is None else process.returncode
        if exit_code == ISOLATION_FAILURE_EXIT:
            return (
                None,
                process_id,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.SANDBOX_UNAVAILABLE,
                    f"the sandbox could not be established: {stderr_text or 'no detail'}",
                ),
            )
        if exit_code != 0:
            return (
                None,
                process_id,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.CRASHED,
                    f"the tool process exited with status {exit_code}"
                    + (f": {stderr_text}" if stderr_text else ""),
                ),
            )
        try:
            response = ToolResponseEnvelope.from_json(stdout_data)
        except (ToolProtocolError, ValueError) as error:
            return (
                None,
                process_id,
                (
                    ToolRunStatus.FAILED,
                    ToolErrorCode.PROTOCOL_ERROR,
                    "the tool response envelope is invalid: "
                    f"{type(error).__name__}" + (f": {stderr_text}" if stderr_text else ""),
                ),
            )
        return response, process_id, None

    @staticmethod
    def _reap(process: subprocess.Popen[bytes]) -> bool:
        """Wait for the worker to be reaped so no zombie is left behind."""
        for _ in range(2):
            try:
                process.wait(timeout=_WORKER_BOOTSTRAP_TIMEOUT_SECONDS)
                return True
            except subprocess.TimeoutExpired:
                _terminate_process_group(process)
        return process.poll() is not None

    @staticmethod
    def _send_request(process: subprocess.Popen[bytes], payload: bytes) -> None:
        stdin = process.stdin
        if stdin is None:
            return
        try:
            stdin.write(payload)
            stdin.flush()
        except (BrokenPipeError, OSError, ValueError):
            pass
        try:
            stdin.close()
        except (BrokenPipeError, OSError, ValueError):
            return

    def _verify_response(
        self,
        response: ToolResponseEnvelope,
        tool: RegisteredTool,
        call: ToolCall,
        workspace: Path,
        *,
        elapsed_seconds: float,
    ) -> ToolResponseEnvelope:
        if response.call_id != call.call_id:
            raise ToolProtocolError("the tool response call id does not match the dispatched call")
        if response.tool_id != call.tool_id or response.tool_version != call.tool_version:
            raise ToolProtocolError("the tool response identity does not match the dispatched call")
        if response.status not in _TOOL_REPORTABLE_STATUSES:
            raise ToolProtocolError("a tool may only report a succeeded or failed run")
        if response.status is ToolRunStatus.FAILED:
            if response.artifacts:
                raise ToolProtocolError("a failed tool run cannot report artifacts")
            return replace(
                response,
                error_code=self._normalize_tool_error_code(response.error_code),
            )
        if response.error_code is not None:
            raise ToolProtocolError("a successful tool run cannot report an error code")
        if response.result is None:
            raise ToolProtocolError("a successful tool run requires a result payload")
        tool.validate_result(cast(Mapping[str, object], response.result))
        artifacts = self._verify_artifacts(response.artifacts, workspace)
        return replace(
            response,
            artifacts=artifacts,
            duration_ms=round(elapsed_seconds * 1_000.0, 3),
        )

    def _verify_artifacts(
        self,
        artifacts: Sequence[ArtifactRef],
        workspace: Path,
    ) -> tuple[ArtifactRef, ...]:
        verified: list[ArtifactRef] = []
        published: list[Path] = []
        seen: set[str] = set()
        for artifact in artifacts:
            if artifact.artifact_id in seen:
                raise ToolWorkspaceError("the tool reported duplicate artifact identifiers")
            seen.add(artifact.artifact_id)
            parts = urlsplit(artifact.uri)
            if parts.scheme != "file":
                raise ToolWorkspaceError("only local file artifacts are accepted from a tool")
            if parts.query or parts.fragment or parts.netloc not in {"", "localhost"}:
                raise ToolWorkspaceError("the artifact uri contains unexpected components")
            candidate = Path(unquote(parts.path))
            if not candidate.is_absolute():
                raise ToolWorkspaceError("the artifact uri must contain an absolute path")
            resolved = self._confine(workspace, candidate)
            data = self._read_artifact_once(resolved, artifact.size_bytes)
            if sha256_hex(data) != artifact.digest:
                raise ToolWorkspaceError("the artifact digest verification failed")
            classification = _classify_header(data, resolved.suffix, artifact.classification)
            verified.append(artifact.model_copy(update={"classification": classification}))
            published.append(resolved)
        if self._result_sink is not None:
            for path in published:
                self._result_sink(path)
        return tuple(verified)

    def _read_artifact_once(self, path: Path, reported_size: int) -> bytes:
        """Read an artifact through one descriptor so every check sees the same bytes."""
        try:
            descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except OSError as error:
            raise ToolWorkspaceError(
                "the artifact is not a regular file inside the workspace"
            ) from error
        try:
            status = os.fstat(descriptor)
            if not stat.S_ISREG(status.st_mode):
                raise ToolWorkspaceError("the artifact is not a regular file inside the workspace")
            if status.st_size > self._profile.max_artifact_bytes:
                raise ToolWorkspaceError("the artifact exceeds the bounded artifact size")
            if status.st_size != reported_size:
                raise ToolWorkspaceError("the artifact size does not match the reported size")
            chunks: list[bytes] = []
            remaining = self._profile.max_artifact_bytes + 1
            while remaining > 0:
                chunk = os.read(descriptor, min(remaining, 1_048_576))
                if not chunk:
                    break
                chunks.append(chunk)
                remaining -= len(chunk)
        except OSError as error:
            raise ToolWorkspaceError("the artifact could not be read") from error
        finally:
            os.close(descriptor)
        data = b"".join(chunks)
        if len(data) != reported_size:
            raise ToolWorkspaceError("the artifact size does not match the reported size")
        return data

    def _confine(self, workspace: Path, candidate: Path) -> Path:
        base = workspace.resolve(strict=False)
        if not base.is_dir():
            raise ToolWorkspaceError("the call workspace does not exist")
        if not candidate.is_absolute():
            raise ToolWorkspaceError("artifact paths must be absolute")
        lexical = Path(os.path.normpath(str(candidate)))
        if ".." in lexical.parts:
            raise ToolWorkspaceError("artifact paths cannot contain traversal segments")
        if not lexical.is_relative_to(base):
            raise ToolWorkspaceError("the artifact path escapes the call workspace")
        current = base
        for part in lexical.relative_to(base).parts:
            current = current / part
            if current.is_symlink():
                raise ToolWorkspaceError("artifact paths cannot traverse symlinks")
        resolved = candidate.resolve(strict=False)
        if not resolved.is_relative_to(base):
            raise ToolWorkspaceError("the artifact path escapes the call workspace")
        return resolved

    def _commit(
        self,
        reservation: BudgetReservation | None,
        elapsed: float,
        artifact_bytes: int,
    ) -> str | None:
        if reservation is None or self._ledger is None:
            return None
        try:
            self._ledger.commit(
                reservation,
                tool_calls=1,
                duration_seconds=min(elapsed, reservation.usage.duration_seconds),
                artifact_bytes=artifact_bytes,
            )
        except BudgetError as error:
            self._release_if_held(reservation)
            return str(error)
        return None

    def _release(self, reservation: BudgetReservation | None) -> None:
        if reservation is None or self._ledger is None:
            return
        self._ledger.release(reservation)

    def _release_if_held(self, reservation: BudgetReservation | None) -> None:
        try:
            self._release(reservation)
        except BudgetReservationUnknownError:
            return

    def _record(
        self,
        tool: RegisteredTool,
        call: ToolCall,
        binding: _ApprovalBinding,
        isolation: IsolationMode,
        *,
        status: ToolRunStatus,
        error_code: ToolErrorCode | None,
        process_id: int | None,
        artifact_bytes: int,
    ) -> None:
        report = ToolExecutionReport(
            call_id=call.call_id,
            tool_id=tool.tool_id,
            tool_version=tool.version,
            isolation=isolation,
            sandbox_profile=self._profile.profile_name,
            status=status,
            error_code=None if error_code is None else str(error_code),
            process_id=process_id,
            granted_capabilities=binding.effective_capabilities,
            artifact_bytes=artifact_bytes,
            enforced_controls=self._profile.enforced_controls,
            envelope_digest=binding.envelope_digest,
        )
        with self._lock:
            self._reports.append(report)

    @staticmethod
    def _normalize_import_roots(roots: Sequence[Path]) -> tuple[Path, ...]:
        selected: list[Path] = []
        for root in roots:
            candidate = Path(root).resolve(strict=False)
            if not candidate.is_dir():
                raise ToolExecutorError("import roots must be existing directories")
            if candidate not in selected:
                selected.append(candidate)
        if not selected:
            selected.append(_PACKAGE_ROOT)
        return tuple(selected)

    @staticmethod
    def _normalize_capabilities(values: Iterable[str]) -> frozenset[str]:
        if isinstance(values, (str, bytes)):
            raise ToolExecutorError("granted capabilities must be a collection of names")
        names: set[str] = set()
        for value in values:
            if not isinstance(value, str):
                raise ToolExecutorError("granted capabilities must be strings")
            normalized = value.strip()
            if not normalized or len(normalized) > 128:
                raise ToolExecutorError("granted capabilities must be nonempty and bounded")
            if any(character.isspace() or ord(character) < 32 for character in normalized):
                raise ToolExecutorError("granted capabilities contain unsafe characters")
            names.add(normalized)
        return frozenset(names)

    @staticmethod
    def _call_component(call_id: str) -> str:
        if not isinstance(call_id, str):
            raise ToolWorkspaceError("call_id must be a string")
        normalized = call_id.strip()
        if not normalized or len(normalized) > 128:
            raise ToolWorkspaceError("call_id must be nonempty and bounded")
        forbidden = {"", ".", ".."}
        if (
            normalized in forbidden
            or "/" in normalized
            or "\\" in normalized
            or "\x00" in normalized
            or any(character.isspace() or ord(character) < 32 for character in normalized)
        ):
            raise ToolWorkspaceError("call_id cannot address a run directory component")
        return normalized

    @staticmethod
    def _ensure_directory(path: Path) -> None:
        """Create a directory that concurrent callers may be creating as well.

        The check and the creation cannot be separated, so the directory type
        is re-read with a single ``lstat`` afterwards and anything that is not a
        real directory, including a symlink planted in between, is refused.
        """
        try:
            os.makedirs(path, mode=0o700, exist_ok=True)
        except FileExistsError as error:
            raise ToolWorkspaceError("the run directory path exists as a file") from error
        except OSError as error:
            raise ToolWorkspaceError(
                f"the run directory path could not be created: {error.strerror}"
            ) from error
        try:
            status = os.lstat(path)
        except OSError as error:
            raise ToolWorkspaceError("the run directory path is not accessible") from error
        if stat.S_ISLNK(status.st_mode) or not stat.S_ISDIR(status.st_mode):
            raise ToolWorkspaceError("run directory paths must be real directories")

    @staticmethod
    def _prepare_run_directory(path: Path) -> Path:
        if not isinstance(path, Path):
            raise ToolExecutorError("run_directory must be a Path")
        expanded = path.expanduser()
        if expanded.is_symlink():
            raise ToolWorkspaceError("the run directory cannot be a symlink")
        resolved = expanded.resolve(strict=False)
        if resolved == Path(resolved.anchor):
            raise ToolWorkspaceError("the run directory cannot be a filesystem root")
        ToolExecutor._ensure_directory(resolved)
        if resolved.is_symlink() or not resolved.is_dir():
            raise ToolWorkspaceError("the run directory must be a directory")
        return resolved


def _child_pre_exec(
    limits: Callable[[], None] | None,
    spec: SandboxSpec | None,
) -> None:  # pragma: no cover - runs in the forked child
    """Apply the child resource limits and the sandbox before the worker starts."""
    if limits is not None:
        limits()
    if spec is not None:
        child_pre_exec(spec)


__all__ = (
    "MAX_ERROR_MESSAGE_LENGTH",
    "MAX_WORKERS",
    "TEXTUAL_MEDIA_SUFFIXES",
    "IsolationMode",
    "SandboxProfile",
    "ToolAuthorityError",
    "ToolExecutionReport",
    "ToolExecutor",
    "ToolExecutorError",
    "ToolIsolationError",
)
