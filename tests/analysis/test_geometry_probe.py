"""Tests for GeometryProbe backend."""

from __future__ import annotations

from unittest.mock import Mock

from kala.analysis.base import AnalysisRequest
from kala.analysis.geometry_probe import GeometryProbeBackend


def test_geometry_probe_no_backend():
    """Test GeometryProbe soft-fails when no backend handle provided."""
    probe = GeometryProbeBackend()
    request = AnalysisRequest(body_id="Box_1", backend_handle=None)
    report = probe.analyze(request)
    
    assert report.body_id == "Box_1"
    assert report.ok is False
    assert "No backend handle" in report.message
    assert report.metrics == {}


def test_geometry_probe_missing_doc():
    """Test GeometryProbe soft-fails when backend has no _doc."""
    probe = GeometryProbeBackend()
    mock_backend = Mock()
    del mock_backend._doc
    request = AnalysisRequest(body_id="Box_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Box_1"
    assert report.ok is False
    assert "no _doc attribute" in report.message


def test_geometry_probe_body_not_found():
    """Test GeometryProbe soft-fails when body not found in document."""
    probe = GeometryProbeBackend()
    mock_doc = Mock()
    mock_doc.getObject.return_value = None
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Missing_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Missing_1"
    assert report.ok is False
    assert "Body not found" in report.message


def test_geometry_probe_no_shape():
    """Test GeometryProbe soft-fails when body has no Shape."""
    probe = GeometryProbeBackend()
    mock_obj = Mock(spec=[])
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="NoShape_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "NoShape_1"
    assert report.ok is False
    assert "no Shape attribute" in report.message


def test_geometry_probe_valid_shape():
    """Test GeometryProbe succeeds with valid shape."""
    probe = GeometryProbeBackend()
    
    mock_bbox = Mock()
    mock_bbox.XMin = 0.0
    mock_bbox.YMin = 0.0
    mock_bbox.ZMin = 0.0
    mock_bbox.XMax = 10.0
    mock_bbox.YMax = 10.0
    mock_bbox.ZMax = 10.0
    mock_bbox.XLength = 10.0
    mock_bbox.YLength = 10.0
    mock_bbox.ZLength = 10.0
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.isNull.return_value = False
    mock_shape.Volume = 1000.0
    mock_shape.BoundBox = mock_bbox
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Box_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Box_1"
    assert report.ok is True
    assert "Geometry probe OK" in report.message
    assert report.metrics["is_valid"] is True
    assert report.metrics["is_null"] is False
    assert report.metrics["volume"] == 1000.0
    assert report.metrics["bbox"]["x_max"] == 10.0


def test_geometry_probe_invalid_shape():
    """Test GeometryProbe detects invalid shape."""
    probe = GeometryProbeBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = False
    mock_shape.isNull.return_value = False
    mock_shape.Volume = 1000.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Bad_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Bad_1"
    assert report.ok is False
    assert "invalid" in report.message.lower()


def test_geometry_probe_null_shape():
    """Test GeometryProbe detects null shape."""
    probe = GeometryProbeBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.isNull.return_value = True
    mock_shape.Volume = 0.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Null_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Null_1"
    assert report.ok is False
    assert "null" in report.message.lower()


def test_geometry_probe_zero_volume():
    """Test GeometryProbe detects zero volume."""
    probe = GeometryProbeBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.isNull.return_value = False
    mock_shape.Volume = 0.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Zero_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Zero_1"
    assert report.ok is False
    assert "zero or negative volume" in report.message.lower()


def test_geometry_probe_soft_fail_on_exception():
    """Test GeometryProbe soft-fails on any exception."""
    probe = GeometryProbeBackend()
    
    mock_backend = Mock()
    mock_backend._doc = Mock()
    mock_backend._doc.getObject.side_effect = RuntimeError("Unexpected error")
    
    request = AnalysisRequest(body_id="Error_1", backend_handle=mock_backend)
    report = probe.analyze(request)
    
    assert report.body_id == "Error_1"
    assert report.ok is False
    assert "Analysis failed" in report.message
