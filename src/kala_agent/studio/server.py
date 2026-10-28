"""The standalone Kala Studio HTTP service.

The server is a thin ASGI surface that wraps the studio geometry,
integrations registry, and provider dispatch. It runs **independently** of
the core ``kala_agent.application`` composition root: the studio is a
shipping product in its own right and must function even when the core
data directory is closed or unavailable. The server enforces the
request boundary (loopback host, same-origin browser, bearer token on
mutating methods, bounded request body) and answers six read-only/status
endpoints plus five mutating endpoints.

Routes:

- ``GET  /api/studio/health`` — liveness and capability summary.
- ``GET  /api/studio/integrations`` — registry rows.
- ``GET  /api/studio/providers`` — provider rows.
- ``GET  /api/studio/settings`` — non-secret provider credential status.
- ``POST /api/studio/settings`` — store or clear a provider API key.
- ``POST /api/studio/preview`` — preview mesh.
- ``POST /api/studio/export`` — STL or STEP file.
- ``POST /api/studio/handoff`` — neutral-file handoff for a CAD target.
- ``POST /api/studio/chat`` — chat reply from a configured provider.

The studio never instantiates the core ``KalaApplication``: this server
and the core API share no state, no token file, and no data directory.
"""

from __future__ import annotations

import hashlib
import logging
import math
import os
import secrets
from collections.abc import AsyncIterator, Callable, Mapping, Sequence
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from pathlib import Path
from typing import TYPE_CHECKING, Final

from fastapi import APIRouter, FastAPI, Query, Request, Response
from starlette.middleware import Middleware

from kala_agent.studio.boundary import (
    TOKEN_REQUIRED_MESSAGE,
    resolve_studio_token,
)
from kala_agent.studio.design.providers.base import (
    CadProvider,
    CadProviderUnavailableError,
    MeasuredBounds,
)
from kala_agent.studio.design.artifacts import ARTIFACT_KINDS
from kala_agent.studio.design.service import (
    DESIGN_ID_PATTERN_LENGTH,
    MAX_SECTION_OFFSET_MM,
    ArtifactDigestMismatchError,
    ArtifactTooLargeError,
    DesignError,
    DesignNotFoundError,
    DesignOutcome,
    DesignRejectedError,
    DesignService,
    DesignUnavailableError,
    check_payload,
    design_available,
)
from kala_agent.studio.errors import install_error_handlers
from kala_agent.studio.geometry import (
    ANALYTIC_WARNING,
    MESH_ENGINE,
    STEP_UNAVAILABLE_MESSAGE,
    Bounds,
    Mesh,
    build_primitive,
    compute_bounds,
    probe_step_capability,
    write_binary_stl,
)
from kala_agent.studio.geometry.step import (
    StepUnavailableError,
    export_step_primitive,
)
from kala_agent.studio.integrations import build_registry, integration_by_id
from kala_agent.studio.middleware import (
    BearerTokenMiddleware,
    HostGuardMiddleware,
    MaxBodySizeMiddleware,
    SameOriginGuardMiddleware,
    SecurityHeadersMiddleware,
    StaticFileMiddleware,
    validate_max_body_bytes,
)
from kala_agent.studio.providers.base import (
    ProviderConfigurationError,
    ProviderMissingCredentialError,
    ProviderNetworkError,
    ProviderProtocolError,
    ProviderSubprocessError,
    ProviderTimeoutError,
    ProviderUnavailableError,
)
from kala_agent.studio.providers.cli_runtime import CliProbe
from kala_agent.studio.providers.dispatch import (
    PROVIDER_IDS,
    StudioChatTurn,
    build_feedback,
    build_repair_turn,
    chat,
    design_turn_budget_seconds,
    list_provider_records,
    probe_subscription_clis_cached,
    provider_call,
)
from kala_agent.studio.providers.local import detect_design_family
from kala_agent.studio.schemas import (
    MAX_DRAWING_VIEWS,
    MAX_JOURNAL_LIMIT,
    BBox,
    CadProviderRecord,
    CadProvidersResponse,
    ChatRequest,
    ChatResponse,
    DesignArtifacts,
    DesignBounds,
    DesignCheck,
    DesignMeasured,
    DesignRequest,
    DesignResult,
    DesignView,
    DrawingPolyline,
    DrawingResponse,
    DrawingView,
    ExportFormat,
    ExportRequest,
    HandoffRequest,
    HandoffResponse,
    HealthResponse,
    IntegrationRecord,
    IntegrationsResponse,
    JournalEventRecord,
    JournalPageResponse,
    JournalVerifyResponse,
    PreviewRequest,
    PreviewResponse,
    PrimitiveKind,
    ProvidersResponse,
    SectionBounds2D,
    SectionLoopResponse,
    SectionResponse,
    StudioSettingsResponse,
    UpdateProviderKeyRequest,
    error_response,
)
from kala_agent.studio.session_journal import JournalEventType
from kala_agent.studio.settings import (
    describe_provider_credentials,
    effective_environ,
    write_stored_key,
)
from kala_agent.studio.static import resolve_static_root

if TYPE_CHECKING:
    from kala_agent.studio.design.drawing import Drawing, Polyline, Section
    from kala_agent.studio.design.measure import Bounds as DesignMeasureBounds
    from kala_agent.studio.session_journal import SessionEvent

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.studio.server")
STUDIO_PREFIX: Final[str] = "/api/studio"
STUDIO_VERSION: Final[str] = "0.1.0"
#: Environment variable carrying the Electron shell's per-launch readiness
#: nonce. It is echoed on ``/health`` in :data:`LAUNCH_NONCE_HEADER`.
LAUNCH_NONCE_ENV: Final[str] = "KALA_STUDIO_LAUNCH_NONCE"
LAUNCH_NONCE_HEADER: Final[str] = "X-Kala-Launch-Nonce"
DESIGN_UNAVAILABLE_MESSAGE: Final[str] = (
    "Design compilation requires the OCP bindings (the 'cadquery-ocp' package). "
    "The studio never fabricates a design; install 'cadquery-ocp' and restart "
    "the studio to enable it."
)
ALLOWED_VIEWS: Final[tuple[str, ...]] = ("front", "top", "right", "iso")
ALLOWED_SECTION_PLANES: Final[tuple[str, ...]] = ("XY", "XZ", "YZ")
DEFAULT_DRAWING_LIMIT: Final[int] = 200


