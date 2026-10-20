"""Signed calibration records and tamper detection."""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from kala_solidedge_bridge.calibration import (
    BINDING,
    TEMPLATE_PROGID,
    CalibrationStore,
    host_fingerprint,
)


def _record(store: CalibrationStore) -> None:
    store.record_probe(
        se_version="2024.0",
        volume_mm3=1000.0,
        range_min_mm=(-5.0, -5.0, -5.0),
        range_max_mm=(5.0, 5.0, 5.0),
        par_sha256="a" * 64,
        step_sha256="b" * 64,
    )


def test_recorded_probe_is_found_for_the_same_fingerprint(tmp_path: Path) -> None:
    store = CalibrationStore(tmp_path)
    _record(store)

    found = store.find(se_version="2024.0")

    assert found is not None
    assert found.calibration_id.startswith("cal-")
    assert found.volume_mm3 == 1000.0
    assert found.template == TEMPLATE_PROGID
    assert found.binding == BINDING


def test_user_edited_record_is_rejected(tmp_path: Path) -> None:
    store = CalibrationStore(tmp_path)
    _record(store)
    record_path = next((tmp_path / "calibration").glob("*.json"))
    payload = json.loads(record_path.read_text(encoding="utf-8"))
    payload["geometry"]["volume_mm3"] = 2000.0
    record_path.write_text(json.dumps(payload), encoding="utf-8")

    assert store.find(se_version="2024.0") is None


def test_record_belongs_only_to_its_host_fingerprint(tmp_path: Path) -> None:
    store = CalibrationStore(tmp_path)
    _record(store)

    assert store.find(se_version="2025.0") is None
    assert host_fingerprint(se_version="2024.0") != host_fingerprint(se_version="2025.0")


@pytest.mark.skipif(os.name == "nt", reason="POSIX mode bits are not meaningful on Windows")
def test_calibration_signing_key_is_private(tmp_path: Path) -> None:
    store = CalibrationStore(tmp_path)
    _record(store)

    assert (tmp_path / "calibration.key").stat().st_mode & 0o777 == 0o600


def test_probe_geometry_can_never_claim_a_native_build(tmp_path: Path) -> None:
    store = CalibrationStore(tmp_path)
    record = store.record_probe(
        se_version="2024.0",
        volume_mm3=1000.0,
        range_min_mm=(-5.0, -5.0, -5.0),
        range_max_mm=(5.0, 5.0, 5.0),
        par_sha256="a" * 64,
        step_sha256="b" * 64,
    )

    assert record.raw["native_build_verified"] is False
