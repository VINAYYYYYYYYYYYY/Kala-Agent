"""Tests for CAD backend argument validation and error handling."""

from __future__ import annotations

import pytest

from kala.cad.mock import MockBackend


class TestCreateCylinderValidation:
    """Test create_cylinder argument validation."""

    def test_valid_cylinder(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius=5.0, height=10.0)
        assert result.ok
        assert result.data["radius"] == 5.0
        assert result.data["height"] == 10.0
        assert "body_id" in result.data

    def test_negative_radius(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius=-5.0, height=10.0)
        assert not result.ok
        assert "radius must be positive" in result.message

    def test_zero_radius(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius=0.0, height=10.0)
        assert not result.ok
        assert "radius must be positive" in result.message

    def test_negative_height(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius=5.0, height=-10.0)
        assert not result.ok
        assert "height must be positive" in result.message

    def test_zero_height(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius=5.0, height=0.0)
        assert not result.ok
        assert "height must be positive" in result.message

    def test_invalid_radius_type(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius="not_a_number", height=10.0)  # type: ignore
        assert not result.ok
        assert "radius must be positive" in result.message

    def test_invalid_height_type(self) -> None:
        backend = MockBackend()
        result = backend.create_cylinder(radius=5.0, height="not_a_number")  # type: ignore
        assert not result.ok
        assert "height must be positive" in result.message


class TestBooleanCutErrorHandling:
    """Test boolean_cut error handling for missing bodies."""

    def test_missing_body_a(self) -> None:
        backend = MockBackend()
        result = backend.boolean_cut(body_a="nonexistent_a", body_b="Box_1")
        assert not result.ok
        assert "body_a" in result.message.lower() or "not found" in result.message.lower()

    def test_missing_body_b(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10)
        result = backend.boolean_cut(body_a="Box_1", body_b="nonexistent_b")
        assert not result.ok
        assert "body_b" in result.message.lower() or "not found" in result.message.lower()

    def test_both_missing(self) -> None:
        backend = MockBackend()
        result = backend.boolean_cut(body_a="nonexistent_a", body_b="nonexistent_b")
        assert not result.ok
        assert "not found" in result.message.lower()

    def test_successful_cut(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10)
        backend.create_box(5, 5, 5)
        result = backend.boolean_cut(body_a="Box_1", body_b="Box_2")
        assert result.ok
        assert "body_id" in result.data
        assert result.data["removed"] == ["Box_1", "Box_2"]


class TestBooleanFuseErrorHandling:
    """Test boolean_fuse error handling for missing bodies."""

    def test_missing_body_a(self) -> None:
        backend = MockBackend()
        result = backend.boolean_fuse(body_a="nonexistent_a", body_b="Box_1")
        assert not result.ok
        assert "body_a" in result.message.lower() or "not found" in result.message.lower()

    def test_missing_body_b(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10)
        result = backend.boolean_fuse(body_a="Box_1", body_b="nonexistent_b")
        assert not result.ok
        assert "body_b" in result.message.lower() or "not found" in result.message.lower()

    def test_successful_fuse(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10)
        backend.create_box(5, 5, 5)
        result = backend.boolean_fuse(body_a="Box_1", body_b="Box_2")
        assert result.ok
        assert "body_id" in result.data
        assert result.data["removed"] == ["Box_1", "Box_2"]


class TestFilletErrorHandling:
    """Test fillet error handling for missing bodies."""

    def test_missing_body(self) -> None:
        backend = MockBackend()
        result = backend.fillet(body_id="nonexistent", radius=2.0)
        assert not result.ok
        assert "not found" in result.message.lower()

    def test_successful_fillet(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10)
        result = backend.fillet(body_id="Box_1", radius=2.0)
        assert result.ok
        assert "body_id" in result.data
        assert result.data["radius"] == 2.0
        assert "Box_1" in result.data["removed"]


class TestAssemblyExport:
    """Test assembly export uses compound for multi-body."""

    def test_assembly_export_creates_file(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10, label="Part1")
        backend.create_cylinder(5, 20, label="Part2")
        
        result = backend.export(body_id="ALL", path="test_assembly.step")
        assert result.ok
        assert result.data["body_id"] == "ASSEMBLY"
        assert result.data["body_count"] == 2

    def test_single_body_export(self) -> None:
        backend = MockBackend()
        backend.create_box(10, 10, 10)
        
        result = backend.export(body_id="Box_1", path="test_single.step")
        assert result.ok
        assert result.data["body_id"] == "Box_1"
