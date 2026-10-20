"""Windows Solid Edge probe: installed evidence first, optional private cube.

The probe is deliberately conservative. It reports platform, interactive-session
hints, attach availability, ``Application.Version``, installed type-library
member metadata, and resolved enum names. It builds a 10 mm cube only when the
installed type information confirms the exact calls it needs; otherwise it
reports what is missing and records no calibration. All measurements come from
the host, never from a literal. A failed geometry probe leaves diagnostics, not
fake geometry.
"""

from __future__ import annotations

import os
import platform as platform_module
import struct
import sys
import uuid
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Final, Protocol

from .calibration import (
    BINDING,
    PROBE_VERSION,
    TEMPLATE_PROGID,
    CalibrationRecord,
    CalibrationStore,
)
from .session import (
    MK_E_UNAVAILABLE,
    BindingProtocol,
    ComUnavailable,
    Session,
    SessionTimeout,
    bounded_message,
    hresult_of,
)
from .units import m2_to_mm2, m3_to_mm3, m_to_mm, mm_to_m
from .workspace import DocumentLedger, atomic_write_json, sha256_file

PROBE_TIMEOUT_SECONDS: Final[float] = 300.0
CUBE_MM: Final[float] = 10.0
CUBE_HALF_MM: Final[float] = CUBE_MM / 2.0
CUBE_VOLUME_MM3: Final[float] = CUBE_MM**3
CUBE_VOLUME_TOLERANCE_MM3: Final[float] = CUBE_VOLUME_MM3 * 0.01
CUBE_EXTENT_TOLERANCE_MM: Final[float] = 0.01
REQUIRED_CONSTANTS: Final[tuple[str, ...]] = ("igSymmetric", "igProfileClosed")
OPTIONAL_CONSTANTS: Final[tuple[str, ...]] = ("seModelingModeOrdered",)
REQUIRED_DOC_MEMBERS: Final[tuple[str, ...]] = (
    "Models",
    "ProfileSets",
    "RefPlanes",
    "SaveAs",
    "SaveCopyAs",
    "Close",
)
_MISSING_WORK: Final[tuple[str, ...]] = (
    "confirm Student Edition automation access on the actual Windows host",
    "derive the RefPlanes index-to-normal mapping from measured GetNormal/GetRootPoint",
    "confirm the installed out-parameter VARIANT shapes for arrays and ProfileArray",
    "confirm the STEP translator behaviour behind SaveCopyAs/SaveAs and its ISO-10303-21 header",
    "record density/accuracy units accepted by ComputePhysicalProperties",
    "implement and live-verify a production native build executor (probe geometry is not a build executor)",
)


class ProbeSession(Protocol):
    def start(self, timeout: float = 15.0) -> None: ...

    def stop(self) -> bool: ...

    def run(
        self, operation: Callable[[BindingProtocol], Any], *, timeout: float | None = None
    ) -> Any: ...  # noqa: ANN401


@dataclass(frozen=True)
class ProbeOutcome:
    exit_code: int
    report: dict[str, Any]
    output: Path
    calibration: CalibrationRecord | None


def _utc_now() -> str:
    return datetime.now(UTC).isoformat()


def _platform_section(active_platform: str) -> dict[str, Any]:
    return {
        "sys_platform": active_platform,
        "python_version": platform_module.python_version(),
        "python_implementation": platform_module.python_implementation(),
        "python_bits": struct.calcsize("P") * 8,
        "machine": platform_module.machine(),
    }


def _host_section(active_platform: str) -> dict[str, Any]:
    session_name = os.environ.get("SESSIONNAME")
    interactive_hint = False
    if active_platform == "win32":
        interactive_hint = bool(session_name and session_name.upper() != "SERVICES")
    return {
        "session_name": session_name,
        "interactive_hint": interactive_hint,
        "interactive_evidence": (
            "SESSIONNAME is non-empty and not SERVICES; this is a hint, not proof of "
            "window-station access"
        ),
        "user": os.environ.get("USERNAME"),
    }


def _unsigned(exc: BaseException) -> int | None:
    return hresult_of(exc)


