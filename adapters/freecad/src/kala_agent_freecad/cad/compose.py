"""Turn a request into FreeCAD tool calls.

design_request asks for a construction script and compiles it. revise_design
asks whether to accept that design, edit a size, or rewrite one component.
realize still expands an older parts list, one part at a time.
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Final, cast

from kala_agent.contracts import ConstructionError, ConstructionScript, load_construction
from kala_agent_freecad.cad.accept import accept_design
from kala_agent_freecad.cad.compile import (
    apply_edits,
    compile_construction,
    fillets_from_edits,
    rewrite_component,
)
from kala_agent_freecad.cad.decide import (
    MAX_PROMPT_CHARS,
    DesignPlan,
    _analysis,
    _call,
    _fit,
    _profile_constraints,
    _rectangle_constraints,
)
from kala_agent_freecad.cad.models import (
    DECIDE_MODEL,
    REASONING_MODEL,
    ModelChoice,
    choose_revision,
    read_analysis,
    write_analysis,
    write_intent,
)
from kala_agent_freecad.cad.procedure import parse_calls
from kala_agent_freecad.errors import FreeCADInputError

MAX_PARTS: Final[int] = 8
MAX_FEATURES: Final[int] = 12
_CONTINUE_CHARS: Final[int] = 24_000
_ADDITIVE: Final[frozenset[str]] = frozenset({"helix", "pad", "revolution"})
_KINDS: Final[frozenset[str]] = frozenset({"helix", "pad", "pocket", "revolution"})
_PLANES: Final[frozenset[str]] = frozenset({"XY", "XZ", "YZ"})
_KEY: Final[re.Pattern[str]] = re.compile(r"[^A-Za-z0-9._-]")
Chooser = Callable[[str, Mapping[str, str]], str | ModelChoice]


@dataclass(frozen=True, slots=True)
class _Feature:
    kind: str
    plane: str
    profile: str
    width: float
    height: float
    diameter: float
    length: float
    pitch: float
    x_mm: float
    y_mm: float
    length_given: bool
    reversed: bool = False


@dataclass(frozen=True, slots=True)
class _Part:
    key: str
    name: str
    x_mm: float
    y_mm: float
    z_mm: float
    features: tuple[_Feature, ...]

    def label(self) -> str:
        kinds = ", ".join(feature.kind for feature in self.features)
        return f"Build {self.name}: {kinds}."


def design_request(
    prompt: str,
    on_log: Callable[[str], None] | None = None,
    *,
    analyze: bool = False,
) -> DesignPlan:
    """Ask for a construction script and compile it to FreeCAD tool calls.

    Analysis runs only when the user turns it on. The reasoning model cannot.
    Components are built in the order they were written.
    """
    _require_prompt(prompt)
    text = prompt.strip()
    script, model = _written_script(text, on_log)
    return _compiled(text, script, model, on_log, analyze=analyze, kind="designed")


def continue_design(
    prompt: str,
    current: Mapping[str, object],
    on_log: Callable[[str], None] | None = None,
    *,
    analyze: bool = False,
    add_part: bool = False,
) -> DesignPlan:
    """Revise the open construction script instead of starting a new design.

    The model must return the whole script. Existing components stay unless the
    change removes one. A new part must add a component and keep the others.
    """
    _require_prompt(prompt)
    try:
        previous = load_construction(current)
    except ConstructionError as error:
        raise FreeCADInputError(str(error)) from error
    message = _continue_message(prompt.strip(), previous, add_part=add_part)
    if len(message) > _CONTINUE_CHARS:
        raise FreeCADInputError("the current design is too large to revise in one request")
    if on_log is not None:
        on_log("Adding a part." if add_part else "Changing the current design.")
    script, model = _written_script(message, on_log)
    if not _same_design(previous, script, add_part=add_part):
        script, model = _written_script(
            _continue_retry(message, previous, add_part=add_part),
            on_log,
        )
    if not _same_design(previous, script, add_part=add_part):
        raise FreeCADInputError("the model replaced the current design instead of changing it")
    return _compiled(prompt.strip(), script, model, on_log, analyze=analyze, kind="revised")


def revise_design(
    prompt: str,
    script: ConstructionScript,
    measurements: Mapping[str, object],
    on_log: Callable[[str], None] | None = None,
) -> DesignPlan | None:
    """Let the decision model choose accept, a tool edit, or a script rewrite."""
    reading = read_analysis(prompt, script, measurements)
    state = (
        f"{reading.verdict} "
        f"tool_edit={'present' if reading.tool_edit else 'absent'} "
        f"rewrite={'present' if reading.rewrite is not None else 'absent'}"
    )
    choice = choose_revision(state)
    lines: list[str] = []
    for line in choice.note.splitlines():
        _add(lines, on_log, line)
    if choice.choice == "tool_edit":
        if not reading.tool_edit:
            return None
        updated = apply_edits(script, reading.tool_edit)
        calls = compile_construction(updated, fillets=fillets_from_edits(reading.tool_edit))
    elif choice.choice == "script_rewrite":
        rewrite = reading.rewrite
        if rewrite is None:
            return None
        updated = rewrite_component(script, rewrite)
        calls = compile_construction(updated)
    else:
        return None
    return _planned(updated, calls, kind="revised", log=lines)


def _compiled(
    prompt: str,
    script: ConstructionScript,
    model: str,
    on_log: Callable[[str], None] | None,
    *,
    analyze: bool,
    kind: str,
) -> DesignPlan:
    if analyze:
        calls = compile_construction(script, analysis=write_analysis(prompt, script))
    else:
        calls = compile_construction(script)
    lines = _script_log(script, model, on_log, analyze=analyze)
    return _planned(script, calls, kind=kind, log=lines)


def _continue_message(prompt: str, script: ConstructionScript, *, add_part: bool) -> str:
    names = ", ".join(component.name for component in script.components)
    if add_part:
        lead = (
            "Add one new component to this same design. "
            f"Keep every existing component ({names}) and all of its steps."
        )
    else:
        lead = (
            "Change this same design. "
            f"Keep every existing component ({names}) unless the change explicitly removes one. "
            "Apply the change by editing sizes or adding steps, such as a pocket. "
            "Do not start a new, unrelated design."
        )
    return (
        f"{lead}\n\nCurrent construction script:\n{script.model_dump_json()}\n\nChange:\n{prompt}"
    )


def _continue_retry(message: str, script: ConstructionScript, *, add_part: bool) -> str:
    names = ", ".join(component.name for component in script.components)
    if add_part:
        fault = (
            "Your previous script did not add one new component. "
            f"Keep these components: {names}. Add one new component for the change."
        )
    else:
        fault = (
            "Your previous script replaced the design. "
            f"Keep these components: {names}. Apply only the requested change."
        )
    return f"{message}\n\n{fault}"


def _same_design(
    previous: ConstructionScript,
    updated: ConstructionScript,
    *,
    add_part: bool,
) -> bool:
    old = {component.name for component in previous.components}
    new = {component.name for component in updated.components}
    if not old <= new:
        return False
    return not add_part or bool(new - old)


def _planned(
    script: ConstructionScript,
    calls: list[dict[str, object]],
    *,
    kind: str,
    log: Sequence[str],
) -> DesignPlan:
    body = cast("dict[str, object]", script.model_dump(mode="json"))
    return DesignPlan(
        calls=parse_calls(calls),
        summary=script.summary,
        kind=kind,
        log=tuple(log),
        source=body,
        script=body,
    )


def _realize_written(
    text: str,
    model: str,
    on_log: Callable[[str], None] | None,
    analyze: bool,
    choose: Chooser,
) -> DesignPlan | None:
    try:
        raw = json.loads(text)
    except json.JSONDecodeError as error:
        raise FreeCADInputError(
            f"the reasoning model {model} did not return a design object"
        ) from error
    try:
        return realize(raw, choose, reasoning_model=model, on_log=on_log, analyze=analyze)
    except FreeCADInputError as error:
        if "shaft before the helix" not in str(error):
            raise
        return None


def realize(
    raw: object,
    choose: Chooser,
    reasoning_model: str = REASONING_MODEL,
    on_log: Callable[[str], None] | None = None,
    *,
    analyze: bool = False,
) -> DesignPlan:
    """Expand a design object into tool calls, asking choose which part is next."""
    intent = _intent(raw)
    lines: list[str] = []
    _reasoning_log(intent, reasoning_model, lines, on_log)
    remaining = list(intent.parts)
    built: list[str] = []
    calls: list[dict[str, object]] = []
    while remaining:
        options = {part.key: part.label() for part in remaining}
        options["stop"] = "Stop only when every requested part is already built."
        state = _state(intent.summary, built, remaining)
        picked = _pick(choose, state, options, lines, on_log)
        part = next(item for item in remaining if item.key == picked)
        remaining.remove(part)
        calls.extend(_part_calls(part))
        built.append(part.name)
    if analyze and built:
        _add(lines, on_log, "Analysis: on, because you asked for it.")
        calls.extend(_analysis())
    else:
        _add(lines, on_log, "Analysis: off. It runs only when you turn it on.")
    parsed = parse_calls(calls)
    return DesignPlan(calls=parsed, summary=intent.summary, kind="designed", log=tuple(lines))


def _written_script(
    prompt: str,
    on_log: Callable[[str], None] | None = None,
) -> tuple[ConstructionScript, str]:
    """Ask the reasoning model for the script, then refuse it when a duty is missing."""
    written = write_intent(prompt)
    _exchange(on_log, prompt, written.text)
    script, reason = _reviewed(prompt, written.text, on_log)
    if script is not None:
        return script, written.model
    follow = (
        f"{prompt}\n\nYour previous construction script was refused: {reason}. "
        "Return one corrected construction script for this same request."
    )
    corrected = write_intent(follow)
    _exchange(on_log, follow, corrected.text)
    script, reason = _reviewed(prompt, corrected.text, on_log)
    if script is None:
        raise FreeCADInputError(reason)
    return script, corrected.model


def _reviewed(
    prompt: str,
    raw: object,
    on_log: Callable[[str], None] | None,
) -> tuple[ConstructionScript | None, str]:
    """Parse one reply and list the engineering duties it still misses."""
    script = _try_script(raw)
    if script is None:
        return None, _script_error(raw)
    duties = accept_design(prompt, script)
    for duty in duties:
        if on_log is not None:
            on_log(duty)
    if duties:
        return None, " ".join(duties)
    return script, ""


def _exchange(on_log: Callable[[str], None] | None, sent: str, received: str) -> None:
    """Write the model request and the model reply into the design log."""
    if on_log is None:
        return
    on_log("Model input:")
    on_log(sent)
    on_log("Model output:")
    on_log(received)


def _try_script(raw: object) -> ConstructionScript | None:
    try:
        return _script(raw)
    except FreeCADInputError:
        return None


def _script_error(raw: object) -> str:
    try:
        _script(raw)
    except FreeCADInputError as error:
        return str(error)
    return "the construction script was refused"


def _script(raw: object) -> ConstructionScript:
    if isinstance(raw, str):
        try:
            raw = json.loads(raw)
        except json.JSONDecodeError as error:
            raise FreeCADInputError("the reasoning model did not return a design object") from error
    try:
        return load_construction(raw)
    except ConstructionError as error:
        raise FreeCADInputError(str(error)) from error


def _script_log(
    script: ConstructionScript,
    model: str,
    on_log: Callable[[str], None] | None,
    *,
    analyze: bool,
) -> list[str]:
    lines: list[str] = []
    _add(lines, on_log, f"Reasoning {model}: {script.summary}")
    for component in script.components:
        steps = ", ".join(step.op for step in component.steps)
        _add(lines, on_log, f"  {component.name}: {steps}")
    for step in script.assembly:
        if step.op == "place":
            _add(
                lines,
                on_log,
                f"  place {step.component} at {step.x:g}, {step.y:g}, {step.z:g} mm",
            )
        else:
            names = ", ".join(step.components)
            _add(lines, on_log, f"  {step.op} {names}")
    if analyze:
        _add(lines, on_log, "Analysis: on, because you asked for it.")
    else:
        _add(lines, on_log, "Analysis: off. It runs only when you turn it on.")
    return lines


def _require_prompt(prompt: str) -> None:
    if not isinstance(prompt, str):
        raise FreeCADInputError("prompt must be a string")
    if not prompt.strip():
        raise FreeCADInputError("prompt must not be empty")
    if len(prompt) > MAX_PROMPT_CHARS:
        raise FreeCADInputError("prompt exceeds the bounded length")


def _pick(
    choose: Chooser,
    state: str,
    options: Mapping[str, str],
    lines: list[str],
    on_log: Callable[[str], None] | None,
) -> str:
    note = state
    for _ in range(2):
        picked = _recorded(choose, note, options, lines, on_log)
        if picked in options and picked != "stop":
            return picked
        note = f"That choice leaves the design unfinished. {state}"
    raise FreeCADInputError(f"the decision model {DECIDE_MODEL} did not choose a legal next part")


def _recorded(
    choose: Chooser,
    state: str,
    options: Mapping[str, str],
    lines: list[str],
    on_log: Callable[[str], None] | None,
) -> str:
    result = choose(state, options)
    if isinstance(result, ModelChoice):
        for line in result.note.splitlines():
            _add(lines, on_log, line)
        return result.choice
    if not isinstance(result, str) or not result:
        raise FreeCADInputError(f"the decision model {DECIDE_MODEL} returned no choice")
    _add(lines, on_log, f"Decision {DECIDE_MODEL}: {result}")
    _add(lines, on_log, f"  state: {state}")
    offered = "; ".join(f"{key}: {text}" for key, text in options.items())
    _add(lines, on_log, f"  options: {offered}")
    return result


def _add(lines: list[str], on_log: Callable[[str], None] | None, text: str) -> None:
    lines.append(text)
    if on_log is not None:
        on_log(text)


def _reasoning_log(
    intent: _Intent,
    reasoning_model: str,
    lines: list[str],
    on_log: Callable[[str], None] | None,
) -> None:
    _add(lines, on_log, f"Reasoning {reasoning_model}: {intent.summary}")
    for part in intent.parts:
        place = f" at {part.x_mm:g}, {part.y_mm:g}, {part.z_mm:g} mm"
        _add(lines, on_log, f"  {part.name}{place}")
        for feature in part.features:
            _add(lines, on_log, f"    {_feature_note(feature)}")


def _feature_note(feature: _Feature) -> str:
    if feature.kind == "helix":
        detail = (
            f"diameter {feature.diameter:g} mm, pitch {feature.pitch:g} mm, "
            f"length {feature.length:g} mm"
        )
    elif feature.profile == "circle":
        detail = f"circle diameter {feature.diameter:g} mm"
    else:
        detail = f"rectangle {feature.width:g} x {feature.height:g} mm"
    if feature.kind == "pad":
        detail = f"{detail}, length {feature.length:g} mm"
    return f"{feature.kind} on {feature.plane}: {detail}"


def _state(summary: str, built: Sequence[str], remaining: Sequence[_Part]) -> str:
    done = ", ".join(built) if built else "nothing"
    left = ", ".join(part.name for part in remaining) if remaining else "nothing"
    return f"{summary} Built: {done}. Remaining: {left}."


@dataclass(frozen=True, slots=True)
class _Intent:
    summary: str
    parts: tuple[_Part, ...]


def _intent(raw: object) -> _Intent:
    if not isinstance(raw, Mapping):
        raise FreeCADInputError("the reasoning model did not return a design object")
    summary = raw.get("summary", "Designed part")
    if not isinstance(summary, str) or not summary.strip():
        raise FreeCADInputError("the design summary must be text")
    parts = raw.get("parts")
    if isinstance(parts, (str, bytes)) or not isinstance(parts, Sequence) or not parts:
        raise FreeCADInputError("the design needs at least one part")
    if len(parts) > MAX_PARTS:
        raise FreeCADInputError("the design has too many parts")
    used: set[str] = set()
    parsed = tuple(_part(item, used) for item in parts)
    return _Intent(summary=summary.strip(), parts=parsed)


def _part(raw: object, used: set[str]) -> _Part:
    if not isinstance(raw, Mapping):
        raise FreeCADInputError("each part must be an object")
    name = raw.get("name", "part")
    if not isinstance(name, str) or not name.strip():
        raise FreeCADInputError("each part needs a name")
    features = raw.get("features")
    if isinstance(features, (str, bytes)) or not isinstance(features, Sequence) or not features:
        raise FreeCADInputError(f"{name} needs at least one feature")
    if len(features) > MAX_FEATURES:
        raise FreeCADInputError(f"{name} has too many features")
    parsed = _cover_pins(tuple(_feature(item) for item in features))
    if parsed[0].kind == "helix":
        raise FreeCADInputError(f"{name} needs a shaft before the helix")
    if parsed[0].kind not in _ADDITIVE:
        raise FreeCADInputError(f"{name} must start with a solid feature")
    return _Part(
        key=_key(name, used),
        name=name.strip()[:40],
        x_mm=_millimetre(raw.get("x", 0), "x"),
        y_mm=_millimetre(raw.get("y", 0), "y"),
        z_mm=_millimetre(raw.get("z", 0), "z"),
        features=parsed,
    )


def _feature(raw: object) -> _Feature:
    if not isinstance(raw, Mapping):
        raise FreeCADInputError("each feature must be an object")
    kind = raw.get("kind")
    if not isinstance(kind, str) or kind not in _KINDS:
        raise FreeCADInputError("a feature kind must be pad, pocket, revolution, or helix")
    plane = raw.get("plane", "XZ" if kind == "helix" else "XY")
    if not isinstance(plane, str) or plane not in _PLANES:
        raise FreeCADInputError("a feature plane must be XY, XZ, or YZ")
    profile = raw.get("profile", "circle" if kind == "helix" else "rectangle")
    if profile not in {"circle", "rectangle"}:
        raise FreeCADInputError("a feature profile must be a rectangle or a circle")
    if profile == "circle" and kind in {"pad", "revolution"}:
        plane = "XY"
    length = raw.get("length")
    return _Feature(
        kind=kind,
        plane="XZ" if kind == "helix" else plane,
        profile="rectangle" if kind == "helix" else profile,
        width=_optional(raw.get("width"), "width", 40.0),
        height=_optional(raw.get("height"), "height", 20.0),
        diameter=_optional(raw.get("diameter"), "diameter", 20.0),
        length=_optional(length, "length", 10.0),
        pitch=_optional(raw.get("pitch"), "pitch", 4.0),
        x_mm=_millimetre(raw.get("x", 0), "x"),
        y_mm=_millimetre(raw.get("y", 0), "y"),
        length_given=isinstance(length, (int, float)) and not isinstance(length, bool),
        reversed=raw.get("reversed") is True,
    )


def _cover_pins(features: tuple[_Feature, ...]) -> tuple[_Feature, ...]:
    """Grow the base plate until every pin centre lies on it.

    A circle drawn beside the plate does not fuse, so FreeCAD rejects the pad.
    """
    if not features or features[0].profile != "rectangle" or features[0].kind != "pad":
        return features
    plate = features[0]
    pins = [
        feature
        for feature in features[1:]
        if feature.profile == "circle" and feature.kind in {"pad", "revolution"}
    ]
    if not pins:
        return features
    margin = 1.0
    left = plate.x_mm
    bottom = plate.y_mm
    right = plate.x_mm + plate.width
    top = plate.y_mm + plate.height
    for pin in pins:
        radius = pin.diameter / 2.0
        left = min(left, pin.x_mm - radius)
        bottom = min(bottom, pin.y_mm - radius)
        right = max(right, pin.x_mm + radius)
        top = max(top, pin.y_mm + radius)
    left -= margin
    bottom -= margin
    right += margin
    top += margin
    covered = _Feature(
        kind=plate.kind,
        plane=plate.plane,
        profile=plate.profile,
        width=right - left,
        height=top - bottom,
        diameter=plate.diameter,
        length=plate.length,
        pitch=plate.pitch,
        x_mm=left,
        y_mm=bottom,
        length_given=plate.length_given,
        reversed=plate.reversed,
    )
    return (covered, *features[1:])


def _optional(value: object, label: str, default: float) -> float:
    if value is None:
        return _fit(default, label)
    return _fit(value if isinstance(value, (int, float)) else default, label)


def _millimetre(value: object, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FreeCADInputError(f"{label} must be a number")
    number = float(value)
    if abs(number) > 5_000:
        raise FreeCADInputError(f"{label} is outside the model bounds")
    return number


def _key(name: str, used: set[str]) -> str:
    raw = _KEY.sub("", name)[:40] or "part"
    key = raw
    suffix = 2
    while key in used or key == "stop":
        key = f"{raw}-{suffix}"
        suffix += 1
    used.add(key)
    return key


def _part_calls(part: _Part) -> list[dict[str, object]]:
    calls = [_call("partdesign.body")]
    if abs(part.x_mm) + abs(part.y_mm) + abs(part.z_mm) > 1e-9:
        calls.append(_call("partdesign.placement", x=part.x_mm, y=part.y_mm, z=part.z_mm))
    for feature in part.features:
        calls.extend(_feature_calls(feature))
    return calls


def _feature_calls(feature: _Feature) -> list[dict[str, object]]:
    calls = [_call("sketch.new", plane=feature.plane), *_profile(feature)]
    calls.append(_call("sketch.validate"))
    calls.append(_call("sketch.leave"))
    if feature.kind == "pad":
        calls.append(_pad_call(feature))
    elif feature.kind == "pocket" and feature.length_given:
        calls.append(_call("partdesign.pocket", length=feature.length))
    elif feature.kind == "pocket":
        calls.append(_call("partdesign.pocket"))
    elif feature.kind == "revolution" and feature.profile == "circle":
        calls.append(_pad_call(feature))
    elif feature.kind == "revolution":
        calls.append(_call("partdesign.revolution", angle=360))
    else:
        calls.append(_call("partdesign.helix", height=feature.length, pitch=feature.pitch))
    calls.append(_call("partdesign.check"))
    return calls


def _pad_call(feature: _Feature) -> dict[str, object]:
    if feature.reversed:
        return _call("partdesign.pad", length=feature.length, reversed=True)
    return _call("partdesign.pad", length=feature.length)


def _profile(feature: _Feature) -> list[dict[str, object]]:
    if feature.kind == "helix":
        radius = feature.diameter / 2.0
        depth = max(1.0, feature.diameter * 0.08)
        tooth = max(0.5, min(feature.pitch * 0.45, feature.length * 0.25))
        points = (
            (radius, 0.5),
            (radius + depth, 0.5),
            (radius + depth, 0.5 + tooth),
            (radius, 0.5 + tooth),
        )
        return [
            _call(
                "sketch.polyline",
                closed=True,
                points=[[x_mm, y_mm] for x_mm, y_mm in points],
            ),
            *_profile_constraints(points),
        ]
    if feature.profile == "circle":
        radius = feature.diameter / 2.0
        calls = [
            _call("sketch.circle", radius=radius, x=feature.x_mm, y=feature.y_mm),
            _call("sketch.diameter", geo=0, length=feature.diameter),
        ]
        if abs(feature.x_mm) <= 1e-9 and abs(feature.y_mm) <= 1e-9:
            calls.append(_call("sketch.coincident", geo=0, geo2=-1, pos=3, pos2=1))
        else:
            calls.append(_call("sketch.distance_x", geo=0, length=feature.x_mm, pos=3))
            calls.append(_call("sketch.distance_y", geo=0, length=feature.y_mm, pos=3))
        return calls
    if abs(feature.x_mm) <= 1e-9 and abs(feature.y_mm) <= 1e-9:
        return [
            _call(
                "sketch.rectangle",
                height=feature.height,
                width=feature.width,
                x=0,
                y=0,
            ),
            *_rectangle_constraints(feature.width, feature.height),
        ]
    points = (
        (feature.x_mm, feature.y_mm),
        (feature.x_mm + feature.width, feature.y_mm),
        (feature.x_mm + feature.width, feature.y_mm + feature.height),
        (feature.x_mm, feature.y_mm + feature.height),
    )
    return [
        _call(
            "sketch.polyline",
            closed=True,
            points=[[x_mm, y_mm] for x_mm, y_mm in points],
        ),
        *_profile_constraints(points),
    ]


__all__ = (
    "design_request",
    "realize",
    "revise_design",
)
