"""Tests for STEP export healing logic."""

from __future__ import annotations

from pathlib import Path
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


def test_heal_shape_for_export_filters_degenerate_solids():
    """Test that solids with zero or negative volume are filtered out."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    backend._try_fix = Mock(side_effect=lambda s: s)
    
    # Create valid solid and degenerate solid
    mock_valid_solid = Mock()
    mock_valid_solid.isValid.return_value = True
    mock_valid_solid.Volume = 100.0
    
    mock_degen_solid = Mock()
    mock_degen_solid.isValid.return_value = True
    mock_degen_solid.Volume = 0.0  # Zero volume
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_valid_solid, mock_degen_solid]
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should extract only the valid solid (degenerate filtered)
    assert result == mock_valid_solid


def test_heal_shape_for_export_progressive_fusion_with_validation():
    """Test that progressive fusion validates each step and skips problematic solids."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    # Create three solids: two good, one problematic
    mock_solid1 = Mock()
    mock_solid1.isValid.return_value = True
    mock_solid1.Volume = 100.0
    
    mock_solid2 = Mock()
    mock_solid2.isValid.return_value = True
    mock_solid2.Volume = 50.0
    
    mock_solid3 = Mock()
    mock_solid3.isValid.return_value = True
    mock_solid3.Volume = 25.0
    
    # First fusion succeeds
    mock_fused1 = Mock()
    mock_fused1.isValid.return_value = True
    mock_fused1.Volume = 150.0
    mock_solid1.fuse.return_value = mock_fused1
    
    # Second fusion fails (problematic solid)
    mock_fused1.fuse.side_effect = RuntimeError("Fusion error")
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = [mock_solid1, mock_solid2, mock_solid3]
    
    # Setup _try_fix to return input
    backend._try_fix = Mock(side_effect=lambda s: s)
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should return the first successful fusion (skipped problematic solid3)
    assert result == mock_fused1


def test_heal_shape_for_export_removes_splitter():
    """Test that removeSplitter is called to clean up geometry."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    backend._try_fix = Mock(side_effect=lambda s: s)
    
    mock_refined = Mock()
    mock_refined.isValid.return_value = True
    mock_refined.Volume = 100.0
    
    mock_copy = Mock()
    mock_copy.removeSplitter.return_value = mock_refined
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    mock_shape.Solids = []
    mock_shape.copy.return_value = mock_copy
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should call removeSplitter for cleanup
    mock_copy.removeSplitter.assert_called_once()
    assert result == mock_refined


def test_heal_shape_for_export_extracts_largest_valid_solid_as_fallback():
    """Test that if shape is invalid but has valid solids, largest is extracted."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    # Shape is invalid but contains valid solids
    mock_small_solid = Mock()
    mock_small_solid.isValid.return_value = True
    mock_small_solid.Volume = 50.0
    
    mock_large_solid = Mock()
    mock_large_solid.isValid.return_value = True
    mock_large_solid.Volume = 200.0
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    # Shape stays invalid throughout
    mock_shape.isValid.return_value = False
    mock_shape.Solids = [mock_small_solid, mock_large_solid]
    mock_shape.fix = Mock()  # Fix doesn't help
    
    result = backend._heal_shape_for_export(mock_shape)
    
    # Should extract the largest valid solid as fallback
    assert result == mock_large_solid


def test_try_fix_returns_shape_unchanged_if_valid():
    """Test that _try_fix returns valid shape unchanged."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = True
    
    result = backend._try_fix(mock_shape)
    
    assert result == mock_shape
    mock_shape.fix.assert_not_called()


def test_try_fix_calls_fix_on_invalid_shape():
    """Test that _try_fix calls fix() on invalid shape."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    backend = FreeCADBackend.__new__(FreeCADBackend)
    
    mock_shape = Mock()
    mock_shape.isNull.return_value = False
    mock_shape.isValid.return_value = False
    
    result = backend._try_fix(mock_shape)
    
    assert result == mock_shape
    mock_shape.fix.assert_called_once()