def _bounds_to_bbox(bounds: Bounds) -> BBox:
    return BBox(
        min=(float(bounds.min_x), float(bounds.min_y), float(bounds.min_z)),
        max=(float(bounds.max_x), float(bounds.max_y), float(bounds.max_z)),
    )


def _make_mesh_response(mesh: Mesh) -> PreviewResponse:
    bounds = compute_bounds(mesh.positions)
    return PreviewResponse(
        positions=mesh.positions,
        normals=mesh.normals,
        indices=mesh.indices,
        bounds=_bounds_to_bbox(bounds),
        engine=MESH_ENGINE,
        warnings=(ANALYTIC_WARNING,),
    )


def _filename_for(kind: PrimitiveKind, fmt: ExportFormat) -> str:
    extension = "stl" if fmt == ExportFormat.STL else "step"
    return f"kala-studio-{kind.value}.{extension}"


def _media_type_for(fmt: ExportFormat) -> str:
    if fmt == ExportFormat.STL:
        return "model/stl"
    return "application/step"


def _resolve_workdir(environ: Mapping[str, str]) -> Path:
    """Return a working directory for transient STEP artefacts.

    The directory lives inside the studio data directory when one is
    available, otherwise in the system temporary folder. The directory is
    not created here; ``export_step_primitive`` creates it on demand.
    """
    explicit = environ.get("KALA_STUDIO_DATA_DIR")
    if explicit:
        try:
            return Path(explicit).expanduser().resolve(strict=False)
        except (OSError, RuntimeError):
            pass
    import tempfile

    return Path(tempfile.gettempdir()) / "kala-studio-work"


def _request_context(request: Request | None) -> tuple[Mapping[str, str], Path | None]:
    if request is None:
        return os.environ, None
    state = getattr(request.app, "state", None)
    data_dir = getattr(state, "studio_data_dir", None)
    use_custom = getattr(state, "studio_use_custom_environ", False)
    base_env: Mapping[str, str] = (
        getattr(state, "studio_environ", os.environ) if use_custom else os.environ
    )
    return base_env, data_dir if isinstance(data_dir, Path) else None


def _effective_request_environ(request: Request | None) -> dict[str, str]:
    base_env, data_dir = _request_context(request)
    return effective_environ(base_env, data_dir)


router = APIRouter()


@router.get(
    "/health",
    summary="Studio process liveness",
    response_model=HealthResponse,
    tags=["studio"],
)
def health(request: Request = None, response: Response = None) -> HealthResponse:  # type: ignore[assignment]
    """Return liveness facts about the studio without leaking secrets."""
    env = _effective_request_environ(request)
    # The Electron shell passes a random per-launch nonce in this process's
    # environment and requires it back on the readiness response. Echoing it
    # proves the listener that answered is the child the shell spawned, not a
    # stale or hostile process that had already bound the loopback port. The
    # body schema is unchanged; the nonce travels in a header because it is
    # proof *about* the process, not a liveness fact.
    launch_nonce = env.get(LAUNCH_NONCE_ENV, "")
    if response is not None and launch_nonce:
        response.headers[LAUNCH_NONCE_HEADER] = launch_nonce
    step_available = probe_step_capability().available
    # `offline` is an evidence-backed answer, not an API-key check: a
    # signed-in subscription CLI makes a remote turn possible even with no
    # stored key, so the CLI verdicts take part in the decision. The cached
    # probe never holds this liveness route open for a CLI.
    clis = probe_subscription_clis_cached(env)
    return HealthResponse(
        status="ok",
        version=STUDIO_VERSION,
        offline=not _any_remote_route_available(env, clis),
        design_turn_budget_seconds=design_turn_budget_seconds(),
        step_available=step_available,
        design_available=design_available(),
        integrations=tuple(row.id for row in build_registry()),
        providers=PROVIDER_IDS,
    )


def _any_remote_route_available(
    environ: Mapping[str, str] | None = None,
    clis: Sequence[CliProbe] = (),
) -> bool:
    """Return whether a remote turn is *known* to be possible right now.

    A remote route is available when an OpenAI-compatible credential is
    configured **or** a subscription CLI is installed and signed in. The
    ``offline`` field of ``/health`` and ``/settings`` is the negation of
    this answer; it is therefore never "offline" while a usable CLI is on
    the host. A probe that has not produced a verdict yet does not count,
    so the field can stay conservative for at most the route probe budget.
    """
    # Only providers the build can actually reach count: the codex route is
    # fail-closed in this build, so a codex CLI on PATH does not make the
    # studio "online" unless the CLI itself reports a signed-in session.
    source = environ if environ is not None else os.environ
    openai = bool(source.get("OPENAI_API_KEY", "").strip())
    openrouter = bool(source.get("OPENROUTER_API_KEY", "").strip())
    return openai or openrouter or any(probe.usable() for probe in clis)


@router.get(
    "/integrations",
    summary="Static integration registry",
    response_model=IntegrationsResponse,
    tags=["studio"],
)
def integrations() -> IntegrationsResponse:
    return IntegrationsResponse(integrations=build_registry())


@router.get(
    "/providers",
    summary="Available chat providers",
    response_model=ProvidersResponse,
    tags=["studio"],
)
def providers(request: Request = None) -> ProvidersResponse:  # type: ignore[assignment]
    env = _effective_request_environ(request)
    clis = probe_subscription_clis_cached(env)
    return ProvidersResponse(providers=list_provider_records(env, clis))


@router.get(
    "/providers/cad",
    summary="CAD provider availability (probed, never assumed)",
    response_model=CadProvidersResponse,
    tags=["studio"],
)
def cad_providers(request: Request) -> CadProvidersResponse | Response:
    """Return each CAD provider's live probe result.

    ADR-0011: availability is evidence from this host, never a constant. The
    route carries the launch bearer like the other new read routes so a
    same-origin page cannot enumerate a Windows bridge it does not own.
    """
    denied = _require_bearer(request)
    if denied is not None:
        return denied
    service = _design_service(request)
    return CadProvidersResponse(
        providers=tuple(
            CadProviderRecord(
                id=status.id,
                name=status.name,
                available=status.available,
                version=status.version,
                reason=status.reason,
                measured_by=status.measured_by,
                mode=status.mode,
            )
            for status in service.probe_providers()
        )
    )


@router.get(
    "/settings",
    summary="Non-secret provider credential status",
    response_model=StudioSettingsResponse,
    tags=["studio"],
)
def get_settings(request: Request) -> StudioSettingsResponse:
    base_env, data_dir = _request_context(request)
    env = effective_environ(base_env, data_dir)
    clis = probe_subscription_clis_cached(env)
    return StudioSettingsResponse(
        credentials=describe_provider_credentials(base_env, data_dir, clis),
        providers=list_provider_records(env, clis),
        offline=not _any_remote_route_available(env, clis),
    )