def _short(value: Any, limit: int = 300) -> str:  # noqa: ANN401
    text = str(value).replace("\r", " ").replace("\n", " ").strip()
    return text[:limit]


def _vector(value: Any, length: int) -> tuple[float, ...]:  # noqa: ANN401
    candidate = getattr(value, "value", value)
    if isinstance(candidate, (list, tuple)) and len(candidate) == length:
        return tuple(float(item) for item in candidate)
    raise ValueError(f"expected a {length}-element array from COM")


def _cube_measurement_valid(
    volume_mm3: float,
    minimum_mm: Sequence[float],
    maximum_mm: Sequence[float],
) -> bool:
    if abs(volume_mm3 - CUBE_VOLUME_MM3) > CUBE_VOLUME_TOLERANCE_MM3:
        return False
    extents = sorted(high - low for low, high in zip(minimum_mm, maximum_mm, strict=True))
    return all(abs(extent - CUBE_MM) <= CUBE_EXTENT_TOLERANCE_MM for extent in extents)


def _step_header_valid(path: Path) -> bool:
    try:
        head = path.read_bytes()[:128]
    except OSError:
        return False
    return b"ISO-10303-21" in head


def _object_members(binding: BindingProtocol, obj: Any) -> tuple[set[str], dict[str, Any]]:  # noqa: ANN401
    info = binding.describe_typelib(obj)
    status = info.get("status")
    methods = info.get("methods")
    if status != "ok" or not isinstance(methods, dict):
        return set(), info
    return {str(name) for name in methods}, info


def _gate(
    binding: BindingProtocol,
    obj: Any,  # noqa: ANN401
    required: Sequence[str],
    label: str,
) -> tuple[list[str], dict[str, Any]]:
    members, info = _object_members(binding, obj)
    missing = [f"{label}.{name}" for name in required if name not in members]
    if not members:
        missing.append(f"{label}: installed type information is unavailable")
    return missing, info


def _probe_native(
    binding: BindingProtocol,
    *,
    launch: bool,
    run_dir: Path,
    ledger: DocumentLedger,
    probe_id: str,
) -> dict[str, Any]:
    report: dict[str, Any] = {
        "attach": {"status": "not_attempted"},
        "application_version": None,
        "typelib": {},
        "constants": {"resolved": {}, "missing": list(REQUIRED_CONSTANTS)},
        "geometry": {"attempted": False, "reason": "not attempted"},
        "artifacts": [],
        "document": {"created": False, "closed": False},
    }

    try:
        application = binding.call(lambda: binding.attach(launch), retry_safe=True)
    except Exception as exc:
        hresult = _unsigned(exc)
        if hresult == MK_E_UNAVAILABLE:
            report["attach"] = {
                "status": "not_running",
                "hresult": hresult,
                "message": (
                    "no running Solid Edge application was found; start Solid Edge in "
                    "this desktop session or pass --launch"
                ),
            }
        else:
            report["attach"] = {
                "status": "failed",
                "hresult": hresult,
                "message": bounded_message(exc),
            }
        return report

    report["attach"] = {"status": "attached", "launch": launch}
    try:
        report["application_version"] = binding.version(application)
    except Exception as exc:
        report["application_version"] = None
        report["attach"]["version_error"] = bounded_message(exc)

    report["typelib"]["application"] = binding.describe_typelib(application)
    resolved, missing = binding.resolve_constants(
        application, REQUIRED_CONSTANTS + OPTIONAL_CONSTANTS
    )
    missing_required = [name for name in REQUIRED_CONSTANTS if name not in resolved]
    report["constants"] = {"resolved": resolved, "missing": missing}
    if missing_required:
        report["geometry"] = {
            "attempted": False,
            "reason": "required enum names could not be resolved from the installed type library",
            "missing_constants": missing_required,
        }
        return report

    document: Any | None = None
    document_path: str | None = None
    tag = f"KalaProbe-{PROBE_VERSION}-{probe_id}"
    try:
        document = binding.call(
            lambda: application.Documents.Add(TEMPLATE_PROGID), retry_safe=False
        )
        report["document"] = {"created": True, "closed": False, "tag": tag}
        ledger.append(
            {
                "event": "created",
                "owner": "probe",
                "probe_id": probe_id,
                "tag": tag,
                "at": _utc_now(),
            }
        )
        try:
            document.Title = tag
            report["document"]["tagged"] = True
        except Exception as exc:
            report["document"]["tagged"] = False
            report["document"]["tag_error"] = bounded_message(exc)

        missing_members, document_info = _gate(
            binding, document, REQUIRED_DOC_MEMBERS, "PartDocument"
        )
        report["typelib"]["document"] = document_info
        models = getattr(document, "Models", None)
        if models is None:
            missing_members.append("PartDocument.Models")
        else:
            model_missing, models_info = _gate(
                binding, models, ("AddFiniteExtrudedProtrusion",), "Models"
            )
            missing_members.extend(model_missing)
            report["typelib"]["models"] = models_info
        if missing_members:
            report["geometry"] = {
                "attempted": False,
                "reason": "installed runtime members required for a safe exact probe are missing",
                "missing_members": missing_members,
            }
            return report

        report["geometry"] = _build_and_measure_cube(
            binding,
            document=document,
            run_dir=run_dir,
            report=report,
            probe_id=probe_id,
        )
        document_path = report["geometry"].get("saved_par")
        return report
    except Exception as exc:
        report["geometry"] = {
            "attempted": True,
            "valid": False,
            "error": bounded_message(exc),
            "hresult": _unsigned(exc),
        }
        return report
    finally:
        if document is not None:
            closed, close_error = _close_owned_document(binding, document)
            report["document"]["closed"] = closed
            if close_error is not None:
                report["document"]["close_error"] = close_error
            ledger.append(
                {
                    "event": "closed" if closed else "close_failed",
                    "owner": "probe",
                    "probe_id": probe_id,
                    "tag": tag,
                    "document_name": document_path,
                    "at": _utc_now(),
                }
            )


