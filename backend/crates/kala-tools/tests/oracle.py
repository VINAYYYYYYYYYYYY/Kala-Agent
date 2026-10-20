#!/usr/bin/env python3
"""Test-only live Python oracle for the native Kala tool protocol port.

This script is never imported by product code. It runs the repository's own
`kala_agent.tools.protocol` implementation over a corpus and prints a
deterministic JSON document that the Rust differential test compares against.

Usage: python oracle.py <mode> < request.json > response.json

Modes:
  parse        strict decode and canonical encode of raw payloads
  encode       canonical encode of values
  validate     schema validation of values
  check        object schema admission
  request      request envelope construction, payload and digests
  response     response envelope construction, payload and digests
  workspace    workspace path resolution
  diagnostics  child standard error sanitising
  registry     tool registry operations over a scripted corpus
  simulator    reference simulator operations over a scripted corpus
  executor     tool executor decisions over a scripted corpus
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any, Callable


def _add_repo_source() -> None:
    repo = Path(__file__).resolve().parents[4]
    source = repo / "src"
    if source.is_dir() and str(source) not in sys.path:
        sys.path.insert(0, str(source))


def _result(action: Callable[[], Any]) -> dict[str, object]:
    try:
        return {"ok": True, "output": action()}
    except BaseException as error:  # noqa: BLE001 - the oracle reports the class
        return {"ok": False, "error": type(error).__name__, "message": str(error)}


def parse_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import canonical_json_dumps, strict_json_loads

    results = []
    for case in cases:
        payload = bytes.fromhex(case["hex"])
        results.append(
            _result(lambda payload=payload: canonical_json_dumps(strict_json_loads(payload)))
        )
    return results


def encode_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import canonical_json_dumps

    results = []
    for case in cases:
        value = case["value"]
        results.append(_result(lambda value=value: canonical_json_dumps(value)))
    return results


def validate_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import validate_against_schema

    results = []
    for case in cases:
        value = case["value"]
        schema = case["schema"]
        path = case.get("path", "$")
        results.append(
            _result(
                lambda value=value, schema=schema, path=path: (
                    validate_against_schema(value, schema, path=path),
                    None,
                )[1]
            )
        )
    return results


def check_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import check_object_schema

    results = []
    for case in cases:
        schema = case["schema"]
        label = case.get("label", "schema")
        results.append(
            _result(lambda schema=schema, label=label: (check_object_schema(schema, label=label), None)[1])
        )
    return results


def request_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import ToolRequestEnvelope

    results = []
    for case in cases:
        payload = case["payload"]

        def run(payload=payload) -> dict[str, object]:
            envelope = ToolRequestEnvelope.from_payload(payload)
            return {
                "arguments_digest": envelope.arguments_digest,
                "arguments_json": envelope.arguments_json,
                "envelope_digest": envelope.envelope_digest,
                "json": envelope.to_json(),
                "payload": envelope.to_payload(),
            }

        results.append(_result(run))
    return results


def response_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import ToolResponseEnvelope

    results = []
    for case in cases:
        payload = case["payload"]

        def run(payload=payload) -> dict[str, object]:
            envelope = ToolResponseEnvelope.from_payload(payload)
            return {
                "json": envelope.to_json(),
                "payload": envelope.to_payload(),
                "result_digest": envelope.result_digest,
                "result_json": envelope.result_json,
                "succeeded": envelope.succeeded,
            }

        results.append(_result(run))
    return results


def workspace_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import resolve_workspace_path

    root = Path(os.environ["KALA_TOOLS_WORKSPACE_ROOT"])
    results = []
    for case in cases:
        relative = case["relative"]
        results.append(
            _result(
                lambda relative=relative: resolve_workspace_path(
                    root, relative
                ).as_posix()
            )
        )
    return results


def diagnostics_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import sanitize_child_stderr

    results = []
    for case in cases:
        payload = bytes.fromhex(case["hex"])
        replacement = case.get("replacement")
        results.append(
            _result(
                lambda payload=payload, replacement=replacement: sanitize_child_stderr(
                    payload, replacement=replacement
                )
            )
        )
    return results


def pattern_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import validate_against_schema

    results = []
    for case in cases:
        pattern = case["pattern"]
        maximum = case.get("maxLength")
        values = case.get("values", [])
        schema: dict[str, object] = {"type": "string"}
        if maximum is not None:
            schema["maxLength"] = maximum
        schema["pattern"] = pattern
        outcomes = []
        for value in values:
            outcomes.append(
                _result(lambda value=value: (validate_against_schema(value, schema, path="$"), None)[1])
            )
        results.append({"values": outcomes})
    return results


def summary_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.protocol import (
        JsonEnvelopeError,
        SchemaValidationError,
        ToolEnvelopeError,
        ToolProtocolError,
        ToolWorkspaceError,
        summarize_error,
    )

    classes = {
        "protocol": ToolProtocolError,
        "json": JsonEnvelopeError,
        "envelope": ToolEnvelopeError,
        "workspace": ToolWorkspaceError,
    }
    results = []
    for case in cases:
        kind = case["kind"]
        message = case["message"]
        path = case.get("path")
        replacement = case.get("replacement")
        if kind == "schema":
            error = SchemaValidationError(message, path=path or "$")
        else:
            error = classes[kind](message)
        results.append(
            _result(
                lambda error=error, replacement=replacement: summarize_error(
                    error, replacement=replacement
                )
            )
        )
    return results


class _ForeignHandler:
    """A handler that is defined outside the kernel package."""

    def __init__(self, descriptor: Any) -> None:
        self._descriptor = descriptor

    @property
    def descriptor(self) -> Any:
        return self._descriptor

    def invoke(self, request: Any) -> Any:
        raise NotImplementedError


class _FirstPartyHandler(_ForeignHandler):
    """A handler that pretends to be defined by the kernel package."""


_FirstPartyHandler.__module__ = "kala_agent.tools.simulator"


def _tool_summary(tool: Any) -> dict[str, object]:
    return {
        "descriptor_digest": tool.descriptor_digest,
        "input_schema_digest": tool.input_schema_digest,
        "output_schema_digest": tool.output_schema_digest,
        "required_capabilities": list(tool.required_capabilities),
        "side_effect_level": tool.side_effect_level.value,
        "timeout_seconds": tool.timeout_seconds,
        "tool_id": tool.tool_id,
        "trusted_in_process": tool.trusted_in_process,
        "version": tool.version,
    }


def _registry_descriptor(spec: dict[str, Any], input_schema: Any, output_schema: Any) -> Any:
    from kala_agent.contracts import SideEffectLevel, ToolDescriptor
    from kala_agent.tools.protocol import canonical_json_digest

    input_digest = spec.get("input_schema_digest") or canonical_json_digest(input_schema)
    output_digest = spec.get("output_schema_digest") or canonical_json_digest(output_schema)
    return ToolDescriptor(
        tool_id=spec["tool_id"],
        version=spec["version"],
        name=spec.get("name", "Test tool"),
        description=spec.get("description", "A test tool."),
        input_schema_digest=input_digest,
        output_schema_digest=output_digest,
        side_effect_level=SideEffectLevel(spec["side_effect_level"]),
        required_capabilities=tuple(spec.get("required_capabilities", ())),
        timeout_seconds=float(spec.get("timeout_seconds", 10.0)),
        destructive=bool(spec.get("destructive", False)),
        supports_structured_output=True,
        idempotent=True,
    )


def _registry_operation(
    registry: Any,
    context: dict[str, Any],
    operation: dict[str, Any],
) -> object:
    name = operation["op"]
    descriptor = context["descriptor"]
    handler = context["handler"]
    input_schema = context["input_schema"]
    output_schema = context["output_schema"]
    built: list[Any] = context["built"]
    if name == "build":
        tool = registry.build(descriptor, handler, input_schema=input_schema, output_schema=output_schema)
        built.append(tool)
        return _tool_summary(tool)
    if name == "build_trusted":
        tool = registry.build_trusted_evaluator(
            descriptor, handler, input_schema=input_schema, output_schema=output_schema
        )
        built.append(tool)
        return _tool_summary(tool)
    if name == "register":
        tool = registry.register(descriptor, handler, input_schema=input_schema, output_schema=output_schema)
        built.append(tool)
        return _tool_summary(tool)
    if name == "register_trusted":
        tool = registry.register_trusted_evaluator(
            descriptor, handler, input_schema=input_schema, output_schema=output_schema
        )
        built.append(tool)
        return _tool_summary(tool)
    if name == "register_built":
        tool = registry.register_tool(built[int(operation["index"])])
        built.append(tool)
        return _tool_summary(tool)
    if name == "require":
        tool = registry.require(operation["tool_id"], operation.get("tool_version"))
        return _tool_summary(tool)
    if name == "latest":
        return _tool_summary(registry.latest(operation["tool_id"]))
    if name == "tools_requiring":
        return [_tool_summary(tool) for tool in registry.tools_requiring(operation["capability"])]
    if name == "require_capabilities":
        tool = registry.require_capabilities(
            operation["tool_id"],
            operation.get("tool_version"),
            list(operation.get("granted", ())),
        )
        return _tool_summary(tool)
    if name == "versions":
        return list(registry.versions(operation["tool_id"]))
    if name == "all_tools":
        return [_tool_summary(tool) for tool in registry.all_tools()]
    if name == "tool_ids":
        return list(registry.tool_ids())
    if name == "required_capabilities":
        return list(registry.required_capabilities())
    if name == "len":
        return len(registry)
    if name == "contains":
        return (operation["tool_id"], operation["tool_version"]) in registry
    if name == "tamper_handler":
        handler._descriptor = descriptor.model_copy(update={"version": "9.9.9"})
        return None
    if name == "verify_immutable":
        return _tool_summary(built[int(operation["index"])].verify_immutable())
    raise ValueError(f"unknown operation: {name!r}")


def registry_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools.registry import ToolRegistry

    # The first party handler pretends to live in the kernel package, so the
    # module has to be importable.
    import kala_agent.tools.simulator  # noqa: F401

    results = []
    for case in cases:
        input_schema = case["input_schema"]
        output_schema = case["output_schema"]
        descriptor = _registry_descriptor(case["descriptor"], input_schema, output_schema)
        handler_class = _FirstPartyHandler if case["handler"].get("first_party") else _ForeignHandler
        handler = handler_class(descriptor)
        registry = ToolRegistry()
        context = {
            "descriptor": descriptor,
            "handler": handler,
            "input_schema": input_schema,
            "output_schema": output_schema,
            "built": [],
        }
        outcomes = []
        for operation in case["operations"]:
            outcomes.append(
                _result(lambda operation=operation: _registry_operation(registry, context, operation))
            )
        results.append({"operations": outcomes})
    return results


def _simulator_request(arguments: Any, call_id: str, workspace: str) -> Any:
    from kala_agent.tools.protocol import PROTOCOL_VERSION, ToolEnvelopeKind, ToolRequestEnvelope

    return ToolRequestEnvelope(
        protocol_version=PROTOCOL_VERSION,
        kind=ToolEnvelopeKind.REQUEST,
        call_id=call_id,
        tool_id="mechanical.reference-simulator",
        tool_version="1.0.0",
        arguments=arguments,
        workspace=workspace,
        deadline_seconds=30.0,
        max_output_bytes=1_000_000,
        max_artifact_bytes=1_000_000,
        granted_capabilities=(),
    )


def _decode_nonfinite(value: Any) -> Any:
    """Decode the non finite marker of the corpus.

    A canonical JSON document cannot carry a non finite number, so the
    corpus marks one and both sides decode it into the same double.
    """
    if isinstance(value, dict):
        if len(value) == 1 and "__nonfinite__" in value:
            return {
                "nan": float("nan"),
                "inf": float("inf"),
                "-inf": float("-inf"),
            }[value["__nonfinite__"]]
        return {key: _decode_nonfinite(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_decode_nonfinite(item) for item in value]
    return value


def simulator_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    from kala_agent.tools import simulator as simulator_module
    from kala_agent.tools.registry import ToolRegistry

    workspace = os.environ["KALA_TOOLS_WORKSPACE_ROOT"]
    results = []
    for case in cases:
        name = case["op"]
        arguments = case.get("arguments", {})
        call_id = case.get("call_id", "call-1")

        def run(name=name, arguments=arguments, call_id=call_id) -> object:
            if name == "parse":
                return simulator_module.parse_parameters(
                    _decode_nonfinite(arguments)
                ).to_payload()
            if name == "metrics":
                parameters = simulator_module.parse_parameters(_decode_nonfinite(arguments))
                return simulator_module.compute_metrics(parameters)
            if name == "report":
                parameters = simulator_module.parse_parameters(arguments)
                metrics = simulator_module.compute_metrics(parameters)
                return simulator_module.report_payload(parameters, metrics, call_id=call_id)
            if name == "write_report":
                parameters = simulator_module.parse_parameters(arguments)
                metrics = simulator_module.compute_metrics(parameters)
                artifact, relative = simulator_module.write_report(
                    Path(workspace), parameters, metrics, call_id=call_id
                )
                return {"artifact": artifact.model_dump(mode="json"), "relative": relative}
            if name == "simulate":
                request = _simulator_request(arguments, call_id, workspace)
                return simulator_module.invoke(request).to_payload()
            if name == "evaluate":
                request = _simulator_request(arguments, call_id, workspace)
                return simulator_module.ReferenceEvaluatorHandler().invoke(request).to_payload()
            if name == "descriptor":
                return simulator_module.simulator_descriptor(
                    tool_id=case.get("tool_id", simulator_module.SIMULATOR_TOOL_ID),
                    version=case.get("version", simulator_module.SIMULATOR_TOOL_VERSION),
                ).model_dump(mode="json")
            if name == "evaluator_descriptor":
                return simulator_module.evaluator_descriptor(
                    tool_id=case.get("tool_id", simulator_module.EVALUATOR_TOOL_ID),
                    version=case.get("version", simulator_module.EVALUATOR_TOOL_VERSION),
                ).model_dump(mode="json")
            if name == "handler":
                handler = simulator_module.simulator_handler(
                    tool_id=case.get("tool_id", simulator_module.SIMULATOR_TOOL_ID),
                    version=case.get("version", simulator_module.SIMULATOR_TOOL_VERSION),
                )
                return handler.descriptor.model_dump(mode="json")
            if name == "register":
                registry = ToolRegistry()
                simulator = simulator_module.register_simulator(registry)
                evaluator = simulator_module.register_reference_evaluator(registry)
                return {
                    "evaluator": evaluator.model_dump(mode="json"),
                    "simulator": simulator.model_dump(mode="json"),
                    "tool_ids": list(registry.tool_ids()),
                }
            if name == "liar":
                request = _simulator_request(arguments, call_id, workspace)
                return simulator_module.liar_invoke(request).to_payload()
            if name == "unrecognized_code":
                request = _simulator_request(arguments, call_id, workspace)
                return simulator_module.unrecognized_code_invoke(request).to_payload()
            if name == "escape":
                request = _simulator_request(arguments, call_id, workspace)
                return simulator_module.escape_invoke(request).to_payload()
            raise ValueError(f"unknown operation: {name!r}")

        results.append(_result(run))
    return results


def _executor_case(case: dict[str, Any]) -> dict[str, object]:
    """Run one tool call through the repository's own tool executor."""
    from datetime import UTC, datetime

    from kala_agent.contracts import SideEffectLevel, ToolCall, ToolDescriptor
    from kala_agent.kernel.policy import PolicyEngine
    from kala_agent.tools.executor import IsolationMode, SandboxProfile, ToolExecutor
    from kala_agent.tools.protocol import (
        SubprocessToolHandler,
        canonical_json_digest,
        canonical_json_dumps,
        sha256_hex,
        strict_json_loads,
    )
    from kala_agent.tools.sandbox import IsolationProfile
    from kala_agent.tools.registry import ToolRegistry
    from kala_agent.tools.simulator import INPUT_SCHEMA, OUTPUT_SCHEMA

    run_directory = Path(case["run_directory"])
    tool_spec = case["tool"]
    input_schema = case.get("input_schema", INPUT_SCHEMA)
    output_schema = case.get("output_schema", OUTPUT_SCHEMA)
    registry = ToolRegistry()
    if tool_spec.get("kind") == "evaluator":
        from kala_agent.tools.simulator import register_reference_evaluator

        descriptor = register_reference_evaluator(registry)
    else:
        descriptor = ToolDescriptor(
            tool_id=tool_spec["tool_id"],
            version=tool_spec.get("version", "1.0.0"),
            name=tool_spec.get("name", "Differential tool"),
            description=tool_spec.get("description", "A tool used by the differential."),
            input_schema_digest=canonical_json_digest(input_schema),
            output_schema_digest=canonical_json_digest(output_schema),
            side_effect_level=SideEffectLevel(tool_spec["side_effect_level"]),
            required_capabilities=tuple(tool_spec.get("capabilities", ())),
            timeout_seconds=float(tool_spec.get("timeout_seconds", 10.0)),
            destructive=bool(tool_spec.get("destructive", False)),
            supports_structured_output=True,
            idempotent=True,
        )
        handler = SubprocessToolHandler(
            descriptor=descriptor, entry_point=tool_spec["entry_point"]
        )
        registry.register(
            descriptor, handler, input_schema=input_schema, output_schema=output_schema
        )
    profile_spec = case.get("profile", {})
    profile = SandboxProfile(
        max_output_bytes=int(profile_spec.get("max_output_bytes", 262_144)),
        max_argument_bytes=int(profile_spec.get("max_argument_bytes", 262_144)),
        max_artifact_bytes=int(profile_spec.get("max_artifact_bytes", 8_388_608)),
        default_timeout_seconds=float(profile_spec.get("default_timeout_seconds", 30.0)),
        max_workers=int(profile_spec.get("max_workers", 4)),
        max_queue_seconds=float(profile_spec.get("max_queue_seconds", 5.0)),
        allow_network=bool(profile_spec.get("allow_network", False)),
        allow_egress=bool(profile_spec.get("allow_egress", False)),
        allow_privileged_capabilities=bool(
            profile_spec.get("allow_privileged_capabilities", False)
        ),
        enforce_child_limits=bool(profile_spec.get("enforce_child_limits", True)),
        isolation_profile=IsolationProfile(profile_spec.get("isolation_profile", "portable")),
    )
    granted = tuple(case.get("granted", ()))
    policy = None
    if case.get("policy") is not None:
        policy = PolicyEngine(tuple(case["policy"]))
    executor = ToolExecutor(
        registry,
        run_directory=run_directory,
        granted_capabilities=granted,
        profile=profile,
        ledger=None,
        policy=policy,
        isolation=IsolationMode(case.get("isolation", "auto")),
        import_roots=[Path(item) for item in case.get("import_roots", ())],
    )
    call_spec = case["call"]
    arguments = call_spec.get("arguments", {})
    arguments_json = call_spec.get("arguments_json")
    if arguments_json is None:
        arguments_json = canonical_json_dumps(arguments)
    call = ToolCall(
        call_id=call_spec["call_id"],
        tool_id=call_spec.get("tool_id", descriptor.tool_id),
        tool_version=descriptor.version,
        arguments_json=arguments_json,
        requested_capabilities=tuple(call_spec.get("requested_capabilities", ())),
        side_effect_level=SideEffectLevel(call_spec["side_effect_level"]),
        timeout_seconds=float(call_spec.get("timeout_seconds", 20.0)),
        issued_at=datetime(2026, 5, 6, 7, 8, 9, tzinfo=UTC),
    )
    try:
        request = executor.policy_request(call)
    except Exception:  # noqa: BLE001 - an unknown tool has no approval binding
        request = None
    if case.get("approve") and policy is not None and request is not None:
        policy.approve(request)
    cancel = None
    if case.get("cancel"):
        import threading

        cancel = threading.Event()
        cancel.set()
    result = executor.execute(call, cancel=cancel)
    reports = [report.to_payload() for report in executor.reports]
    return {
        "status": result.status.value,
        "error_code": result.error_code,
        "error_message": result.error_message,
        "result_json": result.result_json,
        "artifacts": [artifact.model_dump(mode="json") for artifact in result.artifacts],
        "reports": reports,
        "digests": {
            "arguments_digest": sha256_hex(
                canonical_json_dumps(strict_json_loads(call.arguments_json))
            ),
            "state_digest": None if request is None else request.state_digest,
            "action_digest": None if request is None else request.action_digest,
        },
        "probe": {
            "available": executor.sandbox_probe.available,
            "controls": list(executor.sandbox_probe.controls),
            "platform": executor.sandbox_probe.platform,
        },
        "enforced_controls": list(executor.enforced_controls),
    }


def executor_mode(cases: list[dict[str, Any]]) -> list[dict[str, object]]:
    results = []
    for case in cases:
        results.append(_result(lambda case=case: _executor_case(case)))
    return results


MODES: dict[str, Callable[[list[dict[str, Any]]], list[dict[str, object]]]] = {
    "parse": parse_mode,
    "encode": encode_mode,
    "validate": validate_mode,
    "check": check_mode,
    "pattern": pattern_mode,
    "request": request_mode,
    "response": response_mode,
    "workspace": workspace_mode,
    "diagnostics": diagnostics_mode,
    "summary": summary_mode,
    "registry": registry_mode,
    "simulator": simulator_mode,
    "executor": executor_mode,
}


def main() -> int:
    _add_repo_source()
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    handler = MODES.get(mode)
    if handler is None:
        print(f"unknown mode: {mode!r}", file=sys.stderr)
        return 2
    cases = json.load(sys.stdin)
    output = handler(cases)
    json.dump(output, sys.stdout, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
