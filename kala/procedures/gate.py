"""Decompose-or-clarify gate before procedure bind / done.

Product-like or multi-part briefs must emit ClarifyNeeded or a PartPlan BOM
sketch — never silently map to simple_bracket and finish on envelope-only.
Full per-part assembly runner is out of scope (next wave).
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class ClarifyNeeded:
    reason: str
    questions: list[str] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "kind": "clarify_needed",
            "reason": self.reason,
            "questions": list(self.questions),
        }


@dataclass
class PartSpec:
    """One BOM line — bind a packaged procedure later (P1+)."""

    local_name: str
    brief: str = ""
    procedure_id: str | None = None  # existing library id or null; never invent
    keep_separate: bool = True

    def to_dict(self) -> dict:
        return {
            "local_name": self.local_name,
            "brief": self.brief,
            "procedure_id": self.procedure_id,
            "keep_separate": self.keep_separate,
        }


@dataclass
class PartPlan:
    """BOM / part-list sketch only — bind procedures per part later (P1+)."""

    parts: list[PartSpec]
    notes: str = ""

    def to_dict(self) -> dict:
        return {
            "kind": "part_plan",
            "parts": [p.to_dict() for p in self.parts],
            "notes": self.notes,
        }


_PRODUCT_LIKE = (
    "laptop",
    "notebook",
    "phone",
    "smartphone",
    "tablet",
    "iphone",
    "ipad",
    "car",
    "vehicle",
    "automobile",
    "drone",
    "uav",
    "robot",
    "watch",
    "camera",
    "consumer",
)

_MULTIPART = (
    "multi-part",
    "multipart",
    "multi part",
    "assembly",
    "gearbox",
    "gear box",
    "bill of materials",
)

# Single-part mechanical cues that still bind a packaged playbook.
_SINGLE_MECH = (
    "bracket",
    "l-bracket",
    "flange",
    "plate",
    "shaft",
    "bushing",
    "stepped",
    "housing",
    "cover",
    "lid",
)

# Best-effort map from part label → existing packaged procedure id only.
# Labels with no packaged playbook stay procedure_id=None (never invent).
_PART_PROCEDURE_HINTS: dict[str, str] = {
    "shaft": "stepped_shaft",
    "housing": "housing_cover",
    "cover": "housing_cover",
    "plate": "plate_with_holes",
    "bracket": "simple_bracket",
}


def _known_procedure_ids() -> set[str]:
    from kala.procedures.schema import list_procedure_ids

    return set(list_procedure_ids())


def _suggest_part_procedure(local_name: str) -> str | None:
    """Return an existing library procedure id for this part, or None."""
    hint = _PART_PROCEDURE_HINTS.get(local_name)
    if hint is None:
        return None
    known = _known_procedure_ids()
    return hint if hint in known else None


def _sketch_parts(g: str) -> list[PartSpec]:
    catalog = (
        ("motor", "motor", "drive motor"),
        ("nema", "motor", "NEMA stepper / motor"),
        ("gear", "gear", "gear / mesh"),
        ("bearing", "bearing", "bearing"),
        ("shaft", "shaft", "shaft"),
        ("housing", "housing", "housing"),
        ("cover", "cover", "cover / lid"),
        ("plate", "plate", "plate"),
        ("bracket", "bracket", "bracket / flange"),
        ("bolt", "fastener", "fastener"),
        ("screw", "fastener", "fastener"),
        ("frame", "frame", "frame"),
        ("base", "base", "base"),
        ("fixture", "fixture", "fixture"),
    )
    seen: list[PartSpec] = []
    names: set[str] = set()
    for key, label, brief in catalog:
        if key in g and label not in names:
            names.add(label)
            seen.append(
                PartSpec(
                    local_name=label,
                    brief=brief,
                    procedure_id=_suggest_part_procedure(label),
                    keep_separate=True,
                )
            )
    if seen:
        return seen
    return [
        PartSpec(local_name="base", brief="primary body", procedure_id=_suggest_part_procedure("base"), keep_separate=True),
        PartSpec(local_name="features", brief="features / cuts", procedure_id=None, keep_separate=True),
        PartSpec(local_name="fasteners", brief="fasteners", procedure_id=None, keep_separate=True),
    ]


def assess_goal(goal: str) -> ClarifyNeeded | PartPlan | None:
    """Gate before bind/done. None = packaged procedure may bind."""
    g = (goal or "").lower().strip()
    if not g:
        return ClarifyNeeded(
            reason="Empty design brief.",
            questions=["What mechanical part should be modeled (bracket, plate, shaft, …)?"],
        )

    if any(k in g for k in _PRODUCT_LIKE):
        return ClarifyNeeded(
            reason=(
                "Product-like / out-of-domain brief — refuse silent simple_bracket "
                "envelope-only completion."
            ),
            questions=[
                "Which mechanical parts should be modeled (e.g. enclosure halves, hinges, standoffs)?",
                "Or restate as a single mechanical feature (L-bracket, plate, shaft) instead of the whole product.",
            ],
        )

    multipart = any(k in g for k in _MULTIPART)
    single = any(k in g for k in _SINGLE_MECH)
    # Strong multi-part cues always PartPlan; bare "assembly" alone also PartPlan
    # unless the brief is clearly a single packaged part (bracket/plate/…).
    strong_multi = any(
        k in g
        for k in ("multi-part", "multipart", "multi part", "gearbox", "gear box", "bill of materials")
    )
    if strong_multi or (multipart and not single):
        return PartPlan(
            parts=_sketch_parts(g),
            notes="BOM sketch only — bind procedures per part in a later wave.",
        )

    return None