@router.post(
    "/settings",
    summary="Store or clear a provider API key on the local backend",
    response_model=StudioSettingsResponse,
    tags=["studio"],
)
def update_settings(
    payload: UpdateProviderKeyRequest,
    request: Request,
) -> StudioSettingsResponse | Response:
    base_env, data_dir = _request_context(request)
    if data_dir is None:
        import tempfile

        data_dir = Path(tempfile.gettempdir()) / "kala-studio"
        data_dir.mkdir(parents=True, exist_ok=True)
    try:
        write_stored_key(data_dir, payload.provider.value, payload.api_key)
    except ValueError as error:
        return _invalid_request_response(error)
    env = effective_environ(base_env, data_dir)
    clis = probe_subscription_clis_cached(env)
    return StudioSettingsResponse(
        credentials=describe_provider_credentials(base_env, data_dir, clis),
        providers=list_provider_records(env, clis),
        offline=not _any_remote_route_available(env, clis),
    )


@router.post(
    "/preview",
    summary="Build a parametric mesh preview",
    response_model=PreviewResponse,
    tags=["studio"],
)
def preview(payload: PreviewRequest) -> PreviewResponse | Response:
    try:
        mesh = build_primitive(payload.kind.value, _copy_params(payload.parameters))
    except ValueError as error:
        return _invalid_request_response(error)
    return _make_mesh_response(mesh)


@router.post(
    "/export",
    summary="Render a parametric primitive or a stored design to STL or STEP",
    tags=["studio"],
)
def export_primitive(payload: ExportRequest, request: Request) -> Response:
    if payload.design_id is not None:
        return _export_design(payload, request)
    if payload.kind is None:  # pragma: no cover - the request validator guarantees this
        return _invalid_request_response(ValueError("a primitive kind or a design id is required"))
    try:
        mesh = build_primitive(payload.kind.value, _copy_params(payload.parameters))
    except ValueError as error:
        return _invalid_request_response(error)
    if payload.format == ExportFormat.STL:
        data = write_binary_stl(mesh)
    else:
        try:
            data = export_step_primitive(
                payload.kind.value,
                _copy_params(payload.parameters),
                workdir=_resolve_workdir(os.environ),
            )
        except StepUnavailableError as error:
            return _step_unavailable_response(error)
        except ValueError as error:
            return _invalid_request_response(error)
    headers = {
        "content-disposition": f'attachment; filename="{_filename_for(payload.kind, payload.format)}"',
    }
    return Response(
        content=data,
        media_type=_media_type_for(payload.format),
        headers=headers,
    )


def _export_design(payload: ExportRequest, request: Request) -> Response:
    service = _design_service(request)
    if not design_available():
        return _design_unavailable_response()
    if payload.design_id is None:  # pragma: no cover - the caller checked
        return _invalid_request_response(ValueError("a design id is required"))
    try:
        outcome = service.export(payload.design_id, payload.format.value)
    except DesignNotFoundError:
        return _design_not_found_response(payload.design_id)
    except DesignUnavailableError:
        return _design_unavailable_response()
    except DesignRejectedError as error:
        return _design_rejected_response(error.reason)
    except DesignError as error:
        return _invalid_request_response(error)
    headers = {
        "content-disposition": f'attachment; filename="{outcome.filename}"',
        "x-studio-design-id": outcome.design_id,
        "x-studio-digest": outcome.sha256,
        "x-studio-artifact-kind": outcome.format,
    }
    return Response(
        content=outcome.data,
        media_type=_media_type_for(payload.format),
        headers=headers,
    )


@router.post(
    "/handoff",
    summary="Render a neutral-file handoff for one CAD target",
    tags=["studio"],
)
def handoff(payload: HandoffRequest) -> Response:
    record = integration_by_id(payload.integration_id)
    if record is None:
        return _integration_unknown_response(payload.integration_id)
    if payload.format.value not in record.capabilities:
        return _integration_format_unsupported(payload.integration_id, payload.format, record)
    try:
        mesh = build_primitive(payload.kind.value, _copy_params(payload.parameters))
    except ValueError as error:
        return _invalid_request_response(error)
    if payload.format == ExportFormat.STL:
        data = write_binary_stl(mesh)
    else:
        try:
            data = export_step_primitive(
                payload.kind.value,
                _copy_params(payload.parameters),
                workdir=_resolve_workdir(os.environ),
            )
        except StepUnavailableError as error:
            return _step_unavailable_response(error)
        except ValueError as error:
            return _invalid_request_response(error)
    metadata = HandoffResponse(
        integration_id=payload.integration_id,
        format=payload.format,
        bytes=len(data),
        filename=f"{payload.integration_id}.{payload.format.value}",
    )
    return Response(
        content=data,
        media_type=_media_type_for(payload.format),
        headers={
            "content-disposition": (f'attachment; filename="{metadata.filename}"'),
            "x-studio-handoff": (
                f"{payload.integration_id};format={payload.format.value};bytes={metadata.bytes}"
            ),
        },
    )


