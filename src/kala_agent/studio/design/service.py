"""Orchestration for Studio design turns (ADR-0009 + ADR-0011).

The service owns the whole honest pipeline: resolve a
:class:`~kala_agent.studio.design.intent.DesignIntent` into a
``ConstructionScript`` and a declared envelope, hash the canonical script plus
the selected **CAD provider**, ask that provider to build and measure the
model, store its STEP and native artifacts, and import the provider's STEP
with the view-only importer for the display mesh. It never touches the core
``KalaApplication`` database.

Three invariants matter more than throughput:

* **A provider is required.** There is no default and no automatic
  OpenCascade fallback. A missing or unknown provider is refused before any
  geometry work, and the refusal names the allowed values.
* **The view is the provider's export.** For every provider, including the
  opt-in OpenCascade builder, the display mesh comes from importing the exact
  STEP bytes the provider returned. OpenCascade is never the measuring
  authority for an external model: the numbers in ``measured`` are the
  provider's own, and a missing value stays missing.
* **No partial artifact on refusal.** A provider refusal writes
  ``provider.build.failed`` + ``design.rejected`` and stores no artifact and
  no ``provider.build.completed`` event. A successful build is content
  addressed by its digests.

Compiled views live in a bounded LRU (32). On a cold miss the service reloads
the stored STEP artifact and the provider numbers recorded in the journal, so
a drawing, section, or export does not rerun an external CAD application.

One geometry lock serialises every OCP compile/measure/mesh/drawing/section/
export operation and every design turn, because OCP is not safe for concurrent
mutation in the FastAPI thread pool.
"""

from __future__ import annotations

import hashlib
import json
import math
import threading
from collections import OrderedDict
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, replace
from pathlib import Path
from types import TracebackType
from typing import TYPE_CHECKING, Final

from kala_agent.contracts.construction import (
    ConstructionScript,
)
from kala_agent.studio.design.artifacts import (
    ARTIFACT_KINDS,
    MAX_ARTIFACT_DOWNLOAD_BYTES,
    content_type as _artifact_content_type,
    filename as _artifact_filename,
)
from kala_agent.studio.design.importer import (
    StepImportError,
    StepImportOutcome,
    import_step_bytes,
    resolve_step_import_max_bytes,
    validate_step_import_max_bytes,
)
from kala_agent.studio.design.intent import (
    DesignIntent,
    HousingCall,
    LbracketCall,
    SteppedShaftCall,
)
from kala_agent.studio.design.providers import build_providers, probe_providers
from kala_agent.studio.design.providers.base import (
    ALLOWED_PROVIDER_IDS,
    CUT_NO_MATERIAL,
    CadProvider,
    CadProviderBuildError,
    CadProviderUnavailableError,
    MeasuredBounds,
    MeasuredProperties,
    ProviderBuild,
    ProviderStatus,
    bound_warnings as _bounded_warning_lines,
    ineffective_cut_count,
)
from kala_agent.studio.session_journal import (
    DEFAULT_MAX_ARTIFACT_BYTES,
    JournalEventType,
    JournalVerification,
    SessionEvent,
    StudioSessionJournal,
)

if TYPE_CHECKING:
    from OCP.TopoDS import TopoDS_Shape  # type: ignore[import-untyped]

    from kala_agent.studio.design.drawing import Drawing, Section
    from kala_agent.studio.design.measure import Bounds, Check
    from kala_agent.studio.design.mesh import MeshData

DESIGN_ID_PATTERN_LENGTH: Final[int] = 64
DEFAULT_CACHE_SIZE: Final[int] = 32
MAX_SECTION_OFFSET_MM: Final[float] = 10_000.0
PROVIDER_BUILD_REQUESTED: Final[str] = "provider.build.requested"
PROVIDER_BUILD_COMPLETED: Final[str] = "provider.build.completed"
PROVIDER_BUILD_FAILED: Final[str] = "provider.build.failed"
VIEW_IMPORTED: Final[str] = "view.imported"
_REFUSED_EVENT: Final[str] = "design.rejected"
_GEOMETRY_LOCK: Final[threading.RLock] = threading.RLock()
CUTS_EFFECTIVE_TOLERANCE: Final[float] = 0.0

__all__ = (
    "DEFAULT_CACHE_SIZE",
    "DESIGN_ID_PATTERN_LENGTH",
    "MAX_SECTION_OFFSET_MM",
    "PROVIDER_BUILD_COMPLETED",
    "PROVIDER_BUILD_FAILED",
    "PROVIDER_BUILD_REQUESTED",
    "VIEW_IMPORTED",
    "ArtifactDownload",
    "DesignError",
    "DesignNotFoundError",
    "DesignOutcome",
    "DesignRejectedError",
    "DesignService",
    "DesignUnavailableError",
    "ExportOutcome",
    "build_provider_checks",
    "canonical_script_json",
    "check_payload",
    "design_available",
    "design_id_for",
    "expected_pocket_count",
    "expected_solid_count",
)


class DesignError(ValueError):
    """Base class for an honest, user-readable design failure."""


class DesignRejectedError(DesignError):
    """The intent or the selected provider could not build the design."""

    def __init__(self, reason: str, *, design_id: str | None = None) -> None:
        super().__init__(reason)
        self.reason = reason
        self.design_id = design_id


class DesignNotFoundError(DesignError):
    """No stored script artifact exists for the requested ``design_id``."""


class DesignUnavailableError(DesignError):
    """The optional OCP geometry kernel is not importable in this process."""


class ArtifactTooLargeError(DesignError):
    """A stored artifact exceeds the download ceiling."""


class ArtifactDigestMismatchError(DesignError):
    """A stored artifact does not match its journalled digest."""


@dataclass(frozen=True, slots=True)
class DesignOutcome:
    """Every provider-reported fact about one design, plus the display view."""

    design_id: str
    label: str
    script: dict[str, object]
    provider: str
    provider_version: str | None
    measured_by: str
    shape: TopoDS_Shape
    measurement: MeasuredProperties
    valid: bool | None
    solid_count: int | None
    checks: tuple[Check, ...]
    verified: bool
    view: MeshData
    view_bounds: Bounds
    view_solids: int
    view_warnings: tuple[str, ...]
    script_sha256: str
    step_sha256: str | None
    native_sha256: str | None
    native_kind: str | None
    journal_head: str | None
    tolerances: tuple[str, ...]
    material: str | None
    declared_envelope: Bounds | None
    cached: bool
    warnings: tuple[str, ...]
    component_count: int
    fused: bool


@dataclass(frozen=True, slots=True)
class ExportOutcome:
    """One exported design file and the digest it was stored under."""

    design_id: str
    format: str
    filename: str
    sha256: str
    data: bytes


