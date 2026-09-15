"""Tests for FreeCadFemCalculiXBackend."""

from __future__ import annotations

from unittest.mock import Mock, patch

from kala.analysis.base import AnalysisRequest
from kala.analysis.freecad_fem import FreeCadFemCalculiXBackend


def test_freecad_fem_no_backend():
    """Test FEM backend soft-fails when no backend handle provided."""
    backend = FreeCadFemCalculiXBackend()
    request = AnalysisRequest(body_id="Box_1", backend_handle=None)
    report = backend.analyze(request)
    
    assert report.body_id == "Box_1"
    assert report.ok is False
    assert "No backend handle" in report.message
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "no_backend"


def test_freecad_fem_missing_doc():
    """Test FEM backend soft-fails when backend has no _doc."""
    backend = FreeCadFemCalculiXBackend()
    mock_backend = Mock()
    del mock_backend._doc
    request = AnalysisRequest(body_id="Box_1", backend_handle=mock_backend)
    report = backend.analyze(request)
    
    assert report.body_id == "Box_1"
    assert report.ok is False
    assert "no _doc attribute" in report.message
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "no_doc"


def test_freecad_fem_body_not_found():
    """Test FEM backend soft-fails when body not found in document."""
    backend = FreeCadFemCalculiXBackend()
    mock_doc = Mock()
    mock_doc.getObject.return_value = None
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Missing_1", backend_handle=mock_backend)
    report = backend.analyze(request)
    
    assert report.body_id == "Missing_1"
    assert report.ok is False
    assert "Body not found" in report.message
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "body_not_found"


def test_freecad_fem_no_shape():
    """Test FEM backend soft-fails when body has no Shape."""
    backend = FreeCadFemCalculiXBackend()
    mock_obj = Mock(spec=[])
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="NoShape_1", backend_handle=mock_backend)
    report = backend.analyze(request)
    
    assert report.body_id == "NoShape_1"
    assert report.ok is False
    assert "no Shape attribute" in report.message
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "no_shape"


def test_freecad_fem_invalid_geometry():
    """Test FEM backend detects invalid geometry during preflight."""
    backend = FreeCadFemCalculiXBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = False
    mock_shape.Volume = 1000.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Invalid_1", backend_handle=mock_backend)
    report = backend.analyze(request)
    
    assert report.body_id == "Invalid_1"
    assert report.ok is False
    assert "invalid" in report.message.lower()
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "invalid_geometry"


def test_freecad_fem_zero_volume():
    """Test FEM backend detects zero volume during preflight."""
    backend = FreeCadFemCalculiXBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.Volume = 0.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    request = AnalysisRequest(body_id="Zero_1", backend_handle=mock_backend)
    report = backend.analyze(request)
    
    assert report.body_id == "Zero_1"
    assert report.ok is False
    assert "zero or negative volume" in report.message.lower()
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "zero_volume"


def test_freecad_fem_skipped_no_wb():
    """Test FEM backend reports skipped_no_wb when Fem module unavailable."""
    backend = FreeCadFemCalculiXBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.Volume = 1000.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    # Mock import to raise ImportError for Fem module
    with patch("builtins.__import__", side_effect=ImportError("No module named 'Fem'")):
        request = AnalysisRequest(body_id="Box_1", backend_handle=mock_backend)
        report = backend.analyze(request)
    
    assert report.body_id == "Box_1"
    assert report.ok is False
    assert "skipped_no_wb" in report.message
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "skipped_no_wb"
    # Verify no fake stress metrics
    assert "von_mises" not in report.metrics
    assert "max_stress" not in report.metrics
    # Verify geometry info is present
    assert report.metrics.get("is_valid") is True
    assert report.metrics.get("volume") == 1000.0


def test_freecad_fem_protocol_compliance():
    """Test FEM backend implements AnalysisBackend protocol."""
    from kala.analysis.base import AnalysisBackend
    
    backend = FreeCadFemCalculiXBackend()
    assert isinstance(backend, AnalysisBackend)


def test_freecad_fem_soft_fail_on_exception():
    """Test FEM backend soft-fails on any exception."""
    backend = FreeCadFemCalculiXBackend()
    
    mock_backend = Mock()
    mock_backend._doc = Mock()
    mock_backend._doc.getObject.side_effect = RuntimeError("Unexpected error")
    
    request = AnalysisRequest(body_id="Error_1", backend_handle=mock_backend)
    report = backend.analyze(request)
    
    assert report.body_id == "Error_1"
    assert report.ok is False
    assert "FEM analysis failed" in report.message
    assert report.kind == "freecad_fem_calculix"
    assert report.solver_status == "error"