@router.post(
    "/chat",
    summary="Send a bounded prompt to a configured provider",
    response_model=ChatResponse,
    tags=["studio"],
)
def chat_route(payload: ChatRequest, request: Request) -> ChatResponse | Response:
    env = _effective_request_environ(request)
    try:
        outcome = chat(
            provider=payload.provider.value,
            model=payload.model,
            prompt=payload.prompt,
            allow_remote=payload.allow_remote,
            environ=env,
            cad_provider=(payload.cad_provider.value if payload.cad_provider is not None else None),
        )
    except ProviderUnavailableError as error:
        _journal_failed_design_turn(request, payload, "provider_unavailable")
        return _provider_unavailable_response(error)
    except ProviderConfigurationError as error:
        _journal_failed_design_turn(
            request, payload, getattr(error, "code", "provider_not_configured")
        )
        return _provider_configuration_response(error)
    except ProviderTimeoutError as error:
        _journal_failed_design_turn(request, payload, "provider_timeout")
        return _provider_timeout_response(error)
    except ProviderNetworkError as error:
        _journal_failed_design_turn(request, payload, "provider_network_error")
        return _provider_network_response(error)
    except ProviderProtocolError as error:
        _journal_failed_design_turn(request, payload, "provider_protocol_error")
        return _provider_protocol_response(error)
    except ProviderSubprocessError as error:
        _journal_failed_design_turn(request, payload, "provider_subprocess_error")
        return _provider_subprocess_response(error)

    service = _design_service(request)
    text = outcome.text[:4096]
    design: DesignResult | None = None
    design_id: str | None = None
    turn = outcome if isinstance(outcome, StudioChatTurn) else None
    selected_provider = (
        payload.cad_provider.value
        if payload.cad_provider is not None
        else (turn.cad_provider if turn is not None else None)
    )
    if turn is not None and turn.design_intent is not None:
        if selected_provider is None:
            text = _join_text(text, _provider_choice_message(service))
        elif not design_available():
            text = _join_text(text, DESIGN_UNAVAILABLE_MESSAGE)
        else:
            text, design, design_id = _build_and_maybe_repair(
                service=service,
                turn=turn,
                selected_provider=selected_provider,
                provider=payload.provider.value,
                model=payload.model or outcome.model or "",
                prompt=payload.prompt,
                allow_remote=payload.allow_remote,
                environ=env,
                text=text,
            )
    try:
        service.record_chat_turn(
            provider=payload.provider.value,
            model=outcome.model,
            offline=outcome.offline,
            prompt=payload.prompt,
            design_id=design_id,
            design_requested=turn is not None
            and (turn.design_intent is not None or bool(turn.design_missing)),
            missing=() if turn is None else turn.design_missing,
            cad_provider=selected_provider,
        )
    except Exception as error:  # pragma: no cover - a broken journal is a real failure
        LOGGER.error("the studio chat turn could not be journalled: %s", type(error).__name__)
        return _journal_error_response()
    return ChatResponse(
        text=text[:4096],
        provider=payload.provider,
        model=outcome.model,
        offline=outcome.offline,
        design=design,
    )


def _design_service(request: Request) -> DesignService:
    service = getattr(request.app.state, "design_service", None)
    if not isinstance(service, DesignService):
        raise RuntimeError("the studio design service is not attached to the app")
    return service


def _build_and_maybe_repair(
    *,
    service: DesignService,
    turn: StudioChatTurn,
    selected_provider: str,
    provider: str,
    model: str,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str],
    text: str,
) -> tuple[str, DesignResult | None, str | None]:
    """Build the design and, when the build says the part is wrong, repair it.

    The turn spends at most ONE repair call. The validation repair comes
    first: a reply that is not a valid DesignIntent is repaired with the exact
    validation error. When that repair was not needed, the build may spend it
    instead: a design that built unverified, or a builder that refused the
    script, is repaired with the concrete build feedback. The worst case is
    therefore two model calls.

    A design that builds and verifies needs no repair at all. A repair that
    does not build leaves the best honest state in place: the latest built
    design with ``verified`` false visible, or the "No part was built" text
    when nothing built at all.

    A repair CALL that fails is not a turn failure: the first design was built
    and journalled already, so the turn keeps it, says that the repair attempt
    failed, and answers 200.
    """
    outcome: DesignOutcome | None = None
    refusal: str | None = None
    try:
        outcome = service.design(turn.design_intent, provider=selected_provider)
    except CadProviderUnavailableError as error:
        # The provider cannot build on this host: never the model's fault.
        return _join_text(text, _provider_unavailable_chat_message(error)), None, None
    except DesignRejectedError as error:
        refusal = error.reason
        text = _join_text(text, f"The design was rejected: {error.reason}.")
    except DesignUnavailableError:
        return _join_text(text, DESIGN_UNAVAILABLE_MESSAGE), None, None
    except DesignError as error:
        refusal = str(error)
        text = _join_text(text, f"The design could not be built: {error}.")

    design: DesignResult | None = None
    design_id: str | None = None
    if outcome is not None:
        design = _design_result(outcome, service=service)
        design_id = outcome.design_id
        text = _join_text(text, _measured_summary(outcome))

    if turn.design_repair_spent:
        return text, design, design_id

    feedback = build_feedback(
        turn.design_intent,
        verified=outcome.verified if outcome is not None else False,
        script=outcome.script if outcome is not None else None,
        checks=outcome.checks if outcome is not None else (),
        warnings=outcome.warnings if outcome is not None else (),
        refusal=refusal,
    )
    if feedback is None:
        return text, design, design_id

    rejection = f"The first attempt was rejected: {feedback.reason()}."
    repaired = build_repair_turn(
        provider,
        model=model,
        prompt=prompt,
        feedback=feedback.text(),
        allow_remote=allow_remote,
        environ=environ,
        call=provider_call(
            provider,
            model=model,
            allow_remote=allow_remote,
            environ=environ,
        ),
    )
    if repaired.design_intent is None:
        reason = (
            repaired.design_repair_failure or "the repair attempt did not produce a buildable part."
        )
        return (
            _keep_first_build(text, rejection, built=outcome is not None, reason=reason),
            design,
            design_id,
        )
    try:
        second = service.design(repaired.design_intent, provider=selected_provider)
    except CadProviderUnavailableError as error:
        return (
            _join_text(text, rejection, _provider_unavailable_chat_message(error)),
            design,
            design_id,
        )
    except DesignRejectedError as error:
        reason = f"the repair attempt was refused: {error.reason}."
        return (
            _keep_first_build(text, rejection, built=outcome is not None, reason=reason),
            design,
            design_id,
        )
    except DesignUnavailableError:
        return _join_text(text, rejection, DESIGN_UNAVAILABLE_MESSAGE), design, design_id
    except DesignError as error:
        reason = f"the repair attempt could not be built: {error}."
        return (
            _keep_first_build(text, rejection, built=outcome is not None, reason=reason),
            design,
            design_id,
        )
    return (
        _join_text(rejection, repaired.text, _measured_summary(second)),
        _design_result(second, service=service),
        second.design_id,
    )


def _keep_first_build(text: str, rejection: str, *, built: bool, reason: str) -> str:
    """Keep the honest state of the first attempt when the repair did not build.

    A part that did build stays the turn's design, with its own "NOT verified"
    verdict. When nothing built at all, the turn says so in the house wording.
    """
    if built:
        return _join_text(rejection, text, reason)
    return _join_text(rejection, f"No part was built: {reason}")


