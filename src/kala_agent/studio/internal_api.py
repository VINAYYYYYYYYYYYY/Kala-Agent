"""The geometry worker's internal HTTP surface (ADR-0010).

The worker is the Python Studio process started in ``--worker`` mode. It is
never browser-facing: it binds an ephemeral loopback port, accepts exactly one
authority (``127.0.0.1:<port>``), refuses every request that carries an
``Origin`` header, and requires the per-launch ``KALA_WORKER_TOKEN`` bearer on
every method, including the read-only routes. The body limit is the Studio's
own limit.

The routes are the ADR-0010 worker list under ``/internal/v1``, plus the
view-only STEP import added by ADR-0011. They are not a second implementation:
each existing Studio handler from :mod:`kala_agent.studio.server` is mounted
here unchanged, so the public ``/api/studio/*`` behaviour and the worker
behaviour cannot drift. The worker-specific routes are:

- ``import`` reads a STEP file built by an external provider and tessellates
  it for display (view-only; the model is never modified or re-exported).
- ``intent/parse`` maps a prompt through the local strict parser.
- ``intent/validate`` validates a model reply's fenced JSON with the same
  :class:`~kala_agent.studio.design.intent.DesignIntent` model the HTTP route
  uses.
- ``journal/append`` lets Rust append its own events to the Studio journal
  under a fixed allow-list. Design events stay worker-owned.

The ``design``, ``drawing``, and ``section`` routes are the opt-in
OpenCascade builder (``provider: "opencascade"``). Their behaviour is the
ADR-0009 behaviour; only the route metadata labels the provider as opt-in.

The ``providers/cad`` route is the ADR-0011 provider probe. It is the public
server's own handler, mounted unchanged, so the worker and the public server
report the same live availability facts and the front door can proxy the
public route without a second implementation.

Worker mode exposes no ``/api/studio/*`` route, no settings, and no chat
provider route, so it can never make a chat provider network call. The one
provider route it exposes is ``providers/cad``: it probes the *local* CAD
providers only (an OpenCascade import check, a bounded ``FreeCADCmd
--version`` subprocess and the ADR-0008 loopback bridge health probe), and it
never leaves the machine for a hosted model.
"""

from __future__ import annotations

import json
import os
from asyncio import to_thread
from collections.abc import Mapping
from pathlib import Path
from typing import Annotated, Final

from fastapi import APIRouter, FastAPI, Request, Response
from pydantic import Field
from starlette.middleware import Middleware
from starlette.types import ASGIApp, Message, Receive, Scope, Send

from kala_agent.persistence.chain import JournalIntegrityError
from kala_agent.studio.boundary import (
    HOST_REFUSED_MESSAGE,
    TOKEN_REQUIRED_MESSAGE,
    validate_studio_token,
)
from kala_agent.studio.design.importer import (
    DEFAULT_MAX_STEP_BYTES,
    StepImportError,
    StepMalformedError,
    StepTooLargeError,
    import_step_bytes,
    resolve_step_import_max_bytes,
    validate_step_import_max_bytes,
)
from kala_agent.studio.design.intent import (
    DesignIntent,
    DesignIntentParseError,
    design_intent_schema_text,
    parse_design_intent_text,
)
from kala_agent.studio.design.service import DesignService, design_available
from kala_agent.studio.errors import install_error_handlers
from kala_agent.studio.middleware import (
    MAX_REQUEST_BODY_BYTES,
    MaxBodySizeMiddleware,
    SecurityHeadersMiddleware,
    _buffer_body,
    _declared_length,
    _presented_credential,
    _send_refusal,
    validate_max_body_bytes,
)
from kala_agent.studio.providers.local import parse_design_prompt
from kala_agent.studio.schemas import (
    MAX_PROMPT_LENGTH,
    BBox,
    CadProvidersResponse,
    DesignId,
    DesignResult,
    DrawingResponse,
    JournalEventRecord,
    JournalPageResponse,
    JournalVerifyResponse,
    PreviewResponse,
    SectionResponse,
    _RequestModel,
    _StrictModel,
    error_response,
)
from kala_agent.studio.server import (
    STUDIO_VERSION,
    _design_service,
    _design_unavailable_response,
    _journal_error_response,
    _journal_event,
    _json_error,
    build_lifespan,
    cad_providers,
    design_artifact,
    design_drawing,
    design_route,
    design_section,
    export_primitive,
    handoff,
    journal_route,
    journal_verify_route,
    preview,
)
from kala_agent.studio.session_journal import (
    DEFAULT_AGGREGATE_ID,
    DEFAULT_MAX_PAYLOAD_BYTES,
)

