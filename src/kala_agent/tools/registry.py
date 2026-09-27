from __future__ import annotations

import sys
from collections.abc import Iterable, Iterator, Mapping, Sequence
from dataclasses import dataclass, replace
from pathlib import Path
from threading import RLock
from types import MappingProxyType
from typing import Final, cast

from kala_agent.contracts import (
    SideEffectLevel,
    ToolDescriptor,
)
from kala_agent.tools.protocol import (
    JSONValue,
    SchemaValidationError,
    SubprocessToolHandler,
    ToolHandler,
    canonical_json_digest,
    canonical_json_dumps,
    check_object_schema,
    validate_against_schema,
)

MAX_REGISTERED_TOOLS: Final[int] = 512
FIRST_PARTY_PACKAGE: Final[str] = "kala_agent"
_SIDE_EFFECT_CAPABILITY_REQUIRED: Final[frozenset[SideEffectLevel]] = frozenset(
    {
        SideEffectLevel.LOCAL_WRITE,
        SideEffectLevel.NETWORK,
        SideEffectLevel.EGRESS,
    }
)
_TRUSTED_EVALUATOR_CAPABILITIES: Final[frozenset[str]] = frozenset(
    {
        "egress",
        "export",
        "network",
        "network.access",
        "network.egress",
        "network.internet",
        "process.spawn",
        "shell.execute",
        "system.admin",
    }
)


class ToolRegistryError(RuntimeError):
    pass


class ToolRegistrationError(ToolRegistryError, ValueError):
    pass


class DuplicateToolRegistrationError(ToolRegistrationError):
    pass


class SchemaDigestMismatchError(ToolRegistrationError):
    pass


class UnknownToolError(ToolRegistryError):
    pass


class ToolVersionNotFoundError(UnknownToolError):
    pass


class ToolCapabilityError(ToolRegistryError):
    def __init__(self, message: str, *, missing: Sequence[str] = ()) -> None:
        self.missing_capabilities = tuple(sorted(missing))
        super().__init__(message)


def _handler_module(handler: object) -> str:
    return str(getattr(type(handler), "__module__", ""))


def _is_first_party(handler: object) -> bool:
    """Report whether a handler is implemented by the shipped kernel package.

    A handler defined outside the kernel cannot be inspected, reviewed, or
    bounded by this repository, so it is never eligible to run inside the
    kernel process.
    """
    module = _handler_module(handler)
    if not module.startswith(f"{FIRST_PARTY_PACKAGE}."):
        return False
    source = getattr(sys.modules.get(module), "__file__", None)
    if not isinstance(source, str) or not source:
        return False
    try:
        resolved = Path(source).resolve(strict=False)
    except OSError:  # pragma: no cover - defensive
        return False
    root = Path(__file__).resolve().parent.parent
    return resolved.is_relative_to(root)


def _is_pure_evaluator(tool_descriptor: ToolDescriptor) -> bool:
    return (
        tool_descriptor.side_effect_level is SideEffectLevel.NONE
        and not tool_descriptor.destructive
        and not set(tool_descriptor.required_capabilities) & _TRUSTED_EVALUATOR_CAPABILITIES
    )


def _version_key(version: str) -> tuple[tuple[int, int, str], ...]:
    parts: list[tuple[int, int, str]] = []
    for segment in version.split("."):
        if segment.isdigit():
            parts.append((0, int(segment), ""))
        else:
            parts.append((1, 0, segment))
    return tuple(parts)


def _deep_freeze(value: object) -> JSONValue:
    if isinstance(value, Mapping):
        frozen: dict[str, JSONValue] = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise ToolRegistrationError("schema object keys must be strings")
            frozen[key] = _deep_freeze(item)
        return cast(JSONValue, MappingProxyType(frozen))
    if isinstance(value, (list, tuple)):
        return cast(JSONValue, tuple(_deep_freeze(item) for item in value))
    return cast(JSONValue, value)


def _frozen_schema(schema: Mapping[str, object]) -> Mapping[str, JSONValue]:
    return cast(Mapping[str, JSONValue], _deep_freeze(dict(schema)))