def test_freecad_fem_no_fake_metrics():
    """Test FEM backend never generates fake von Mises or stress numbers."""
    backend = FreeCadFemCalculiXBackend()
    
    # Test with missing Fem module (skipped_no_wb path)
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.Volume = 1000.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = Mock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    with patch("builtins.__import__", side_effect=ImportError("No module")):
        request = AnalysisRequest(body_id="Test_1", backend_handle=mock_backend)
        report = backend.analyze(request)
    
    # Verify NO fake stress metrics in any failure mode
    forbidden_keys = ["von_mises", "max_stress", "min_stress", "stress", "displacement"]
    for key in forbidden_keys:
        assert key not in report.metrics, f"Found forbidden fake metric: {key}"
    
    # Only allowed: geometry validation metrics
    allowed_keys = {"is_valid", "volume"}
    for key in report.metrics:
        assert key in allowed_keys, f"Unexpected metric key: {key}"


def test_freecad_fem_setup_only_returns_false():
    """Test FEM backend returns ok=False when prerequisites fail (setup_only path)."""
    from unittest.mock import MagicMock, patch
    import sys
    
    backend = FreeCadFemCalculiXBackend()
    
    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.Volume = 1000.0
    
    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    
    mock_doc = MagicMock()
    mock_doc.getObject.return_value = mock_obj
    
    mock_backend = Mock()
    mock_backend._doc = mock_doc
    
    mock_solver = MagicMock()
    mock_mesh = MagicMock()
    mock_material = MagicMock()
    mock_analysis = MagicMock()
    
    mock_objects_fem = MagicMock()
    mock_objects_fem.makeAnalysis.return_value = mock_analysis
    mock_objects_fem.makeSolverCalculix.return_value = mock_solver
    mock_objects_fem.makeMaterialSolid.return_value = mock_material
    mock_objects_fem.makeMeshGmsh.return_value = mock_mesh
    mock_objects_fem.makeConstraintFixed.return_value = MagicMock(References=[])
    mock_objects_fem.makeConstraintSelfWeight.return_value = MagicMock()
    
    mock_fea = MagicMock()
    mock_fea.check_prerequisites.return_value = "Missing prerequisite: test error"
    
    mock_ccxtools = MagicMock()
    mock_ccxtools.FemToolsCcx.return_value = mock_fea
    
    mock_femtools = MagicMock()
    mock_femtools.ccxtools = mock_ccxtools
    
    with patch.dict(sys.modules, {
        'Fem': MagicMock(),
        'ObjectsFem': mock_objects_fem,
        'femtools': mock_femtools,
        'femtools.ccxtools': mock_ccxtools,
    }):
        with patch("kala.analysis.freecad_fem.Path"):
            backend._check_solver_available = lambda: True
            
            request = AnalysisRequest(body_id="Setup_1", backend_handle=mock_backend)
            report = backend.analyze(request)
            
            assert report.ok is False
            assert report.solver_status in ["setup_only", "constraints_incomplete"]
            assert "prerequisite" in report.message.lower() or "incomplete" in report.message.lower()


def test_freecad_fem_extract_results_with_stress_data():
    """Test FEM backend extracts real von Mises stress from result objects."""
    from unittest.mock import MagicMock
    
    backend = FreeCadFemCalculiXBackend()
    
    mock_result_obj = MagicMock()
    mock_result_obj.vonMises = [100.5, 250.3, 150.0, 300.8, 200.1]
    mock_result_obj.DisplacementLengths = [0.1, 0.2, 0.15, 0.25, 0.18]
    mock_result_obj.Mesh = MagicMock()
    
    mock_analysis = MagicMock()
    mock_analysis.Group = [mock_result_obj]
    
    mock_doc = MagicMock()
    
    mock_fea = MagicMock()
    mock_fea.ccx_stdout = "CalculiX output"
    
    from pathlib import Path
    report_path = Path("test_report.json")
    metrics = {"is_valid": True, "volume": 1000.0}
    
    report = backend._extract_results(mock_doc, mock_analysis, "Test_1", metrics, report_path, mock_fea)
    
    assert report.ok is True
    assert report.body_id == "Test_1"
    assert report.solver_status == "completed"
    assert "max_von_mises_stress_mpa" in report.metrics
    assert report.metrics["max_von_mises_stress_mpa"] == 300.8
    assert report.metrics["min_von_mises_stress_mpa"] == 100.5
    assert report.metrics["num_nodes"] == 5
    assert "max_displacement_mm" in report.metrics
    assert report.metrics["ccx_stdout_available"] is True