@dataclass(frozen=True, slots=True)
class ArtifactDownload:
    """One stored artifact served by the download route."""

    design_id: str
    kind: str
    sha256: str
    content_type: str
    filename: str
    data: bytes


@dataclass(frozen=True, slots=True)
class _ArtifactRecord:
    """The label and the recorded digest of one downloadable artifact."""

    label: str
    digest: str | None


@dataclass(frozen=True, slots=True)
class _ResolvedIntent:
    script: ConstructionScript
    envelope: Bounds | None
    tolerances: tuple[str, ...]
    material: str | None
    label: str
    template_name: str | None


@dataclass(frozen=True, slots=True)
class _StoredDesign:
    """The journal records needed to reload one completed design."""

    intent: Mapping[str, object]
    build: Mapping[str, object] | None


def design_available() -> bool:
    """Return whether the optional OCP geometry kernel is importable."""
    try:
        __import__("OCP")
    except Exception:
        return False
    return True


def canonical_script_json(script: ConstructionScript) -> bytes:
    """Return the canonical, deterministic JSON bytes of a construction script."""
    payload = script.model_dump(mode="json", exclude_none=True)
    return json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        allow_nan=False,
    ).encode("utf-8")


def design_id_for(script: ConstructionScript, provider: str) -> str:
    """Return ``sha256(canonical script JSON || 0x00 || provider)``.

    The provider is part of the identity: the same script built by FreeCAD and
    by the opt-in OpenCascade builder are two different designs with two
    different measured numbers. The exact composition is documented for the
    Rust mirror in the cell report.
    """
    return _design_id_from_script_bytes(canonical_script_json(script), provider)


def _design_id_from_script_bytes(script_bytes: bytes, provider: str) -> str:
    if provider not in ALLOWED_PROVIDER_IDS:
        raise ValueError(f"unknown CAD provider {provider!r}")
    return hashlib.sha256(script_bytes + b"\x00" + provider.encode("ascii")).hexdigest()


def expected_solid_count(script: ConstructionScript) -> int:
    """Return the number of solids the script declares its result to have.

    The count comes from the script's own assembly semantics
    (``docs/construction-semantics.md``, "Assembly occurrences"):

    * a script with no assembly step builds one part, so one solid;
    * a place-only assembly is a set of occurrences, one solid per placed
      component;
    * a ``fuse`` step declares its members as one joined solid;
    * a ``compound`` step keeps one solid per compounded component.

    A component named by a join is counted once even when the script also
    places it; a placed component that no join names is one occurrence. The
    derivation never guesses a number the script does not describe: a script
    whose assembly steps place nothing still has the single tip feature, so
    the answer is one.
    """
    consumed: set[str] = set()
    count = 0
    for step in script.assembly:
        if step.op == "fuse":
            members = [name for name in step.components if name not in consumed]
            if members:
                count += 1
                consumed.update(members)
        elif step.op == "compound":
            members = [name for name in step.components if name not in consumed]
            if members:
                count += len(members)
                consumed.update(members)
    for step in script.assembly:
        if step.op != "place" or step.component is None:
            continue
        if step.component in consumed:
            continue
        count += 1
        consumed.add(step.component)
    return max(count, 1)


def expected_pocket_count(script: ConstructionScript) -> int:
    """Return the number of pocket steps the script declares."""
    return sum(
        1 for component in script.components for step in component.steps if step.op == "pocket"
    )


def build_provider_checks(
    *,
    measured: MeasuredProperties,
    valid: bool | None,
    solid_count: int | None,
    expected_solids: int = 1,
    expected_envelope: Bounds | None,
    expected_pockets: int = 0,
    warnings: Sequence[str] = (),
) -> tuple[Check, ...]:
    """Build the honesty checks from the provider's own facts.

    ``brep_valid`` is always present. A single-solid result keeps the
    ``single_solid`` row (expected 1), exactly as before. An assembly gets a
    ``solid_count`` row instead, whose expected value is the occurrence count
    the construction script declares. A value the provider did not report is
    ``measured=None`` with status ``not_reported``; it is never replaced by an
    OpenCascade measurement. The three envelope checks are present only when
    the intent declared an envelope. A missing provider bound makes the
    envelope row ``not_reported`` too.

    A build whose warnings report a cut that ``removed no material`` appends
    one failed ``cuts_effective`` row: expected is the number of pocket steps
    in the script, measured is the number that did remove material (expected
    minus the ineffective-cut warnings). The row is appended only for that
    failure, so every passing design keeps its exact check list.
    """
    from kala_agent.studio.design.measure import (
        BREP_VALID_TOLERANCE,
        ENVELOPE_TOLERANCE_MM,
        SINGLE_SOLID_TOLERANCE,
        SOLID_COUNT_TOLERANCE,
        Check,
    )

    if expected_solids < 1:
        raise ValueError("an expected solid count must be at least one")
    measured_solids = None if solid_count is None else float(solid_count)
    checks: list[Check] = [
        Check(
            id="brep_valid",
            label="B-Rep valid",
            expected=True,
            measured=valid,
            tolerance=BREP_VALID_TOLERANCE,
            unit="bool",
            passed=valid is True,
        )
    ]
    if expected_solids == 1:
        checks.append(
            Check(
                id="single_solid",
                label="Single solid",
                expected=1.0,
                measured=measured_solids,
                tolerance=SINGLE_SOLID_TOLERANCE,
                unit="solid",
                passed=solid_count == 1,
            )
        )
    else:
        checks.append(
            Check(
                id="solid_count",
                label="Solid count",
                expected=float(expected_solids),
                measured=measured_solids,
                tolerance=SOLID_COUNT_TOLERANCE,
                unit="solid",
                passed=solid_count == expected_solids,
            )
        )

    if expected_envelope is not None:
        bounds = measured.bounds
        for axis, name in enumerate(("x", "y", "z")):
            expected_pair = (expected_envelope.min_mm[axis], expected_envelope.max_mm[axis])
            if bounds is None:
                checks.append(
                    Check(
                        id=f"envelope_{name}",
                        label=f"Envelope {name.upper()}",
                        expected=expected_pair,
                        measured=None,
                        tolerance=ENVELOPE_TOLERANCE_MM,
                        unit="mm",
                        passed=False,
                    )
                )
                continue
            measured_pair = (bounds.min_mm[axis], bounds.max_mm[axis])
            passed = (
                abs(expected_pair[0] - measured_pair[0]) <= ENVELOPE_TOLERANCE_MM
                and abs(expected_pair[1] - measured_pair[1]) <= ENVELOPE_TOLERANCE_MM
            )
            checks.append(
                Check(
                    id=f"envelope_{name}",
                    label=f"Envelope {name.upper()}",
                    expected=expected_pair,
                    measured=measured_pair,
                    tolerance=ENVELOPE_TOLERANCE_MM,
                    unit="mm",
                    passed=passed,
                )
            )
    # A provider that reports a cut which removed nothing is never verified,
    # even when its B-Rep and envelope are valid. The row is added only for
    # that failure, so every passing design keeps its exact check list.
    ineffective = ineffective_cut_count(warnings)
    if ineffective:
        checks.append(
            Check(
                id="cuts_effective",
                label="Cuts removed material",
                expected=float(expected_pockets),
                measured=float(max(expected_pockets - ineffective, 0)),
                tolerance=CUTS_EFFECTIVE_TOLERANCE,
                unit="cut",
                passed=False,
            )
        )
    return tuple(checks)