def test_export_step_compound_all_invalid_raises():
    """All-invalid shapes after fix loop must raise, not fall back to max(shapes)."""
    from kala.cad.freecad.backend import FreeCADBackend
    import pytest

    backend = FreeCADBackend.__new__(FreeCADBackend)
    backend._step_path = Path("/tmp/kala_test_never_written.step")

    def _bad_shape():
        s = Mock()
        s.isNull.return_value = False
        s.isValid.return_value = False
        s.Volume = 10.0
        return s

    obj1 = Mock()
    obj1.Name = "Box"
    obj1.Label = "Box"
    obj1.Shape = _bad_shape()
    obj2 = Mock()
    obj2.Name = "Cylinder"
    obj2.Label = "Cylinder"
    obj2.Shape = _bad_shape()

    backend._doc = Mock()
    backend._doc.Objects = [obj1, obj2]
    backend._try_fix = Mock(side_effect=lambda s: s)

    with pytest.raises(RuntimeError, match="No valid shapes"):
        backend._export_step_compound()


def test_export_step_compound_creates_soft_assembly():
    """Multiple valid shapes create a compound (soft assembly), not a single largest solid."""
    from kala.cad.freecad.backend import FreeCADBackend

    backend = FreeCADBackend.__new__(FreeCADBackend)
    backend._step_path = Path("/tmp/kala_test_soft_assembly.step")
    backend._Part = Mock()

    mock_shape1 = Mock()
    mock_shape1.isNull.return_value = False
    mock_shape1.isValid.return_value = True
    mock_shape1.Volume = 100.0

    mock_shape2 = Mock()
    mock_shape2.isNull.return_value = False
    mock_shape2.isValid.return_value = True
    mock_shape2.Volume = 50.0

    mock_compound = Mock()
    mock_compound.isNull.return_value = False
    mock_compound.isValid.return_value = True
    mock_compound.Volume = 150.0
    mock_compound.Solids = []
    mock_compound.removeSplitter.return_value = mock_compound
    backend._Part.makeCompound.return_value = mock_compound

    obj1 = Mock()
    obj1.Name = "Box"
    obj1.Label = "Box"
    obj1.Shape = mock_shape1
    obj2 = Mock()
    obj2.Name = "Cylinder"
    obj2.Label = "Cylinder"
    obj2.Shape = mock_shape2

    backend._doc = Mock()
    backend._doc.Objects = [obj1, obj2]
    backend._try_fix = Mock(side_effect=lambda s: s)

    backend._export_step_compound()

    backend._Part.makeCompound.assert_called_once()
    args = backend._Part.makeCompound.call_args[0][0]
    assert len(args) == 2


def test_export_step_compound_heals_compound():
    """Healing is applied to the soft assembly compound before export."""
    from kala.cad.freecad.backend import FreeCADBackend

    backend = FreeCADBackend.__new__(FreeCADBackend)
    backend._step_path = Path("/tmp/kala_test_healed_compound.step")
    backend._Part = Mock()

    mock_shape1 = Mock()
    mock_shape1.isNull.return_value = False
    mock_shape1.isValid.return_value = True
    mock_shape1.Volume = 100.0

    mock_shape2 = Mock()
    mock_shape2.isNull.return_value = False
    mock_shape2.isValid.return_value = True
    mock_shape2.Volume = 50.0

    mock_compound = Mock()
    mock_compound.isNull.return_value = False
    mock_compound.Solids = []
    mock_compound.removeSplitter.return_value = mock_compound
    backend._Part.makeCompound.return_value = mock_compound
    
    # Track that _heal_shape_for_export is called with the compound
    backend._heal_shape_for_export = Mock(return_value=mock_compound)

    obj1 = Mock()
    obj1.Name = "Box"
    obj1.Label = "Box"
    obj1.Shape = mock_shape1
    obj2 = Mock()
    obj2.Name = "Cylinder"
    obj2.Label = "Cylinder"
    obj2.Shape = mock_shape2

    backend._doc = Mock()
    backend._doc.Objects = [obj1, obj2]
    backend._try_fix = Mock(side_effect=lambda s: s)

    backend._export_step_compound()

    backend._Part.makeCompound.assert_called_once()
    # _heal_shape_for_export must be called on the compound (not a single solid)
    assert backend._heal_shape_for_export.called
