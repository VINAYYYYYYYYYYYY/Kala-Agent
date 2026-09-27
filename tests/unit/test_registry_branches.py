from __future__ import annotations

from collections.abc import Mapping
from typing import Any, cast

import pytest

from kala_agent.contracts import SideEffectLevel, ToolDescriptor, ToolRunStatus
from kala_agent.tools import registry as registry_module
from kala_agent.tools.protocol import (
    JSONValue,
    SchemaValidationError,
    SubprocessToolHandler,
    ToolEnvelopeKind,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    canonical_json_digest,
    canonical_json_dumps,
)
from kala_agent.tools.registry import (
    MAX_REGISTERED_TOOLS,
    DuplicateToolRegistrationError,
    RegisteredTool,
    SchemaDigestMismatchError,
    ToolCapabilityError,
    ToolRegistrationError,
    ToolRegistry,
    ToolRegistryError,
    ToolVersionNotFoundError,
    UnknownToolError,
    _deep_freeze,
    _frozen_schema,
    _handler_module,
    _is_first_party,
    _is_pure_evaluator,
    _version_key,
)

INPUT_SCHEMA: dict[str, object] = {
    "additionalProperties": False,
    "properties": {"operation": {"enum": ["beam_volume"], "type": "string"}},
    "required": ["operation"],
    "title": "tool.input",
    "type": "object",
}
OUTPUT_SCHEMA: dict[str, object] = {
    "additionalProperties": False,
    "properties": {"ok": {"type": "boolean"}},
    "required": ["ok"],
    "title": "tool.output",
    "type": "object",
}


class _Evaluator:
    def __init__(self, descriptor: ToolDescriptor) -> None:
        self._descriptor = descriptor

    @property
    def descriptor(self) -> ToolDescriptor:
        return self._descriptor

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope:
        return ToolResponseEnvelope.succeeded_envelope(
            call_id=request.call_id,
            tool_id=request.tool_id,
            tool_version=request.tool_version,
            result=cast("dict[str, JSONValue]", {"ok": True}),
        )


class _SideEffectingHandler(_Evaluator):
    pass


def _descriptor(
    tool_id: str = "mechanical.reference-evaluator",
    version: str = "1.0.0",
    *,
    side_effect_level: SideEffectLevel = SideEffectLevel.NONE,
    capabilities: tuple[str, ...] = (),
    destructive: bool = False,
    timeout_seconds: float = 10.0,
    input_schema: Mapping[str, object] | None = None,
    output_schema: Mapping[str, object] | None = None,
) -> ToolDescriptor:
    return ToolDescriptor(
        tool_id=tool_id,
        version=version,
        name="Test tool",
        description="A test tool.",
        input_schema_digest=canonical_json_digest(input_schema or INPUT_SCHEMA),
        output_schema_digest=canonical_json_digest(output_schema or OUTPUT_SCHEMA),
        side_effect_level=side_effect_level,
        required_capabilities=capabilities,
        timeout_seconds=timeout_seconds,
        destructive=destructive,
        supports_structured_output=True,
        idempotent=True,
    )


def _registered(
    *,
    tool_id: str = "mechanical.reference-evaluator",
    version: str = "1.0.0",
    trusted: bool = False,
    capabilities: tuple[str, ...] = (),
) -> RegisteredTool:
    registry = ToolRegistry()
    descriptor = _descriptor(tool_id, version, capabilities=capabilities)
    handler = _Evaluator(descriptor)
    if trusted:
        return registry.build_trusted_evaluator(
            descriptor,
            _FirstPartyEvaluator(descriptor),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )
    return registry.build(
        descriptor,
        handler,
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )


def test_version_key_orders_numeric_segments_numerically() -> None:
    assert _version_key("1.0.0") < _version_key("1.0.10")
    assert _version_key("2.0.0") > _version_key("1.99.99")
    assert _version_key("1.0.0-alpha") > _version_key("1.0.0")
    assert _version_key("1.0") < _version_key("1.0.0")


def test_handler_module_reports_the_defining_module() -> None:
    assert _handler_module(_Evaluator(_descriptor())) == __name__


def test_first_party_detection_rejects_foreign_handlers() -> None:
    assert _is_first_party(_Evaluator(_descriptor())) is False
    assert _is_first_party(object()) is False