def _journal_failed_design_turn(request: Request, payload: ChatRequest, code: str) -> None:
    """Journal one failed remote design turn without masking its error.

    A provider failure (timeout, network, protocol, configuration) inside a
    remote design turn still leaves one ``chat.turn`` event: the same payload
    shape as a completed turn, ``design_requested`` true, ``design_id`` null,
    plus ``failure`` naming the error envelope's code. It carries no prompt
    text, key or endpoint. A broken journal is logged and never replaces the
    provider error the route is about to answer.
    """
    if payload.provider.value not in ("openai", "openrouter"):
        return
    if payload.cad_provider is None and detect_design_family(payload.prompt) is None:
        return
    try:
        service = getattr(request.app.state, "design_service", None)
        if not isinstance(service, DesignService):
            return
        service.journal.append(
            JournalEventType.CHAT_TURN,
            {
                "provider": payload.provider.value,
                "model": payload.model,
                "offline": False,
                "prompt_sha256": hashlib.sha256(payload.prompt.encode("utf-8")).hexdigest(),
                "prompt_chars": len(payload.prompt),
                "design_id": None,
                "design_requested": True,
                "cad_provider": (
                    payload.cad_provider.value if payload.cad_provider is not None else None
                ),
                "missing": [],
                "failure": code,
            },
        )
    except Exception as error:  # pragma: no cover - the provider error must win
        LOGGER.error(
            "the failed studio chat turn could not be journalled: %s",
            type(error).__name__,
        )


def _require_bearer(request: Request) -> Response | None:
    """Require the launch token on a new GET route.

    The middleware stack already guards mutating methods; ADR-0009 criterion
    7 asks the new read routes to carry the same bearer requirement, so the
    journal history and design drawings are not readable by a same-origin
    page that never proved it holds the launch token.
    """
    expected = getattr(request.app.state, "studio_api_token", None)
    values = request.headers.getlist("authorization")
    if not isinstance(expected, str) or not expected or len(values) != 1:
        return _unauthorized_response()
    scheme, separator, credential = values[0].partition(" ")
    if not separator or scheme.lower() != "bearer" or not credential.strip():
        return _unauthorized_response()
    if not secrets.compare_digest(credential.strip().encode("utf-8"), expected.encode("utf-8")):
        return _unauthorized_response()
    return None


@router.post(
    "/design",
    summary="Compile, measure, and journal one design intent",
    response_model=DesignResult,
    tags=["studio"],
)
def design_route(payload: DesignRequest, request: Request) -> DesignResult | Response:
    if not design_available():
        return _design_unavailable_response()
    service = _design_service(request)
    provider = payload.provider.value
    try:
        outcome = service.design(payload, provider=provider)
    except CadProviderUnavailableError as error:
        return _cad_provider_unavailable_response(error)
    except DesignUnavailableError:
        return _design_unavailable_response()
    except DesignRejectedError as error:
        return _design_rejected_response(error.reason)
    except DesignError as error:
        return _invalid_request_response(error)
    return _design_result(outcome, service=service)


@router.get(
    "/design/{design_id}/drawing",
    summary="Hidden-line views of a stored design",
    response_model=DrawingResponse,
    tags=["studio"],
)
def design_drawing(
    design_id: str,
    request: Request,
    views: str | None = Query(default=None, max_length=64),
) -> DrawingResponse | Response:
    denied = _require_bearer(request)
    if denied is not None:
        return denied
    if not design_available():
        return _design_unavailable_response()
    if not _valid_design_id(design_id):
        return _invalid_request_response(ValueError("the design id is not a sha256 digest"))
    try:
        selected = _parse_views(views)
    except ValueError as error:
        return _invalid_request_response(error)
    service = _design_service(request)
    try:
        drawing = service.drawing(design_id, selected)
    except DesignNotFoundError:
        return _design_not_found_response(design_id)
    except DesignRejectedError as error:
        return _design_rejected_response(error.reason)
    except DesignUnavailableError:
        return _design_unavailable_response()
    except ValueError as error:
        return _invalid_request_response(error)
    return _drawing_response(design_id, drawing)


@router.get(
    "/design/{design_id}/section",
    summary="Section loops of a stored design",
    response_model=SectionResponse,
    tags=["studio"],
)
def design_section(
    design_id: str,
    request: Request,
    plane: str = Query(default="XZ", max_length=2),
    offset: float | None = Query(default=None),
) -> SectionResponse | Response:
    denied = _require_bearer(request)
    if denied is not None:
        return denied
    if not design_available():
        return _design_unavailable_response()
    if not _valid_design_id(design_id):
        return _invalid_request_response(ValueError("the design id is not a sha256 digest"))
    normalized_plane = plane.strip().upper()
    if normalized_plane not in ALLOWED_SECTION_PLANES:
        return _invalid_request_response(ValueError("plane must be one of XY, XZ, or YZ"))
    if offset is not None:
        if not math.isfinite(offset) or abs(offset) > MAX_SECTION_OFFSET_MM:
            return _invalid_request_response(
                ValueError(f"offset must be finite and within +/-{MAX_SECTION_OFFSET_MM:g} mm")
            )
    service = _design_service(request)
    try:
        section = service.section(design_id, plane=normalized_plane, offset_mm=offset)
    except DesignNotFoundError:
        return _design_not_found_response(design_id)
    except DesignRejectedError as error:
        return _design_rejected_response(error.reason)
    except DesignUnavailableError:
        return _design_unavailable_response()
    except ValueError as error:
        return _invalid_request_response(error)
    return _section_response(design_id, section)


@router.get(
    "/design/{design_id}/artifact/{kind}",
    summary="Download one stored artifact of a design",
    tags=["studio"],
)
def design_artifact(design_id: str, kind: str, request: Request) -> Response:
    """Serve the exact stored bytes of one artifact, digest-verified.

    The route carries the launch bearer like the other stored-design reads.
    ``kind`` is the allow-list of kinds a design can store (``script``,
    ``step``, ``fcstd``, ``stl``); a design that did not store the requested
    kind answers 404. The bytes are hashed and compared with the journalled
    digest before they are sent, so a corrupted store object is a refusal, not
    a download.
    """
    denied = _require_bearer(request)
    if denied is not None:
        return denied
    if not _valid_design_id(design_id):
        return _invalid_request_response(ValueError("the design id is not a sha256 digest"))
    if kind not in ARTIFACT_KINDS:
        return _invalid_request_response(
            ValueError("the artifact kind must be one of " + ", ".join(ARTIFACT_KINDS))
        )
    service = _design_service(request)
    try:
        artifact = service.artifact(design_id, kind)
    except DesignNotFoundError:
        return _artifact_not_found_response(design_id, kind)
    except ArtifactTooLargeError as error:
        return _artifact_too_large_response(str(error))
    except ArtifactDigestMismatchError as error:
        return _artifact_digest_mismatch_response(str(error))
    except DesignUnavailableError:
        return _design_unavailable_response()
    except ValueError as error:
        return _invalid_request_response(error)
    headers = {
        "content-disposition": f'attachment; filename="{artifact.filename}"',
        "x-studio-design-id": artifact.design_id,
        "x-studio-digest": artifact.sha256,
        "x-studio-artifact-kind": artifact.kind,
    }
    return Response(
        content=artifact.data,
        media_type=artifact.content_type,
        headers=headers,
    )


