"""Analysis backends for geometry validation and reporting."""

from __future__ import annotations

from kala.analysis.base import AnalysisBackend, AnalysisReport, AnalysisRequest
from kala.analysis.freecad_fem import FreeCadFemCalculiXBackend
from kala.analysis.geometry_probe import GeometryProbeBackend

__all__ = [
    "AnalysisBackend",
    "AnalysisReport",
    "AnalysisRequest",
    "FreeCadFemCalculiXBackend",
    "GeometryProbeBackend",
]