def test_first_party_detection_requires_a_source_file(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setitem(
        __import__("sys").modules,
        __name__,
        type(__name__, (), {"__file__": None}),
    )
    assert _is_first_party(_Evaluator(_descriptor())) is False


def test_first_party_detection_rejects_sources_outside_the_package() -> None:
    class _Elsewhere(_Evaluator):
        pass

    _Elsewhere.__module__ = "kala_agent.fake_module"
    assert _is_first_party(_Elsewhere(_descriptor())) is False


def test_pure_evaluator_requires_no_side_effects() -> None:
    assert _is_pure_evaluator(_descriptor()) is True
    assert (
        _is_pure_evaluator(
            _descriptor(
                side_effect_level=SideEffectLevel.NETWORK,
                destructive=True,
                capabilities=("artifact.write",),
            )
        )
        is False
    )
    assert (
        _is_pure_evaluator(
            _descriptor(
                side_effect_level=SideEffectLevel.LOCAL_WRITE,
                capabilities=("artifact.write",),
            )
        )
        is False
    )
    assert _is_pure_evaluator(_descriptor(capabilities=("artifact.read",))) is True


def test_deep_freeze_converts_containers() -> None:
    frozen = _deep_freeze({"a": [1, {"b": 2}]})
    assert isinstance(frozen, Mapping)
    assert canonical_json_dumps(frozen) == '{"a":[1,{"b":2}]}'
    assert cast("object", _deep_freeze((1, 2))) == (1, 2)
    assert _deep_freeze("text") == "text"


def test_deep_freeze_rejects_non_string_keys() -> None:
    with pytest.raises(ToolRegistrationError, match="keys must be strings"):
        _deep_freeze(cast("Mapping[object, object]", {7: 1}))


def test_frozen_schema_freezes_the_top_level() -> None:
    frozen = _frozen_schema(INPUT_SCHEMA)
    with pytest.raises(TypeError):
        cast("dict[str, object]", frozen)["extra"] = 1


def test_registered_tool_exposes_descriptor_fields() -> None:
    tool = _registered(capabilities=("artifact.read",))
    assert tool.tool_id == "mechanical.reference-evaluator"
    assert tool.version == "1.0.0"
    assert tool.required_capabilities == ("artifact.read",)
    assert tool.side_effect_level is SideEffectLevel.NONE
    assert tool.timeout_seconds == 10.0
    assert tool.trusted_in_process is False
    assert tool.descriptor_digest == canonical_json_digest(tool.descriptor.model_dump(mode="json"))


def test_registered_tool_rejects_bad_construction() -> None:
    descriptor = _descriptor()
    handler = _Evaluator(descriptor)
    with pytest.raises(ToolRegistrationError, match="descriptor must be"):
        RegisteredTool(
            descriptor=cast(Any, object()),
            handler=handler,
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
        )
    with pytest.raises(ToolRegistrationError, match="must implement the tool protocol"):
        RegisteredTool(
            descriptor=descriptor,
            handler=cast(Any, object()),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
        )
    with pytest.raises(ToolRegistrationError, match="must be a boolean"):
        RegisteredTool(
            descriptor=descriptor,
            handler=handler,
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
            trusted_in_process=cast(Any, "yes"),
        )
    with pytest.raises(ToolRegistrationError, match="schemas must be JSON objects"):
        RegisteredTool(
            descriptor=descriptor,
            handler=handler,
            input_schema=cast(Any, ["not-a-schema"]),
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
        )


def test_registered_tool_requires_a_matching_handler_descriptor() -> None:
    descriptor = _descriptor()
    with pytest.raises(ToolRegistrationError, match="handler descriptor does not match"):
        RegisteredTool(
            descriptor=descriptor,
            handler=_Evaluator(_descriptor(version="9.9.9")),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
        )


def test_registered_tool_requires_a_typed_handler_descriptor() -> None:
    descriptor = _descriptor()
    with pytest.raises(ToolRegistrationError, match="handler must expose a ToolDescriptor"):
        RegisteredTool(
            descriptor=descriptor,
            handler=cast(Any, _UntypedDescriptorHandler()),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
        )


def test_registered_tool_enforces_schema_digests() -> None:
    descriptor = _descriptor()
    handler = _Evaluator(descriptor)
    with pytest.raises(SchemaDigestMismatchError, match="input schema digest"):
        RegisteredTool(
            descriptor=descriptor,
            handler=handler,
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest="0" * 64,
            output_schema_digest=descriptor.output_schema_digest,
        )
    with pytest.raises(SchemaDigestMismatchError, match="output schema digest"):
        RegisteredTool(
            descriptor=descriptor,
            handler=handler,
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest="0" * 64,
        )


def test_registered_tool_refuses_subprocess_trusted_handlers() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor()
    with pytest.raises(ToolRegistrationError, match="subprocess handler cannot run inside"):
        registry.build_trusted_evaluator(
            descriptor,
            SubprocessToolHandler(descriptor=descriptor, entry_point="module:invoke"),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )


def test_registered_tool_refuses_impure_trusted_evaluators() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor(
        side_effect_level=SideEffectLevel.NETWORK,
        destructive=True,
        capabilities=("artifact.write",),
    )
    with pytest.raises(ToolRegistrationError, match="non-destructive tool"):
        registry.build_trusted_evaluator(
            descriptor,
            _FirstPartyEvaluator(descriptor),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )


def test_registered_tool_refuses_foreign_trusted_evaluators() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor()
    with pytest.raises(ToolRegistrationError, match="kernel package"):
        registry.build_trusted_evaluator(
            descriptor,
            _Evaluator(descriptor),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )


def test_registered_tool_validates_arguments_and_results() -> None:
    tool = _registered()
    assert tool.validate_arguments({"operation": "beam_volume"}) == ('{"operation":"beam_volume"}')
    assert tool.validate_result({"ok": True}) == '{"ok":true}'
    with pytest.raises(SchemaValidationError):
        tool.validate_arguments({})
    with pytest.raises(SchemaValidationError):
        tool.validate_result({})


def test_registered_tool_reports_capability_authorization() -> None:
    tool = _registered(capabilities=("artifact.read", "tool.execute"))
    assert tool.is_authorized_for(["artifact.read", "tool.execute"]) is True
    assert tool.is_authorized_for(["artifact.read"]) is False


def test_verify_immutable_detects_a_mutated_descriptor() -> None:
    tool = _registered()
    cast("Any", tool.handler)._descriptor = _descriptor(version="9.9.9")
    with pytest.raises(ToolRegistryError, match="modified after registration"):
        tool.verify_immutable()


def test_verify_immutable_detects_a_mutated_input_schema() -> None:
    tool = _registered()
    object.__setattr__(tool, "input_schema", {"tampered": True})
    with pytest.raises(ToolRegistryError, match="input schema"):
        tool.verify_immutable()


def test_verify_immutable_detects_a_mutated_output_schema() -> None:
    tool = _registered()
    object.__setattr__(tool, "output_schema", {"tampered": True})
    with pytest.raises(ToolRegistryError, match="output schema"):
        tool.verify_immutable()


def test_verify_immutable_rechecks_trusted_evaluators(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    tool = _registered(trusted=True)
    monkeypatch.setattr(
        registry_module,
        "_is_pure_evaluator",
        lambda _descriptor: False,
    )
    with pytest.raises(ToolRegistrationError, match="non-destructive tool"):
        tool.verify_immutable()


def test_verify_immutable_passes_for_an_untouched_tool() -> None:
    assert _registered().verify_immutable() is not None
    assert _registered(trusted=True).verify_immutable() is not None


def test_registry_build_rejects_bad_descriptors() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor()
    with pytest.raises(ToolRegistrationError, match="descriptor must be"):
        registry.build(
            cast(Any, object()),
            _Evaluator(descriptor),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )
    with pytest.raises(ToolRegistrationError, match="declare its required capabilities"):
        registry.build(
            _descriptor(side_effect_level=SideEffectLevel.LOCAL_WRITE),
            _Evaluator(_descriptor(side_effect_level=SideEffectLevel.LOCAL_WRITE)),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )
    negative = ToolDescriptor.model_construct(
        **{
            **_descriptor().model_dump(),
            "timeout_seconds": -1.0,
        }
    )
    with pytest.raises(ToolRegistrationError, match="nonnegative"):
        registry.build(
            negative,
            _Evaluator(negative),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )


def test_registry_build_reports_malformed_schemas() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor()
    with pytest.raises(ToolRegistrationError, match="input_schema"):
        registry.build(
            descriptor,
            _Evaluator(descriptor),
            input_schema=cast("Mapping[str, object]", {"type": "string"}),
            output_schema=OUTPUT_SCHEMA,
        )
    with pytest.raises(ToolRegistrationError, match="output_schema"):
        registry.build(
            descriptor,
            _Evaluator(descriptor),
            input_schema=INPUT_SCHEMA,
            output_schema=cast("Mapping[str, object]", {"type": "string"}),
        )


def test_registry_build_trusted_evaluator_reraises_registration_errors() -> None:
    registry = ToolRegistry()
    with pytest.raises(ToolRegistrationError):
        registry.build_trusted_evaluator(
            cast(Any, object()),
            _Evaluator(_descriptor()),
            input_schema=INPUT_SCHEMA,
            output_schema=OUTPUT_SCHEMA,
        )


def test_registry_rejects_non_registered_tools_on_construction() -> None:
    with pytest.raises(TypeError, match="RegisteredTool"):
        ToolRegistry(cast(Any, [object()]))


def test_registry_accepts_prebuilt_tools() -> None:
    tool = _registered()
    registry = ToolRegistry([tool])
    assert len(registry) == 1
    assert registry.require(tool.tool_id, tool.version) is tool


def test_registry_refuses_duplicate_registrations() -> None:
    registry = ToolRegistry()
    tool = _registered()
    registry.register_tool(tool)
    with pytest.raises(DuplicateToolRegistrationError, match="already registered"):
        registry.register_tool(tool)


def test_registry_enforces_the_registration_capacity(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    registry = ToolRegistry()
    tool = _registered()
    monkeypatch.setattr(registry_module, "MAX_REGISTERED_TOOLS", 1)
    assert MAX_REGISTERED_TOOLS > 1
    registry.register_tool(tool)
    with pytest.raises(ToolRegistrationError, match="registry is full"):
        registry.register_tool(_registered(version="2.0.0"))


def test_registry_require_reports_unknown_tools() -> None:
    registry = ToolRegistry()
    with pytest.raises(UnknownToolError, match="is not registered"):
        registry.require("mechanical.reference-evaluator")


def test_registry_require_reports_unknown_versions() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered())
    with pytest.raises(ToolVersionNotFoundError, match="no registered version"):
        registry.require("mechanical.reference-evaluator", "9.9.9")


def test_registry_version_lookup_is_a_subclass_of_unknown_tool() -> None:
    assert issubclass(ToolVersionNotFoundError, UnknownToolError)


def test_registry_require_without_a_version_returns_the_latest() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered(version="1.9.0"))
    latest = _registered(version="1.10.0")
    registry.register_tool(latest)
    assert registry.require("mechanical.reference-evaluator") is latest
    assert registry.latest("mechanical.reference-evaluator") is latest


def test_registry_latest_reports_unknown_tools() -> None:
    with pytest.raises(UnknownToolError, match="is not registered"):
        ToolRegistry().latest("mechanical.reference-evaluator")


def test_registry_require_detects_a_mutated_descriptor(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    registry = ToolRegistry()
    tool = _registered()
    registry.register_tool(tool)
    monkeypatch.setattr(
        registry_module,
        "canonical_json_digest",
        lambda _value: "0" * 64,
    )
    with pytest.raises(ToolRegistryError, match="was modified"):
        registry.require(tool.tool_id, tool.version)


def test_registry_require_detects_a_mutated_handler_descriptor() -> None:
    registry = ToolRegistry()
    tool = _registered()
    registry.register_tool(tool)
    cast("Any", tool.handler)._descriptor = _descriptor(version="9.9.9")
    with pytest.raises(ToolRegistryError, match="modified after registration"):
        registry.require(tool.tool_id, tool.version)


def test_registry_tools_requiring_filters_and_sorts() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered(version="1.0.0", capabilities=("artifact.read",)))
    registry.register_tool(_registered(version="1.10.0", capabilities=("artifact.read",)))
    registry.register_tool(
        _registered(tool_id="other.tool", capabilities=("artifact.write",)),
    )
    selected = registry.tools_requiring("artifact.read")
    assert [tool.version for tool in selected] == ["1.0.0", "1.10.0"]
    assert registry.tools_requiring("tool.execute") == ()
    with pytest.raises(ToolRegistryError, match="contains unsafe characters"):
        registry.tools_requiring("artifact read")


def test_registry_require_capabilities_reports_missing_grants() -> None:
    registry = ToolRegistry()
    tool = _registered(capabilities=("artifact.read", "tool.execute"))
    registry.register_tool(tool)
    assert (
        registry.require_capabilities(tool.tool_id, None, ["artifact.read", "tool.execute"]) is tool
    )
    with pytest.raises(ToolCapabilityError) as error:
        registry.require_capabilities(tool.tool_id, None, ["artifact.read"])
    assert error.value.missing_capabilities == ("tool.execute",)


def test_registry_require_capabilities_validates_identifiers() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered())
    with pytest.raises(ToolRegistryError, match="granted capability"):
        registry.require_capabilities("mechanical.reference-evaluator", None, cast(Any, [7]))