INTERNAL_PREFIX: Final[str] = "/internal/v1"
WORKER_HOST: Final[str] = "127.0.0.1"
WORKER_TOKEN_ENV_VAR: Final[str] = "KALA_WORKER_TOKEN"
WORKER_TOKEN_MIN_LENGTH: Final[int] = 32
WORKER_STDIN_LIFELINE_ENV_VAR: Final[str] = "KALA_WORKER_STDIN_LIFELINE"
WORKER_READY_PREFIX: Final[str] = "KALA_WORKER_READY"
MAX_INTENT_TEXT_LENGTH: Final[int] = 65_536
MAX_APPEND_PAYLOAD_BYTES: Final[int] = min(DEFAULT_MAX_PAYLOAD_BYTES, 64 * 1024)
ALLOWED_APPEND_EVENT_TYPES: Final[frozenset[str]] = frozenset(
    {"chat.turn", "settings.changed", "provider.call", "export.completed"}
)
WORKER_BEARER_REALM: Final[bytes] = b'Bearer realm="kala-studio-worker"'
BUILDER_PROVIDER: Final[str] = "opencascade"
BUILDER_PROVIDER_MODE: Final[str] = "opt-in"
BUILDER_ROUTE_DESCRIPTION: Final[str] = (
    'POST /design requires a "provider". "opencascade" is the opt-in Kala '
    "builder: it is never selected automatically and runs only when the caller "
    'explicitly asks for it. "freecad" builds in a real FreeCADCmd subprocess '
    'and "solidedge" uses the Windows ADR-0008 bridge; both are viewed through '
    "the view-only OpenCascade importer."
)
IMPORT_ROUTE_DESCRIPTION: Final[str] = (
    "View-only import of a STEP file built by an external CAD provider "
    '(Solid Edge or FreeCAD). Provider "opencascade" reads and tessellates the '
    "imported shape for display; the model is never modified or re-exported. "
    "This route has its own large STEP limit (64 MiB by default, "
    "KALA_STEP_IMPORT_MAX_BYTES); every other route keeps the generic "
    "Studio limit."
)
CAD_PROVIDERS_ROUTE_DESCRIPTION: Final[str] = (
    "The live availability of every CAD provider, probed on this host and "
    "never assumed. This is the public server's own handler: the worker and "
    "the public server report the same facts, so the front door can proxy "
    "GET /api/studio/providers/cad without a second implementation. The "
    "worker probes only the local providers (an OpenCascade import check, a "
    "bounded FreeCADCmd --version subprocess and the ADR-0008 loopback "
    "bridge health probe); it never calls a chat provider."
)

__all__ = (
    "ALLOWED_APPEND_EVENT_TYPES",
    "BUILDER_PROVIDER",
    "BUILDER_PROVIDER_MODE",
    "BUILDER_ROUTE_DESCRIPTION",
    "CAD_PROVIDERS_ROUTE_DESCRIPTION",
    "IMPORT_ROUTE_DESCRIPTION",
    "INTERNAL_PREFIX",
    "MAX_APPEND_PAYLOAD_BYTES",
    "MAX_INTENT_TEXT_LENGTH",
    "WORKER_HOST",
    "WORKER_READY_PREFIX",
    "WORKER_STDIN_LIFELINE_ENV_VAR",
    "WORKER_TOKEN_ENV_VAR",
    "WORKER_TOKEN_MIN_LENGTH",
    "IntentParseRequest",
    "IntentParseResponse",
    "IntentValidateRequest",
    "IntentValidateResponse",
    "JournalAppendRequest",
    "StepImportResponse",
    "StepImportView",
    "WorkerBearerMiddleware",
    "WorkerBodyLimitMiddleware",
    "WorkerHostGuardMiddleware",
    "WorkerOriginGuardMiddleware",
    "build_worker_middleware",
    "create_worker_app",
    "router",
    "step_import",
    "validate_worker_token",
)