def test_freecad_fem_extract_results_no_stress_data():
    """Test FEM backend returns ok=False when result object has no stress data."""
    from unittest.mock import MagicMock
    
    backend = FreeCadFemCalculiXBackend()
    
    mock_result_obj = MagicMock()
    mock_result_obj.vonMises = []
    mock_result_obj.Mesh = MagicMock()
    
    mock_analysis = MagicMock()
    mock_analysis.Group = [mock_result_obj]
    
    mock_doc = MagicMock()
    
    from pathlib import Path
    report_path = Path("test_report.json")
    metrics = {"is_valid": True, "volume": 1000.0}
    
    report = backend._extract_results(mock_doc, mock_analysis, "Test_1", metrics, report_path)
    
    assert report.ok is False
    assert report.solver_status == "no_stress_data"
    assert "von_mises" not in report.metrics
    assert "max_stress" not in report.metrics


def test_freecad_fem_extract_results_no_result_objects():
    """Test FEM backend returns ok=False when no result objects are found."""
    from unittest.mock import MagicMock
    
    backend = FreeCadFemCalculiXBackend()
    
    mock_analysis = MagicMock()
    mock_analysis.Group = []
    
    mock_doc = MagicMock()
    
    from pathlib import Path
    report_path = Path("test_report.json")
    metrics = {"is_valid": True, "volume": 1000.0}
    
    report = backend._extract_results(mock_doc, mock_analysis, "Test_1", metrics, report_path)

    assert report.ok is False
    assert report.solver_status == "no_results"


def test_freecad_fem_material_name_is_solids():
    """Test FEM backend creates material with name 'Solids'."""
    from unittest.mock import MagicMock, patch
    import sys

    backend = FreeCadFemCalculiXBackend()

    mock_shape = Mock()
    mock_shape.isValid.return_value = True
    mock_shape.Volume = 1000.0
    mock_shape.Solids = []

    mock_obj = Mock()
    mock_obj.Shape = mock_shape
    mock_obj.Name = "Body_1"

    mock_doc = MagicMock()
    mock_doc.getObject.return_value = mock_obj

    mock_backend = Mock()
    mock_backend._doc = mock_doc

    mock_material = MagicMock()
    mock_material.Name = "Solids"

    mock_objects_fem = MagicMock()
    mock_objects_fem.makeAnalysis = MagicMock()
    mock_objects_fem.makeSolverCalculix = MagicMock()
    mock_objects_fem.makeMaterialSolid.return_value = mock_material
    mock_objects_fem.makeMeshGmsh = MagicMock()
    mock_objects_fem.makeConstraintFixed = MagicMock(References=[])
    mock_objects_fem.makeConstraintSelfWeight = MagicMock()

    mock_femtools = MagicMock()
    mock_femtools.ccxtools = MagicMock()

    with patch.dict(sys.modules, {
        'Fem': MagicMock(),
        'ObjectsFem': mock_objects_fem,
        'femtools': mock_femtools,
        'femtools.ccxtools': MagicMock(),
    }):
        with patch("kala.analysis.freecad_fem.Path"):
            backend._check_solver_available = lambda: True

            request = AnalysisRequest(body_id="Solids_1", backend_handle=mock_backend)
            report = backend.analyze(request)

            assert report.body_id == "Solids_1"
            mock_objects_fem.makeMaterialSolid.assert_called_once()
            call_args = mock_objects_fem.makeMaterialSolid.call_args
            assert "Solids" in str(call_args)


def test_freecad_fem_pick_lowest_z_face():
    """Test _pick_fixed_face returns the face with lowest Z center of mass."""
    from unittest.mock import MagicMock

    backend = FreeCadFemCalculiXBackend()

    face_low = MagicMock()
    face_low.CenterOfMass.z = -5.0
    face_mid = MagicMock()
    face_mid.CenterOfMass.z = 0.0
    face_high = MagicMock()
    face_high.CenterOfMass.z = 10.0

    mock_shape = MagicMock()
    mock_shape.Faces = [face_high, face_mid, face_low]

    mock_obj = MagicMock()
    mock_obj.Shape = mock_shape

    result = backend._pick_fixed_face(mock_obj)

    assert result == "Face3"


def test_freecad_fem_pick_fixed_face_no_faces():
    """Test _pick_fixed_face returns None when no faces exist."""
    from unittest.mock import MagicMock

    backend = FreeCadFemCalculiXBackend()

    mock_shape = MagicMock()
    mock_shape.Faces = []

    mock_obj = MagicMock()
    mock_obj.Shape = mock_shape

    result = backend._pick_fixed_face(mock_obj)

    assert result is None