def test_registry_versions_lists_every_registered_version() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered(version="1.9.0"))
    registry.register_tool(_registered(version="1.10.0"))
    assert registry.versions("mechanical.reference-evaluator") == ("1.9.0", "1.10.0")
    assert registry.versions("other.tool") == ()
    with pytest.raises(ToolRegistryError, match="must be a string"):
        registry.versions(cast(str, 7))


def test_registry_enumerates_tools_and_capabilities() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered(version="1.0.0", capabilities=("artifact.read",)))
    registry.register_tool(
        _registered(tool_id="other.tool", capabilities=("artifact.write",)),
    )
    assert registry.tool_ids() == ("mechanical.reference-evaluator", "other.tool")
    assert registry.required_capabilities() == ("artifact.read", "artifact.write")
    assert [tool.tool_id for tool in registry.all_tools()] == [
        "mechanical.reference-evaluator",
        "other.tool",
    ]
    assert [tool.tool_id for tool in registry] == [
        "mechanical.reference-evaluator",
        "other.tool",
    ]


def test_registry_membership_accepts_only_identifier_pairs() -> None:
    registry = ToolRegistry()
    registry.register_tool(_registered())
    assert ("mechanical.reference-evaluator", "1.0.0") in registry
    assert ("mechanical.reference-evaluator", "9.9.9") not in registry
    assert "mechanical.reference-evaluator" not in registry
    assert (7, "1.0.0") not in registry
    assert ("a", "b", "c") not in registry
    assert (("a", "b"), "c") not in registry


