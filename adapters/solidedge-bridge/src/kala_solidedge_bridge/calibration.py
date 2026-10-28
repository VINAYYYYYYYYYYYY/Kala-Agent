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
import os
import secrets
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Final

from .workspace import (
    atomic_write_bytes,
    atomic_write_json,
    ensure_private_dir,
    read_json,
    restrict_windows_acl,
)

PROBE_VERSION: Final[str] = "2"
TEMPLATE_PROGID: Final[str] = "SolidEdge.PartDocument"
#: The binding reads installed type-library enums directly; it does not rely on
#: a partially generated ``win32com.client.constants`` module.
BINDING: Final[str] = "win32com.dynamic+typelib-enums"
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
    #: ``Profile.End`` code that this host returned for a profile proven valid
    #: by a successful extrusion and measured geometry (``-103`` on Solid Edge
    #: 2024). ``None`` for an older record or a host that returned 0.
    profile_end_code: int | None = None


class CalibrationStore:
    """Verify and look up probe records in a private directory."""

    def __init__(self, data_dir: Path) -> None:
        self.data_dir = data_dir
        self.records_dir = data_dir / "calibration"
        self.key_path = data_dir / "calibration.key"

    def _read_key(self) -> bytes | None:
        """Return the stored key, or ``None`` when it is missing or incomplete.

        A crash between creating the file and writing it can leave a short or
        empty key. Such a key must never sign or verify anything: an empty key
        would let anyone who can write a record forge its signature.
        """
        try:
            key = self.key_path.read_bytes()
        except FileNotFoundError:
            return None
        if len(key) != _KEY_BYTES:
            return None
        return key

    def _key(self) -> bytes:
        """Return the signing key, creating a complete one when needed.

        Creation is exclusive and always writes the full key before it is used,
        so a crash can leave a short file but never a key that signs a record.
        A short file is replaced atomically on the next signing operation.
        """
        key = self._read_key()
        if key is not None:
            return key
        ensure_private_dir(self.data_dir)
        candidate = secrets.token_bytes(_KEY_BYTES)
        # ``O_BINARY`` matters on Windows: without it ``os.write`` translates
        # 0x0A and the key file would not be exactly ``_KEY_BYTES`` long.
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
        try:
            descriptor = os.open(self.key_path, flags, 0o600)
        except FileExistsError:
            # Another process won the race; use its key only if it is complete.
            key = self._read_key()
            if key is not None:
                return key
            atomic_write_bytes(self.key_path, candidate)
        else:
            try:
                os.write(descriptor, candidate)
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            try:
                self.key_path.chmod(0o600)
            except OSError:
                pass
        restrict_windows_acl(self.key_path)
        return candidate

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
        profile_end_code: int | None = None,
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
                "profile_end_code": profile_end_code,
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
            profile_end_code=profile_end_code,
        )

    def _verify(self, payload: Mapping[str, Any]) -> bool:
        signature = payload.get("signature")
        if not isinstance(signature, str):
            return False
        signing_key = self._read_key()
        if signing_key is None:
            # Verification never creates a key: without a complete stored key
            # this data directory has no records it can vouch for.
            return False
        unsigned = {key: value for key, value in payload.items() if key != "signature"}
        expected = hmac.new(signing_key, canonical_json(unsigned), hashlib.sha256).hexdigest()
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
        raw_profile_end = geometry.get("profile_end_code")
        if raw_profile_end is not None and (
            isinstance(raw_profile_end, bool) or not isinstance(raw_profile_end, int)
        ):
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
            try:
                payload = read_json(path)
            except OSError:
                # One unreadable record must not hide the rest.
                continue
            if not isinstance(payload, dict):
                continue
            try:
                if not self._verify(payload):
                    continue
            except (OSError, TypeError, ValueError):
                continue
            try:
                geometry = payload["geometry"]
                artifacts = payload["artifacts"]
                raw_profile_end = geometry.get("profile_end_code")
                profile_end_code = (
                    int(raw_profile_end)
                    if isinstance(raw_profile_end, int) and not isinstance(raw_profile_end, bool)
                    else None
                )
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
                        profile_end_code=profile_end_code,
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
