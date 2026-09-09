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
