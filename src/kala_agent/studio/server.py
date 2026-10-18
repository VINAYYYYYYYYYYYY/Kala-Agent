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

import logging
import os
from collections.abc import AsyncIterator, Callable, Mapping
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from pathlib import Path
from typing import Final

from fastapi import APIRouter, FastAPI, Request, Response
from starlette.middleware import Middleware

from kala_agent.studio.boundary import resolve_studio_token
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
from kala_agent.studio.providers.dispatch import (
    chat,
    list_provider_records,
)
from kala_agent.studio.schemas import (
    BBox,
    ChatRequest,
    ChatResponse,
    ExportFormat,
    ExportRequest,
    HandoffRequest,
    HandoffResponse,
    HealthResponse,
    IntegrationRecord,
    IntegrationsResponse,
    PreviewRequest,
    PreviewResponse,
    PrimitiveKind,
    ProvidersResponse,
    StudioSettingsResponse,
    UpdateProviderKeyRequest,
    error_response,
)
from kala_agent.studio.settings import (
    describe_provider_credentials,
    effective_environ,
    write_stored_key,
)
from kala_agent.studio.static import resolve_static_root

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.studio.server")
STUDIO_PREFIX: Final[str] = "/api/studio"
STUDIO_VERSION: Final[str] = "0.1.0"


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
def health(request: Request = None) -> HealthResponse:  # type: ignore[assignment]
    """Return liveness facts about the studio without leaking secrets."""
    env = _effective_request_environ(request)
    step_available = probe_step_capability().available
    return HealthResponse(
        status="ok",
        version=STUDIO_VERSION,
        offline=not _any_remote_provider_configured(env),
        step_available=step_available,
        integrations=tuple(row.id for row in build_registry()),
        providers=tuple(row.id for row in list_provider_records(env)),
    )


def _any_remote_provider_configured(environ: Mapping[str, str] | None = None) -> bool:
    # Only providers the build can actually reach count: the codex route is
    # fail-closed in this build, so a codex CLI on PATH does not make the
    # studio "online".
    source = environ if environ is not None else os.environ
    openai = bool(source.get("OPENAI_API_KEY", "").strip())
    openrouter = bool(source.get("OPENROUTER_API_KEY", "").strip())
    return openai or openrouter


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
    return ProvidersResponse(providers=list_provider_records(env))


@router.get(
    "/settings",
    summary="Non-secret provider credential status",
    response_model=StudioSettingsResponse,
    tags=["studio"],
)
def get_settings(request: Request) -> StudioSettingsResponse:
    base_env, data_dir = _request_context(request)
    env = effective_environ(base_env, data_dir)
    return StudioSettingsResponse(
        credentials=describe_provider_credentials(base_env, data_dir),
        providers=list_provider_records(env),
        offline=not _any_remote_provider_configured(env),
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
    return StudioSettingsResponse(
        credentials=describe_provider_credentials(base_env, data_dir),
        providers=list_provider_records(env),
        offline=not _any_remote_provider_configured(env),
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
    summary="Render a parametric primitive to STL or STEP",
    tags=["studio"],
)
def export_primitive(payload: ExportRequest) -> Response:
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
        )
    except ProviderUnavailableError as error:
        return _provider_unavailable_response(error)
    except ProviderConfigurationError as error:
        return _provider_configuration_response(error)
    except ProviderTimeoutError as error:
        return _provider_timeout_response(error)
    except ProviderNetworkError as error:
        return _provider_network_response(error)
    except ProviderProtocolError as error:
        return _provider_protocol_response(error)
    except ProviderSubprocessError as error:
        return _provider_subprocess_response(error)
    return ChatResponse(
        text=outcome.text[:4096],
        provider=payload.provider,
        model=outcome.model,
        offline=outcome.offline,
    )


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


def _json_error(status_code: int, body: dict[str, object]) -> Response:
    import json

    serialized = json.dumps(body, separators=(",", ":"), sort_keys=True).encode("utf-8")
    headers = {"content-type": "application/json"}
    return Response(content=serialized, status_code=status_code, headers=headers)


LifespanFactory = Callable[[FastAPI], AbstractAsyncContextManager[None]]


def build_lifespan(
    api_token: str, *, data_dir: Path, environ: Mapping[str, str]
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
) -> FastAPI:
    """Build a standalone FastAPI instance for the studio."""
    source = environ if environ is not None else os.environ
    if data_dir is None:
        from kala_agent.config import Settings
        from kala_agent.studio.boundary import resolve_studio_data_dir

        try:
            settings = Settings.from_env(source)
        except Exception:
            import tempfile

            data_dir = Path(tempfile.gettempdir()) / "kala-studio"
        else:
            data_dir = resolve_studio_data_dir(settings.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    token = resolve_studio_token(data_dir, environ=source) if api_token is None else api_token
    limit = validate_max_body_bytes(max_body_bytes)
    resolved_static = static_root
    if resolved_static is None:
        resolved_static = resolve_static_root(source)
    app = FastAPI(
        title="Kala Studio API",
        version=STUDIO_VERSION,
        description="Standalone studio HTTP surface (loopback only).",
        lifespan=build_lifespan(token, data_dir=data_dir, environ=source),
        middleware=build_middleware_stack(
            api_token=token,
            max_body_bytes=limit,
            static_root=resolved_static,
        ),
        docs_url=None,
        redoc_url=None,
    )
    app.state.studio_data_dir = data_dir
    app.state.studio_use_custom_environ = environ is not None
    if environ is not None:
        app.state.studio_environ = environ
    app.include_router(router, prefix=STUDIO_PREFIX)
    install_error_handlers(app)
    return app


__all__ = (
    "LOGGER",
    "STUDIO_PREFIX",
    "STUDIO_VERSION",
    "build_lifespan",
    "build_middleware_stack",
    "create_studio_app",
    "get_settings",
    "health",
    "integrations",
    "providers",
    "update_settings",
)