def validate_worker_token(value: object) -> str:
    """Return the worker bearer token, or refuse a missing or short one."""
    if not isinstance(value, str):
        raise ValueError(
            f"{WORKER_TOKEN_ENV_VAR} must be set to a string of at least "
            f"{WORKER_TOKEN_MIN_LENGTH} characters"
        )
    token = value.strip()
    if len(token) < WORKER_TOKEN_MIN_LENGTH:
        raise ValueError(
            f"{WORKER_TOKEN_ENV_VAR} must be at least {WORKER_TOKEN_MIN_LENGTH} characters"
        )
    return validate_studio_token(token)


class WorkerHostGuardMiddleware:
    """Refuse a request whose Host is not exactly ``127.0.0.1:<port>``."""

    def __init__(self, app: ASGIApp, *, port: int) -> None:
        self._app = app
        self._expected = f"{WORKER_HOST}:{port}"

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        hosts = _header_values(scope, b"host")
        if len(hosts) != 1 or hosts[0].strip() != self._expected:
            await _send_refusal(
                send,
                status_code=421,
                code="invalid_host",
                message=HOST_REFUSED_MESSAGE,
            )
            return
        await self._app(scope, receive, send)


class WorkerOriginGuardMiddleware:
    """Refuse any request that carries an ``Origin`` header."""

    def __init__(self, app: ASGIApp) -> None:
        self._app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        if _header_values(scope, b"origin"):
            await _send_refusal(
                send,
                status_code=403,
                code="cross_origin_denied",
                message="the geometry worker is not browser-facing and refuses every Origin",
            )
            return
        await self._app(scope, receive, send)


class WorkerBearerMiddleware:
    """Require the worker token on every request, including GET routes."""

    def __init__(self, app: ASGIApp, *, api_token: str) -> None:
        self._app = app
        self._expected = validate_worker_token(api_token).encode("utf-8")

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        presented = _presented_credential(scope)
        if presented is None or not _tokens_equal(presented, self._expected):
            await _send_refusal(
                send,
                status_code=401,
                code="unauthorized",
                message=TOKEN_REQUIRED_MESSAGE,
                extra_headers=((b"www-authenticate", WORKER_BEARER_REALM),),
            )
            return
        await self._app(scope, receive, send)


def _tokens_equal(presented: bytes, expected: bytes) -> bool:
    import secrets

    return secrets.compare_digest(presented, expected)


def _header_values(scope: Scope, name: bytes) -> tuple[str, ...]:
    found: list[str] = []
    for key, value in scope.get("headers", ()):
        if key != name:
            continue
        try:
            found.append(value.decode("latin-1"))
        except UnicodeDecodeError:
            return ()
    return tuple(found)


class IntentParseRequest(_RequestModel):
    """One prompt for the local, deterministic design parser."""

    prompt: Annotated[str, Field(min_length=1, max_length=MAX_PROMPT_LENGTH)]


class IntentParseResponse(_StrictModel):
    """The strict reading of a prompt.

    ``family`` is ``None`` when the prompt does not ask for a design at all.
    ``intent`` is present only when every required value was stated (or is a
    documented template default); otherwise ``missing`` names exactly what the
    caller has to ask the operator for.
    """

    family: Annotated[str | None, Field(default=None, max_length=64)] = None
    intent: DesignIntent | None = None
    missing: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...] = ()
    notes: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...] = ()


class IntentValidateRequest(_RequestModel):
    """One model reply that may carry a fenced ``DesignIntent`` JSON block."""

    text: Annotated[str, Field(min_length=1, max_length=MAX_INTENT_TEXT_LENGTH)]


class IntentValidateResponse(_StrictModel):
    """The validated intent parsed out of a model reply."""

    intent: DesignIntent


class JournalAppendRequest(_RequestModel):
    """One Rust-originated journal event.

    Only the allow-listed event types are accepted; the payload is a JSON
    object bounded both by the request body limit and by
    :data:`MAX_APPEND_PAYLOAD_BYTES`.
    """

    event_type: Annotated[str, Field(min_length=1, max_length=64)]
    payload: dict[str, object]
    aggregate_id: Annotated[str, Field(min_length=1, max_length=64)] = DEFAULT_AGGREGATE_ID


class StepImportView(_StrictModel):
    """The display tessellation of an imported STEP shape."""

    positions: tuple[float, ...]
    normals: tuple[float, ...]
    indices: tuple[int, ...]
    bounds: BBox