@router.get(
    "/journal",
    summary="A bounded page of Studio journal events",
    response_model=JournalPageResponse,
    tags=["studio"],
)
def journal_route(
    request: Request,
    after: int = Query(default=0, ge=0),
    limit: int = Query(default=DEFAULT_DRAWING_LIMIT, ge=1, le=MAX_JOURNAL_LIMIT),
) -> JournalPageResponse | Response:
    denied = _require_bearer(request)
    if denied is not None:
        return denied
    service = _design_service(request)
    effective_limit = min(limit, MAX_JOURNAL_LIMIT)
    events = service.journal_page(after, effective_limit)
    next_after = events[-1].sequence if events else after
    has_more = bool(events) and bool(service.journal_page(next_after, 1))
    return JournalPageResponse(
        events=tuple(_journal_event(event) for event in events),
        next_after=next_after,
        has_more=has_more,
        limit=effective_limit,
    )


@router.get(
    "/journal/verify",
    summary="Verify the Studio journal hash chain",
    response_model=JournalVerifyResponse,
    tags=["studio"],
)
def journal_verify_route(request: Request) -> JournalVerifyResponse | Response:
    denied = _require_bearer(request)
    if denied is not None:
        return denied
    service = _design_service(request)
    verification = service.verify()
    return JournalVerifyResponse(
        ok=verification.ok,
        event_count=verification.event_count,
        head_digest=verification.head_digest,
        first_bad_sequence=verification.first_bad_sequence,
    )


def _parse_views(views: str | None) -> tuple[str, ...]:
    if views is None or not views.strip():
        return ALLOWED_VIEWS
    selected: list[str] = []
    for piece in views.split(","):
        name = piece.strip().lower()
        if not name:
            raise ValueError("views must not contain an empty name")
        if name not in ALLOWED_VIEWS:
            raise ValueError("views must be a subset of front, top, right, iso")
        if name in selected:
            raise ValueError("views must not repeat a name")
        selected.append(name)
    if not selected or len(selected) > MAX_DRAWING_VIEWS:
        raise ValueError(f"views must select between 1 and {MAX_DRAWING_VIEWS} names")
    return tuple(selected)


def _valid_design_id(value: str) -> bool:
    return (
        isinstance(value, str)
        and len(value) == DESIGN_ID_PATTERN_LENGTH
        and all(character in "0123456789abcdef" for character in value)
    )


def _design_result(outcome: DesignOutcome, *, service: DesignService) -> DesignResult:
    del service
    measurement = outcome.measurement
    bounds = measurement.bounds
    return DesignResult(
        design_id=outcome.design_id,
        label=outcome.label[:120],
        provider=outcome.provider,
        provider_version=outcome.provider_version,
        measured_by=outcome.measured_by,
        script=outcome.script,
        measured=DesignMeasured(
            volume_mm3=measurement.volume_mm3,
            surface_area_mm2=measurement.area_mm2,
            centroid_mm=measurement.centroid_mm,
            bounds=None if bounds is None else _design_bounds(bounds),
            size_mm=None if bounds is None else bounds.size_mm,
            valid=outcome.valid,
            solid_count=outcome.solid_count,
            mass_g=measurement.mass_g,
        ),
        checks=tuple(DesignCheck(**check_payload(check)) for check in outcome.checks),
        verified=outcome.verified,
        view=DesignView(
            positions=outcome.view.positions,
            normals=outcome.view.normals,
            indices=outcome.view.indices,
            vertex_count=outcome.view.vertex_count,
            triangle_count=outcome.view.triangle_count,
            deflection_mm=outcome.view.deflection_mm,
            engine="ocp",
            bounds=_design_bounds(outcome.view_bounds),
            solids=outcome.view_solids,
            warnings=outcome.view_warnings,
        ),
        artifacts=DesignArtifacts(
            script_sha256=outcome.script_sha256,
            step_sha256=outcome.step_sha256,
            native_sha256=outcome.native_sha256,
            native_kind=outcome.native_kind,
        ),
        journal_head=outcome.journal_head,
        material=outcome.material,
        tolerances=outcome.tolerances,
        declared_envelope=None
        if outcome.declared_envelope is None
        else _design_bounds(outcome.declared_envelope),
        cached=outcome.cached,
        warnings=outcome.warnings,
        component_count=outcome.component_count,
        fused=outcome.fused,
    )


def _design_bounds(bounds: DesignMeasureBounds | MeasuredBounds) -> DesignBounds:
    return DesignBounds(
        min=(float(bounds.min_mm[0]), float(bounds.min_mm[1]), float(bounds.min_mm[2])),
        max=(float(bounds.max_mm[0]), float(bounds.max_mm[1]), float(bounds.max_mm[2])),
    )


def _drawing_response(design_id: str, drawing: Drawing) -> DrawingResponse:
    return DrawingResponse(
        design_id=design_id,
        views=tuple(
            DrawingView(
                name=view.name,
                visible=tuple(_drawing_polyline(polyline) for polyline in view.visible),
                hidden=tuple(_drawing_polyline(polyline) for polyline in view.hidden),
                truncated=view.truncated,
                point_count=view.point_count,
                polyline_count=view.polyline_count,
            )
            for view in drawing.views
        ),
        chord_tolerance_mm=drawing.chord_tolerance_mm,
        max_points=drawing.max_points,
        point_count=drawing.total_point_count,
    )


def _drawing_polyline(polyline: Polyline) -> DrawingPolyline:
    return DrawingPolyline(
        points=tuple((float(point[0]), float(point[1])) for point in polyline.points),
        closed=bool(polyline.closed),
        point_count=int(polyline.point_count),
    )