class DesignService:
    """The Studio design pipeline over one Studio data directory."""

    def __init__(
        self,
        data_dir: Path,
        *,
        journal: StudioSessionJournal | None = None,
        cache_size: int = DEFAULT_CACHE_SIZE,
        providers: Mapping[str, CadProvider] | None = None,
        max_step_bytes: int | None = None,
        environ: Mapping[str, str] | None = None,
    ) -> None:
        if cache_size < 1:
            raise ValueError("the design cache size must be positive")
        self._data_dir = Path(data_dir).expanduser().resolve(strict=False)
        self._data_dir.mkdir(parents=True, exist_ok=True)
        self._max_step_bytes = (
            resolve_step_import_max_bytes(environ)
            if max_step_bytes is None
            else validate_step_import_max_bytes(max_step_bytes)
        )
        artifact_limit = max(DEFAULT_MAX_ARTIFACT_BYTES, self._max_step_bytes)
        self._journal = (
            journal
            if journal is not None
            else StudioSessionJournal(self._data_dir, max_artifact_bytes=artifact_limit)
        )
        self._owns_journal = journal is None
        self._providers: dict[str, CadProvider] = (
            dict(providers)
            if providers is not None
            else build_providers(
                max_step_bytes=self._max_step_bytes,
                environ=environ,
            )
        )
        self._cache: OrderedDict[str, DesignOutcome] = OrderedDict()
        self._cache_size = cache_size
        self._cache_lock = threading.RLock()

    @property
    def journal(self) -> StudioSessionJournal:
        """Return the Studio session journal this service writes."""
        return self._journal

    @property
    def data_dir(self) -> Path:
        """Return the Studio data directory."""
        return self._data_dir

    @property
    def max_step_bytes(self) -> int:
        """Return the STEP import/export ceiling this service enforces."""
        return self._max_step_bytes

    @property
    def provider_names(self) -> tuple[str, ...]:
        """Return the closed set of provider ids this service accepts."""
        return tuple(self._providers)

    def probe_providers(self) -> tuple[ProviderStatus, ...]:
        """Return each provider's live probe result, never an assumption."""
        return probe_providers(self._providers)

    def available_provider_ids(self) -> tuple[str, ...]:
        """Return the ids of the providers that can build right now."""
        return tuple(status.id for status in self.probe_providers() if status.available)

    def close(self) -> None:
        """Close the owned journal; a caller-supplied journal is left open."""
        if self._owns_journal:
            self._journal.close()

    def __enter__(self) -> DesignService:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    # ------------------------------------------------------------- design turns

    def design(self, intent: DesignIntent, *, provider: str) -> DesignOutcome:
        """Build, measure, import, and journal one design with one provider.

        Cancellation boundary: this reference route is synchronous inside the
        FastAPI thread pool, so a client disconnect cannot drop it (the event
        loop cannot cancel a thread). The shipped native service threads a
        cancellation token from the request into this same pipeline, checks it
        between phases and kills an out-of-process FreeCAD build's process
        group; it records one ``design.cancelled`` event and no later
        ``design.*``/``provider.build.*``/``artifact.stored`` event. The
        Python reference keeps no equivalent path, and says so instead of
        pretending a cancelled turn stopped.
        """
        if not design_available():
            raise DesignUnavailableError("the design pipeline requires the OCP bindings")
        self._validate_provider(provider)
        with _GEOMETRY_LOCK:
            return self._design_locked(intent, provider)

    def _validate_provider(self, provider: object) -> None:
        allowed = ", ".join(ALLOWED_PROVIDER_IDS)
        if not isinstance(provider, str) or not provider.strip():
            raise DesignRejectedError(f"a CAD provider is required; choose one of: {allowed}")
        if provider not in ALLOWED_PROVIDER_IDS:
            raise DesignRejectedError(
                f"unknown CAD provider {provider!r}; choose one of: {allowed}"
            )
        if provider not in self._providers:
            raise DesignRejectedError(
                f"the CAD provider {provider!r} is not configured; choose one of: {allowed}"
            )

    def _design_locked(self, intent: DesignIntent, provider_name: str) -> DesignOutcome:
        resolved = self._resolve_intent(intent)
        script_bytes = canonical_script_json(resolved.script)
        script_sha256 = hashlib.sha256(script_bytes).hexdigest()
        design_id = _design_id_from_script_bytes(script_bytes, provider_name)

        cached_in_memory = self._cache_get(design_id) is not None
        stored: _StoredDesign | None = None
        if not cached_in_memory:
            stored = self._find_journal_design(design_id)
        cached_on_disk = stored is not None and self._stored_design_is_available(stored)
        cached = cached_in_memory or cached_on_disk

        intent_event = self._journal.append(
            JournalEventType.DESIGN_INTENT,
            self._intent_payload(
                design_id,
                resolved,
                provider=provider_name,
                script_sha256=script_sha256,
                cache_hit=cached,
            ),
            aggregate_id=design_id,
        )
        if cached:
            outcome = self._load_locked(design_id, stored=stored)
            return replace(outcome, cached=True, journal_head=intent_event.event_digest)

        provider = self._providers[provider_name]
        availability = provider.probe()
        if not availability.available:
            reason = availability.reason or "the provider reported no reason"
            self._provider_failed(design_id, provider_name, "provider_unavailable", reason)
            self._reject(design_id, f"provider {provider_name} is not available: {reason}")
            raise CadProviderUnavailableError(
                provider_name,
                f"the {provider_name} CAD provider is not available",
                reason=reason,
                available=self.available_provider_ids(),
            )

        self._journal.append(
            PROVIDER_BUILD_REQUESTED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "provider_version": availability.version,
                "script_sha256": script_sha256,
            },
            aggregate_id=design_id,
        )
        try:
            build = provider.build(resolved.script, intent)
        except CadProviderUnavailableError as error:
            self._provider_failed(design_id, provider_name, "provider_unavailable", error.reason)
            self._reject(design_id, str(error))
            raise
        except CadProviderBuildError as error:
            self._provider_failed(design_id, provider_name, "build_refused", str(error))
            self._reject(design_id, str(error))
            raise DesignRejectedError(str(error), design_id=design_id) from None
        except Exception as error:
            reason = f"the {provider_name} provider failed: {type(error).__name__}"
            self._provider_failed(design_id, provider_name, "provider_error", reason)
            self._reject(design_id, reason)
            raise DesignRejectedError(reason, design_id=design_id) from None

        self._validate_build(design_id, provider_name, build)
        imported = self._import_view(design_id, provider_name, build)

        step_sha256 = hashlib.sha256(build.step_bytes).hexdigest()
        native_kind = build.native_kind
        native_sha256 = (
            None if build.native_bytes is None else hashlib.sha256(build.native_bytes).hexdigest()
        )
        checks = build_provider_checks(
            measured=build.measured,
            valid=build.valid,
            solid_count=build.solid_count,
            expected_solids=expected_solid_count(resolved.script),
            expected_envelope=resolved.envelope,
            expected_pockets=expected_pocket_count(resolved.script),
            warnings=build.warnings,
        )
        verified = bool(checks) and all(check.passed for check in checks)

        measured_by = provider.measured_by
        self._journal.append(
            PROVIDER_BUILD_COMPLETED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "provider_version": build.provider_version,
                "measured_by": measured_by,
                "script_sha256": script_sha256,
                "step_sha256": step_sha256,
                "native_sha256": native_sha256,
                "native_kind": native_kind,
                "measured": _measured_payload(build.measured),
                "valid": build.valid,
                "solid_count": build.solid_count,
                "warnings": list(build.warnings),
            },
            aggregate_id=design_id,
        )

        stored_script = self._journal.put_artifact(script_bytes)
        if stored_script != script_sha256:
            raise DesignRejectedError(
                "the stored script artifact digest did not match the script digest",
                design_id=design_id,
            )
        self._journal.append(
            JournalEventType.ARTIFACT_STORED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "kind": "script",
                "sha256": stored_script,
                "size_bytes": len(script_bytes),
            },
            aggregate_id=design_id,
        )

        stored_step = self._journal.put_artifact(build.step_bytes)
        if stored_step != step_sha256:
            raise DesignRejectedError(
                "the stored STEP artifact digest did not match the provider export",
                design_id=design_id,
            )
        self._journal.append(
            JournalEventType.ARTIFACT_STORED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "kind": "step",
                "sha256": stored_step,
                "size_bytes": len(build.step_bytes),
                "source": f"provider:{provider_name}",
            },
            aggregate_id=design_id,
        )

        if build.native_bytes is not None:
            stored_native = self._journal.put_artifact(build.native_bytes)
            if stored_native != native_sha256:
                raise DesignRejectedError(
                    "the stored native artifact digest did not match the provider export",
                    design_id=design_id,
                )
            self._journal.append(
                JournalEventType.ARTIFACT_STORED,
                {
                    "design_id": design_id,
                    "provider": provider_name,
                    "kind": native_kind or "native",
                    "sha256": stored_native,
                    "size_bytes": len(build.native_bytes),
                    "source": f"provider:{provider_name}",
                },
                aggregate_id=design_id,
            )

        self._journal.append(
            VIEW_IMPORTED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "step_sha256": stored_step,
                "viewer": "opencascade",
                "role": "view-only",
                "solids": imported.solids,
                "vertices": imported.mesh.vertex_count,
                "triangles": imported.mesh.triangle_count,
                "bounds": {
                    "min": list(imported.bounds.min_mm),
                    "max": list(imported.bounds.max_mm),
                },
                "warnings": list(imported.warnings),
            },
            aggregate_id=design_id,
        )
        if provider_name == "opencascade":
            self._journal.append(
                JournalEventType.DESIGN_COMPILED,
                {
                    "design_id": design_id,
                    "provider": provider_name,
                    "components": len(resolved.script.components),
                    "fused": _script_fused(resolved.script),
                    "warnings": list(build.warnings),
                    "envelope": _envelope_payload(resolved.envelope),
                    "tolerances": list(resolved.tolerances),
                    "material": resolved.material,
                    "label": resolved.label,
                },
                aggregate_id=design_id,
            )
        self._journal.append(
            JournalEventType.DESIGN_MEASURED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "measured_by": measured_by,
                **_measured_payload(build.measured),
                "valid": build.valid,
                "solid_count": build.solid_count,
            },
            aggregate_id=design_id,
        )
        checked_event = self._journal.append(
            JournalEventType.DESIGN_CHECKED,
            {
                "design_id": design_id,
                "provider": provider_name,
                "verified": verified,
                "checks": [check_payload(check) for check in checks],
            },
            aggregate_id=design_id,
        )

        outcome = self._make_outcome(
            design_id=design_id,
            resolved=resolved,
            provider=provider_name,
            provider_version=build.provider_version,
            measured_by=measured_by,
            shape=imported.shape,
            measurement=build.measured,
            valid=build.valid,
            solid_count=build.solid_count,
            checks=checks,
            verified=verified,
            view=imported.mesh,
            view_bounds=imported.bounds,
            view_solids=imported.solids,
            view_warnings=imported.warnings,
            step_sha256=stored_step,
            native_sha256=native_sha256,
            native_kind=native_kind,
            journal_head=checked_event.event_digest,
            warnings=build.warnings,
            cached=False,
        )
        self._cache_put(outcome)
        return outcome

    def _validate_build(
        self,
        design_id: str,
        provider_name: str,
        build: ProviderBuild,
    ) -> None:
        if len(build.step_bytes) > self._max_step_bytes:
            reason = (
                f"the {provider_name} STEP export is larger than the configured "
                f"{self._max_step_bytes} byte STEP import limit"
            )
            self._provider_failed(design_id, provider_name, "step_too_large", reason)
            self._reject(design_id, reason)
            raise DesignRejectedError(reason, design_id=design_id)
        if not build.step_bytes.startswith(b"ISO-10303-21"):
            reason = f"the {provider_name} export is not an ISO-10303-21 STEP file"
            self._provider_failed(design_id, provider_name, "invalid_step_export", reason)
            self._reject(design_id, reason)
            raise DesignRejectedError(reason, design_id=design_id)
        artifact_limit = max(DEFAULT_MAX_ARTIFACT_BYTES, self._max_step_bytes)
        if build.native_bytes is not None and len(build.native_bytes) > artifact_limit:
            reason = (
                f"the {provider_name} native export is larger than the configured "
                f"{artifact_limit} byte artifact limit"
            )
            self._provider_failed(design_id, provider_name, "native_too_large", reason)
            self._reject(design_id, reason)
            raise DesignRejectedError(reason, design_id=design_id)

    def _import_view(
        self,
        design_id: str,
        provider_name: str,
        build: ProviderBuild,
    ) -> StepImportOutcome:
        try:
            return import_step_bytes(
                build.step_bytes,
                max_step_bytes=self._max_step_bytes,
                store_artifact=False,
            )
        except StepImportError as error:
            reason = f"the {provider_name} STEP export could not be imported for display: {error}"
            self._provider_failed(design_id, provider_name, error.code, reason)
            self._reject(design_id, reason)
            raise DesignRejectedError(reason, design_id=design_id) from None

    def load(self, design_id: str) -> DesignOutcome:
        """Load a design from the cache, or from its stored STEP and journal."""
        if not design_available():
            raise DesignUnavailableError("the design pipeline requires the OCP bindings")
        _validate_design_id(design_id)
        with _GEOMETRY_LOCK:
            return self._load_locked(design_id)

    def _load_locked(
        self,
        design_id: str,
        *,
        stored: _StoredDesign | None = None,
    ) -> DesignOutcome:
        cached = self._cache_get(design_id)
        if cached is not None:
            return replace(cached, cached=True)
        record = stored if stored is not None else self._find_journal_design(design_id)
        if record is None:
            raise DesignNotFoundError("no design is stored under that id")
        if record.build is None:
            raise DesignNotFoundError("the design has no completed provider build")
        build = record.build
        intent = record.intent
        provider_name = build.get("provider")
        if not isinstance(provider_name, str) or provider_name not in ALLOWED_PROVIDER_IDS:
            raise DesignRejectedError(
                "the stored design does not name a known CAD provider",
                design_id=design_id,
            )
        script_sha256 = build.get("script_sha256")
        if not isinstance(script_sha256, str) or not self._journal.has_artifact(script_sha256):
            raise DesignRejectedError(
                "the stored design has no script artifact",
                design_id=design_id,
            )
        try:
            script_text = self._journal.get_artifact_text(script_sha256)
        except FileNotFoundError:
            raise DesignRejectedError(
                "the stored script artifact is missing",
                design_id=design_id,
            ) from None
        try:
            script = ConstructionScript.model_validate_json(script_text)
        except Exception as error:
            raise DesignRejectedError(
                f"the stored script artifact is not a valid construction script: "
                f"{type(error).__name__}",
                design_id=design_id,
            ) from None
        if design_id_for(script, provider_name) != design_id:
            raise DesignRejectedError(
                "the stored script artifact does not hash to its design id",
                design_id=design_id,
            )

        step_sha256 = build.get("step_sha256")
        if not isinstance(step_sha256, str) or not self._journal.has_artifact(step_sha256):
            raise DesignRejectedError(
                "the stored design has no STEP artifact",
                design_id=design_id,
            )
        try:
            step_bytes = self._journal.get_artifact(step_sha256)
            imported = import_step_bytes(
                step_bytes,
                max_step_bytes=self._max_step_bytes,
                store_artifact=False,
            )
        except (FileNotFoundError, StepImportError) as error:
            raise DesignRejectedError(
                f"the stored STEP artifact could not be imported: {type(error).__name__}",
                design_id=design_id,
            ) from None

        envelope = _envelope_from_payload(intent.get("envelope"))
        tolerances = _string_tuple(intent.get("tolerances"))
        material_value = intent.get("material")
        material = material_value if isinstance(material_value, str) else None
        label_value = intent.get("label")
        label = label_value if isinstance(label_value, str) and label_value else str(script.summary)
        resolved = _ResolvedIntent(
            script=script,
            envelope=envelope,
            tolerances=tolerances,
            material=material,
            label=label,
            template_name=None,
        )
        measured = _measured_from_payload(build.get("measured"))
        raw_valid = build.get("valid")
        valid = raw_valid if isinstance(raw_valid, bool) else None
        solid_count = build.get("solid_count")
        if isinstance(solid_count, bool) or not isinstance(solid_count, int) or solid_count < 0:
            solid_count = None
        checks = build_provider_checks(
            measured=measured,
            valid=valid,
            solid_count=solid_count,
            expected_solids=expected_solid_count(script),
            expected_envelope=envelope,
            expected_pockets=expected_pocket_count(script),
            warnings=_string_tuple(build.get("warnings")),
        )
        verified = bool(checks) and all(check.passed for check in checks)
        provider_version = build.get("provider_version")
        measured_by_value = build.get("measured_by")
        measured_by = (
            measured_by_value
            if isinstance(measured_by_value, str) and measured_by_value
            else provider_name
        )
        native_kind_value = build.get("native_kind")
        native_kind = native_kind_value if isinstance(native_kind_value, str) else None
        native_sha_value = build.get("native_sha256")
        native_sha256 = native_sha_value if isinstance(native_sha_value, str) else None
        outcome = self._make_outcome(
            design_id=design_id,
            resolved=resolved,
            provider=provider_name,
            provider_version=(provider_version if isinstance(provider_version, str) else None),
            measured_by=measured_by,
            shape=imported.shape,
            measurement=measured,
            valid=valid,
            solid_count=solid_count,
            checks=checks,
            verified=verified,
            view=imported.mesh,
            view_bounds=imported.bounds,
            view_solids=imported.solids,
            view_warnings=imported.warnings,
            step_sha256=step_sha256,
            native_sha256=native_sha256,
            native_kind=native_kind,
            journal_head=None,
            warnings=_string_tuple(build.get("warnings")),
            cached=True,
        )
        self._cache_put(outcome)
        return outcome

    # ---------------------------------------------------------------- artifacts

    def drawing(self, design_id: str, views: Sequence[str]) -> Drawing:
        """Return the HLR views of a stored design's imported shape."""
        if not design_available():
            raise DesignUnavailableError("the design pipeline requires the OCP bindings")
        from kala_agent.studio.design.drawing import hlr_views

        with _GEOMETRY_LOCK:
            outcome = self._load_locked(design_id)
            return hlr_views(outcome.shape, views=views)

    def section(
        self,
        design_id: str,
        *,
        plane: str = "XZ",
        offset_mm: float | None = None,
    ) -> Section:
        """Return a section of a stored design's imported shape."""
        if not design_available():
            raise DesignUnavailableError("the design pipeline requires the OCP bindings")
        from kala_agent.studio.design.drawing import section

        with _GEOMETRY_LOCK:
            outcome = self._load_locked(design_id)
            return section(outcome.shape, plane=plane, offset_mm=offset_mm)

    def export(self, design_id: str, fmt: str) -> ExportOutcome:
        """Export the provider's stored STEP, or a display STL, for a design.

        A STEP export is the exact bytes the provider returned, never a
        re-export of the imported shape. OpenCascade may only re-export when
        it was the provider that built the model; even then this code returns
        the stored bytes, which are identical to what it exported.
        """
        if not design_available():
            raise DesignUnavailableError("the design pipeline requires the OCP bindings")
        extension = fmt.strip().lower()
        if extension not in {"step", "stl"}:
            raise DesignError("the export format must be step or stl")
        with _GEOMETRY_LOCK:
            outcome = self._load_locked(design_id)
            if extension == "step":
                if outcome.step_sha256 is None:
                    raise DesignError("the design has no stored provider STEP artifact")
                try:
                    data = self._journal.get_artifact(outcome.step_sha256)
                except FileNotFoundError:
                    raise DesignError("the stored provider STEP artifact is missing") from None
            else:
                from kala_agent.studio.design.mesh import stl_bytes

                data = stl_bytes(outcome.view)
            digest = hashlib.sha256(data).hexdigest()
            if extension == "step" and digest != outcome.step_sha256:
                raise DesignError("the stored STEP artifact does not match its digest")
            stored = self._journal.put_artifact(data)
            if stored != digest:
                raise DesignError("the exported artifact digest did not match its bytes")
            filename = f"kala-studio-{design_id[:12]}.{extension}"
            self._journal.append(
                JournalEventType.ARTIFACT_STORED,
                {
                    "design_id": design_id,
                    "provider": outcome.provider,
                    "kind": extension,
                    "sha256": digest,
                    "size_bytes": len(data),
                },
                aggregate_id=design_id,
            )
            self._journal.append(
                JournalEventType.EXPORT_COMPLETED,
                {
                    "design_id": design_id,
                    "provider": outcome.provider,
                    "format": extension,
                    "sha256": digest,
                    "size_bytes": len(data),
                    "filename": filename,
                },
                aggregate_id=design_id,
            )
            return ExportOutcome(
                design_id=design_id,
                format=extension,
                filename=filename,
                sha256=digest,
                data=data,
            )

    def artifact(self, design_id: str, kind: str) -> ArtifactDownload:
        """Read one stored artifact of one design for download.

        The bytes are the exact bytes the provider returned (never a
        re-export), and their SHA-256 must equal the digest the journal
        recorded before a single byte is served. A kind the design did not
        store is a 404, an artifact above the download ceiling is a 413, and a
        digest mismatch is a refusal rather than a download.
        """
        _validate_design_id(design_id)
        normalized = kind.strip().lower()
        if normalized not in ARTIFACT_KINDS:
            raise DesignError(
                "the artifact kind must be one of " + ", ".join(ARTIFACT_KINDS)
            )
        record = self._find_artifact_record(design_id, normalized)
        if record is None:
            raise DesignNotFoundError(f"design_id={design_id[:64]}")
        digest = record.digest
        if digest is None:
            raise DesignNotFoundError(
                f"no {normalized} artifact is stored for design_id={design_id[:64]}"
            )
        try:
            data = self._journal.get_artifact(digest)
        except FileNotFoundError:
            raise DesignNotFoundError(
                f"the stored {normalized} artifact is missing for "
                f"design_id={design_id[:64]}"
            ) from None
        except ValueError as error:
            # The content-addressed store verifies the digest on read. A
            # mismatch is an integrity refusal, not a missing file; a size
            # refusal is the download ceiling.
            message = str(error)
            if "digest verification failed" in message:
                raise ArtifactDigestMismatchError(
                    f"the stored {normalized} artifact does not match its "
                    "journalled digest"
                ) from None
            if "exceeds the configured size limit" in message:
                raise ArtifactTooLargeError(
                    f"the stored {normalized} artifact exceeds the download limit"
                ) from None
            raise DesignNotFoundError(
                f"the stored {normalized} artifact could not be read for "
                f"design_id={design_id[:64]}"
            ) from None
        if len(data) > MAX_ARTIFACT_DOWNLOAD_BYTES:
            raise ArtifactTooLargeError(
                f"the stored {normalized} artifact is {len(data)} bytes; the "
                f"download limit is {MAX_ARTIFACT_DOWNLOAD_BYTES} bytes"
            )
        actual = hashlib.sha256(data).hexdigest()
        if actual != digest:
            raise ArtifactDigestMismatchError(
                f"the stored {normalized} artifact does not match its journalled digest"
            )
        return ArtifactDownload(
            design_id=design_id,
            kind=normalized,
            sha256=digest,
            content_type=_artifact_content_type(normalized),
            filename=_artifact_filename(record.label, digest, normalized),
            data=data,
        )

    def step_digest(self, design_id: str) -> str | None:
        """Return the stored provider STEP digest for ``design_id``."""
        cached = self._cache_get(design_id)
        if cached is not None:
            return cached.step_sha256
        stored = self._find_journal_design(design_id)
        if stored is None or stored.build is None:
            return None
        digest = stored.build.get("step_sha256")
        return digest if isinstance(digest, str) else None

    # -------------------------------------------------------------------- chat

    def record_chat_turn(
        self,
        *,
        provider: str,
        model: str | None,
        offline: bool,
        prompt: str,
        design_id: str | None,
        design_requested: bool,
        missing: Sequence[str] = (),
        cad_provider: str | None = None,
    ) -> SessionEvent:
        """Append the ``chat.turn`` event without storing prompt text."""
        payload: dict[str, object] = {
            "provider": provider,
            "model": model,
            "offline": offline,
            "prompt_sha256": hashlib.sha256(prompt.encode("utf-8")).hexdigest(),
            "prompt_chars": len(prompt),
            "design_id": design_id,
            "design_requested": design_requested,
            "cad_provider": cad_provider,
            "missing": [item[:64] for item in missing[:16]],
        }
        return self._journal.append(JournalEventType.CHAT_TURN, payload)

    # ---------------------------------------------------------------- journal

    def journal_page(self, after: int = 0, limit: int = 200) -> tuple[SessionEvent, ...]:
        """Return one bounded page of Studio journal events."""
        return self._journal.page(after, limit)

    def verify(self) -> JournalVerification:
        """Return the chain verification verdict."""
        return self._journal.verify()

    # ---------------------------------------------------------------- internals

    def _resolve_intent(self, intent: DesignIntent) -> _ResolvedIntent:
        from kala_agent.studio.design import templates

        call = intent.template
        if call is None:
            script = intent.script
            if script is None:  # pragma: no cover - the intent validator guarantees this
                raise DesignRejectedError(
                    "the design intent carries neither a template nor a script"
                )
            return _ResolvedIntent(
                script=script,
                envelope=None,
                tolerances=tuple(intent.tolerances),
                material=intent.material,
                label=intent.label or str(script.summary),
                template_name=None,
            )
        try:
            if isinstance(call, LbracketCall):
                result = templates.l_bracket(
                    width=call.width,
                    depth=call.depth,
                    height=call.height,
                    wall=call.wall,
                    bore_d=call.bore_d,
                    hole_d=call.hole_d,
                    hole_count=call.hole_count,
                    edge=call.edge,
                )
            elif isinstance(call, SteppedShaftCall):
                result = templates.stepped_shaft(
                    [(segment.diameter, segment.length) for segment in call.segments]
                )
            elif isinstance(call, HousingCall):
                result = templates.housing_envelope(
                    width=call.width,
                    depth=call.depth,
                    height=call.height,
                    wall=call.wall,
                )
            else:  # pragma: no cover - the discriminated union is closed
                raise DesignRejectedError("unknown template")
        except templates.TemplateError as error:
            raise DesignRejectedError(str(error)) from None
        except ValueError as error:
            raise DesignRejectedError(f"the template produced an invalid script: {error}") from None
        return _ResolvedIntent(
            script=result.script,
            envelope=result.envelope,
            tolerances=tuple(intent.tolerances) or result.tolerances,
            material=intent.material,
            label=intent.label or result.label,
            template_name=call.template,
        )

    def _reject(self, design_id: str, reason: str) -> None:
        self._journal.append(
            _REFUSED_EVENT,
            {"design_id": design_id, "reason": reason[:512]},
            aggregate_id=design_id,
        )

    def _provider_failed(
        self,
        design_id: str,
        provider: str,
        code: str,
        reason: str,
    ) -> None:
        self._journal.append(
            PROVIDER_BUILD_FAILED,
            {
                "design_id": design_id,
                "provider": provider,
                "code": code[:64],
                "reason": reason[:512],
            },
            aggregate_id=design_id,
        )

    def _intent_payload(
        self,
        design_id: str,
        resolved: _ResolvedIntent,
        *,
        provider: str,
        script_sha256: str,
        cache_hit: bool,
    ) -> dict[str, object]:
        return {
            "design_id": design_id,
            "provider": provider,
            "script_sha256": script_sha256,
            "template": resolved.template_name,
            "label": resolved.label,
            "material": resolved.material,
            "tolerances": list(resolved.tolerances),
            "envelope": _envelope_payload(resolved.envelope),
            "cache_hit": cache_hit,
        }

    def _make_outcome(
        self,
        *,
        design_id: str,
        resolved: _ResolvedIntent,
        provider: str,
        provider_version: str | None,
        measured_by: str,
        shape: TopoDS_Shape,
        measurement: MeasuredProperties,
        valid: bool | None,
        solid_count: int | None,
        checks: tuple[Check, ...],
        verified: bool,
        view: MeshData,
        view_bounds: Bounds,
        view_solids: int,
        view_warnings: tuple[str, ...],
        step_sha256: str | None,
        native_sha256: str | None,
        native_kind: str | None,
        journal_head: str | None,
        warnings: tuple[str, ...],
        cached: bool,
    ) -> DesignOutcome:
        script = resolved.script
        combined_warnings = _bounded_warnings((*warnings, *view_warnings))
        return DesignOutcome(
            design_id=design_id,
            label=resolved.label,
            script=script.model_dump(mode="json", exclude_none=True),
            provider=provider,
            provider_version=provider_version,
            measured_by=measured_by,
            shape=shape,
            measurement=measurement,
            valid=valid,
            solid_count=solid_count,
            checks=checks,
            verified=verified,
            view=view,
            view_bounds=view_bounds,
            view_solids=view_solids,
            view_warnings=view_warnings,
            script_sha256=hashlib.sha256(canonical_script_json(script)).hexdigest(),
            step_sha256=step_sha256,
            native_sha256=native_sha256,
            native_kind=native_kind,
            journal_head=journal_head,
            tolerances=resolved.tolerances,
            material=resolved.material,
            declared_envelope=resolved.envelope,
            cached=cached,
            warnings=combined_warnings,
            component_count=len(script.components),
            fused=_script_fused(script),
        )

    def _cache_get(self, design_id: str) -> DesignOutcome | None:
        with self._cache_lock:
            outcome = self._cache.get(design_id)
            if outcome is not None:
                self._cache.move_to_end(design_id)
            return outcome

    def _cache_put(self, outcome: DesignOutcome) -> None:
        with self._cache_lock:
            self._cache[outcome.design_id] = outcome
            self._cache.move_to_end(outcome.design_id)
            while len(self._cache) > self._cache_size:
                self._cache.popitem(last=False)

    def _find_journal_design(self, design_id: str) -> _StoredDesign | None:
        intent_payload: Mapping[str, object] | None = None
        build_payload: Mapping[str, object] | None = None
        after = 0
        page_size = self._journal.max_page_size
        while True:
            events = self._journal.page(after, page_size)
            if not events:
                break
            for event in events:
                payload = event.payload
                if payload is None or payload.get("design_id") != design_id:
                    continue
                if event.event_type == JournalEventType.DESIGN_INTENT.value:
                    intent_payload = payload
                elif event.event_type == PROVIDER_BUILD_COMPLETED:
                    build_payload = payload
            after = events[-1].sequence
        if intent_payload is None and build_payload is None:
            return None
        if build_payload is not None and intent_payload is None:
            return _StoredDesign(intent={}, build=build_payload)
        return _StoredDesign(
            intent=intent_payload or {},
            build=build_payload,
        )

    def _find_artifact_record(
        self, design_id: str, kind: str
    ) -> _ArtifactRecord | None:
        """Return the label and recorded digest of one artifact kind.

        The completed build record is authoritative for ``script``, ``step``
        and the provider's native kind; an ``artifact.stored`` record supplies
        the kinds only an export writes (``stl``), and is the fallback for the
        rest. The scan is the same bounded page walk as
        :meth:`_find_journal_design`, so an evicted cache entry still
        downloads.
        """
        label: str | None = None
        build_digest: str | None = None
        stored_digest: str | None = None
        seen = False
        after = 0
        page_size = self._journal.max_page_size
        while True:
            events = self._journal.page(after, page_size)
            if not events:
                break
            for event in events:
                payload = event.payload
                if payload is None or payload.get("design_id") != design_id:
                    continue
                seen = True
                if event.event_type == JournalEventType.DESIGN_INTENT.value:
                    value = payload.get("label")
                    label = value if isinstance(value, str) else None
                elif event.event_type == PROVIDER_BUILD_COMPLETED:
                    if kind in {"script", "step"}:
                        value = payload.get(f"{kind}_sha256")
                        build_digest = value if isinstance(value, str) else None
                    elif payload.get("native_kind") == kind:
                        value = payload.get("native_sha256")
                        build_digest = value if isinstance(value, str) else None
                elif (
                    event.event_type == JournalEventType.ARTIFACT_STORED.value
                    and payload.get("kind") == kind
                ):
                    value = payload.get("sha256")
                    stored_digest = value if isinstance(value, str) else None
            after = events[-1].sequence
        if not seen:
            return None
        return _ArtifactRecord(
            label=label or "",
            digest=build_digest or stored_digest,
        )

    def _stored_design_is_available(self, stored: _StoredDesign) -> bool:
        build = stored.build
        if build is None:
            return False
        script_sha256 = build.get("script_sha256")
        step_sha256 = build.get("step_sha256")
        if not isinstance(script_sha256, str) or not isinstance(step_sha256, str):
            return False
        return self._journal.has_artifact(script_sha256) and self._journal.has_artifact(step_sha256)


