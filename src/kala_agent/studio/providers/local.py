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
from dataclasses import dataclass
from typing import Final

from kala_agent.studio.design.intent import (
    DEFAULT_BRACKET_DEPTH_MM,
    DEFAULT_BRACKET_WIDTH_MM,
    DesignIntent,
    HousingCall,
    LbracketCall,
    ShaftSegment,
    SteppedShaftCall,
)
from kala_agent.studio.providers.base import ChatOutcome, ProviderProtocolError

DEFAULT_LOCAL_MODEL: Final[str] = "deterministic-paraphrase-v1"
MAX_PROMPT_LENGTH: Final[int] = 4096
MAX_RESPONSE_LENGTH: Final[int] = 1024

_MM_PER_UNIT: Final[dict[str, float]] = {"mm": 1.0, "cm": 10.0, "m": 1000.0}
_NUMBER: Final[str] = r"\d+(?:\.\d+)?"
_UNIT: Final[str] = r"(mm|cm|m)?"
_SEPARATOR: Final[str] = r"(?:x|\u00d7|\*|by)"

# ISO 273 medium clearance holes for metric bolts: M5 -> 5.5 mm.
_ISO_273_MEDIUM: Final[dict[float, float]] = {
    3.0: 3.4,
    4.0: 4.5,
    5.0: 5.5,
    6.0: 6.6,
    8.0: 9.0,
    10.0: 11.0,
    12.0: 13.5,
    16.0: 17.5,
    20.0: 22.0,
}
_BRACKET_RE: Final[re.Pattern[str]] = re.compile(r"\b(?:l[\s-]?brackets?|brackets?)\b")
_STEPPED_RE: Final[re.Pattern[str]] = re.compile(r"\bstepped[\s-]shafts?\b")
_HOUSING_RE: Final[re.Pattern[str]] = re.compile(r"\b(?:housing|enclosure)\b")
_CYLINDRICAL_RE: Final[re.Pattern[str]] = re.compile(r"\b(?:cylindrical|round|circular)\b")
_FAMILY_LABELS: Final[dict[str, str]] = {
    "l_bracket": "L-bracket",
    "stepped_shaft": "stepped shaft",
    "housing_envelope": "rectangular housing envelope",
}


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


# --------------------------------------------------------------- design parsing


@dataclass(frozen=True, slots=True)
class DesignPrompt:
    """The strict design reading of one local-helper prompt.

    ``intent`` is present only when every required value was stated (or is a
    documented canonical template default). When it is ``None``, ``missing``
    names exactly what the operator has to state; the helper never guesses a
    dimension to fill the gap.
    """

    family: str
    intent: DesignIntent | None
    missing: tuple[str, ...] = ()
    notes: tuple[str, ...] = ()


def _normalise_lowered(prompt: str) -> str:
    return re.sub(r"\s+", " ", prompt.strip().lower())


def _mm_value(pattern: str, text: str) -> float | None:
    match = re.search(pattern, text)
    if match is None:
        return None
    return float(match.group(1))


def _detect_material(lowered: str) -> str | None:
    for keyword, material in (
        ("stainless steel", "stainless steel"),
        ("aluminium", "aluminium"),
        ("aluminum", "aluminium"),
        ("steel", "steel"),
        ("brass", "brass"),
        ("titanium", "titanium"),
        ("nylon", "nylon"),
        ("plastic", "plastic"),
    ):
        if re.search(rf"\b{re.escape(keyword)}\b", lowered):
            return material
    return None


def detect_design_family(prompt: str) -> str | None:
    """Return the template family a prompt asks for, or ``None``.

    The match is deliberately narrow: a family keyword must be present, not
    merely a loose noun. "Design a small clamp ... 12 mm shaft" is an
    ordinary analytic reply, while "make me a bracket" is an L-bracket turn
    that still has to ask for its dimensions. A cylindrical housing is not
    the rectangular ``housing_envelope`` template, so it stays an analytic
    reply rather than being forced into the wrong family.
    """
    lowered = _normalise_lowered(prompt)
    if not lowered:
        return None
    if _BRACKET_RE.search(lowered):
        return "l_bracket"
    if _STEPPED_RE.search(lowered):
        return "stepped_shaft"
    if _HOUSING_RE.search(lowered) and not _CYLINDRICAL_RE.search(lowered):
        return "housing_envelope"
    return None


def parse_design_prompt(prompt: str) -> DesignPrompt | None:
    """Strictly parse one local prompt into a template call or a missing list."""
    family = detect_design_family(prompt)
    if family is None:
        return None
    lowered = _normalise_lowered(prompt)
    if family == "l_bracket":
        return _parse_lbracket(lowered)
    if family == "stepped_shaft":
        return _parse_stepped_shaft(lowered)
    return _parse_housing(lowered)