@dataclass(frozen=True, slots=True)
class RegisteredTool:
    descriptor: ToolDescriptor
    handler: ToolHandler
    input_schema: Mapping[str, object]
    output_schema: Mapping[str, object]
    input_schema_digest: str
    output_schema_digest: str
    descriptor_digest: str = ""
    trusted_in_process: bool = False

    def __post_init__(self) -> None:
        if not isinstance(self.descriptor, ToolDescriptor):
            raise ToolRegistrationError("descriptor must be a ToolDescriptor")
        if not isinstance(self.handler, ToolHandler):
            raise ToolRegistrationError("handler must implement the tool protocol")
        if not isinstance(self.trusted_in_process, bool):
            raise ToolRegistrationError("trusted_in_process must be a boolean")
        handler_descriptor = getattr(self.handler, "descriptor", None)
        if not isinstance(handler_descriptor, ToolDescriptor):
            raise ToolRegistrationError("handler must expose a ToolDescriptor")
        if handler_descriptor != self.descriptor:
            raise ToolRegistrationError(
                "handler descriptor does not match the registered descriptor"
            )
        if not isinstance(self.input_schema, Mapping) or not isinstance(
            self.output_schema, Mapping
        ):
            raise ToolRegistrationError("schemas must be JSON objects")
        object.__setattr__(self, "input_schema", _frozen_schema(self.input_schema))
        object.__setattr__(self, "output_schema", _frozen_schema(self.output_schema))
        expected_input = canonical_json_digest(self.input_schema)
        expected_output = canonical_json_digest(self.output_schema)
        if self.input_schema_digest != expected_input:
            raise SchemaDigestMismatchError("input schema digest does not match the descriptor")
        if self.output_schema_digest != expected_output:
            raise SchemaDigestMismatchError("output schema digest does not match the descriptor")
        if self.trusted_in_process:
            self._require_trusted_evaluator()
        object.__setattr__(
            self,
            "descriptor_digest",
            canonical_json_digest(self.descriptor.model_dump(mode="json")),
        )

    def _require_trusted_evaluator(self) -> None:
        """Refuse to mark anything but a first-party pure evaluator as trusted."""
        if isinstance(self.handler, SubprocessToolHandler):
            raise ToolRegistrationError("a subprocess handler cannot run inside the kernel process")
        if not _is_pure_evaluator(self.descriptor):
            raise ToolRegistrationError(
                "a trusted in-process evaluator must be a non-destructive tool with no side "
                "effects and no privileged capability"
            )
        if not _is_first_party(self.handler):
            raise ToolRegistrationError(
                "a trusted in-process evaluator must be implemented by the kernel package"
            )

    @property
    def tool_id(self) -> str:
        return self.descriptor.tool_id

    @property
    def version(self) -> str:
        return self.descriptor.version

    @property
    def required_capabilities(self) -> tuple[str, ...]:
        return self.descriptor.required_capabilities

    @property
    def side_effect_level(self) -> SideEffectLevel:
        return self.descriptor.side_effect_level

    @property
    def timeout_seconds(self) -> float:
        return self.descriptor.timeout_seconds

    def validate_arguments(self, arguments: Mapping[str, object]) -> str:
        validate_against_schema(arguments, self.input_schema, path="$")
        return canonical_json_dumps(arguments)

    def validate_result(self, result: Mapping[str, object]) -> str:
        validate_against_schema(result, self.output_schema, path="$")
        return canonical_json_dumps(result)

    def is_authorized_for(self, granted_capabilities: Iterable[str]) -> bool:
        granted = frozenset(granted_capabilities)
        return set(self.required_capabilities).issubset(granted)

    def verify_immutable(self) -> RegisteredTool:
        if self.handler.descriptor != self.descriptor:
            raise ToolRegistryError(
                f"registered descriptor for {self.tool_id} was modified after registration"
            )
        if canonical_json_digest(self.input_schema) != self.input_schema_digest:
            raise ToolRegistryError(f"registered input schema for {self.tool_id} was modified")
        if canonical_json_digest(self.output_schema) != self.output_schema_digest:
            raise ToolRegistryError(f"registered output schema for {self.tool_id} was modified")
        if self.trusted_in_process:
            self._require_trusted_evaluator()
        return self