def _close_owned_document(binding: BindingProtocol, document: Any) -> tuple[bool, str | None]:  # noqa: ANN401
    """Close the document this probe created. Never close ActiveDocument."""
    try:
        binding.call(lambda: document.Close(False), retry_safe=False)
    except Exception as exc:
        return False, bounded_message(exc)
    return True, None


def _build_and_measure_cube(
    binding: BindingProtocol,
    *,
    document: Any,  # noqa: ANN401
    run_dir: Path,
    report: dict[str, Any],
    probe_id: str,
) -> dict[str, Any]:
    constants: dict[str, int] = report["constants"]["resolved"]
    geometry: dict[str, Any] = {"attempted": True, "valid": False, "attempts": []}

    modeling_mode = getattr(document, "ModelingMode", None)
    try:
        geometry["previous_modeling_mode"] = (
            int(modeling_mode) if modeling_mode is not None else None
        )
    except (TypeError, ValueError):
        geometry["previous_modeling_mode"] = _short(modeling_mode, 64)
    if "seModelingModeOrdered" in constants:
        try:
            document.ModelingMode = constants["seModelingModeOrdered"]
            geometry["modeling_mode_set"] = True
        except Exception as exc:
            geometry["modeling_mode_set"] = False
            geometry["modeling_mode_error"] = bounded_message(exc)

    profile_set = document.ProfileSets.Add()
    ref_plane = document.RefPlanes.Item(1)
    plane: dict[str, Any] = {"index": 1}
    for label, method_name in (("normal", "GetNormal"), ("root_point", "GetRootPoint")):
        buffer = binding.variant_byref_float_array([0.0, 0.0, 0.0])
        try:
            getattr(ref_plane, method_name)(buffer)
            plane[label] = [float(value) for value in _vector(buffer, 3)]
        except Exception as exc:
            plane[f"{label}_error"] = bounded_message(exc)
    geometry["ref_plane"] = plane

    profile = profile_set.Profiles.Add(ref_plane)
    line_missing, lines_info = _gate(
        binding, getattr(profile, "Lines2d", None), ("AddBy2Points",), "Lines2d"
    )
    report["typelib"]["lines"] = lines_info
    if line_missing:
        geometry["reason"] = "profile line API is not confirmed by the installed type library"
        geometry["missing_members"] = line_missing
        return geometry

    points = [
        (-CUBE_HALF_MM, -CUBE_HALF_MM),
        (CUBE_HALF_MM, -CUBE_HALF_MM),
        (CUBE_HALF_MM, CUBE_HALF_MM),
        (-CUBE_HALF_MM, CUBE_HALF_MM),
    ]
    for index in range(4):
        start = points[index]
        end = points[(index + 1) % 4]
        profile.Lines2d.AddBy2Points(
            mm_to_m(start[0]), mm_to_m(start[1]), mm_to_m(end[0]), mm_to_m(end[1])
        )
    validation = int(profile.End(constants["igProfileClosed"]))
    geometry["profile_end"] = validation
    if validation != 0:
        geometry["reason"] = "profile validation did not return 0"
        return geometry

    extrusion_arguments = (1, (profile,), constants["igSymmetric"], mm_to_m(CUBE_MM))
    # One documented form, one call. If the installed binding rejects it, the
    # probe reports the real failure and creates no second solid: a mutating
    # creation call must never be replayed after an exception, because the first
    # call may already have changed the document.
    try:
        model = document.Models.AddFiniteExtrudedProtrusion(*extrusion_arguments)
        geometry["attempts"].append({"profile_array": "tuple", "ok": True})
    except Exception as exc:
        geometry["attempts"].append(
            {
                "profile_array": "tuple",
                "ok": False,
                "hresult": _unsigned(exc),
                "error": bounded_message(exc),
            }
        )
        geometry["reason"] = (
            "AddFiniteExtrudedProtrusion rejected the documented tuple form; the mutating "
            "creation call was not retried, so no solid exists"
        )
        return geometry

    body = getattr(model, "Body", None)
    model_missing, model_info = _gate(binding, model, ("ComputePhysicalProperties",), "Model")
    body_missing, body_info = _gate(binding, body, ("GetRange",), "Body")
    report["typelib"]["model"] = model_info
    report["typelib"]["body"] = body_info
    missing = model_missing + body_missing
    method = model_info.get("methods", {}).get("ComputePhysicalProperties", {})
    if int(method.get("c_params", 0)) != 13:
        missing.append("Model.ComputePhysicalProperties: expected 13 parameters")
    if missing:
        geometry["reason"] = "measurement API is not confirmed by the installed type library"
        geometry["missing_members"] = missing
        return geometry

    minimum = binding.variant_byref_float_array([0.0, 0.0, 0.0])
    maximum = binding.variant_byref_float_array([0.0, 0.0, 0.0])
    if body is None:
        raise RuntimeError("the created model exposes no Body")
    range_result = binding.call(lambda: body.GetRange(minimum, maximum), retry_safe=True)
    if isinstance(range_result, (tuple, list)) and len(range_result) == 2:
        minimum_values = _vector(range_result[0], 3)
        maximum_values = _vector(range_result[1], 3)
    else:
        minimum_values = _vector(minimum, 3)
        maximum_values = _vector(maximum, 3)
    minimum_mm = [m_to_mm(value) for value in minimum_values]
    maximum_mm = [m_to_mm(value) for value in maximum_values]
    geometry["range_min_mm"] = minimum_mm
    geometry["range_max_mm"] = maximum_mm

    volume_variant = binding.variant_byref_float(0.0)
    area_variant = binding.variant_byref_float(0.0)
    mass_variant = binding.variant_byref_float(0.0)
    center_of_gravity = binding.variant_byref_float_array([0.0, 0.0, 0.0])
    center_of_volume = binding.variant_byref_float_array([0.0, 0.0, 0.0])
    global_moments = binding.variant_byref_float_array([0.0] * 9)
    principal_moments = binding.variant_byref_float_array([0.0, 0.0, 0.0])
    principal_axes = binding.variant_byref_float_array([0.0] * 9)
    radii_of_gyration = binding.variant_byref_float_array([0.0, 0.0, 0.0])
    relative_accuracy = binding.variant_byref_float(0.0)
    status = binding.variant_byref_int(0)
    binding.call(
        lambda: model.ComputePhysicalProperties(
            1000.0,
            0.001,
            volume_variant,
            area_variant,
            mass_variant,
            center_of_gravity,
            center_of_volume,
            global_moments,
            principal_moments,
            principal_axes,
            radii_of_gyration,
            relative_accuracy,
            status,
        ),
        retry_safe=True,
    )
    volume_m3 = float(getattr(volume_variant, "value", volume_variant))
    volume_mm3 = m3_to_mm3(volume_m3)
    physical_status = int(getattr(status, "value", status))
    geometry["volume_mm3"] = volume_mm3
    geometry["surface_area_mm2"] = m2_to_mm2(float(getattr(area_variant, "value", area_variant)))
    geometry["physical_properties_status"] = physical_status
    geometry["relative_accuracy"] = float(getattr(relative_accuracy, "value", relative_accuracy))

    par_path = run_dir / "probe.par"
    step_path = run_dir / "probe.stp"
    saved: list[str] = []
    try:
        binding.call(lambda: document.SaveAs(str(par_path)), retry_safe=False)
        if par_path.is_file() and par_path.stat().st_size > 0:
            saved.append("probe.par")
    except Exception as exc:
        geometry["save_par_error"] = bounded_message(exc)
    try:
        binding.call(lambda: document.SaveCopyAs(str(step_path)), retry_safe=False)
    except Exception as exc:
        geometry["save_copy_step_error"] = bounded_message(exc)
    if not step_path.is_file() or not _step_header_valid(step_path):
        try:
            binding.call(lambda: document.SaveAs(str(step_path)), retry_safe=False)
            if _step_header_valid(step_path):
                saved.append("probe.stp")
        except Exception as exc:
            geometry["save_step_error"] = bounded_message(exc)
    if step_path.is_file() and _step_header_valid(step_path) and "probe.stp" not in saved:
        saved.append("probe.stp")

    artifacts: list[dict[str, Any]] = []
    for name in ("probe.par", "probe.stp"):
        path = run_dir / name
        if path.is_file() and path.stat().st_size > 0:
            artifacts.append(
                {
                    "name": name,
                    "size_bytes": path.stat().st_size,
                    "sha256": sha256_file(path),
                }
            )
    geometry["saved_par"] = "probe.par" if par_path.is_file() else None
    geometry["saved_step"] = "probe.stp" if _step_header_valid(step_path) else None
    report["artifacts"] = artifacts
    geometry["valid"] = (
        (physical_status in (0, 1))
        and _cube_measurement_valid(volume_mm3, minimum_mm, maximum_mm)
        and _step_header_valid(step_path)
        and par_path.is_file()
    )
    if not geometry["valid"]:
        geometry["reason"] = (
            "measured geometry did not match a 10 mm cube within tolerance, or the "
            "STEP artifact was not produced"
        )
    geometry["probe_id"] = probe_id
    return geometry