# ------------------------------------------------------------------ serializers


def _validate_design_id(design_id: str) -> None:
    if (
        not isinstance(design_id, str)
        or len(design_id) != DESIGN_ID_PATTERN_LENGTH
        or any(character not in "0123456789abcdef" for character in design_id)
    ):
        raise DesignError("the design id must be a 64-character lowercase hex digest")


def _script_fused(script: ConstructionScript) -> bool:
    return any(step.op == "fuse" for step in script.assembly)


def _bounds_payload(bounds: Bounds | MeasuredBounds | None) -> dict[str, object] | None:
    if bounds is None:
        return None
    return {"min": list(bounds.min_mm), "max": list(bounds.max_mm)}


def _envelope_payload(bounds: Bounds | None) -> dict[str, object] | None:
    return _bounds_payload(bounds)


def _measured_payload(measured: MeasuredProperties) -> dict[str, object]:
    return {
        "volume_mm3": measured.volume_mm3,
        "surface_area_mm2": measured.area_mm2,
        "centroid_mm": None if measured.centroid_mm is None else list(measured.centroid_mm),
        "bounds": _bounds_payload(measured.bounds),
        "mass_g": measured.mass_g,
    }


def _measured_from_payload(value: object) -> MeasuredProperties:
    if not isinstance(value, Mapping):
        return MeasuredProperties()
    return MeasuredProperties(
        volume_mm3=_finite_or_none(value.get("volume_mm3")),
        area_mm2=_measured_area_from_payload(value),
        bounds=_measured_bounds_from_payload(value.get("bounds")),
        mass_g=_finite_or_none(value.get("mass_g")),
        centroid_mm=_centroid_from_payload(value.get("centroid_mm")),
    )


