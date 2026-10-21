"""The FreeCAD provider: a real ``FreeCADCmd`` subprocess (ADR-0007/0011).

Core code never imports ``FreeCAD`` or the adapter distribution. This provider
writes a small driver script, runs it in a separate ``FreeCADCmd`` process
with a bounded wall time and no shell, and reads back the adapter's own result
envelope. The driver uses the adapter's official ``worker.invoke_design``
entry point: it lowers the shared ``ConstructionScript`` with
``kala_agent_freecad.cad.compile.compile_construction``, builds the model in a
FreeCAD document, exports STEP and FCStd, and reports FreeCAD's measured
properties. The adapter owns the export target, so a placed part and a
place-only assembly export the geometry they actually describe.

Bounds are measured by FreeCAD itself: after the build, the driver reopens the
exported FCStd and reads the exported shape's ``BoundBox`` and ``Solids``. If
that reopen fails, bounds and the solid count are left unreported (never
guessed) and a warning says so.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sysconfig
import tempfile
import time
from pathlib import Path
from typing import TYPE_CHECKING, ClassVar, Final

from kala_agent.studio.design.importer import (
    resolve_step_import_max_bytes,
    validate_step_import_max_bytes,
)
from kala_agent.studio.design.providers.base import (
    Availability,
    CadProviderBuildError,
    CadProviderUnavailableError,
    MeasuredBounds,
    MeasuredProperties,
    ProviderBuild,
)

if TYPE_CHECKING:
    from kala_agent.contracts.construction import ConstructionScript
    from kala_agent.studio.design.intent import DesignIntent

FREECAD_EXECUTABLE_NAMES: Final[tuple[str, ...]] = ("FreeCADCmd", "freecadcmd")
DEFAULT_TIMEOUT_SECONDS: Final[float] = 1_800.0
MIN_TIMEOUT_SECONDS: Final[float] = 5.0
MAX_TIMEOUT_SECONDS: Final[float] = 3_600.0
DEFAULT_PROBE_CACHE_SECONDS: Final[float] = 30.0
VERSION_PROBE_TIMEOUT_SECONDS: Final[float] = 20.0
NATIVE_MAX_BYTES: Final[int] = 64 * 1024 * 1024
MAX_RESPONSE_BYTES: Final[int] = 4 * 1024 * 1024
DRIVER_FILENAME: Final[str] = "kala_freecad_provider_driver.py"
REQUEST_FILENAME: Final[str] = "kala-freecad-request.json"
RESPONSE_FILENAME: Final[str] = "kala-freecad-response.json"
SYS_PATH_ENV_VAR: Final[str] = "KALA_FREECAD_SYS_PATH"
REQUEST_ENV_VAR: Final[str] = "KALA_FREECAD_REQUEST"
WORKSPACE_ENV_VAR: Final[str] = "KALA_FREECAD_WORKSPACE"
OUTPUT_ENV_VAR: Final[str] = "KALA_FREECAD_OUTPUT"
MAX_STEP_ENV_VAR: Final[str] = "KALA_FREECAD_MAX_STEP_BYTES"
MIN_ARTIFACT_BYTES_ENV_VAR: Final[str] = "KALA_FREECAD_MAX_ARTIFACT_BYTES"
VERSION_PATTERN: Final[re.Pattern[str]] = re.compile(
    r"FreeCAD\s+([0-9]+(?:\.[0-9]+){1,3}(?:[-+][0-9A-Za-z.]+)?)"
)
UNAVAILABLE_REASON: Final[str] = (
    "FreeCADCmd was not found on PATH; install FreeCAD 1.1 or later to enable the FreeCAD provider"
)
# The FreeCAD driver needs PATH/HOME/locale, not the Studio's credentials. A
# key whose name looks credential-shaped is removed before the subprocess
# starts, so a CAD application never inherits the worker token or API keys.
_SENSITIVE_ENV_MARKERS: Final[tuple[str, ...]] = (
    "TOKEN",
    "API_KEY",
    "SECRET",
    "PASSWORD",
    "CREDENTIAL",
)


class FreeCadProvider:
    """Build a construction script in a real, out-of-process FreeCAD."""

    name: ClassVar[str] = "freecad"
    display_name: ClassVar[str] = "FreeCAD"
    measured_by: ClassVar[str] = "FreeCAD"
    mode: ClassVar[str] = "external"

    def __init__(
        self,
        *,
        executable: str | Path | None = None,
        timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
        max_step_bytes: int | None = None,
        probe_cache_seconds: float = DEFAULT_PROBE_CACHE_SECONDS,
        environ: dict[str, str] | None = None,
    ) -> None:
        if not MIN_TIMEOUT_SECONDS <= timeout_seconds <= MAX_TIMEOUT_SECONDS:
            raise ValueError(
                f"the FreeCAD build timeout must be between {MIN_TIMEOUT_SECONDS:g} "
                f"and {MAX_TIMEOUT_SECONDS:g} seconds"
            )
        if probe_cache_seconds < 0:
            raise ValueError("the probe cache duration must be non-negative")
        self._executable = str(executable) if executable is not None else None
        self._timeout_seconds = float(timeout_seconds)
        self._max_step_bytes = (
            resolve_step_import_max_bytes(environ)
            if max_step_bytes is None
            else validate_step_import_max_bytes(max_step_bytes)
        )
        self._probe_cache_seconds = float(probe_cache_seconds)
        self._environ = dict(os.environ if environ is None else environ)
        self._probe_cached: Availability | None = None
        self._probe_at = 0.0

    # ------------------------------------------------------------------ probe

    def probe(self) -> Availability:
        """Return whether a usable ``FreeCADCmd`` is present on this host."""
        now = time.monotonic()
        if self._probe_cached is not None and now - self._probe_at < self._probe_cache_seconds:
            return self._probe_cached
        availability = self._probe_uncached()
        self._probe_cached = availability
        self._probe_at = now
        return availability

    def _probe_uncached(self) -> Availability:
        executable = self._resolve_executable()
        if executable is None:
            return Availability(available=False, version=None, reason=UNAVAILABLE_REASON)
        try:
            completed = subprocess.run(
                [executable, "--version"],
                capture_output=True,
                text=True,
                timeout=VERSION_PROBE_TIMEOUT_SECONDS,
                env=self._probe_environ(),
                check=False,
            )
        except subprocess.TimeoutExpired:
            return Availability(
                available=False,
                version=None,
                reason=(
                    f"the FreeCADCmd version probe at {executable!r} did not finish within "
                    f"{VERSION_PROBE_TIMEOUT_SECONDS:g} seconds"
                ),
            )
        except OSError as error:
            return Availability(
                available=False,
                version=None,
                reason=f"the FreeCADCmd version probe failed: {type(error).__name__}",
            )
        output = "\n".join((completed.stdout or "", completed.stderr or ""))
        match = VERSION_PATTERN.search(output)
        if match is None:
            detail = " ".join(output.split())[:200]
            return Availability(
                available=False,
                version=None,
                reason=(
                    "FreeCADCmd did not report a FreeCAD version"
                    + (f": {detail}" if detail else "")
                ),
            )
        return Availability(available=True, version=match.group(1), reason=None)

    def _resolve_executable(self) -> str | None:
        if self._executable is not None:
            candidate = Path(self._executable)
            return str(candidate) if candidate.is_file() else None
        for name in FREECAD_EXECUTABLE_NAMES:
            found = shutil.which(name, path=self._environ.get("PATH"))
            if found is not None:
                return found
        return None

    # ------------------------------------------------------------------- build

    def build(self, script: ConstructionScript, intent: DesignIntent) -> ProviderBuild:
        """Build ``script`` in FreeCAD and return its STEP, FCStd, and numbers."""
        del intent
        availability = self.probe()
        if not availability.available:
            raise CadProviderUnavailableError(
                self.name,
                f"the {self.name} CAD provider is not available",
                reason=availability.reason or UNAVAILABLE_REASON,
            )

        executable = self._resolve_executable()
        if executable is None:  # pragma: no cover - the probe just checked this
            raise CadProviderUnavailableError(
                self.name,
                f"the {self.name} CAD provider is not available",
                reason=UNAVAILABLE_REASON,
            )

        with tempfile.TemporaryDirectory(prefix="kala-freecad-provider-") as directory:
            root = Path(directory)
            workspace = root / "work"
            workspace.mkdir(parents=True, exist_ok=True)
            request_path = root / REQUEST_FILENAME
            response_path = root / RESPONSE_FILENAME
            driver_path = root / DRIVER_FILENAME
            request_path.write_text(
                json.dumps(
                    {"script": script.model_dump(mode="json", exclude_none=True)},
                    sort_keys=True,
                    separators=(",", ":"),
                    ensure_ascii=True,
                ),
                encoding="utf-8",
            )
            driver_path.write_text(_DRIVER_SOURCE, encoding="utf-8")
            environment = self._build_environ(
                request_path=request_path,
                workspace=workspace,
                response_path=response_path,
            )
            stderr_path = root / "kala-freecad-stderr.log"
            try:
                with stderr_path.open("wb") as stderr_handle:
                    completed = subprocess.run(
                        [executable, str(driver_path)],
                        cwd=str(root),
                        env=environment,
                        stdout=subprocess.DEVNULL,
                        stderr=stderr_handle,
                        timeout=self._timeout_seconds,
                        check=False,
                    )
            except subprocess.TimeoutExpired:
                raise CadProviderBuildError(
                    f"FreeCAD did not finish within {self._timeout_seconds:g} seconds"
                ) from None
            except OSError as error:
                raise CadProviderBuildError(
                    f"the FreeCADCmd subprocess could not start: {type(error).__name__}"
                ) from None
            payload = _read_response(response_path)
            if payload is None:
                detail = _stderr_tail(stderr_path)
                raise CadProviderBuildError(
                    "FreeCAD did not report a result"
                    f" (exit {completed.returncode})" + (f": {detail}" if detail else "")
                )
            if payload.get("ok") is not True:
                message = str(payload.get("error", "FreeCAD refused the build"))
                raise CadProviderBuildError(_op_refusal_message(message, script))
            return self._build_from_payload(payload, workspace=workspace)

    def _build_from_payload(
        self,
        payload: dict[str, object],
        *,
        workspace: Path,
    ) -> ProviderBuild:
        entries = payload.get("artifacts")
        if not isinstance(entries, list):
            raise CadProviderBuildError("FreeCAD reported no artifacts")
        step_entry = _artifact_entry(entries, "step")
        native_entry = _artifact_entry(entries, "fcstd")
        if step_entry is None:
            raise CadProviderBuildError("FreeCAD did not export a STEP file")
        if native_entry is None:
            raise CadProviderBuildError("FreeCAD did not export an FCStd file")
        step_bytes = _read_artifact(
            step_entry,
            workspace=workspace,
            limit=self._max_step_bytes,
            label="STEP",
        )
        native_bytes = _read_artifact(
            native_entry,
            workspace=workspace,
            limit=NATIVE_MAX_BYTES,
            label="FCStd",
        )
        measurement = payload.get("measurement")
        measured_dict = measurement if isinstance(measurement, dict) else {}
        measured = MeasuredProperties(
            volume_mm3=_finite_or_none(measured_dict.get("volume_mm3")),
            area_mm2=_finite_or_none(measured_dict.get("area_mm2")),
            bounds=_bounds_or_none(payload.get("bounds")),
            mass_g=None,
            centroid_mm=None,
        )
        warnings = _string_tuple(payload.get("warnings"))
        raw_valid = payload.get("valid")
        valid = raw_valid if isinstance(raw_valid, bool) else None
        solid_count = payload.get("solid_count")
        if isinstance(solid_count, bool) or not isinstance(solid_count, int) or solid_count < 0:
            solid_count = None
        version = payload.get("freecad_version")
        return ProviderBuild(
            step_bytes=step_bytes,
            measured=measured,
            provider_version=version if isinstance(version, str) else None,
            warnings=warnings,
            native_bytes=native_bytes,
            native_kind="fcstd",
            valid=valid,
            solid_count=solid_count,
        )

    def _scrubbed_environment(self) -> dict[str, str]:
        """Return the provider environment without credential-shaped keys."""
        return {
            key: value
            for key, value in self._environ.items()
            if not any(marker in key.upper() for marker in _SENSITIVE_ENV_MARKERS)
        }

    def _probe_environ(self) -> dict[str, str]:
        environment = self._scrubbed_environment()
        environment.pop("PYTHONPATH", None)
        return environment

    def _build_environ(
        self,
        *,
        request_path: Path,
        workspace: Path,
        response_path: Path,
    ) -> dict[str, str]:
        environment = self._scrubbed_environment()
        environment.pop("PYTHONPATH", None)
        environment[SYS_PATH_ENV_VAR] = os.pathsep.join(self._driver_sys_path())
        environment[REQUEST_ENV_VAR] = str(request_path)
        environment[WORKSPACE_ENV_VAR] = str(workspace)
        environment[OUTPUT_ENV_VAR] = str(response_path)
        environment[MAX_STEP_ENV_VAR] = str(self._max_step_bytes)
        environment[MIN_ARTIFACT_BYTES_ENV_VAR] = str(NATIVE_MAX_BYTES)
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        return environment

    def _driver_sys_path(self) -> tuple[str, ...]:
        source_root = Path(__file__).resolve().parents[4]
        repository_root = source_root.parent
        purelib = sysconfig.get_paths().get("purelib")
        candidates = (
            repository_root / "adapters" / "freecad" / "src",
            source_root,
            Path(purelib) if purelib else None,
        )
        return tuple(str(path) for path in candidates if path is not None and path.is_dir())


def _op_refusal_message(message: str, script: ConstructionScript) -> str:
    """Name the failing assembly op when the adapter message does not."""
    lowered = message.lower()
    if "one valid solid" in lowered or "did not produce a solid" in lowered:
        for step in script.assembly:
            if step.op in {"fuse", "compound"}:
                return f"{message} (op: {step.op})"
    return message


def _artifact_entry(entries: list[object], export_format: str) -> dict[str, object] | None:
    for entry in entries:
        if isinstance(entry, dict) and entry.get("format") == export_format:
            return entry
    return None


def _read_artifact(
    entry: dict[str, object],
    *,
    workspace: Path,
    limit: int,
    label: str,
) -> bytes:
    raw_path = entry.get("path")
    raw_size = entry.get("size_bytes")
    raw_digest = entry.get("digest")
    if not isinstance(raw_path, str) or not isinstance(raw_digest, str):
        raise CadProviderBuildError(f"the FreeCAD {label} artifact descriptor is incomplete")
    if isinstance(raw_size, bool) or not isinstance(raw_size, int) or raw_size < 0:
        raise CadProviderBuildError(f"the FreeCAD {label} artifact size is invalid")
    if raw_size > limit:
        raise CadProviderBuildError(
            f"the FreeCAD {label} export is {raw_size} bytes, above the configured "
            f"limit of {limit} bytes"
        )
    try:
        path = Path(raw_path).resolve()
        workspace_root = workspace.resolve()
        path.relative_to(workspace_root)
    except (OSError, ValueError):
        raise CadProviderBuildError(
            f"the FreeCAD {label} artifact is outside the build workspace"
        ) from None
    try:
        data = path.read_bytes()
    except OSError as error:
        raise CadProviderBuildError(
            f"the FreeCAD {label} artifact could not be read: {type(error).__name__}"
        ) from None
    if len(data) != raw_size:
        raise CadProviderBuildError(f"the FreeCAD {label} artifact changed size before it was read")
    if hashlib.sha256(data).hexdigest() != raw_digest:
        raise CadProviderBuildError(
            f"the FreeCAD {label} artifact does not match the digest FreeCAD reported"
        )
    return data


def _stderr_tail(path: Path, *, limit: int = 400) -> str:
    """Return a bounded, whitespace-collapsed tail of the FreeCAD stderr log."""
    try:
        size = path.stat().st_size
        with path.open("rb") as handle:
            if size > limit * 4:
                handle.seek(size - limit * 4)
            raw = handle.read(limit * 4)
    except OSError:
        return ""
    return " ".join(raw.decode("utf-8", errors="replace").split())[-limit:]


def _read_response(path: Path) -> dict[str, object] | None:
    try:
        size = path.stat().st_size
    except OSError:
        return None
    if size <= 0 or size > MAX_RESPONSE_BYTES:
        return None
    try:
        parsed = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None
    return parsed if isinstance(parsed, dict) else None


def _finite_or_none(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    return number if math.isfinite(number) else None


def _bounds_or_none(value: object) -> MeasuredBounds | None:
    if not isinstance(value, dict):
        return None
    minimum = value.get("min")
    maximum = value.get("max")
    if not isinstance(minimum, (list, tuple)) or not isinstance(maximum, (list, tuple)):
        return None
    if len(minimum) != 3 or len(maximum) != 3:
        return None
    try:
        coordinates = tuple(float(item) for item in (*minimum, *maximum))
    except (TypeError, ValueError):
        return None
    if not all(math.isfinite(item) for item in coordinates):
        return None
    try:
        return MeasuredBounds(
            min_mm=(coordinates[0], coordinates[1], coordinates[2]),
            max_mm=(coordinates[3], coordinates[4], coordinates[5]),
        )
    except ValueError:
        return None


def _string_tuple(value: object) -> tuple[str, ...]:
    if not isinstance(value, (list, tuple)):
        return ()
    return tuple(str(item)[:256] for item in value if isinstance(item, str) and item)


_DRIVER_SOURCE: Final[str] = r'''
"""Build one Kala construction script inside FreeCADCmd and report the facts.

