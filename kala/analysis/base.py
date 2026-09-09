"""Analysis backend protocol and data structures."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable


@dataclass
class AnalysisRequest:
    """Request for geometry analysis."""

    body_id: str
    backend_handle: Any = None


@dataclass
class AnalysisReport:
    """Analysis results for a CAD body."""

    body_id: str
    ok: bool
    message: str
    metrics: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "body_id": self.body_id,
            "ok": self.ok,
            "message": self.message,
            "metrics": self.metrics,
        }


@runtime_checkable
class AnalysisBackend(Protocol):
    """Protocol for geometry analysis backends."""

    def analyze(self, request: AnalysisRequest) -> AnalysisReport: ...