def _measured_area_from_payload(value: Mapping[str, object]) -> float | None:
    """Read the measured surface area the way :func:`_measured_payload` writes it.

    The payload has always written ``surface_area_mm2`` (the API's own field
    name), while an earlier reader looked for ``area_mm2`` (the measurement
    dataclass's field name), so a design restored from the journal lost its
    area. The written key wins; the legacy key stays as a fallback so a
    journal written before the fix still restores its area.
    """
    area = _finite_or_none(value.get("surface_area_mm2"))
    if area is None:
        area = _finite_or_none(value.get("area_mm2"))
    return area


def _finite_or_none(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    return number if math.isfinite(number) else None


def _centroid_from_payload(value: object) -> tuple[float, float, float] | None:
    if not isinstance(value, (list, tuple)) or len(value) != 3:
        return None
    coordinates: list[float] = []
    for item in value:
        number = _finite_or_none(item)
        if number is None:
            return None
        coordinates.append(number)
    return (coordinates[0], coordinates[1], coordinates[2])


def _measured_bounds_from_payload(value: object) -> MeasuredBounds | None:
    if not isinstance(value, Mapping):
        return None
    minimum = value.get("min")
    maximum = value.get("max")
    if not isinstance(minimum, (list, tuple)) or not isinstance(maximum, (list, tuple)):
        return None
    if len(minimum) != 3 or len(maximum) != 3:
        return None
    coordinates: list[float] = []
    for item in (*minimum, *maximum):
        number = _finite_or_none(item)
        if number is None:
            return None
        coordinates.append(number)
    try:
        return MeasuredBounds(
            min_mm=(coordinates[0], coordinates[1], coordinates[2]),
            max_mm=(coordinates[3], coordinates[4], coordinates[5]),
        )
    except ValueError:
        return None


def _bounded_warnings(warnings: Sequence[str]) -> tuple[str, ...]:
    """Return the bounded, de-duplicated display of a design's warnings.

    Duplicates are dropped first, then the shared bounding rule keeps at most
    ``MAX_WARNINGS`` lines and writes the number of any suppressed warnings on
    the last lines, so the ineffective-cut count stays exact.
    """
    seen: set[str] = set()
    unique: list[str] = []
    for warning in warnings:
        text = str(warning).strip()
        if not text or text in seen:
            continue
        seen.add(text)
        unique.append(text)
    return _bounded_warning_lines(unique)


def _string_tuple(value: object) -> tuple[str, ...]:
    if not isinstance(value, (list, tuple)):
        return ()
    return tuple(item for item in value if isinstance(item, str))


def _envelope_from_payload(value: object) -> Bounds | None:
    if not isinstance(value, Mapping):
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
    from kala_agent.studio.design.measure import Bounds

    try:
        return Bounds(
            min_mm=(coordinates[0], coordinates[1], coordinates[2]),
            max_mm=(coordinates[3], coordinates[4], coordinates[5]),
        )
    except ValueError:
        return None


def check_payload(check: Check) -> dict[str, object]:
    """Return one check row as a JSON-safe mapping."""
    return {
        "id": check.id,
        "label": check.label,
        "expected": _check_value(check.expected),
        "measured": _check_value(check.measured),
        "tolerance": check.tolerance,
        "unit": check.unit,
        "passed": check.passed,
        "status": check.status,
    }


def _check_value(value: object) -> object:
    if isinstance(value, tuple):
        return [float(item) for item in value]
    return value
