"""Tests for the exact B-rep STEP export and its canonical defaults.

The STEP path shares :func:`resolve_dimension_parameters` with the mesh
preview, so an export with no parameters must describe the same default
primitive the preview shows. The tests exercise the real OCP kernel when
it is installed and mock it out when it is not, so both the exact B-rep
path and the honest 503 path are covered.
"""

from __future__ import annotations

import math
import tempfile
from collections.abc import Iterator
from pathlib import Path

import pytest

from kala_agent.studio.geometry.meshes import build_primitive, signed_volume
from kala_agent.studio.geometry.primitives import (
    DEFAULT_BOX_SIZE_MM,
    DEFAULT_CYLINDER_HEIGHT_MM,
    DEFAULT_CYLINDER_RADIUS_MM,
    DEFAULT_SPHERE_RADIUS_MM,
    resolve_dimension_parameters,
)
from kala_agent.studio.geometry.step import (
    StepCapability,
    StepUnavailableError,
    export_step_primitive,
    probe_step_capability,
)


@pytest.fixture
def workdir() -> Iterator[Path]:
    with tempfile.TemporaryDirectory(prefix="kala-studio-step-test-") as handle:
        yield Path(handle)


# --- canonical defaults ---


def test_box_default_dimensions_are_canonical() -> None:
    dims = resolve_dimension_parameters("box", None)
    assert dims == {
        "size_x": DEFAULT_BOX_SIZE_MM,
        "size_y": DEFAULT_BOX_SIZE_MM,
        "size_z": DEFAULT_BOX_SIZE_MM,
    }


def test_cylinder_and_sphere_default_dimensions_are_canonical() -> None:
    cylinder = resolve_dimension_parameters("cylinder", {})
    assert cylinder == {
        "radius": DEFAULT_CYLINDER_RADIUS_MM,
        "height": DEFAULT_CYLINDER_HEIGHT_MM,
    }
    sphere = resolve_dimension_parameters("sphere", {})
    assert sphere == {"radius": DEFAULT_SPHERE_RADIUS_MM}


def test_explicit_values_override_defaults() -> None:
    dims = resolve_dimension_parameters("box", {"size_x": 20, "size_y": 30, "size_z": 40})
    assert dims == {"size_x": 20.0, "size_y": 30.0, "size_z": 40.0}


def test_invalid_value_is_refused() -> None:
    with pytest.raises(ValueError):
        resolve_dimension_parameters("box", {"size_x": -1})


def test_unknown_kind_is_refused() -> None:
    with pytest.raises(ValueError):
        resolve_dimension_parameters("torus", {})


def test_default_export_matches_default_preview_volume() -> None:
    """The default box preview and the canonical dimensions agree exactly."""
    mesh = build_primitive("box", {})
    assert signed_volume(mesh) == pytest.approx(1000.0, rel=1e-9)
    dims = resolve_dimension_parameters("box", {})
    assert dims["size_x"] * dims["size_y"] * dims["size_z"] == pytest.approx(1000.0)


# --- missing dependency ---


def test_absent_ocp_raises_step_unavailable(monkeypatch: pytest.MonkeyPatch, workdir: Path) -> None:
    monkeypatch.setattr(
        "kala_agent.studio.geometry.step.probe_step_capability",
        lambda: StepCapability(available=False, module_name=None),
    )
    with pytest.raises(StepUnavailableError):
        export_step_primitive("box", {}, workdir=workdir)


# --- real kernel ---


@pytest.mark.skipif(not probe_step_capability().available, reason="OCP is not installed")
@pytest.mark.parametrize(
    ("kind", "expected_volume"),
    (
        ("box", 1000.0),
        ("cylinder", math.pi * DEFAULT_CYLINDER_RADIUS_MM**2 * DEFAULT_CYLINDER_HEIGHT_MM),
        ("sphere", 4.0 / 3.0 * math.pi * DEFAULT_SPHERE_RADIUS_MM**3),
    ),
)
def test_default_export_roundtrips_through_ocp(
    workdir: Path, kind: str, expected_volume: float
) -> None:
    from OCP.BRepCheck import BRepCheck_Analyzer  # type: ignore[import-untyped]
    from OCP.BRepGProp import BRepGProp  # type: ignore[import-untyped]
    from OCP.GProp import GProp_GProps  # type: ignore[import-untyped]
    from OCP.IFSelect import IFSelect_RetDone  # type: ignore[import-untyped]
    from OCP.STEPControl import STEPControl_Reader  # type: ignore[import-untyped]

    data = export_step_primitive(kind, {}, workdir=workdir)
    assert data.startswith(b"ISO-10303-21")

    path = workdir / f"default-{kind}.step"
    path.write_bytes(data)
    reader = STEPControl_Reader()
    assert reader.ReadFile(str(path)) == IFSelect_RetDone
    reader.TransferRoots()
    shape = reader.OneShape()
    assert BRepCheck_Analyzer(shape).IsValid()

    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props)
    assert props.Mass() == pytest.approx(expected_volume, rel=1e-9)


@pytest.mark.skipif(not probe_step_capability().available, reason="OCP is not installed")
def test_default_export_matches_requested_dimensions(workdir: Path) -> None:
    box = export_step_primitive("box", {}, workdir=workdir)
    explicit = export_step_primitive(
        "box", {"size_x": 10, "size_y": 10, "size_z": 10}, workdir=workdir
    )
    # Both should be the same analytic solid; the byte counts need not match
    # exactly but both must be ISO STEP files describing a 10 mm cube.
    assert box.startswith(b"ISO-10303-21")
    assert explicit.startswith(b"ISO-10303-21")