class ToolRegistry:
    def __init__(self, tools: Iterable[RegisteredTool] = ()) -> None:
        self._tools: dict[tuple[str, str], RegisteredTool] = {}
        self._lock = RLock()
        for tool in tools:
            if not isinstance(tool, RegisteredTool):
                raise TypeError("tools must be RegisteredTool values")
            self.register_tool(tool)

    def build(
        self,
        descriptor: ToolDescriptor,
        handler: ToolHandler,
        *,
        input_schema: Mapping[str, object],
        output_schema: Mapping[str, object],
    ) -> RegisteredTool:
        if not isinstance(descriptor, ToolDescriptor):
            raise ToolRegistrationError("descriptor must be a ToolDescriptor")
        if descriptor.side_effect_level in _SIDE_EFFECT_CAPABILITY_REQUIRED and not (
            descriptor.required_capabilities
        ):
            raise ToolRegistrationError(
                "a tool with side effects must declare its required capabilities"
            )
        if descriptor.timeout_seconds < 0.0:
            raise ToolRegistrationError("timeout_seconds must be nonnegative")
        try:
            check_object_schema(input_schema, label="input_schema")
            check_object_schema(output_schema, label="output_schema")
        except SchemaValidationError as error:
            raise ToolRegistrationError(str(error)) from error
        return RegisteredTool(
            descriptor=descriptor,
            handler=handler,
            input_schema=input_schema,
            output_schema=output_schema,
            input_schema_digest=descriptor.input_schema_digest,
            output_schema_digest=descriptor.output_schema_digest,
        )

    def build_trusted_evaluator(
        self,
        descriptor: ToolDescriptor,
        handler: ToolHandler,
        *,
        input_schema: Mapping[str, object],
        output_schema: Mapping[str, object],
    ) -> RegisteredTool:
        """Build a registration that is allowed to run inside the kernel process.

        The privilege is granted only to a first-party, side-effect free
        evaluator. Registration itself is not a grant: the executor also has to
        be configured with the explicit trusted in-process isolation mode.
        """
        try:
            tool = self.build(
                descriptor,
                handler,
                input_schema=input_schema,
                output_schema=output_schema,
            )
        except ToolRegistrationError:
            raise
        return replace(tool, trusted_in_process=True)

    def register(
        self,
        descriptor: ToolDescriptor,
        handler: ToolHandler,
        *,
        input_schema: Mapping[str, object],
        output_schema: Mapping[str, object],
    ) -> RegisteredTool:
        return self.register_tool(
            self.build(
                descriptor,
                handler,
                input_schema=input_schema,
                output_schema=output_schema,
            )
        )

    def register_trusted_evaluator(
        self,
        descriptor: ToolDescriptor,
        handler: ToolHandler,
        *,
        input_schema: Mapping[str, object],
        output_schema: Mapping[str, object],
    ) -> RegisteredTool:
        return self.register_tool(
            self.build_trusted_evaluator(
                descriptor,
                handler,
                input_schema=input_schema,
                output_schema=output_schema,
            )
        )

    def register_tool(self, tool: RegisteredTool) -> RegisteredTool:
        key = (tool.tool_id, tool.version)
        with self._lock:
            if key in self._tools:
                raise DuplicateToolRegistrationError(
                    f"tool {tool.tool_id} version {tool.version} is already registered"
                )
            if len(self._tools) >= MAX_REGISTERED_TOOLS:
                raise ToolRegistrationError("the tool registry is full")
            self._tools[key] = tool
            return tool

    def require(self, tool_id: str, tool_version: str | None = None) -> RegisteredTool:
        normalized_id = self._identifier(tool_id, "tool_id")
        with self._lock:
            if tool_version is None:
                return self.latest(normalized_id)
            normalized_version = self._version(tool_version)
            tool = self._tools.get((normalized_id, normalized_version))
            known_versions = {
                version for identifier, version in self._tools if identifier == normalized_id
            }
        if tool is None:
            if not known_versions:
                raise UnknownToolError(f"tool {normalized_id} is not registered")
            raise ToolVersionNotFoundError(
                f"tool {normalized_id} has no registered version {normalized_version}"
            )
        self._verify_unmodified(tool)
        return tool

    def latest(self, tool_id: str) -> RegisteredTool:
        normalized_id = self._identifier(tool_id, "tool_id")
        with self._lock:
            candidates = [tool for tool in self._tools.values() if tool.tool_id == normalized_id]
        if not candidates:
            raise UnknownToolError(f"tool {normalized_id} is not registered")
        selected = max(candidates, key=lambda tool: _version_key(tool.version))
        self._verify_unmodified(selected)
        return selected

    def tools_requiring(self, capability: str) -> tuple[RegisteredTool, ...]:
        normalized = self._identifier(capability, "capability")
        with self._lock:
            selected = tuple(
                tool
                for tool in self._tools.values()
                if normalized in tool.descriptor.required_capabilities
            )
        return tuple(sorted(selected, key=lambda tool: (tool.tool_id, _version_key(tool.version))))

    def require_capabilities(
        self,
        tool_id: str,
        tool_version: str | None,
        granted_capabilities: Iterable[str],
    ) -> RegisteredTool:
        tool = self.require(tool_id, tool_version)
        granted = frozenset(
            self._identifier(value, "granted capability") for value in granted_capabilities
        )
        missing = tuple(
            sorted(set(tool.required_capabilities) - granted),
        )
        if missing:
            raise ToolCapabilityError(
                f"tool {tool.tool_id} requires capabilities that are not granted: "
                f"{', '.join(missing)}",
                missing=missing,
            )
        return tool

    def versions(self, tool_id: str) -> tuple[str, ...]:
        normalized_id = self._identifier(tool_id, "tool_id")
        with self._lock:
            selected = [
                tool.version for tool in self._tools.values() if tool.tool_id == normalized_id
            ]
        return tuple(sorted(selected, key=_version_key))

    def all_tools(self) -> tuple[RegisteredTool, ...]:
        with self._lock:
            return tuple(
                sorted(
                    self._tools.values(),
                    key=lambda tool: (tool.tool_id, _version_key(tool.version)),
                )
            )

    def tool_ids(self) -> tuple[str, ...]:
        return tuple(sorted({tool.tool_id for tool in self.all_tools()}))

    def required_capabilities(self) -> tuple[str, ...]:
        names: set[str] = set()
        for tool in self.all_tools():
            names.update(tool.descriptor.required_capabilities)
        return tuple(sorted(names))

    def __len__(self) -> int:
        with self._lock:
            return len(self._tools)

    def __iter__(self) -> Iterator[RegisteredTool]:
        return iter(self.all_tools())

    def __contains__(self, key: object) -> bool:
        if not isinstance(key, tuple) or len(key) != 2:
            return False
        tool_id, version = key
        if not isinstance(tool_id, str) or not isinstance(version, str):
            return False
        with self._lock:
            return (tool_id, version) in self._tools

    def _verify_unmodified(self, tool: RegisteredTool) -> None:
        current = canonical_json_digest(tool.descriptor.model_dump(mode="json"))
        if current != tool.descriptor_digest:
            raise ToolRegistryError(f"registered descriptor for {tool.tool_id} was modified")
        tool.verify_immutable()

    @staticmethod
    def _identifier(value: object, field_name: str) -> str:
        if not isinstance(value, str):
            raise ToolRegistryError(f"{field_name} must be a string")
        normalized = value.strip()
        if not normalized or len(normalized) > 128:
            raise ToolRegistryError(f"{field_name} must be nonempty and bounded")
        if any(character.isspace() or ord(character) < 32 for character in normalized):
            raise ToolRegistryError(f"{field_name} contains unsafe characters")
        return normalized

    @staticmethod
    def _version(value: object) -> str:
        if not isinstance(value, str):
            raise ToolRegistryError("tool_version must be a string")
        normalized = value.strip()
        if not normalized or len(normalized) > 64:
            raise ToolRegistryError("tool_version must be nonempty and bounded")
        if any(character.isspace() or ord(character) < 32 for character in normalized):
            raise ToolRegistryError("tool_version contains unsafe characters")
        return normalized


__all__ = (
    "MAX_REGISTERED_TOOLS",
    "DuplicateToolRegistrationError",
    "RegisteredTool",
    "SchemaDigestMismatchError",
    "ToolCapabilityError",
    "ToolRegistrationError",
    "ToolRegistry",
    "ToolRegistryError",
    "ToolVersionNotFoundError",
    "UnknownToolError",
)