def _section_response(design_id: str, section: Section) -> SectionResponse:
    loop_responses: list[SectionLoopResponse] = []
    for loop in section.loops:
        loop_bounds = loop.bounds
        loop_responses.append(
            SectionLoopResponse(
                points=tuple((float(point[0]), float(point[1])) for point in loop.points),
                area_mm2=loop.area_mm2,
                area_exact=loop.area_exact,
                outer=loop.outer,
                bounds=SectionBounds2D(
                    min_u=loop_bounds.min_u,
                    min_v=loop_bounds.min_v,
                    max_u=loop_bounds.max_u,
                    max_v=loop_bounds.max_v,
                    width=loop_bounds.width,
                    height=loop_bounds.height,
                    center_u=loop_bounds.center_u,
                    center_v=loop_bounds.center_v,
                ),
                centroid=(float(loop.centroid[0]), float(loop.centroid[1])),
                point_count=loop.point_count,
            )
        )
    return SectionResponse(
        design_id=design_id,
        plane=section.plane,
        offset_mm=section.offset_mm,
        loops=tuple(loop_responses),
        area_mm2=section.area_mm2,
        area_exact=section.area_exact,
        chord_tolerance_mm=section.chord_tolerance_mm,
        truncated=section.truncated,
        outer_loop_count=len(section.outer_loops),
        inner_loop_count=len(section.inner_loops),
        point_count=section.point_count,
    )


def _journal_event(event: SessionEvent) -> JournalEventRecord:
    payload = None if event.payload is None else dict(event.payload)
    return JournalEventRecord(
        sequence=event.sequence,
        event_type=event.event_type,
        aggregate_id=event.aggregate_id,
        payload=payload,
        digest=event.digest,
        occurred_at=event.occurred_at,
        previous_event_digest=event.previous_event_digest,
        event_digest=event.event_digest,
    )


def _measured_summary(outcome: DesignOutcome) -> str:
    measurement = outcome.measurement
    verdict = "B-Rep verified" if outcome.verified else "B-Rep NOT verified"
    if measurement.volume_mm3 is None or measurement.area_mm2 is None or measurement.bounds is None:
        return (
            f"{outcome.label}: {verdict}; measured by {outcome.measured_by}. "
            "The provider did not report volume, surface area, or envelope; "
            f"no number is substituted. design_id {outcome.design_id}."
        )
    size = measurement.bounds.size_mm
    return (
        f"{outcome.label}: {verdict}; measured by {outcome.measured_by}. "
        f"Measured volume {measurement.volume_mm3:.3f} mm^3, "
        f"surface area {measurement.area_mm2:.3f} mm^2, "
        f"envelope {size[0]:.3f} x {size[1]:.3f} x {size[2]:.3f} mm. "
        f"design_id {outcome.design_id}."
    )


def _provider_choice_message(service: DesignService) -> str:
    statuses = service.probe_providers()
    options = ", ".join(f"{status.id} ({status.mode})" for status in statuses)
    available = ", ".join(status.id for status in statuses if status.available)
    return (
        f"Choose a CAD provider before I build this design: {options}. "
        f"Available on this host: {available or 'none'}. "
        "No design was built."
    )


def _provider_unavailable_chat_message(error: CadProviderUnavailableError) -> str:
    available = ", ".join(error.available) if error.available else "none"
    return (
        f"The {error.provider} CAD provider is not available: {error.reason}. "
        f"Available: {available}. No design was built."
    )


def _join_text(*parts: str) -> str:
    return " ".join(part.strip() for part in parts if part and part.strip())[:4096]


def _copy_params(params: dict[str, object] | None) -> dict[str, object]:
    if not params:
        return {}
    return dict(params)


