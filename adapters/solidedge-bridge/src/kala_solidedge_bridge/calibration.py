"""Signed probe calibration records.

A calibration record is written only by a probe that measured a real part. The
record is signed with a per-data-directory key, so editing the JSON by hand
invalidates it; readiness is never derived from an unsigned flag. Even a valid
record does not make native production builds available: no live-verified
production executor exists in this milestone.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import secrets
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Final

from .workspace import (
    atomic_write_json,
    ensure_private_dir,
    read_json,
    restrict_windows_acl,
)

PROBE_VERSION: Final[str] = "1"
TEMPLATE_PROGID: Final[str] = "SolidEdge.PartDocument"
BINDING: Final[str] = "win32com.dynamic+gencache-constants"
CALIBRATION_SCHEMA: Final[int] = 1
_KEY_BYTES: Final[int] = 32


def canonical_json(payload: Mapping[str, Any]) -> bytes:
    return json.dumps(
        dict(payload),
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def host_fingerprint(
    *,
    se_version: str,
    template: str = TEMPLATE_PROGID,
    binding: str = BINDING,
    probe_version: str = PROBE_VERSION,
) -> str:
    """Fingerprint the exact host evidence a record belongs to."""
    material = {
        "se_version": se_version,
        "template": template,
        "binding": binding,
        "probe_version": probe_version,
    }
    return hashlib.sha256(canonical_json(material)).hexdigest()


def _triple(values: object) -> tuple[float, float, float]:
    if not isinstance(values, (list, tuple)) or len(values) != 3:
        raise ValueError("range must contain exactly three numbers")
    first, second, third = (float(value) for value in values)
    return (first, second, third)


@dataclass(frozen=True)
class CalibrationRecord:
    calibration_id: str
    probe_version: str
    se_version: str
    template: str
    binding: str
    fingerprint: str
    recorded_at: str
    volume_mm3: float
    range_min_mm: tuple[float, float, float]
    range_max_mm: tuple[float, float, float]
    par_sha256: str
    step_sha256: str
    signature: str
    raw: Mapping[str, Any]


class CalibrationStore:
    """Verify and look up probe records in a private directory."""

    def __init__(self, data_dir: Path) -> None:
        self.data_dir = data_dir
        self.records_dir = data_dir / "calibration"
        self.key_path = data_dir / "calibration.key"

    def _key(self) -> bytes:
        ensure_private_dir(self.data_dir)
        if self.key_path.exists():
            return self.key_path.read_bytes()
        key = secrets.token_bytes(_KEY_BYTES)
        try:
            descriptor = self.key_path.open("xb")
        except FileExistsError:
            return self.key_path.read_bytes()
        try:
            descriptor.write(key)
            descriptor.flush()
            import os

            os.fsync(descriptor.fileno())
        finally:
            descriptor.close()
        self.key_path.chmod(0o600)
        restrict_windows_acl(self.key_path)
        return key

    def _signature(self, payload: Mapping[str, Any]) -> str:
        return hmac.new(self._key(), canonical_json(payload), hashlib.sha256).hexdigest()

    def record_probe(
        self,
        *,
        se_version: str,
        volume_mm3: float,
        range_min_mm: tuple[float, float, float],
        range_max_mm: tuple[float, float, float],
        par_sha256: str,
        step_sha256: str,
        template: str = TEMPLATE_PROGID,
        binding: str = BINDING,
        probe_version: str = PROBE_VERSION,
    ) -> CalibrationRecord:
        """Persist a signed record for a probe that measured a valid part."""
        fingerprint = host_fingerprint(
            se_version=se_version,
            template=template,
            binding=binding,
            probe_version=probe_version,
        )
        recorded_at = datetime.now(UTC).isoformat()
        calibration_id = f"cal-{fingerprint[:24]}"
        payload: dict[str, Any] = {
            "schema": CALIBRATION_SCHEMA,
            "calibration_id": calibration_id,
            "probe_version": probe_version,
            "se_version": se_version,
            "template": template,
            "binding": binding,
            "fingerprint": fingerprint,
            "recorded_at": recorded_at,
            "geometry": {
                "volume_mm3": volume_mm3,
                "range_min_mm": list(range_min_mm),
                "range_max_mm": list(range_max_mm),
            },
            "artifacts": {
                "par_sha256": par_sha256,
                "step_sha256": step_sha256,
            },
            "native_build_verified": False,
        }
        signature = self._signature(payload)
        signed = dict(payload)
        signed["signature"] = signature
        ensure_private_dir(self.records_dir)
        atomic_write_json(self.records_dir / f"{calibration_id}.json", signed)
        return CalibrationRecord(
            calibration_id=calibration_id,
            probe_version=probe_version,
            se_version=se_version,
            template=template,
            binding=binding,
            fingerprint=fingerprint,
            recorded_at=recorded_at,
            volume_mm3=volume_mm3,
            range_min_mm=range_min_mm,
            range_max_mm=range_max_mm,
            par_sha256=par_sha256,
            step_sha256=step_sha256,
            signature=signature,
            raw=signed,
        )

    def _verify(self, payload: Mapping[str, Any]) -> bool:
        signature = payload.get("signature")
        if not isinstance(signature, str):
            return False
        unsigned = {key: value for key, value in payload.items() if key != "signature"}
        expected = self._signature(unsigned)
        if not hmac.compare_digest(expected, signature):
            return False
        if payload.get("schema") != CALIBRATION_SCHEMA:
            return False
        if payload.get("template") != TEMPLATE_PROGID or payload.get("binding") != BINDING:
            return False
        geometry = payload.get("geometry")
        if not isinstance(geometry, dict):
            return False
        volume = geometry.get("volume_mm3")
        if isinstance(volume, bool) or not isinstance(volume, (int, float)) or volume <= 0:
            return False
        expected_fingerprint = host_fingerprint(
            se_version=str(payload.get("se_version", "")),
            template=str(payload.get("template", "")),
            binding=str(payload.get("binding", "")),
            probe_version=str(payload.get("probe_version", "")),
        )
        return payload.get("fingerprint") == expected_fingerprint

    def load_all(self) -> tuple[CalibrationRecord, ...]:
        """Return every record whose signature and fingerprint verify."""
        if not self.records_dir.is_dir():
            return ()
        records: list[CalibrationRecord] = []
        for path in sorted(self.records_dir.glob("*.json")):
            payload = read_json(path)
            if not isinstance(payload, dict) or not self._verify(payload):
                continue
            try:
                geometry = payload["geometry"]
                artifacts = payload["artifacts"]
                records.append(
                    CalibrationRecord(
                        calibration_id=str(payload["calibration_id"]),
                        probe_version=str(payload["probe_version"]),
                        se_version=str(payload["se_version"]),
                        template=str(payload["template"]),
                        binding=str(payload["binding"]),
                        fingerprint=str(payload["fingerprint"]),
                        recorded_at=str(payload["recorded_at"]),
                        volume_mm3=float(geometry["volume_mm3"]),
                        range_min_mm=_triple(geometry["range_min_mm"]),
                        range_max_mm=_triple(geometry["range_max_mm"]),
                        par_sha256=str(artifacts["par_sha256"]),
                        step_sha256=str(artifacts["step_sha256"]),
                        signature=str(payload["signature"]),
                        raw=payload,
                    )
                )
            except (KeyError, TypeError, ValueError):
                continue
        return tuple(records)

    def find(
        self,
        *,
        se_version: str,
        template: str = TEMPLATE_PROGID,
        binding: str = BINDING,
        probe_version: str = PROBE_VERSION,
    ) -> CalibrationRecord | None:
        """Return the newest matching verified record, if any."""
        fingerprint = host_fingerprint(
            se_version=se_version,
            template=template,
            binding=binding,
            probe_version=probe_version,
        )
        matches = [record for record in self.load_all() if record.fingerprint == fingerprint]
        if not matches:
            return None
        return max(matches, key=lambda record: record.recorded_at)