def _base_report(active_platform: str) -> dict[str, Any]:
    return {
        "schema": 1,
        "probe_version": PROBE_VERSION,
        "binding": BINDING,
        "template": TEMPLATE_PROGID,
        "generated_at": _utc_now(),
        "platform": _platform_section(active_platform),
        "host": _host_section(active_platform),
        "attach": {"status": "not_attempted"},
        "application_version": None,
        "typelib": {},
        "constants": {"resolved": {}, "missing": list(REQUIRED_CONSTANTS)},
        "geometry": {"attempted": False, "reason": "not attempted"},
        "artifacts": [],
        "document": {"created": False, "closed": False},
        "calibration_required": True,
        "native_build_capability": False,
        "missing_probe_work": list(_MISSING_WORK),
    }


def run_probe(
    *,
    launch: bool,
    data_dir: Path,
    output: Path,
    active_platform: str | None = None,
    session_factory: Callable[[], ProbeSession] | None = None,
) -> ProbeOutcome:
    """Run the probe and write one diagnostic JSON document atomically.

    ``active_platform`` and ``session_factory`` are test seams. Production
    callers leave both unset.
    """
    host_platform = active_platform if active_platform is not None else sys.platform
    report = _base_report(host_platform)
    calibration_store = CalibrationStore(data_dir)
    calibration: CalibrationRecord | None = None

    if host_platform != "win32":
        report["attach"] = {
            "status": "unsupported",
            "message": (
                "non-Windows host: Solid Edge COM is unavailable; this is a development "
                "host, not a supported runtime"
            ),
        }
        report["geometry"] = {
            "attempted": False,
            "reason": "non-Windows host: native Solid Edge automation is unavailable",
        }
        atomic_write_json(output, report)
        return ProbeOutcome(exit_code=2, report=report, output=output, calibration=None)

    run_dir = (
        data_dir / "probe" / f"probe-{datetime.now(UTC):%Y%m%dT%H%M%SZ}-{uuid.uuid4().hex[:8]}"
    )
    run_dir.mkdir(parents=True, exist_ok=True)
    report["probe_run"] = {"run_dir": str(run_dir.relative_to(data_dir))}
    ledger = DocumentLedger(data_dir)
    probe_id = uuid.uuid4().hex[:16]
    session: ProbeSession | None = None
    clean_stop = False
    try:
        session = (session_factory or Session)()
        session.start()
        native = session.run(
            lambda binding: _probe_native(
                binding,
                launch=launch,
                run_dir=run_dir,
                ledger=ledger,
                probe_id=probe_id,
            ),
            timeout=PROBE_TIMEOUT_SECONDS,
        )
        if isinstance(native, dict):
            report.update(native)
    except ComUnavailable as exc:
        report["attach"] = {"status": "unavailable", "message": bounded_message(exc)}
    except SessionTimeout as exc:
        report["geometry"] = {
            "attempted": True,
            "valid": False,
            "reason": "probe exceeded its deadline; COM state is indeterminate",
            "error": bounded_message(exc),
        }
    except Exception as exc:
        report["error"] = bounded_message(exc)
        report["error_hresult"] = hresult_of(exc)
    finally:
        if session is not None:
            clean_stop = session.stop()
    report["session"] = {"stopped_cleanly": clean_stop}

    geometry = report.get("geometry")
    se_version = report.get("application_version")
    if (
        isinstance(geometry, dict)
        and geometry.get("valid") is True
        and isinstance(se_version, str)
        and se_version != "unknown"
    ):
        artifacts = {
            artifact.get("name"): artifact
            for artifact in report.get("artifacts", [])
            if isinstance(artifact, dict)
        }
        par = artifacts.get("probe.par")
        step = artifacts.get("probe.stp")
        if isinstance(par, dict) and isinstance(step, dict):
            minimum = geometry["range_min_mm"]
            maximum = geometry["range_max_mm"]
            if not (
                isinstance(minimum, (list, tuple))
                and isinstance(maximum, (list, tuple))
                and len(minimum) == 3
                and len(maximum) == 3
            ):
                minimum = (0.0, 0.0, 0.0)
                maximum = (0.0, 0.0, 0.0)
            try:
                calibration = calibration_store.record_probe(
                    se_version=se_version,
                    volume_mm3=float(geometry["volume_mm3"]),
                    range_min_mm=(float(minimum[0]), float(minimum[1]), float(minimum[2])),
                    range_max_mm=(float(maximum[0]), float(maximum[1]), float(maximum[2])),
                    par_sha256=str(par["sha256"]),
                    step_sha256=str(step["sha256"]),
                )
            except Exception as exc:
                report["calibration_error"] = bounded_message(exc)

    report["calibration_required"] = calibration is None
    report["calibration_id"] = calibration.calibration_id if calibration is not None else None
    report["native_build_capability"] = False
    atomic_write_json(output, report)
    exit_code = 0 if calibration is not None else 1
    return ProbeOutcome(
        exit_code=exit_code,
        report=report,
        output=output,
        calibration=calibration,
    )


__all__ = (
    "CUBE_MM",
    "PROBE_VERSION",
    "REQUIRED_CONSTANTS",
    "ProbeOutcome",
    "run_probe",
)
