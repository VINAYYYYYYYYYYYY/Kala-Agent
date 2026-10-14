"""The feature-modeling order FreeCAD's PartDesign UI already enforces.

A tool call is refused when that toolbar command would not be available yet:
geometry before a sketch, a pad before a closed profile, a fillet before a
solid, analysis before a solid.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Final

from kala_agent_freecad.cad.catalog import (
    CATALOG,
    FEM_TOOLS,
    PART_BOOLEANS,
    PART_PRIMITIVES,
    PART_PROFILES,
    PART_SOLID,
    PARTDESIGN_ADDITIVE,
    PARTDESIGN_DRESSUP,
    PARTDESIGN_PATTERN,
    PARTDESIGN_SETUP,
    PARTDESIGN_SUBTRACTIVE,
    SKETCH_CONSTRAINTS,
    SKETCH_EDIT,
    SKETCH_GEOMETRY,
    SKETCH_LIFECYCLE,
)
from kala_agent_freecad.errors import FreeCADInputError

MAX_CALLS: Final[int] = 160
_SKETCH_TOOLS: Final[frozenset[str]] = frozenset(
    (
        *SKETCH_GEOMETRY,
        *SKETCH_CONSTRAINTS,
        *SKETCH_EDIT,
        *(name for name in SKETCH_LIFECYCLE if name not in {"sketch.leave", "sketch.new"}),
    )
)
_SETUP: Final[frozenset[str]] = frozenset(
    name for name in PARTDESIGN_SETUP if name != "partdesign.body"
)
_ADDITIVE: Final[frozenset[str]] = frozenset(PARTDESIGN_ADDITIVE)
_SUBTRACTIVE: Final[frozenset[str]] = frozenset(PARTDESIGN_SUBTRACTIVE)
_PROFILE_PART: Final[frozenset[str]] = frozenset(PART_PROFILES)
_SOLID: Final[frozenset[str]] = frozenset(
    (
        *PARTDESIGN_DRESSUP,
        *PARTDESIGN_PATTERN,
        *FEM_TOOLS,
        *PART_BOOLEANS,
        *PART_SOLID,
        "partdesign.boolean",
        "partdesign.check",
    )
)
CLASSIFIED_TOOLS: Final[frozenset[str]] = frozenset(
    (
        "partdesign.body",
        "sketch.new",
        "sketch.leave",
        *PART_PRIMITIVES,
        *_SKETCH_TOOLS,
        *_SETUP,
        *_ADDITIVE,
        *_SUBTRACTIVE,
        *_PROFILE_PART,
        *_SOLID,
    )
)


@dataclass(frozen=True, slots=True)
class DesignCall:
    """One toolbar command and the arguments the model chose."""

    tool: str
    arguments: Mapping[str, object]


@dataclass
class _Progress:
    stage: str = "start"
    has_body: bool = False
    has_solid: bool = False
    solid_before_sketch: bool = False


def parse_calls(value: object) -> tuple[DesignCall, ...]:
    """Read a list of tool calls and refuse a list the procedure rejects."""
    if isinstance(value, (str, bytes)) or not isinstance(value, Sequence) or not value:
        raise FreeCADInputError("calls must be a nonempty list of tool calls")
    if len(value) > MAX_CALLS:
        raise FreeCADInputError("the design exceeds the bounded number of tool calls")
    calls: list[DesignCall] = []
    for item in value:
        if not isinstance(item, Mapping):
            raise FreeCADInputError("each tool call must be an object")
        tool = item.get("tool")
        if not isinstance(tool, str) or tool not in CATALOG:
            raise FreeCADInputError(f"unknown CAD tool {tool!s}")
        arguments = item.get("arguments", {})
        if not isinstance(arguments, Mapping):
            raise FreeCADInputError(f"{tool} arguments must be an object")
        calls.append(DesignCall(tool=tool, arguments=arguments))
    check_procedure(calls)
    return tuple(calls)


def check_procedure(calls: Sequence[DesignCall]) -> str:
    """Walk the calls and return the stage after the last legal call."""
    if not calls:
        raise FreeCADInputError("calls must be a nonempty list of tool calls")
    progress = _Progress()
    for call in calls:
        progress = _advance(progress, call.tool)
    if progress.stage == "sketch":
        raise FreeCADInputError("the sketch was left open")
    if not progress.has_solid:
        raise FreeCADInputError("the procedure did not produce a solid")
    return progress.stage


def _advance(progress: _Progress, tool: str) -> _Progress:
    stage = progress.stage
    if tool == "partdesign.body":
        if stage not in {"start", "solid"}:
            _refuse(tool, stage)
        return _Progress(stage="body", has_body=True)
    if tool in PART_PRIMITIVES:
        if stage != "start":
            _refuse(tool, stage)
        return _Progress(stage="solid", has_solid=True)
    if tool == "sketch.new":
        if stage not in {"start", "body", "solid"}:
            _refuse(tool, stage)
        return _Progress(
            stage="sketch",
            has_body=progress.has_body,
            has_solid=progress.has_solid,
            solid_before_sketch=progress.has_solid,
        )
    if tool in _SKETCH_TOOLS:
        if stage != "sketch":
            _refuse(tool, stage)
        return progress
    if tool == "sketch.leave":
        if stage != "sketch":
            _refuse(tool, stage)
        return _Progress(
            stage="profile",
            has_body=progress.has_body,
            has_solid=progress.has_solid,
            solid_before_sketch=progress.solid_before_sketch,
        )
    if tool in _ADDITIVE:
        if stage != "profile" or not progress.has_body:
            _refuse(tool, stage)
        return _Progress(stage="solid", has_body=True, has_solid=True)
    if tool in _PROFILE_PART:
        if stage != "profile":
            _refuse(tool, stage)
        return _Progress(
            stage="solid",
            has_body=progress.has_body,
            has_solid=True,
        )
    if tool in _SUBTRACTIVE:
        if stage != "profile" or not progress.solid_before_sketch:
            _refuse(tool, stage)
        return _Progress(stage="solid", has_body=True, has_solid=True)
    if tool in _SETUP:
        if stage not in {"body", "solid"}:
            _refuse(tool, stage)
        return progress
    if tool in _SOLID:
        if stage != "solid" or not progress.has_solid:
            _refuse(tool, stage)
        return progress
    _refuse(tool, stage)
    return progress


def _refuse(tool: str, stage: str) -> None:
    raise FreeCADInputError(f"{tool} is not available during the {stage} stage")


__all__ = (
    "CLASSIFIED_TOOLS",
    "MAX_CALLS",
    "DesignCall",
    "check_procedure",
    "parse_calls",
)
