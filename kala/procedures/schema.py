"""Procedure playbook schema and loader."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class ProcedureStep:
    id: str
    goal: str
    allowed_tools: list[str] = field(default_factory=list)
    exit_criteria: str = ""
    optional_parts: bool = False


@dataclass
class Procedure:
    id: str
    name: str
    description: str
    steps: list[ProcedureStep]

    def step(self, index: int) -> ProcedureStep | None:
        if 0 <= index < len(self.steps):
            return self.steps[index]
        return None


def load_procedure(path: Path) -> Procedure:
    data = json.loads(path.read_text(encoding="utf-8"))
    steps = [
        ProcedureStep(
            id=s["id"],
            goal=s["goal"],
            allowed_tools=list(s.get("allowed_tools", [])),
            exit_criteria=s.get("exit_criteria", ""),
            optional_parts=bool(s.get("optional_parts", False)),
        )
        for s in data["steps"]
    ]
    return Procedure(
        id=data["id"],
        name=data["name"],
        description=data.get("description", ""),
        steps=steps,
    )


def library_dir() -> Path:
    return Path(__file__).resolve().parent / "library"


def load_default_procedure(procedure_id: str = "simple_bracket") -> Procedure:
    path = library_dir() / f"{procedure_id}.json"
    if not path.exists():
        raise FileNotFoundError(f"Procedure not found: {path}")
    return load_procedure(path)


def list_procedures() -> list[Procedure]:
    """Load every procedure JSON shipped under the package library."""
    root = library_dir()
    procs: list[Procedure] = []
    for path in sorted(root.glob("*.json")):
        procs.append(load_procedure(path))
    return procs


def list_procedure_ids() -> list[str]:
    return [p.id for p in list_procedures()]


def require_known_procedure(procedure_id: str) -> None:
    """Raise ValueError if procedure_id is not in the packaged library."""
    if procedure_id not in list_procedure_ids():
        known = ", ".join(list_procedure_ids())
        raise ValueError(f"Unknown procedure {procedure_id!r}. Known: {known}")


def resolve_procedure_for_send(
    goal: str,
    procedure_id: str,
    *,
    user_picked: bool,
) -> str:
    """Return procedure id for a run after the goal gate has passed.

    Callers must handle ``assess_goal`` first (``ClarifyNeeded`` / ``PartPlan``).
    Re-checks the gate so an explicit procedure pick cannot override it.
    Keyword suggest applies only when the user has not chosen a procedure.
    """
    from kala.procedures.gate import assess_goal

    if assess_goal(goal) is not None:
        raise ValueError(
            "resolve_procedure_for_send called on a gated goal; "
            "handle ClarifyNeeded/PartPlan before binding a procedure"
        )
    if user_picked:
        return procedure_id
    suggested = suggest_procedure(goal)
    return suggested if suggested else procedure_id


def suggest_procedure(goal: str) -> str | None:
    """Map design brief keywords to a packaged procedure id.

    Returns None when assess_goal gates (product-like / multi-part) so callers
    never silently default those briefs to simple_bracket.
    """
    from kala.procedures.gate import assess_goal

    if assess_goal(goal) is not None:
        return None
    g = (goal or "").lower()
    known = set(list_procedure_ids())
    rules: list[tuple[tuple[str, ...], str]] = [
        (("shaft", "bushing", "stepped"), "stepped_shaft"),
        (("housing", "cover", "lid"), "housing_cover"),
        (("plate", "holes", "hole pattern"), "plate_with_holes"),
        (("assembly", "machine", "fixture"), "machine_assembly"),
        (("bracket", "flange", "l-bracket"), "simple_bracket"),
    ]
    for keys, pid in rules:
        if any(k in g for k in keys) and pid in known:
            return pid
    return "simple_bracket" if "simple_bracket" in known else next(iter(sorted(known)), "simple_bracket")