def test_registry_identifier_and_version_validation() -> None:
    registry = ToolRegistry()
    with pytest.raises(ToolRegistryError, match="must be a string"):
        registry.require(cast(str, 7))
    with pytest.raises(ToolRegistryError, match="nonempty and bounded"):
        registry.require("   ")
    with pytest.raises(ToolRegistryError, match="nonempty and bounded"):
        registry.require("x" * 129)
    with pytest.raises(ToolRegistryError, match="unsafe characters"):
        registry.require("has space")
    with pytest.raises(ToolRegistryError, match="unsafe characters"):
        registry.require("has\x00null")
    with pytest.raises(ToolRegistryError, match="tool_version must be a string"):
        registry.require("mechanical.reference-evaluator", cast(str, 7))
    with pytest.raises(ToolRegistryError, match="nonempty and bounded"):
        registry.require("mechanical.reference-evaluator", "x" * 65)
    with pytest.raises(ToolRegistryError, match="tool_version contains unsafe"):
        registry.require("mechanical.reference-evaluator", "has space")


def test_registry_trims_identifiers_before_lookup() -> None:
    registry = ToolRegistry()
    tool = _registered()
    registry.register_tool(tool)
    assert registry.require("  mechanical.reference-evaluator  ") is tool
    assert registry.require("mechanical.reference-evaluator", " 1.0.0 ") is tool