class StepImportResponse(_StrictModel):
    """A view-only STEP import, never a measured or rebuilt model.

    ``step_sha256`` is the digest of the exact bytes the external provider
    exported and the key they were stored under. ``solids`` is a structural
    count of the imported shape, not a mass-properties measurement.
    """

    view: StepImportView
    step_sha256: DesignId
    solids: Annotated[int, Field(ge=0)]
    warnings: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...] = ()


async def step_import(request: Request) -> StepImportResponse | Response:
    """Import externally built STEP bytes for view-only display.

    The request body is the STEP file itself. This route has its own ceiling
    (:data:`DEFAULT_MAX_STEP_BYTES` unless configured); the route-aware body
    middleware reads the body in bounded chunks and refuses as soon as the
    limit is exceeded. The handler repeats the check for direct ASGI callers.
    """
    if not design_available():
        return _design_unavailable_response()
    limit = getattr(request.app.state, "step_import_max_bytes", DEFAULT_MAX_STEP_BYTES)
    body = await request.body()
    if not body:
        return _step_error_response(StepMalformedError("the STEP file is empty"))
    if len(body) > limit:
        return _step_error_response(
            StepTooLargeError(f"the STEP file must not exceed {limit} bytes")
        )
    service = _design_service(request)
    try:
        outcome = await to_thread(
            import_step_bytes,
            body,
            journal=service.journal,
            max_step_bytes=limit,
        )
    except StepImportError as error:
        return _step_error_response(error)
    except JournalIntegrityError:
        return _journal_error_response()
    return StepImportResponse(
        view=StepImportView(
            positions=outcome.mesh.positions,
            normals=outcome.mesh.normals,
            indices=outcome.mesh.indices,
            bounds=BBox(min=outcome.bounds.min_mm, max=outcome.bounds.max_mm),
        ),
        step_sha256=outcome.step_sha256,
        solids=outcome.solids,
        warnings=outcome.warnings,
    )


router = APIRouter(prefix=INTERNAL_PREFIX, tags=["internal"])

router.add_api_route(
    "/preview",
    preview,
    methods=["POST"],
    response_model=PreviewResponse,
    summary="Build a parametric mesh preview",
)
router.add_api_route(
    "/export",
    export_primitive,
    methods=["POST"],
    response_model=None,
    summary="Render a primitive or a stored design to STL or STEP",
)
router.add_api_route(
    "/handoff",
    handoff,
    methods=["POST"],
    response_model=None,
    summary="Render a neutral-file handoff for one CAD target",
)
router.add_api_route(
    "/design",
    design_route,
    methods=["POST"],
    response_model=DesignResult,
    summary="Build one design with the caller-selected CAD provider",
    description=BUILDER_ROUTE_DESCRIPTION,
    openapi_extra={
        "x-kala-provider": BUILDER_PROVIDER,
        "x-kala-provider-mode": BUILDER_PROVIDER_MODE,
    },
)
router.add_api_route(
    "/design/{design_id}/drawing",
    design_drawing,
    methods=["GET"],
    response_model=DrawingResponse,
    summary='Hidden-line views of a design built by "opencascade" (opt-in)',
    description=BUILDER_ROUTE_DESCRIPTION,
    openapi_extra={
        "x-kala-provider": BUILDER_PROVIDER,
        "x-kala-provider-mode": BUILDER_PROVIDER_MODE,
    },
)
router.add_api_route(
    "/design/{design_id}/section",
    design_section,
    methods=["GET"],
    response_model=SectionResponse,
    summary='Section loops of a design built by "opencascade" (opt-in)',
    description=BUILDER_ROUTE_DESCRIPTION,
    openapi_extra={
        "x-kala-provider": BUILDER_PROVIDER,
        "x-kala-provider-mode": BUILDER_PROVIDER_MODE,
    },
)
router.add_api_route(
    "/design/{design_id}/artifact/{kind}",
    design_artifact,
    methods=["GET"],
    summary="Download one stored artifact of a design",
    description=(
        "Serves the exact stored bytes of one artifact kind, digest-verified "
        "against the journal before a byte is sent."
    ),
)
router.add_api_route(
    "/import",
    step_import,
    methods=["POST"],
    response_model=StepImportResponse,
    summary="Import an externally built STEP file for view-only display",
    description=IMPORT_ROUTE_DESCRIPTION,
    openapi_extra={
        "x-kala-provider": BUILDER_PROVIDER,
        "x-kala-role": "view-only",
    },
)
router.add_api_route(
    "/journal",
    journal_route,
    methods=["GET"],
    response_model=JournalPageResponse,
    summary="A bounded page of Studio journal events",
)
router.add_api_route(
    "/journal/verify",
    journal_verify_route,
    methods=["GET"],
    response_model=JournalVerifyResponse,
    summary="Verify the Studio journal hash chain",
)
router.add_api_route(
    "/providers/cad",
    cad_providers,
    methods=["GET"],
    response_model=CadProvidersResponse,
    summary="CAD provider availability (probed, never assumed)",
    description=CAD_PROVIDERS_ROUTE_DESCRIPTION,
)


