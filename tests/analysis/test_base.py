"""Tests for analysis base protocol and data structures."""

from __future__ import annotations

from kala.analysis.base import AnalysisBackend, AnalysisReport, AnalysisRequest


def test_analysis_request_creation():
    """Test creating an AnalysisRequest."""
    request = AnalysisRequest(body_id="Box_1", backend_handle=None)
    assert request.body_id == "Box_1"
    assert request.backend_handle is None


def test_analysis_report_creation():
    """Test creating an AnalysisReport."""
    report = AnalysisReport(
        body_id="Box_1",
        ok=True,
        message="All checks passed",
        metrics={"volume": 1000.0},
    )
    assert report.body_id == "Box_1"
    assert report.ok is True
    assert report.message == "All checks passed"
    assert report.metrics["volume"] == 1000.0


def test_analysis_report_to_dict():
    """Test AnalysisReport serialization."""
    report = AnalysisReport(
        body_id="Box_1",
        ok=True,
        message="All checks passed",
        metrics={"volume": 1000.0, "is_valid": True},
    )
    data = report.to_dict()
    assert data["body_id"] == "Box_1"
    assert data["ok"] is True
    assert data["message"] == "All checks passed"
    assert data["metrics"]["volume"] == 1000.0
    assert data["metrics"]["is_valid"] is True


def test_analysis_backend_protocol():
    """Test that AnalysisBackend is a protocol."""
    
    class MockBackend:
        def analyze(self, request: AnalysisRequest) -> AnalysisReport:
            return AnalysisReport(
                body_id=request.body_id,
                ok=True,
                message="Mock analysis",
                metrics={},
            )
    
    backend = MockBackend()
    assert isinstance(backend, AnalysisBackend)
    
    request = AnalysisRequest(body_id="Test_1")
    report = backend.analyze(request)
    assert report.body_id == "Test_1"
    assert report.ok is True