This file is generated by the Kala Studio FreeCAD provider and runs only in a
short-lived FreeCADCmd process. It imports the adapter distribution; core Kala
code never imports it.
"""

import json
import os
import sys
from pathlib import Path


def _write(payload):
    try:
        with open(os.environ["KALA_FREECAD_OUTPUT"], "w", encoding="utf-8") as handle:
            json.dump(payload, handle, sort_keys=True, default=str)
    except Exception:
        pass


def _extend_path():
    entries = [
        item
        for item in os.environ.get("KALA_FREECAD_SYS_PATH", "").split(os.pathsep)
        if item
    ]
    for entry in reversed(entries):
        if entry not in sys.path:
            sys.path.insert(0, entry)


try:
    _extend_path()
    from kala_agent.contracts import ToolRunStatus
    from kala_agent.contracts.construction import ConstructionScript
    from kala_agent.tools.protocol import (
        PROTOCOL_VERSION,
        ToolEnvelopeKind,
        ToolRequestEnvelope,
    )
    from kala_agent_freecad.cad.compile import compile_construction
    from kala_agent_freecad.contracts import (
        DESIGN_TIMEOUT_SECONDS,
        DESIGN_TOOL_ID,
        DESIGN_TOOL_VERSION,
        TOOL_CAPABILITIES,
    )
    from kala_agent_freecad.worker import invoke_design
    import FreeCAD

    workspace = Path(os.environ["KALA_FREECAD_WORKSPACE"]).resolve()
    max_artifact_bytes = int(os.environ["KALA_FREECAD_MAX_ARTIFACT_BYTES"])
    request_payload = json.loads(
        Path(os.environ["KALA_FREECAD_REQUEST"]).read_text(encoding="utf-8")
    )
    script = ConstructionScript.model_validate(request_payload["script"])
    calls = compile_construction(script)
    # The official worker entry point, wrapped in the shipped input and output
    # schemas. The driver only supplies the script's call list; it does not
    # change geometry or choose an export target.
    envelope = ToolRequestEnvelope(
        protocol_version=PROTOCOL_VERSION,
        kind=ToolEnvelopeKind.REQUEST,
        call_id="kala-provider",
        tool_id=DESIGN_TOOL_ID,
        tool_version=DESIGN_TOOL_VERSION,
        arguments={
            "artifact_stem": "kala-provider",
            "calls_json": json.dumps(calls, separators=(",", ":"), sort_keys=True),
            "document_name": "kala-provider",
            "export_formats": ["step", "fcstd"],
        },
        workspace=str(workspace),
        deadline_seconds=DESIGN_TIMEOUT_SECONDS,
        max_output_bytes=1048576,
        max_artifact_bytes=max_artifact_bytes,
        granted_capabilities=TOOL_CAPABILITIES,
    )
    outcome = invoke_design(envelope)
    if outcome.status is not ToolRunStatus.SUCCEEDED or outcome.result is None:
        detail = ": ".join(
            str(part) for part in (outcome.error_code, outcome.error_message) if part
        )
        _write({"ok": False, "error": (detail or "the adapter refused the build")[:1024]})
    else:
        result = dict(outcome.result)
        artifacts = []
        fcstd_path = None
        for entry in result.get("artifacts", []):
            if not isinstance(entry, dict):
                continue
            relative = entry.get("relative_path")
            if not isinstance(relative, str):
                continue
            path = workspace / relative
            enriched = dict(entry)
            enriched["path"] = str(path)
            artifacts.append(enriched)
            if entry.get("format") == "fcstd":
                fcstd_path = path
        result["artifacts"] = artifacts
        response = {
            "ok": True,
            "result": result,
            "measurement": result.get("measurement"),
            "freecad_version": result.get("freecad_version"),
            "warnings": list(result.get("warnings") or []),
            "ineffective_cuts": result.get("ineffective_cuts"),
            "valid": result.get("valid"),
            "artifacts": artifacts,
        }
        if fcstd_path is not None and fcstd_path.is_file():
            try:
                document = FreeCAD.openDocument(str(fcstd_path))
                try:
                    shape = None
                    for item in reversed(list(document.Objects)):
                        candidate = getattr(item, "Shape", None)
                        if candidate is None:
                            continue
                        shape_type = getattr(candidate, "ShapeType", "")
                        solids = getattr(candidate, "Solids", None)
                        try:
                            count = len(solids) if solids is not None else 0
                        except TypeError:
                            count = 0
                        if shape_type == "Solid" or (
                            shape_type == "Compound" and count >= 1
                        ):
                            shape = candidate
                            break
                    if shape is None:
                        response["warnings"].append(
                            "FreeCAD could not find a solid in the exported FCStd to report bounds"
                        )
                    else:
                        box = shape.BoundBox
                        response["bounds"] = {
                            "min": [box.XMin, box.YMin, box.ZMin],
                            "max": [box.XMax, box.YMax, box.ZMax],
                        }
                        solids = getattr(shape, "Solids", None)
                        try:
                            response["solid_count"] = (
                                len(solids) if solids is not None else 1
                            )
                        except TypeError:
                            response["solid_count"] = 1
                finally:
                    FreeCAD.closeDocument(document.Name)
            except Exception as error:
                response["warnings"].append(
                    "FreeCAD could not reopen the exported FCStd: "
                    + type(error).__name__
                )
        _write(response)
except BaseException as error:
    _write({"ok": False, "error": (type(error).__name__ + ": " + str(error))[:1024]})
'''


__all__ = ("FREECAD_EXECUTABLE_NAMES", "UNAVAILABLE_REASON", "FreeCadProvider")
