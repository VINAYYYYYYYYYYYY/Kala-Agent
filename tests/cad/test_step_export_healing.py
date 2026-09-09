"""Tests for STEP export healing logic."""

from __future__ import annotations

from unittest.mock import Mock, call


def test_heal_shape_for_export_null_shape():
    """Test that null shapes are handled gracefully."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = True
    
    result = backend._heal_shape_for_export(mock_shape)
    
    assert result == mock_shape
    mock_shape.isNull.assert_called_once()


def test_heal_shape_for_export_invalid_shape_calls_fix():
    """Test that invalid shapes trigger fix()."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = False
    mock_shape.Solids = []
    
    result = backend._heal_shape_for_export(mock_shape)
    
    mock_shape.fix.assert_called()
    assert mock_shape.isValid.call_count >= 1


def test_heal_shape_for_export_single_solid_extraction():
    """Test that single solid is extracted from shape."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_solid = Mock()
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    assert result == mock_solid


def test_heal_shape_for_export_multiple_solids_fuse():
    """Test that multiple solids are fused together."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_solid1 = Mock()
    mock_solid2 = Mock()
    mock_fused = Mock()
    mock_fused.isValid.return_value = True
    mock_fused.Volume = 100.0
    
    mock_solid1.fuse.return_value = mock_fused
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid1, mock_solid2]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    mock_solid1.fuse.assert_called_once_with(mock_solid2)
    assert result == mock_fused


def test_heal_shape_for_export_fused_invalid_triggers_fix():
    """Test that invalid fused shapes trigger fix()."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_solid1 = Mock()
    mock_solid2 = Mock()
    mock_fused = Mock()
    mock_fused.isValid.return_value = False
    mock_fused.Volume = 100.0
    
    mock_solid1.fuse.return_value = mock_fused
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid1, mock_solid2]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    mock_fused.fix.assert_called()


def test_heal_shape_for_export_final_fix_attempt():
    """Test that final validation and fix is attempted."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    
    # First call (initial check): invalid
    # Subsequent calls (after fixes): still invalid
    mock_shape.isValid.side_effect = [False, False, False, False]
    mock_shape.Solids = []
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should call fix() at least twice: initial fix + final fix
    assert mock_shape.fix.call_count >= 2


def test_heal_shape_for_export_fusion_exception_fallback():
    """Test that fusion exceptions fall back to original shape."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_solid1 = Mock()
    mock_solid2 = Mock()
    
    # Fusion raises exception
    mock_solid1.fuse.side_effect = RuntimeError("Fusion failed")
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid1, mock_solid2]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should fall back to original shape on exception
    assert result == mock_shape


def test_heal_shape_for_export_rejects_invalid_with_volume():
    """Test that fused shape with volume but invalid is rejected (AND condition)."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_solid1 = Mock()
    mock_solid2 = Mock()
    mock_fused = Mock()
    mock_fused.isValid.return_value = False  # Invalid
    mock_fused.Volume = 100.0  # But has volume
    mock_fused.fix = Mock()  # Fix doesn't help
    
    mock_solid1.fuse.return_value = mock_fused
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid1, mock_solid2]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should reject fused and fall back to original (invalid AND volume required)
    assert result == mock_shape
    assert result != mock_fused


def test_heal_shape_for_export_rejects_valid_with_zero_volume():
    """Test that fused shape that is valid but has zero volume is rejected (AND condition)."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_solid1 = Mock()
    mock_solid2 = Mock()
    mock_fused = Mock()
    mock_fused.isValid.return_value = True  # Valid
    mock_fused.Volume = 0.0  # But zero volume
    
    mock_solid1.fuse.return_value = mock_fused
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid1, mock_solid2]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should reject fused and fall back to original (valid AND volume>0 required)
    assert result == mock_shape
    assert result != mock_fused