def test_register_trusted_evaluator_registers_the_tool() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor()
    tool = registry.register_trusted_evaluator(
        descriptor,
        cast(Any, _FirstPartyEvaluator(descriptor)),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    assert tool.trusted_in_process is True
    assert registry.require(descriptor.tool_id, descriptor.version) is tool


def test_register_registers_a_built_tool() -> None:
    registry = ToolRegistry()
    descriptor = _descriptor()
    tool = registry.register(
        descriptor,
        _Evaluator(descriptor),
        input_schema=INPUT_SCHEMA,
        output_schema=OUTPUT_SCHEMA,
    )
    assert registry.require(descriptor.tool_id, descriptor.version) is tool


def test_evaluator_invoke_returns_a_successful_envelope() -> None:
    descriptor = _descriptor()
    handler = _Evaluator(descriptor)
    request = ToolRequestEnvelope(
        protocol_version="1.0",
        kind=ToolEnvelopeKind.REQUEST,
        call_id="call-1",
        tool_id=descriptor.tool_id,
        tool_version=descriptor.version,
        arguments=cast("dict[str, JSONValue]", {"operation": "beam_volume"}),
        workspace="/tmp/workspace",
        deadline_seconds=5.0,
        max_output_bytes=1_000,
        max_artifact_bytes=1_000,
        granted_capabilities=(),
    )
    response = handler.invoke(request)
    assert response.status is ToolRunStatus.SUCCEEDED
    assert response.result == {"ok": True}


class _FirstPartyEvaluator(_Evaluator):
    pass


_FirstPartyEvaluator.__module__ = "kala_agent.tools.registry"


class _UntypedDescriptorHandler:
    descriptor = "not-a-descriptor"

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope:
        raise NotImplementedError