def _step_unavailable_response(error: StepUnavailableError) -> Response:
    body = error_response(
        "step_unavailable",
        STEP_UNAVAILABLE_MESSAGE,
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(503, body)


def _cad_provider_unavailable_response(error: CadProviderUnavailableError) -> Response:
    details = [str(error)[:256]]
    if error.reason and error.reason != str(error):
        details.append(f"reason: {error.reason}"[:256])
    available = ", ".join(error.available) if error.available else "none"
    details.append(f"available: {available}"[:256])
    body = error_response(
        "provider_unavailable",
        f"the {error.provider} CAD provider is not available",
        tuple(details),
    ).model_dump(mode="json")
    return _json_error(503, body)


def _design_unavailable_response() -> Response:
    body = error_response(
        "design_unavailable",
        DESIGN_UNAVAILABLE_MESSAGE,
    ).model_dump(mode="json")
    return _json_error(503, body)


def _design_rejected_response(reason: str) -> Response:
    body = error_response(
        "design_rejected",
        "the design could not be compiled or measured",
        (reason[:256],),
    ).model_dump(mode="json")
    return _json_error(422, body)


def _design_not_found_response(design_id: str) -> Response:
    body = error_response(
        "design_not_found",
        "no design is stored under that id",
        (f"design_id={design_id[:64]}",),
    ).model_dump(mode="json")
    return _json_error(404, body)


def _artifact_not_found_response(design_id: str, kind: str) -> Response:
    body = error_response(
        "artifact_not_found",
        "no artifact of that kind is stored for this design",
        (f"design_id={design_id[:64]}; kind={kind[:16]}",),
    ).model_dump(mode="json")
    return _json_error(404, body)


def _artifact_too_large_response(reason: str) -> Response:
    body = error_response(
        "artifact_too_large",
        "the stored artifact exceeds the download limit",
        (reason[:256],),
    ).model_dump(mode="json")
    return _json_error(413, body)


def _artifact_digest_mismatch_response(reason: str) -> Response:
    body = error_response(
        "artifact_digest_mismatch",
        "the stored artifact does not match its journalled digest",
        (reason[:256],),
    ).model_dump(mode="json")
    return _json_error(500, body)


def _journal_error_response() -> Response:
    body = error_response(
        "journal_unavailable",
        "the Studio journal could not record this turn",
    ).model_dump(mode="json")
    return _json_error(500, body)


def _unauthorized_response() -> Response:
    body = error_response(
        "unauthorized",
        TOKEN_REQUIRED_MESSAGE,
    ).model_dump(mode="json")
    return _json_error(401, body, headers={"www-authenticate": "Bearer"})


def _invalid_request_response(error: ValueError) -> Response:
    body = error_response(
        "invalid_request",
        "the request body or parameters are invalid",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(422, body)


def _integration_unknown_response(integration_id: str) -> Response:
    body = error_response(
        "unknown_integration",
        "the requested integration is not in the registry",
        (f"unknown integration: {integration_id[:128]}",),
    ).model_dump(mode="json")
    return _json_error(404, body)


def _integration_format_unsupported(
    integration_id: str,
    fmt: ExportFormat,
    record: IntegrationRecord,
) -> Response:
    body = error_response(
        "handoff_format_unsupported",
        ("the integration does not list this format as a supported neutral-file capability"),
        (
            f"integration={integration_id[:64]}; "
            f"requested_format={fmt.value}; "
            f"supported={','.join(record.capabilities)}",
        ),
    ).model_dump(mode="json")
    return _json_error(409, body)


def _provider_configuration_response(error: ProviderConfigurationError) -> Response:
    code = getattr(error, "code", "provider_not_configured")
    if isinstance(error, ProviderMissingCredentialError):
        code = "missing_api_key"
    body = error_response(
        code,
        "the chat provider is not configured for this request",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(400, body)


def _provider_unavailable_response(error: ProviderUnavailableError) -> Response:
    """Return a 503 for a documented provider this build refuses to run."""
    body = error_response(
        "provider_unavailable",
        "the chat provider is not available in this studio build",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(503, body)


def _provider_timeout_response(error: ProviderTimeoutError) -> Response:
    body = error_response(
        "provider_timeout",
        "the chat provider exceeded its bounded timeout",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(504, body)


def _provider_network_response(error: ProviderNetworkError) -> Response:
    body = error_response(
        "provider_network_error",
        "the studio could not reach the configured provider",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(502, body)


def _provider_protocol_response(error: ProviderProtocolError) -> Response:
    body = error_response(
        "provider_protocol_error",
        "the provider returned an unparseable payload",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(502, body)


def _provider_subprocess_response(error: ProviderSubprocessError) -> Response:
    body = error_response(
        "provider_subprocess_error",
        "the chat provider subprocess failed",
        (str(error)[:256],),
    ).model_dump(mode="json")
    return _json_error(502, body)


def _json_error(
    status_code: int,
    body: dict[str, object],
    *,
    headers: Mapping[str, str] | None = None,
) -> Response:
    import json

    serialized = json.dumps(body, separators=(",", ":"), sort_keys=True).encode("utf-8")
    response_headers = {"content-type": "application/json"}
    if headers:
        response_headers.update(headers)
    return Response(content=serialized, status_code=status_code, headers=response_headers)


LifespanFactory = Callable[[FastAPI], AbstractAsyncContextManager[None]]


def build_lifespan(
    api_token: str,
    *,
    data_dir: Path,
    environ: Mapping[str, str],
    design_service: DesignService | None = None,
) -> LifespanFactory:
    """Return a lifespan that mints the studio token at startup.

    The token is owned by the studio: the lifespan mints the file at
    ``data_dir / STUDIO_TOKEN_FILE_NAME`` and the secret never reaches an
    HTTP response. The lifespan returns successfully even when the file
    write fails (the in-memory token still guards the running process).
    """

    @asynccontextmanager
    async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
        try:
            yield
        finally:
            _ = api_token
            _ = data_dir
            _ = environ
            if design_service is not None:
                design_service.close()

    return lifespan


def build_middleware_stack(
    *,
    api_token: str,
    max_body_bytes: int,
    static_root: Path | None,
) -> list[Middleware]:
    """Return the studio middleware stack, outermost first.

    The order is the policy: security headers wrap everything so a refusal
    is as complete as a success; the Host guard runs before any body is
    buffered; the size limit answers every method without consulting the
    token; the same-origin guard refuses a cross-origin browser request
    before the router runs; the static-file middleware serves only safe
    GET / HEAD paths and reserves the ``/api/*`` prefix; the bearer token
    is the last guard so a mutating method is rejected before the router.
    """
    return [
        Middleware(SecurityHeadersMiddleware),
        Middleware(HostGuardMiddleware),
        Middleware(MaxBodySizeMiddleware, max_body_bytes=max_body_bytes),
        Middleware(SameOriginGuardMiddleware),
        Middleware(BearerTokenMiddleware, api_token=api_token),
        Middleware(StaticFileMiddleware, root=static_root),
    ]


def create_studio_app(
    *,
    data_dir: Path | None = None,
    api_token: str | None = None,
    max_body_bytes: int = 256 * 1024,
    static_root: Path | None = None,
    environ: Mapping[str, str] | None = None,
    design_providers: Mapping[str, CadProvider] | None = None,
    max_step_bytes: int | None = None,
) -> FastAPI:
    """Build a standalone FastAPI instance for the studio.

    ``design_providers`` and ``max_step_bytes`` are the embedding/test seam for
    the CAD provider registry; production uses the explicit default registry
    and the ``KALA_STEP_IMPORT_MAX_BYTES`` setting.
    """
    source = environ if environ is not None else os.environ
    if data_dir is None:
        from kala_agent.studio.settings import resolve_studio_data_dir_from_env

        data_dir = resolve_studio_data_dir_from_env(source)
    data_dir.mkdir(parents=True, exist_ok=True)
    token = resolve_studio_token(data_dir, environ=source) if api_token is None else api_token
    limit = validate_max_body_bytes(max_body_bytes)
    resolved_static = static_root
    if resolved_static is None:
        resolved_static = resolve_static_root(source)
    design_service = DesignService(
        data_dir,
        providers=design_providers,
        max_step_bytes=max_step_bytes,
        environ=source,
    )
    app = FastAPI(
        title="Kala Studio API",
        version=STUDIO_VERSION,
        description="Standalone studio HTTP surface (loopback only).",
        lifespan=build_lifespan(
            token,
            data_dir=data_dir,
            environ=source,
            design_service=design_service,
        ),
        middleware=build_middleware_stack(
            api_token=token,
            max_body_bytes=limit,
            static_root=resolved_static,
        ),
        docs_url=None,
        redoc_url=None,
    )
    app.state.studio_data_dir = data_dir
    app.state.studio_api_token = token
    app.state.design_service = design_service
    app.state.studio_use_custom_environ = environ is not None
    if environ is not None:
        app.state.studio_environ = environ
    app.include_router(router, prefix=STUDIO_PREFIX)
    install_error_handlers(app)
    return app


__all__ = (
    "DESIGN_UNAVAILABLE_MESSAGE",
    "LOGGER",
    "STUDIO_PREFIX",
    "STUDIO_VERSION",
    "build_lifespan",
    "build_middleware_stack",
    "cad_providers",
    "create_studio_app",
    "design_drawing",
    "design_route",
    "design_section",
    "get_settings",
    "health",
    "integrations",
    "journal_route",
    "journal_verify_route",
    "providers",
    "update_settings",
)