@router.post(
    "/intent/parse",
    response_model=IntentParseResponse,
    summary="Strictly parse a prompt into a design intent or a missing-values list",
)
def intent_parse(payload: IntentParseRequest) -> IntentParseResponse:
    parsed = parse_design_prompt(payload.prompt)
    if parsed is None:
        return IntentParseResponse()
    return IntentParseResponse(
        family=parsed.family,
        intent=parsed.intent,
        missing=parsed.missing,
        notes=parsed.notes,
    )


@router.post(
    "/intent/validate",
    response_model=IntentValidateResponse,
    summary="Validate a model reply's fenced DesignIntent JSON",
)
def intent_validate(payload: IntentValidateRequest) -> IntentValidateResponse | Response:
    try:
        intent = parse_design_intent_text(payload.text)
    except DesignIntentParseError as error:
        return _invalid_intent_response(str(error))
    return IntentValidateResponse(intent=intent)


@router.get(
    "/intent/schema",
    summary="The DesignIntent JSON Schema text for the remote provider prompt",
)
def intent_schema() -> Response:
    return Response(content=design_intent_schema_text(), media_type="text/plain")


@router.post(
    "/journal/append",
    response_model=JournalEventRecord,
    status_code=201,
    summary="Append one allow-listed Rust-originated journal event",
)
def journal_append(
    payload: JournalAppendRequest, request: Request
) -> JournalEventRecord | Response:
    if payload.event_type not in ALLOWED_APPEND_EVENT_TYPES:
        return _append_event_type_response(payload.event_type)
    try:
        size = _canonical_payload_size(payload.payload)
    except (TypeError, ValueError):
        return _append_payload_response("the payload must be a canonical JSON object")
    if size > MAX_APPEND_PAYLOAD_BYTES:
        return _append_payload_response(
            f"the payload must not exceed {MAX_APPEND_PAYLOAD_BYTES} bytes"
        )
    service = _design_service(request)
    try:
        event = service.journal.append(
            payload.event_type,
            payload.payload,
            aggregate_id=payload.aggregate_id,
        )
    except JournalIntegrityError:
        return _journal_error_response()
    except (TypeError, ValueError) as error:
        return _append_payload_response(str(error))
    return _journal_event(event)


def _canonical_payload_size(payload: Mapping[str, object]) -> int:
    text = json.dumps(
        dict(payload),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        allow_nan=False,
    )
    return len(text.encode("utf-8"))


def _invalid_intent_response(reason: str) -> Response:
    return _json_error(
        422,
        error_response(
            "invalid_intent",
            "the model reply did not contain a valid DesignIntent",
            (reason[:256],),
        ).model_dump(mode="json"),
    )


def _append_event_type_response(event_type: str) -> Response:
    allowed = ", ".join(sorted(ALLOWED_APPEND_EVENT_TYPES))
    return _json_error(
        422,
        error_response(
            "event_type_not_allowed",
            "the worker journal accepts only Rust-originated event types",
            (f"event_type={event_type[:64]}; allowed={allowed}",),
        ).model_dump(mode="json"),
    )


def _append_payload_response(reason: str) -> Response:
    return _json_error(
        422,
        error_response(
            "invalid_request",
            "the journal payload is invalid",
            (reason[:256],),
        ).model_dump(mode="json"),
    )


def _step_error_response(error: StepImportError) -> Response:
    """Return the typed 4xx refusal for an unimportable STEP file."""
    status_code = 413 if isinstance(error, StepTooLargeError) else 422
    return _json_error(
        status_code,
        error_response(
            error.code,
            error.message,
            (str(error)[:256],),
        ).model_dump(mode="json"),
    )


