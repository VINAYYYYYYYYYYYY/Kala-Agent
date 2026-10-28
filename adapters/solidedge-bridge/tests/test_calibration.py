"""Signed calibration records and tamper detection."""

from __future__ import annotations

import hashlib
import hmac
import json
import os
from pathlib import Path
from typing import Any

import pytest

from kala_solidedge_bridge.calibration import (
    BINDING,
    CALIBRATION_SCHEMA,
    PROBE_VERSION,
    TEMPLATE_PROGID,
    CalibrationStore,
    canonical_json,
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


def _forged_payload() -> dict[str, Any]:
    """A structurally valid record signed with an empty key."""
    payload: dict[str, Any] = {
        "schema": CALIBRATION_SCHEMA,
        "calibration_id": "cal-forged",
        "probe_version": PROBE_VERSION,
        "se_version": "2024.0",
        "template": TEMPLATE_PROGID,
        "binding": BINDING,
        "fingerprint": host_fingerprint(se_version="2024.0"),
        "recorded_at": "2026-01-01T00:00:00+00:00",
        "geometry": {
            "volume_mm3": 1000.0,
            "range_min_mm": [-5.0, -5.0, -5.0],
            "range_max_mm": [5.0, 5.0, 5.0],
        },
        "artifacts": {"par_sha256": "a" * 64, "step_sha256": "b" * 64},
        "native_build_verified": False,
    }
    payload["signature"] = hmac.new(b"", canonical_json(payload), hashlib.sha256).hexdigest()
    return payload


def test_an_empty_key_file_cannot_verify_a_forged_record(tmp_path: Path) -> None:
    records = tmp_path / "calibration"
    records.mkdir()
    (tmp_path / "calibration.key").write_bytes(b"")
    (records / "cal-forged.json").write_text(json.dumps(_forged_payload()), encoding="utf-8")

    store = CalibrationStore(tmp_path)

    assert store.load_all() == ()
    assert store.find(se_version="2024.0") is None
    # Verification never creates or repairs the key as a side effect.
    assert (tmp_path / "calibration.key").read_bytes() == b""


def test_a_short_key_is_replaced_before_any_record_is_signed(tmp_path: Path) -> None:
    (tmp_path / "calibration.key").write_bytes(b"short")
    store = CalibrationStore(tmp_path)
    _record(store)

    assert len((tmp_path / "calibration.key").read_bytes()) == 32
    assert store.find(se_version="2024.0") is not None


def test_verification_does_not_create_a_missing_key(tmp_path: Path) -> None:
    records = tmp_path / "calibration"
    records.mkdir()
    (records / "any.json").write_text(json.dumps({"schema": CALIBRATION_SCHEMA}), encoding="utf-8")

    store = CalibrationStore(tmp_path)

    assert store.load_all() == ()
    assert not (tmp_path / "calibration.key").exists()


def test_an_unreadable_record_does_not_abort_the_load(tmp_path: Path) -> None:
    store = CalibrationStore(tmp_path)
    _record(store)
    # A directory matching the glob cannot be read as JSON; PermissionError on
    # Windows and IsADirectoryError on POSIX are both OSError.
    (tmp_path / "calibration" / "unreadable.json").mkdir()

    found = store.find(se_version="2024.0")

    assert found is not None
    assert found.volume_mm3 == 1000.0
