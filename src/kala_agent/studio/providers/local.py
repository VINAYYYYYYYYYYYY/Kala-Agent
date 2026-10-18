"""Local deterministic offline provider for the studio chat surface.

The local provider is an offline mechanical-engineering helper. It performs a
deterministic first pass over the operator's prompt: it extracts any numeric
dimensions, computes nominal analytic geometry estimates (volume and surface
area for a box, cylinder, or sphere), and always restates the unified
OpenCascade (OCP) workflow the studio ships with.

The provider never reads environment variables that look like keys, never
opens a socket, and never imports anything outside the standard library. It
does not run a solver and never fabricates finite-element, CFD, stress, or
thermal numbers. The studio advertises it as ``configured: true`` because the
provider is always present on a working studio install; it is the only
provider that the studio can guarantee does not require credentials.
"""

from __future__ import annotations

import hashlib
import math
import re
from typing import Final

from kala_agent.studio.providers.base import ChatOutcome, ProviderProtocolError

DEFAULT_LOCAL_MODEL: Final[str] = "deterministic-paraphrase-v1"
MAX_PROMPT_LENGTH: Final[int] = 4096
MAX_RESPONSE_LENGTH: Final[int] = 1024

_MM_PER_UNIT: Final[dict[str, float]] = {"mm": 1.0, "cm": 10.0, "m": 1000.0}
_NUMBER: Final[str] = r"\d+(?:\.\d+)?"
_UNIT: Final[str] = r"(mm|cm|m)?"
_SEPARATOR: Final[str] = r"(?:x|\u00d7|\*|by)"

_TRIPLE_RE: Final[re.Pattern[str]] = re.compile(
    rf"({_NUMBER})\s*{_SEPARATOR}\s*({_NUMBER})\s*{_SEPARATOR}\s*({_NUMBER})\s*{_UNIT}"
)
_NUMBER_RE: Final[re.Pattern[str]] = re.compile(rf"(?<![\w.])({_NUMBER})\s*{_UNIT}")

# Ordered primitive keyword table. The first keyword found in the prompt
# selects the estimate shape; the order is fixed so the result is
# deterministic for a prompt that names more than one primitive.
_PRIMITIVE_KEYWORDS: Final[tuple[tuple[str, str], ...]] = (
    ("box", "box"),
    ("block", "box"),
    ("bracket", "box"),
    ("plate", "box"),
    ("cylinder", "cylinder"),
    ("shaft", "cylinder"),
    ("pin", "cylinder"),
    ("rod", "cylinder"),
    ("pipe", "cylinder"),
    ("sphere", "sphere"),
    ("ball", "sphere"),
    ("bearing", "sphere"),
)

# External suite names the studio can hand a neutral file to. The studio
# never claims it drives a native vendor API, so this table only selects the
# wording of the neutral-handoff reminder.
_SUITE_NAMES: Final[tuple[tuple[str, str], ...]] = (
    ("solidworks", "SOLIDWORKS"),
    ("solid edge", "Solid Edge"),
    ("solidedge", "Solid Edge"),
    ("autocad", "AutoCAD"),
    ("fusion 360", "Fusion 360"),
    ("fusion360", "Fusion 360"),
    ("freecad", "FreeCAD"),
    ("openscad", "OpenSCAD"),
    ("ansys", "ANSYS Mechanical"),
    ("abaqus", "Abaqus"),
)


def _normalise(text: str) -> str:
    cleaned = re.sub(r"\s+", " ", text.strip())
    if not cleaned:
        return ""
    return cleaned


def _first_sentences(text: str, count: int = 3) -> str:
    cleaned = _normalise(text)
    if not cleaned:
        return ""
    end_marks = (".", "!", "?")
    pieces: list[str] = []
    cursor = 0
    while cursor < len(cleaned) and len(pieces) < count:
        for marker in end_marks:
            position = cleaned.find(marker, cursor)
            if position >= 0:
                pieces.append(cleaned[cursor : position + 1].strip())
                cursor = position + 1
                break
        else:
            pieces.append(cleaned[cursor:].strip())
            cursor = len(cleaned)
            break
    summary = " ".join(piece for piece in pieces if piece)
    return summary[:MAX_RESPONSE_LENGTH]


def _reasoning_hint(prompt: str) -> str:
    cleaned = _normalise(prompt)
    digest = hashlib.sha256(cleaned.encode("utf-8")).hexdigest()[:8]
    return (
        f"Local helper. Reasoning trace id {digest}. "
        "Deterministic offline mechanical-engineering helper; no network "
        "egress, no solver runs and no fabricated results."
    )


def _to_mm(value: float, unit: str | None) -> float:
    return value * _MM_PER_UNIT.get(unit or "mm", 1.0)


def _format_number(value: float) -> str:
    magnitude = abs(value)
    if magnitude and (magnitude >= 1e15 or magnitude < 1e-6):
        return f"{value:.3g}"
    text = f"{value:.3f}".rstrip("0").rstrip(".")
    return text or "0"


def _detect_kind(lowered: str) -> str | None:
    for keyword, kind in _PRIMITIVE_KEYWORDS:
        if re.search(rf"\b{keyword}\b", lowered):
            return kind
    return None


def _detect_suites(lowered: str) -> tuple[str, ...]:
    found: list[str] = []
    for keyword, label in _SUITE_NAMES:
        if keyword in lowered and label not in found:
            found.append(label)
    return tuple(found)