class WorkerBodyLimitMiddleware:
    """Apply the STEP import ceiling only to ``/internal/v1/import``.

    Every other worker route keeps the generic Studio body limit. The import
    route is read in bounded chunks by the same helper the generic middleware
    uses; the loop stops as soon as a chunk would cross the ceiling, so the
    worker never holds more than the configured limit.
    """

    def __init__(
        self,
        app: ASGIApp,
        *,
        default_limit: int,
        import_limit: int,
        import_path: str,
    ) -> None:
        self._app = app
        self._import_limit = validate_step_import_max_bytes(import_limit)
        self._import_path = import_path
        self._default = MaxBodySizeMiddleware(
            app,
            max_body_bytes=validate_max_body_bytes(default_limit),
        )

    @property
    def import_limit(self) -> int:
        """Return the STEP import ceiling in bytes."""
        return self._import_limit

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http" or scope.get("path") != self._import_path:
            await self._default(scope, receive, send)
            return
        declared = _declared_length(scope)
        if declared is not None and declared > self._import_limit:
            await self._too_large(send)
            return
        body, exceeded = await _buffer_body(receive, self._import_limit)
        if exceeded:
            await self._too_large(send)
            return
        if body is None:
            return

        async def replayed_receive() -> Message:
            return {"type": "http.request", "body": body, "more_body": False}

        await self._app(scope, replayed_receive, send)

    async def _too_large(self, send: Send) -> None:
        await _send_refusal(
            send,
            status_code=413,
            code="payload_too_large",
            message="the request body exceeds the configured size limit",
            details=(f"the STEP file must not exceed {self._import_limit} bytes",),
        )


def build_worker_middleware(
    *,
    api_token: str,
    port: int,
    max_body_bytes: int,
    step_import_max_bytes: int = DEFAULT_MAX_STEP_BYTES,
) -> list[Middleware]:
    """Return the worker middleware stack, outermost first.

    The order mirrors the Studio policy: security headers wrap every refusal,
    the Host guard answers before a body is buffered, the route-aware size
    limit answers without consulting the token, the Origin guard refuses a
    browser before the router, and the bearer gate is the last check before a
    handler.
    """
    return [
        Middleware(SecurityHeadersMiddleware),
        Middleware(WorkerHostGuardMiddleware, port=port),
        Middleware(
            WorkerBodyLimitMiddleware,
            default_limit=max_body_bytes,
            import_limit=step_import_max_bytes,
            import_path=f"{INTERNAL_PREFIX}/import",
        ),
        Middleware(WorkerOriginGuardMiddleware),
        Middleware(WorkerBearerMiddleware, api_token=api_token),
    ]


def create_worker_app(
    *,
    api_token: str,
    port: int,
    data_dir: Path,
    max_body_bytes: int = MAX_REQUEST_BODY_BYTES,
    environ: Mapping[str, str] | None = None,
    step_import_max_bytes: int | None = None,
) -> FastAPI:
    """Build the internal geometry-worker ASGI application."""
    token = validate_worker_token(api_token)
    if isinstance(port, bool) or not isinstance(port, int) or not 1 <= port <= 65_535:
        raise ValueError("port must be an integer between 1 and 65535")
    source = environ if environ is not None else os.environ
    resolved_dir = Path(data_dir).expanduser().resolve(strict=False)
    resolved_dir.mkdir(parents=True, exist_ok=True)
    limit = validate_max_body_bytes(max_body_bytes)
    import_limit = (
        resolve_step_import_max_bytes(source)
        if step_import_max_bytes is None
        else validate_step_import_max_bytes(step_import_max_bytes)
    )
    design_service = DesignService(resolved_dir, max_step_bytes=import_limit, environ=source)
    app = FastAPI(
        title="Kala Studio Geometry Worker",
        version=STUDIO_VERSION,
        description="Internal worker surface (loopback only, bearer on every route).",
        lifespan=build_lifespan(
            token,
            data_dir=resolved_dir,
            environ=source,
            design_service=design_service,
        ),
        middleware=build_worker_middleware(
            api_token=token,
            port=port,
            max_body_bytes=limit,
            step_import_max_bytes=import_limit,
        ),
        docs_url=None,
        redoc_url=None,
        openapi_url=None,
    )
    app.state.studio_data_dir = resolved_dir
    app.state.studio_api_token = token
    app.state.design_service = design_service
    app.state.step_import_max_bytes = import_limit
    app.state.studio_use_custom_environ = environ is not None
    app.state.worker_port = port
    if environ is not None:
        app.state.studio_environ = environ
    app.include_router(router)
    install_error_handlers(app)
    return app
