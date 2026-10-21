"""OpenRouter calls for the design loop.

North Mini Code writes the construction script, the analysis, and the reading.
Span-01 Lite chooses the next part or the revision. Each call reads its own
key from the environment or ``~/.config/kala`` and never stores or returns
that key.
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
import uuid
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Final, TypeVar

from kala_agent.contracts import (
    AnalysisReading,
    AnalysisScript,
    ConstructionError,
    ConstructionScript,
    load_analysis,
    load_reading,
)
from kala_agent_freecad.errors import FreeCADDependencyError

REASONING_MODEL: Final[str] = "cohere/north-mini-code:free"
DECIDE_MODEL: Final[str] = "respan/span-01-lite"
CHAT_URL: Final[str] = "https://openrouter.ai/api/v1/chat/completions"
DECIDE_URL: Final[str] = "https://openrouter.ai/api/alpha/decisions"
_REASONING_KEY_NAME: Final[str] = "OPENROUTER_API_KEY"
_DECIDE_KEY_NAME: Final[str] = "OPENROUTER_DECIDE_API_KEY"
_REASONING_KEY_FILE: Final[str] = "openrouter.key"
_DECIDE_KEY_FILE: Final[str] = "openrouter-decide.key"
_MAX_RESPONSE_BYTES: Final[int] = 65_536
_ATTEMPTS: Final[int] = 2
_RETRY: Final[str] = (
    "The previous reply was not one JSON design object. "
    "Return only that object, with summary, components, and assembly. "
    "No markdown and no explanation."
)
_ParsedT = TypeVar("_ParsedT")


def designer_ready() -> bool:
    """Return whether a designer can be asked. The keys themselves are not read out."""
    return bool(api_key()) and bool(decide_key())


def api_key() -> str:
    """Return the OpenRouter key used only for the reasoning model."""
    return _key(_REASONING_KEY_NAME, _REASONING_KEY_FILE)


def decide_key() -> str:
    """Return the OpenRouter key used only for the decision model."""
    return _key(_DECIDE_KEY_NAME, _DECIDE_KEY_FILE)


def _key(env_name: str, filename: str) -> str:
    """Return one OpenRouter key from the environment or the user config file."""
    env = os.environ.get(env_name, "").strip()
    if env:
        return env
    path = Path.home() / ".config" / "kala" / filename
    try:
        if path.is_file():
            return path.read_text(encoding="utf-8").strip()
    except OSError:
        return ""
    return ""


@dataclass(frozen=True, slots=True)
class ModelChoice:
    """One decision, plus the line that belongs in the design log."""

    choice: str
    note: str


@dataclass(frozen=True, slots=True)
class WrittenIntent:
    """The design object text, and the free model that wrote it."""

    text: str
    model: str


def write_intent(prompt: str) -> WrittenIntent:
    """Ask North Mini Code for one construction script, as one JSON object.

    A free model often answers in prose. That reply is asked again once.
    """
    correction = ""
    for _attempt in range(_ATTEMPTS):
        messages: list[dict[str, str]] = [
            {"content": _SYSTEM, "role": "system"},
            {"content": prompt, "role": "user"},
        ]
        if correction:
            messages.append({"content": correction, "role": "user"})
        payload = _post(
            CHAT_URL,
            {
                "max_tokens": 2_000,
                "messages": messages,
                "model": REASONING_MODEL,
                "reasoning": {"effort": "none"},
                "temperature": 0.2,
            },
            timeout=90,
            idempotent=False,
            key=api_key(),
            missing="the OpenRouter reasoning key is not set",
            who=f"reasoning model {REASONING_MODEL}",
        )
        design = _design_text(payload)
        if design is not None:
            return WrittenIntent(text=design, model=_routed_name(payload))
        correction = _RETRY
    raise FreeCADDependencyError(
        f"the reasoning model {REASONING_MODEL} did not return a design object"
    )


def explain_design(prompt: str, script_text: str) -> str:
    """Ask the reasoning model to describe the script it just wrote, in prose."""
    payload = _post(
        CHAT_URL,
        {
            "max_tokens": 700,
            "messages": [
                {"content": _EXPLAIN, "role": "system"},
                {
                    "content": f"Request:\n{prompt}\n\nConstruction script:\n{script_text}",
                    "role": "user",
                },
            ],
            "model": REASONING_MODEL,
            "reasoning": {"effort": "none"},
            "temperature": 0.3,
        },
        timeout=60,
        idempotent=False,
        key=api_key(),
        missing="the OpenRouter reasoning key is not set",
        who=f"reasoning model {REASONING_MODEL}",
    )
    texts = _reply_texts(payload)
    if not texts:
        raise FreeCADDependencyError(
            f"the reasoning model {REASONING_MODEL} did not explain the design"
        )
    return texts[0]


def choose_part(state: str, options: Mapping[str, str]) -> ModelChoice:
    """Ask Span-01 Lite which remaining option comes next.

    That model scores yes-or-no questions. Each option is one question, and the
    highest yes-probability among the parts is the next part.
    """
    payload = _post(
        DECIDE_URL,
        {
            "model": DECIDE_MODEL,
            "questions": {key: _noul(key, text) for key, text in options.items()},
            "state": state,
        },
        timeout=30,
        key=decide_key(),
        missing="the OpenRouter decision key is not set",
        who=f"decision model {DECIDE_MODEL}",
    )
    scores = _noul_scores(payload)
    ranked = {key: score for key, score in scores.items() if key in options and key != "stop"}
    if not ranked:
        raise FreeCADDependencyError(f"the decision model {DECIDE_MODEL} returned no choice")
    choice = max(ranked, key=lambda key: ranked[key])
    return ModelChoice(
        choice=choice,
        note=_decision_note(choice, {"probabilities": scores}, state, options),
    )


REVISION_OPTIONS: Final[Mapping[str, str]] = {
    "accept": "Accept the design. The result meets the request, or no change is justified.",
    "tool_edit": "Apply a small tool-call edit: move, resize, or fillet.",
    "script_rewrite": "Rewrite one component as a new construction script.",
}


def write_analysis(prompt: str, script: ConstructionScript) -> AnalysisScript:
    """Ask North Mini Code for the analysis script that belongs with a design."""
    return _ask_model(
        _ANALYSIS_SYSTEM,
        f"Request:\n{prompt}\n\nDesign:\n{script.model_dump_json()}",
        load_analysis,
        "analysis script",
    )


def read_analysis(
    prompt: str,
    script: ConstructionScript,
    measurements: Mapping[str, object],
) -> AnalysisReading:
    """Ask North Mini Code to read the analysis numbers against the request."""
    measured = json.dumps(measurements, default=str)
    return _ask_model(
        _READING_SYSTEM,
        (f"Request:\n{prompt}\n\nDesign:\n{script.model_dump_json()}\n\nMeasurements:\n{measured}"),
        _load_reading,
        "analysis reading",
    )


def choose_revision(state: str) -> ModelChoice:
    """Ask Span-01 Lite whether to accept, edit with tools, or rewrite a script."""
    payload = _post(
        DECIDE_URL,
        {
            "model": DECIDE_MODEL,
            "questions": {key: _revision_noul(key, text) for key, text in REVISION_OPTIONS.items()},
            "state": state,
        },
        timeout=30,
        key=decide_key(),
        missing="the OpenRouter decision key is not set",
        who=f"decision model {DECIDE_MODEL}",
    )
    scores = _noul_scores(payload)
    ranked = {key: score for key, score in scores.items() if key in REVISION_OPTIONS}
    if not ranked:
        raise FreeCADDependencyError(f"the decision model {DECIDE_MODEL} returned no choice")
    choice = max(ranked, key=lambda key: ranked[key])
    return ModelChoice(
        choice=choice,
        note=_decision_note(choice, {"probabilities": scores}, state, REVISION_OPTIONS),
    )


def _ask_model(
    system: str,
    user: str,
    load: Callable[[object], _ParsedT],
    label: str,
) -> _ParsedT:
    correction = ""
    for _attempt in range(_ATTEMPTS):
        messages: list[dict[str, str]] = [
            {"content": system, "role": "system"},
            {"content": user, "role": "user"},
        ]
        if correction:
            messages.append({"content": correction, "role": "user"})
        payload = _post(
            CHAT_URL,
            {
                "max_tokens": 2_000,
                "messages": messages,
                "model": REASONING_MODEL,
                "reasoning": {"effort": "none"},
                "temperature": 0.2,
            },
            timeout=90,
            idempotent=False,
            key=api_key(),
            missing="the OpenRouter reasoning key is not set",
            who=f"reasoning model {REASONING_MODEL}",
        )
        parsed = _loaded(payload, load)
        if parsed is not None:
            return parsed
        correction = (
            f"The previous reply was not one JSON {label}. "
            "Return only that object. No markdown and no explanation."
        )
    raise FreeCADDependencyError(f"the reasoning model {REASONING_MODEL} did not return a {label}")


def _load_reading(raw: object) -> AnalysisReading:
    reading = load_reading(raw)
    if reading.tool_edit and reading.rewrite is not None:
        raise ConstructionError("a reading proposes a tool edit or a rewrite, not both")
    return reading


def _loaded(
    payload: Mapping[str, object],
    load: Callable[[object], _ParsedT],
) -> _ParsedT | None:
    for text in _reply_texts(payload):
        for blob in _json_objects(text):
            try:
                raw = json.loads(blob)
            except json.JSONDecodeError:
                continue
            try:
                return load(raw)
            except ConstructionError:
                continue
    return None


def _revision_noul(key: str, text: str) -> dict[str, object]:
    return {
        "criteria": {
            "false": "A different action fits this analysis better.",
            "true": text,
        },
        "instructions": f"Is {key} the right next action?",
        "type": "noul",
    }


def _provider_message(detail: str) -> str:
    if "sk-" in detail or "http" in detail:
        try:
            parsed = json.loads(detail)
        except json.JSONDecodeError:
            return ""
        error = parsed.get("error")
        text = error.get("message") if isinstance(error, dict) else ""
        if not isinstance(text, str):
            return ""
        sentence = text.split(".", 1)[0].strip()
        if not sentence or "http" in sentence or "sk-" in sentence:
            return ""
        return sentence[:160]
    return " ".join(detail.split())[:160]


def _noul(key: str, text: str) -> dict[str, object]:
    """One yes-or-no question. The free model rejects a choice question."""
    if key == "stop":
        return {
            "criteria": {
                "false": "At least one requested part is still remaining.",
                "true": text,
            },
            "instructions": "Is the design already finished?",
            "type": "noul",
        }
    return {
        "criteria": {
            "false": "This part is already built, or another remaining part should come first.",
            "true": f"{text} It is still remaining and should be built next.",
        },
        "instructions": f"Is {key} the next part to build?",
        "type": "noul",
    }


def _noul_scores(payload: Mapping[str, object]) -> dict[str, float]:
    answers = payload.get("answers")
    if not isinstance(answers, dict):
        data = payload.get("data")
        if isinstance(data, dict):
            answers = data.get("answers")
    if not isinstance(answers, dict):
        return {}
    scores: dict[str, float] = {}
    for key, answer in answers.items():
        if not isinstance(answer, dict):
            continue
        value = answer.get("noul")
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            continue
        scores[str(key)] = float(value)
    return scores


def _decision_note(
    choice: str,
    answer: Mapping[str, object],
    state: str,
    options: Mapping[str, str],
) -> str:
    confidence = answer.get("confidence")
    chosen = f"Decision {DECIDE_MODEL}: {choice}"
    if isinstance(confidence, (int, float)) and not isinstance(confidence, bool):
        chosen = f"{chosen}, confidence {float(confidence):.2f}"
    rows = [
        chosen,
        f"  state: {state}",
        "  options: " + "; ".join(f"{key}: {text}" for key, text in options.items()),
    ]
    probabilities = answer.get("probabilities")
    if isinstance(probabilities, dict):
        rated: list[str] = []
        for key, value in probabilities.items():
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                continue
            rated.append(f"{key} {float(value):.2f}")
        if rated:
            rows.append("  probabilities: " + ", ".join(rated))
    return "\n".join(rows)


def _routed_name(payload: Mapping[str, object]) -> str:
    routed = payload.get("model")
    if isinstance(routed, str) and routed.strip() and routed.strip() != REASONING_MODEL:
        return f"{REASONING_MODEL} via {routed.strip()}"
    return REASONING_MODEL


def _design_text(payload: Mapping[str, object]) -> str | None:
    for text in _reply_texts(payload):
        for blob in _json_objects(text):
            design = _loads_design(blob)
            if design is not None:
                return design
    return None


def _reply_texts(payload: Mapping[str, object]) -> tuple[str, ...]:
    choices = payload.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
        return ()
    message = choices[0].get("message")
    if not isinstance(message, dict):
        return ()
    texts: list[str] = []
    for key in ("content", "reasoning", "reasoning_content"):
        text = _text(message.get(key))
        if text:
            texts.append(text)
    return tuple(texts)


def _loads_design(blob: str) -> str | None:
    try:
        parsed = json.loads(blob)
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, dict):
        return None
    components = parsed.get("components")
    if isinstance(components, (str, bytes)) or not isinstance(components, list) or not components:
        return None
    for component in components:
        if not isinstance(component, dict):
            return None
        steps = component.get("steps")
        if isinstance(steps, (str, bytes)) or not isinstance(steps, list) or not steps:
            return None
    return json.dumps(parsed)


def _json_objects(text: str) -> tuple[str, ...]:
    found: list[str] = []
    start = -1
    depth = 0
    quoted = False
    escaped = False
    for index, char in enumerate(text):
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
            continue
        if char == '"':
            quoted = True
        elif char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}" and depth:
            depth -= 1
            if depth == 0 and start >= 0:
                found.append(text[start : index + 1])
                start = -1
    return tuple(found)


def _text(value: object) -> str:
    if isinstance(value, str):
        return value.strip()
    if not isinstance(value, list):
        return ""
    parts: list[str] = []
    for item in value:
        if isinstance(item, str) and item.strip():
            parts.append(item.strip())
        elif isinstance(item, dict):
            text = item.get("text") or item.get("content")
            if isinstance(text, str) and text.strip():
                parts.append(text.strip())
    return "\n".join(parts)


def _post(
    url: str,
    payload: Mapping[str, object],
    *,
    timeout: int,
    who: str,
    key: str,
    missing: str,
    idempotent: bool = True,
) -> dict[str, object]:
    if not key:
        raise FreeCADDependencyError(missing)
    headers = {
        "Authorization": f"Bearer {key}",
        "Content-Type": "application/json",
    }
    if idempotent:
        headers["Idempotency-Key"] = uuid.uuid4().hex
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read(_MAX_RESPONSE_BYTES)
    except urllib.error.HTTPError as error:
        detail = error.read(400).decode("utf-8", "replace")
        message = f"the {who} refused the request ({error.code})"
        reason = _provider_message(detail)
        if reason:
            message = f"{message}: {reason}"
        raise FreeCADDependencyError(message) from error
    except urllib.error.URLError as error:
        raise FreeCADDependencyError(f"the {who} did not respond") from error
    try:
        parsed = json.loads(raw.decode("utf-8"))
    except json.JSONDecodeError as error:
        raise FreeCADDependencyError(f"the {who} returned malformed JSON") from error
    if not isinstance(parsed, dict):
        raise FreeCADDependencyError(f"the {who} returned malformed JSON")
    if "error" in parsed and "choices" not in parsed and "answers" not in parsed:
        raise FreeCADDependencyError(f"the {who} refused the request")
    return parsed


_EXPLAIN: Final[str] = """You just wrote a construction script for a CAD build.
Explain it the way a coding agent explains a change: plain sentences, no JSON.
Name the parts, the important sizes and where they sit, and anything the request asked for that this script cannot make.
Stay under 160 words. Do not use markdown headings.
"""
_SYSTEM: Final[str] = """You are a mechanical designer. Return one JSON object and no other text.
Do not write FreeCAD Python. Do not return a parts list or pad and pocket feature lists.
The object is {summary, components, assembly}. All sizes are millimetres.
components has 1 to 64 objects. Each has name and steps. steps has 1 to 500 objects, and a whole script may use at most 500 steps in total across its components and assembly.
Each solid body is one component; do not split a body into components just to save steps.
A step is {op, plane, profile, width, height, diameter, length, pitch, x, y, reversed}.
A component op is sketch, extrude, revolve, pocket, or helix.
A sketch has plane (XY, XZ, or YZ) and profile (rectangle or circle).
A rectangle has width and height. A circle has diameter. x and y are optional.
extrude has length. Set reversed to true to extrude the other way.
revolve is only for a profile that is not a circle.
pocket may have length; without length it goes through. A pocket follows a solid on the same component.
helix has diameter, pitch, and length, and follows an extrude or revolve of the shaft on the same component.
The first solid step is extrude or revolve. Do not make a separate component for a hole.
A gear, disk, pin, or shaft is a circle sketch and an extrude, not a revolve.
assembly is a list of {op, component, components, x, y, z}.
An assembly op is place, fuse, or compound.
place has component, x, y, and z. fuse and compound have components, a list of names.
Identical copies that sit in different places are separate components.
A screw jack is a flanged base, a screw with a helix, a cup, and a tommy bar.
Do not request an analysis. Do not wrap the JSON in markdown.
"""
_ANALYSIS_SYSTEM: Final[
    str
] = """You write one analysis script. Return one JSON object and no other text.
The object is {steps: [{op, name, face, force_n, pressure_mpa}]}.
A step op is only material, fix, force, pressure, mesh, or solve.
material may have name. fix has face, an index starting at 1.
force has face and force_n. pressure has face and pressure_mpa.
Use one of force or pressure. Include fix, mesh, and solve, in that order.
The script must include solve.
Do not wrap the JSON in markdown.
"""
_READING_SYSTEM: Final[
    str
] = """You read analysis numbers against the user's request. Return one JSON object and no other text.
The object is {verdict, tool_edit, rewrite}.
verdict is a string. tool_edit is a list of edit steps. rewrite is one component or null.
An edit op is only move, resize, or fillet.
move has component, x, y, and z. resize has component and length or diameter. fillet has component and radius.
A rewrite component has name and steps.
At most one kind of change: if rewrite is present, tool_edit is empty, and the reverse.
If the design meets the request, verdict says so, tool_edit is empty, and rewrite is null.
Do not wrap the JSON in markdown.
"""