def _label_value(lowered: str, *labels: str) -> float | None:
    for label in labels:
        pattern = re.compile(rf"(?<![\w]){re.escape(label)}\s*[:=]?\s*({_NUMBER})\s*{_UNIT}")
        match = pattern.search(lowered)
        if match:
            return _to_mm(float(match.group(1)), match.group(2))
    return None


def _numbers(lowered: str) -> list[float]:
    return [_to_mm(float(match.group(1)), match.group(2)) for match in _NUMBER_RE.finditer(lowered)]


def _box_estimate(size_x: float, size_y: float, size_z: float) -> str:
    volume = size_x * size_y * size_z
    area = 2 * (size_x * size_y + size_y * size_z + size_z * size_x)
    dims = f"{_format_number(size_x)}x{_format_number(size_y)}x{_format_number(size_z)}"
    return (
        f"Nominal analytic estimate (box {dims} mm): "
        f"volume {_format_number(volume)} mm^3 "
        f"({_format_number(volume / 1000)} cm^3), "
        f"surface area {_format_number(area)} mm^2."
    )


def _cylinder_estimate(radius: float, height: float) -> str:
    volume = math.pi * radius * radius * height
    area = 2 * math.pi * radius * (radius + height)
    return (
        f"Nominal analytic estimate "
        f"(cylinder r={_format_number(radius)} h={_format_number(height)} mm): "
        f"volume {_format_number(volume)} mm^3 "
        f"({_format_number(volume / 1000)} cm^3), "
        f"surface area {_format_number(area)} mm^2."
    )


def _sphere_estimate(radius: float) -> str:
    volume = 4 / 3 * math.pi * radius**3
    area = 4 * math.pi * radius * radius
    return (
        f"Nominal analytic estimate (sphere r={_format_number(radius)} mm): "
        f"volume {_format_number(volume)} mm^3 "
        f"({_format_number(volume / 1000)} cm^3), "
        f"surface area {_format_number(area)} mm^2."
    )


def _estimate(prompt: str) -> str:
    """Return a nominal analytic estimate line, or ``""`` when no dimensions.

    Only geometry is computed; the helper never emits a solver quantity.
    """
    lowered = prompt.lower()
    kind = _detect_kind(lowered)

    triple = _TRIPLE_RE.search(lowered)
    if triple:
        unit = triple.group(4)
        return _box_estimate(
            _to_mm(float(triple.group(1)), unit),
            _to_mm(float(triple.group(2)), unit),
            _to_mm(float(triple.group(3)), unit),
        )

    size_x = _label_value(lowered, "size_x", "size x", "x")
    size_y = _label_value(lowered, "size_y", "size y", "y")
    size_z = _label_value(lowered, "size_z", "size z", "z")
    if size_x is not None and size_y is not None and size_z is not None:
        return _box_estimate(size_x, size_y, size_z)

    radius = _label_value(lowered, "radius", "r")
    height = _label_value(lowered, "height", "h")
    numbers = _numbers(lowered)

    if kind == "sphere":
        candidate = radius if radius is not None else (numbers[0] if numbers else None)
        if candidate is not None:
            return _sphere_estimate(candidate)
    if kind == "cylinder":
        candidate_radius = radius if radius is not None else (numbers[0] if numbers else None)
        candidate_height = (
            height if height is not None else (numbers[1] if len(numbers) > 1 else None)
        )
        if candidate_radius is not None and candidate_height is not None:
            return _cylinder_estimate(candidate_radius, candidate_height)
    if kind == "box" and len(numbers) >= 3:
        return _box_estimate(numbers[0], numbers[1], numbers[2])
    if radius is not None and height is not None:
        return _cylinder_estimate(radius, height)
    if radius is not None:
        return _sphere_estimate(radius)
    if height is not None and numbers:
        return _cylinder_estimate(numbers[0], height)
    if kind is None and len(numbers) >= 3:
        return _box_estimate(numbers[0], numbers[1], numbers[2])
    return ""


def _opencascade_note(prompt: str) -> str:
    suites = _detect_suites(prompt.lower())
    note = (
        "OpenCascade (OCP) workbench is active by default for quick "
        "geometry/envelope estimates and neutral STEP/STL handoff to external "
        "CAD/CAE, and for complete standalone STEP/STL builds inside Kala "
        "Studio. One shared workflow for every user, without tiers; FEA, CFD, "
        "stress and thermal solver numbers are never fabricated."
    )
    if suites:
        note += (
            " Neutral STEP/STL exports are ready to import into "
            + ", ".join(suites)
            + "; the studio does not drive a native vendor API."
        )
    return note


def local_chat(prompt: str, *, model: str | None = None) -> ChatOutcome:
    """Return a deterministic offline mechanical-engineering reply."""
    if not isinstance(prompt, str):
        raise ProviderProtocolError("prompt must be a string")
    if len(prompt) > MAX_PROMPT_LENGTH:
        raise ProviderProtocolError("prompt exceeds the local provider's bounded length")
    summary = _first_sentences(prompt)
    if not summary:
        summary = "(empty prompt)"
    sections = (
        _reasoning_hint(prompt),
        _estimate(prompt),
        _opencascade_note(prompt),
        summary,
    )
    body = " ".join(section for section in sections if section).strip()
    body = body[:MAX_RESPONSE_LENGTH]
    return ChatOutcome(
        text=body,
        provider="local",
        model=model or DEFAULT_LOCAL_MODEL,
        offline=True,
    )


__all__ = (
    "DEFAULT_LOCAL_MODEL",
    "MAX_PROMPT_LENGTH",
    "MAX_RESPONSE_LENGTH",
    "local_chat",
)