def _parse_lbracket(lowered: str) -> DesignPrompt:
    notes: list[str] = []
    missing: list[str] = []

    wall = _mm_value(rf"({_NUMBER})\s*mm\s*(?:thick\s+)?wall", lowered)
    if wall is None:
        wall = _mm_value(rf"wall(?:\s+thickness)?\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)

    bore = _mm_value(
        rf"({_NUMBER})\s*mm\s*(?:centered|centred|center|centre)?\s*bore",
        lowered,
    )
    if bore is None:
        bore = _mm_value(rf"bore\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)

    width: float | None = None
    depth: float | None = None
    height: float | None = None
    triple = re.search(
        rf"({_NUMBER})\s*[x\u00d7]\s*({_NUMBER})\s*[x\u00d7]\s*({_NUMBER})\s*mm",
        lowered,
    )
    if triple is not None:
        width, depth, height = (
            float(triple.group(1)),
            float(triple.group(2)),
            float(triple.group(3)),
        )
    legs = re.search(
        rf"legs?\s*(?:of|are|:)?\s*({_NUMBER})\s*mm\s*(?:and|,)\s*({_NUMBER})\s*mm",
        lowered,
    )
    if legs is not None:
        width, depth = float(legs.group(1)), float(legs.group(2))
    if width is None:
        width = _mm_value(rf"width\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)
    if width is None:
        width = _mm_value(rf"({_NUMBER})\s*mm\s*(?:wide|across)", lowered)
    if depth is None:
        depth = _mm_value(rf"depth\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)
    if depth is None:
        depth = _mm_value(rf"({_NUMBER})\s*mm\s*(?:deep|long)", lowered)
    if height is None:
        height = _mm_value(rf"({_NUMBER})\s*mm\s*mounting\s+plate", lowered)
    if height is None:
        height = _mm_value(rf"({_NUMBER})\s*mm\s*(?:tall|high)", lowered)
    if height is None:
        height = _mm_value(rf"(?:height|tall|high)\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)

    hole_count = 0
    hole_d: float | None = None
    metric = re.search(rf"(\d+)\s*[x\u00d7]\s*m\s*({_NUMBER})", lowered)
    explicit = re.search(
        rf"(\d+)\s*[x\u00d7]\s*(?:\u00f8|dia(?:meter)?\s*)?({_NUMBER})\s*mm\s*"
        r"(?:corner\s+)?(?:mounting\s+)?(?:clearance\s+)?holes?",
        lowered,
    )
    count_only = re.search(r"(\d+)\s+(?:corner\s+)?(?:mounting\s+)?holes?\b", lowered)
    if metric is not None:
        hole_count = int(metric.group(1))
        nominal = float(metric.group(2))
        clearance = _ISO_273_MEDIUM.get(nominal)
        if clearance is None:
            missing.append(f"hole diameter (no ISO 273 medium value for M{nominal:g})")
        else:
            hole_d = clearance
            notes.append(
                f"{hole_count}x M{nominal:g} mapped to {clearance:g} mm clearance holes "
                "(ISO 273 medium)."
            )
    elif explicit is not None:
        hole_count = int(explicit.group(1))
        hole_d = float(explicit.group(2))
    elif count_only is not None:
        hole_count = int(count_only.group(1))
        missing.append("hole diameter (mm) or an M-size")
    if hole_count not in (0, 2, 4):
        missing.append("2 or 4 holes")
        hole_count = 0
    if hole_count and hole_d is None and not missing:
        missing.append("hole diameter (mm) or an M-size")

    if (
        width is None
        and depth is None
        and height is not None
        and (wall is not None or bore is not None or hole_count)
    ):
        width = DEFAULT_BRACKET_WIDTH_MM
        depth = DEFAULT_BRACKET_DEPTH_MM
        notes.append(
            f"Plate width {width:g} mm and depth {depth:g} mm are canonical template "
            "defaults; state them to override."
        )

    if width is None:
        missing.append("width (mm)")
    if depth is None:
        missing.append("depth (mm)")
    if height is None:
        missing.append("height (mm)")
    if wall is None:
        missing.append("wall thickness (mm)")
    if missing or width is None or depth is None or height is None or wall is None:
        return DesignPrompt(
            family="l_bracket",
            intent=None,
            missing=tuple(dict.fromkeys(missing)),
            notes=tuple(notes),
        )

    intent = DesignIntent(
        template=LbracketCall(
            width=width,
            depth=depth,
            height=height,
            wall=wall,
            bore_d=bore,
            hole_d=hole_d,
            hole_count=hole_count,
        ),
        material=_detect_material(lowered),
    )
    return DesignPrompt(family="l_bracket", intent=intent, notes=tuple(notes))


def _parse_stepped_shaft(lowered: str) -> DesignPrompt:
    notes: list[str] = []
    segments: list[tuple[float, float]] = []
    main = re.search(
        rf"({_NUMBER})\s*mm\s*(?:diameter|dia)\s*(?:by|and|,|[\u00d7x])\s*"
        rf"({_NUMBER})\s*mm\s*(?:long|length)",
        lowered,
    )
    if main is None:
        main = re.search(
            rf"({_NUMBER})\s*mm\s*(?:diameter|dia)\s*(?:by|and|,|[\u00d7x])\s*({_NUMBER})\s*mm",
            lowered,
        )
    if main is not None:
        segments.append((float(main.group(1)), float(main.group(2))))
    shoulder = re.search(
        rf"({_NUMBER})\s*mm\s*(?:diameter\s*)?shoulder\s*(?:of\s*)?({_NUMBER})\s*mm\s*"
        r"(?:long|length)",
        lowered,
    )
    if shoulder is not None:
        segments.append((float(shoulder.group(1)), float(shoulder.group(2))))

    if not segments:
        return DesignPrompt(
            family="stepped_shaft",
            intent=None,
            missing=("segment diameters and lengths (mm)",),
            notes=tuple(notes),
        )
    intent = DesignIntent(
        template=SteppedShaftCall(
            segments=tuple(
                ShaftSegment(diameter=diameter, length=length) for diameter, length in segments
            )
        ),
        material=_detect_material(lowered),
    )
    return DesignPrompt(family="stepped_shaft", intent=intent, notes=tuple(notes))


def _parse_housing(lowered: str) -> DesignPrompt:
    notes: list[str] = []
    missing: list[str] = []
    width: float | None = None
    depth: float | None = None
    height: float | None = None
    triple = re.search(
        rf"({_NUMBER})\s*[x\u00d7]\s*({_NUMBER})\s*[x\u00d7]\s*({_NUMBER})\s*mm",
        lowered,
    )
    if triple is not None:
        width, depth, height = (
            float(triple.group(1)),
            float(triple.group(2)),
            float(triple.group(3)),
        )
    if width is None:
        width = _mm_value(rf"width\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)
    if depth is None:
        depth = _mm_value(rf"depth\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)
    if height is None:
        height = _mm_value(rf"(?:height|tall|high)\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)
    wall = _mm_value(rf"wall(?:\s+thickness)?\s*(?:of|is|:|=)?\s*({_NUMBER})\s*mm", lowered)
    if wall is None:
        wall = _mm_value(rf"({_NUMBER})\s*mm\s*(?:thick\s+)?walls?", lowered)

    if width is None:
        missing.append("width (mm)")
    if depth is None:
        missing.append("depth (mm)")
    if height is None:
        missing.append("height (mm)")
    if wall is None:
        missing.append("wall thickness (mm)")
    if missing or width is None or depth is None or height is None or wall is None:
        return DesignPrompt(
            family="housing_envelope",
            intent=None,
            missing=tuple(missing),
            notes=tuple(notes),
        )
    intent = DesignIntent(
        template=HousingCall(width=width, depth=depth, height=height, wall=wall),
        material=_detect_material(lowered),
    )
    return DesignPrompt(family="housing_envelope", intent=intent, notes=tuple(notes))


def _design_prompt_reply(parsed: DesignPrompt) -> str:
    if parsed.intent is None:
        family = _FAMILY_LABELS.get(parsed.family, parsed.family.replace("_", " "))
        article = "an" if family[:1].lower() in "aeiou" else "a"
        notes = " ".join(parsed.notes)
        if notes:
            notes = f" {notes}"
        need = ", ".join(parsed.missing) or "a complete set of declared dimensions"
        return (
            f"Local helper. I can draft {article} {family}, but I need these declared "
            f"values: {need}.{notes} No design was compiled."
        )[:MAX_RESPONSE_LENGTH]
    notes = " ".join(parsed.notes)
    if notes:
        notes += " "
    return (
        f"Local helper. Design intent parsed from declared values. {notes}"
        "Compiling and measuring with OpenCASCADE."
    )[:MAX_RESPONSE_LENGTH]


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
    design_prompt = parse_design_prompt(prompt)
    if design_prompt is not None:
        return ChatOutcome(
            text=_design_prompt_reply(design_prompt),
            provider="local",
            model=model or DEFAULT_LOCAL_MODEL,
            offline=True,
        )
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
    "DesignPrompt",
    "detect_design_family",
    "local_chat",
    "parse_design_prompt",
)
