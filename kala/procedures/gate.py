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
class PartPlan:
    """BOM / part-list sketch only — bind procedures per part later (P1+)."""

    parts: list[str]
    notes: str = ""

    def to_dict(self) -> dict:
        return {"kind": "part_plan", "parts": list(self.parts), "notes": self.notes}


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


def _sketch_parts(g: str) -> list[str]:
    catalog = (
        ("motor", "motor"),
        ("nema", "motor"),
        ("gear", "gear"),
        ("bearing", "bearing"),
        ("shaft", "shaft"),
        ("housing", "housing"),
        ("cover", "cover"),
        ("plate", "plate"),
        ("bracket", "bracket"),
        ("bolt", "fastener"),
        ("screw", "fastener"),
        ("frame", "frame"),
        ("base", "base"),
        ("fixture", "fixture"),
    )
    seen: list[str] = []
    for key, label in catalog:
        if key in g and label not in seen:
            seen.append(label)
    return seen or ["base", "features", "fasteners"]


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
