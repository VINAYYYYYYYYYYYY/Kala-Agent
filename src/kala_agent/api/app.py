"""The local Kala HTTP adapter.

This module is transport only. It builds the ASGI application, binds it to a
loopback-facing default configuration, and forwards validated requests to the
composition root. It holds no engineering rule, writes no database row, calls no
provider, and never opens an outbound connection.

The trust model, in one paragraph: one local operator, no multi-user isolation,
loopback only. A request is served only when its Host header names the loopback
interface, which is what blocks a rebound name, and every mutating method must
present the per-launch bearer token that this process stored in the local data
directory. Reads stay open to a caller that already has that directory. The
token is never logged and never returned; ``kala-agent token`` prints it for the
operator. See the OpenAPI description for the full statement.

Serve this application without a proxy-header middleware. One that trusts
``X-Forwarded-Host`` rewrites the very header the Host guard judges, and would
hand the guard an attacker-chosen name.
"""

from __future__ import annotations

from collections.abc import AsyncIterator, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from typing import Annotated, Final

from fastapi import APIRouter, Body, FastAPI, Response, status
from starlette.middleware import Middleware

from kala_agent.api.boundary import (
    BearerTokenMiddleware,
    CrossOriginGuardMiddleware,
    HostGuardMiddleware,
    issue_api_token,
    validate_api_token,
)
from kala_agent.api.dependencies import (
    APPLICATION_STATE_KEY,
    OWNS_APPLICATION_STATE_KEY,
    AfterSequenceQuery,
    AggregateIdQuery,
    ApplicationDependency,
    DesignIdQuery,
    DomainPath,
    EventTypeQuery,
    LimitQuery,
    RequestIdPath,
    RevisionIdPath,
    WorkflowIdPath,
    WorkflowIdQuery,
)
from kala_agent.api.errors import install_error_handlers
from kala_agent.api.middleware import (
    MAX_REQUEST_BODY_BYTES,
    MaxBodySizeMiddleware,
    SecurityHeadersMiddleware,
    validate_max_body_bytes,
)
from kala_agent.api.schemas import (
    DEFAULT_LIST_LIMIT,
    SERVICE_NAME,
    CheckpointDecisionRequest,
    CheckpointListResponse,
    CreateRevisionRequest,
    DomainListResponse,
    EvaluateDomainRequest,
    EvaluationResponse,
    HealthResponse,
    PrepareRevisionRequest,
    RevisionListResponse,
    RunMechanicalWorkflowRequest,
    RunResponse,
)
from kala_agent.application import (
    DomainDetail,
    EventPage,
    KalaApplication,
    MechanicalRunResult,
    RevisionCommitView,
    RevisionRecord,
    WorkflowStateView,
)
from kala_agent.config import Settings

API_VERSION: Final[str] = "1.0.0"
API_PREFIX: Final[str] = "/api/v1"
LOOPBACK_SERVER: Final[dict[str, str]] = {
    "url": "http://127.0.0.1:8000",
    "description": "Local loopback only",
}
LOCAL_DISCLAIMER: Final[str] = (
    "Bounded surrogate reference arithmetic only; not CAD, CAE, a code check, or an "
    "engineering certification."
)
OPENAPI_DESCRIPTION: Final[str] = """
A local, single-user engineering design agent over its declarative domain
packages.

Scope, stated plainly: the domain packages compute bounded arithmetic
surrogates. Nothing here is CAD or CAE, not a solver, not a building-code
implementation, and not a certification of fitness for any purpose. Real design
sign-off belongs to a competent engineer using validated tools.

Trust model, stated plainly: this API performs no multi-user isolation and no
authorization model. It is designed for one local operator who already has
filesystem access to the same data directory, and it serves loopback traffic
only. Binding it to a routable address would publish a design surface, so do not
do that.

The request boundary, stated plainly: a request whose Host header does not name
the loopback interface is refused, which is what stops a rebound DNS name from
driving this process. Every mutating method (POST, PUT, PATCH, DELETE) must
present the per-launch bearer token as an Authorization header; GET, HEAD, and
OPTIONS are read-only and need no token. The token is minted once per launch,
stored in the local data directory with owner-only permissions, printed by the
`kala-agent token` command, and never logged, returned, or echoed in an error.
Request bodies are size limited on every method, whether or not a length was
declared, and every request carrying a cross-origin Origin is refused because
this process serves no browser client. Every response, including every refusal,
carries a restrictive security header set.

Revisions, stated plainly: a revision is never written without a pre-commit
approval. `POST /revisions` is the local single-user flow and does both phases
in one call, which is what the operator asking for a commit means. A caller that
wants to see what it is approving calls `POST /revision-commits`, reads the two
digests the approval will be bound to from `GET /revision-commits/{request_id}`,
and then approves or rejects it. A commit whose digests no longer describe the
current design head writes nothing and is reported as a conflict, and a
revision commit must extend the current head, so a design history is never
forked. The durable policy and budget state this process restores at startup
lives under aggregates the kernel reserves for itself, so a workflow identifier
naming one of them is refused rather than allowed to collide with it.

Everything stays local: the model gateway is bound to a deterministic in-process
client, provider egress is denied, and no response ever carries a traceback, a
submitted value, or credential-shaped text. Declaring a provider endpoint in the
local settings is configuration kept for a future adapter and does not wire one:
`GET /health` reports egress and offline state from the model client this process
actually holds, and reports a declared endpoint only as declared, so a hosted
path is never reported until one is constructed and bound.
""".strip()
TAGS: Final[tuple[tuple[str, str], ...]] = (
    ("health", "Process liveness"),
    ("domains", "Registered domain packages and their evaluations"),
    ("revisions", "Immutable design revisions and their provenance"),
    ("workflows", "Deterministic reference workflows and policy checkpoints"),
    ("journal", "Append-only event journal"),
)
router = APIRouter()
root_router = APIRouter()


@root_router.get(
    "/health",
    tags=["health"],
    summary="Report local process liveness",
    response_model=HealthResponse,
)
def health(application: ApplicationDependency) -> HealthResponse:
    """Return provider, model, and schema facts without endpoints or secrets.

    ``egress_configured`` and ``offline`` are facts about the model client this
    process actually holds, so they can never claim a hosted path that is not
    wired. A provider-neutral endpoint declared in the settings is reported as
    declared, which is the whole of what it is: this process has no client bound
    to it unless the operator constructed one.
    """
    return HealthResponse(
        status="ok",
        service=SERVICE_NAME,
        version=API_VERSION,
        provider=application.provider_id,
        model=application.model_id,
        offline=application.offline,
        egress_configured=application.egress_configured,
        provider_endpoint_declared=application.provider_endpoint_declared,
        data_dir=str(application.settings.data_dir),
        schema_version=application.database.schema_version,
        domains=application.domains,
        disclaimer=LOCAL_DISCLAIMER,
    )


@router.get(
    "/domains",
    tags=["domains"],
    summary="List every registered domain package and workflow",
    response_model=DomainListResponse,
)
def list_domains(application: ApplicationDependency) -> DomainListResponse:
    """Return the registered packages, their digests, and their workflows."""
    packages = application.list_domain_packages()
    return DomainListResponse(
        count=len(packages),
        packages=packages,
        workflows=application.list_workflow_definitions(),
        domains=application.domains,
    )


@router.get(
    "/domains/{domain}",
    tags=["domains"],
    summary="Inspect one registered domain package",
    response_model=DomainDetail,
)
def read_domain(domain: DomainPath, application: ApplicationDependency) -> DomainDetail:
    """Return one package declaration with its workflow and catalog steps."""
    return application.describe_domain(domain)


@router.post(
    "/domains/{domain}/evaluations",
    tags=["domains"],
    summary="Evaluate one reference design of a domain package",
    response_model=EvaluationResponse,
    status_code=status.HTTP_200_OK,
)
def evaluate_domain(
    domain: DomainPath,
    application: ApplicationDependency,
    payload: Annotated[EvaluateDomainRequest | None, Body()] = None,
) -> EvaluationResponse:
    """Evaluate a declared reference design, optionally with parameter overrides."""
    request = payload if payload is not None else EvaluateDomainRequest()
    report = application.evaluate_domain_design(
        domain,
        parameters=request.parameters,
        design_id=request.design_id,
    )
    return EvaluationResponse(evaluation=report)


@router.post(
    "/revisions",
    tags=["revisions"],
    summary="Create one immutable design revision",
    response_model=RevisionRecord,
    status_code=status.HTTP_201_CREATED,
)
def create_revision(
    payload: CreateRevisionRequest,
    response: Response,
    application: ApplicationDependency,
) -> RevisionRecord:
    """Prepare and commit a content-addressed revision with its provenance.

    This is the local single-user flow: the commit is derived, bound to two
    strong digests, approved against them, and written under a transactional
    head compare-and-swap. Supplying the digests a caller prepared earlier
    binds this call to that review, and a design head that has moved since
    writes nothing at all.
    """
    record = application.create_design_revision(
        design_id=payload.design_id,
        domain=payload.domain.value,
        summary=payload.summary,
        parameters=payload.parameters,
        parent_revision_id=payload.parent_revision_id,
        state_digest=payload.state_digest,
        action_digest=payload.action_digest,
    )
    response.headers["location"] = f"{API_PREFIX}/revisions/{record.revision.revision_id}"
    return record


@router.post(
    "/revision-commits",
    tags=["revisions"],
    summary="Prepare one revision commit and open its pre-commit checkpoint",
    response_model=RevisionCommitView,
    status_code=status.HTTP_201_CREATED,
)
def prepare_revision_commit(
    payload: PrepareRevisionRequest,
    response: Response,
    application: ApplicationDependency,
) -> RevisionCommitView:
    """Derive one commit, publish its digests, and write nothing yet.

    Nothing is written to the design here. The returned digests are what the
    later approve or reject is bound to, and they are refused if the design head,
    the specification, or the artifact bytes change in the meantime.
    """
    prepared = application.prepare_design_revision(
        design_id=payload.design_id,
        domain=payload.domain.value,
        summary=payload.summary,
        parameters=payload.parameters,
        parent_revision_id=payload.parent_revision_id,
    )
    response.headers["location"] = f"{API_PREFIX}/revision-commits/{prepared.request_id}"
    return prepared


@router.get(
    "/revision-commits/{request_id}",
    tags=["revisions"],
    summary="Read one prepared or decided revision commit",
    response_model=RevisionCommitView,
)
def read_revision_commit(
    request_id: RequestIdPath,
    application: ApplicationDependency,
) -> RevisionCommitView:
    """Return the stored digests and the decision recorded against them."""
    return application.get_revision_commit(request_id)


@router.post(
    "/revision-commits/{request_id}/approve",
    tags=["revisions"],
    summary="Approve one prepared revision commit",
    response_model=RevisionRecord,
)
def approve_revision_commit(
    request_id: RequestIdPath,
    application: ApplicationDependency,
    payload: Annotated[CheckpointDecisionRequest | None, Body()] = None,
) -> RevisionRecord:
    """Approve the prepared commit against its digests and write the revision."""
    request = payload if payload is not None else CheckpointDecisionRequest()
    return application.commit_design_revision(
        request_id,
        actor_id=request.actor_id,
        state_digest=request.state_digest,
        action_digest=request.action_digest,
        reason=request.reason,
    )


@router.post(
    "/revision-commits/{request_id}/reject",
    tags=["revisions"],
    summary="Reject one prepared revision commit",
    response_model=RevisionCommitView,
)
def reject_revision_commit(
    request_id: RequestIdPath,
    application: ApplicationDependency,
    payload: Annotated[CheckpointDecisionRequest | None, Body()] = None,
) -> RevisionCommitView:
    """Record the rejection. No revision is written and none can follow it."""
    request = payload if payload is not None else CheckpointDecisionRequest()
    return application.decide_revision_commit(
        request_id,
        "reject",
        actor_id=request.actor_id,
        state_digest=request.state_digest,
        action_digest=request.action_digest,
        reason=request.reason,
    )


@router.get(
    "/revisions",
    tags=["revisions"],
    summary="List the revisions of one design",
    response_model=RevisionListResponse,
)
def list_revisions(
    design_id: DesignIdQuery,
    application: ApplicationDependency,
    limit: LimitQuery = DEFAULT_LIST_LIMIT,
) -> RevisionListResponse:
    """Return every stored revision of one design in sequence order."""
    revisions = application.list_design_revisions(design_id, limit=limit)
    return RevisionListResponse(design_id=design_id, count=len(revisions), revisions=revisions)


@router.get(
    "/revisions/{revision_id}",
    tags=["revisions"],
    summary="Read one immutable revision with its provenance",
    response_model=RevisionRecord,
)
def read_revision(
    revision_id: RevisionIdPath,
    application: ApplicationDependency,
) -> RevisionRecord:
    """Return one stored revision exactly as it was committed."""
    return application.get_design_revision(revision_id)


def _run_response(result: MechanicalRunResult) -> RunResponse:
    return RunResponse(
        run=result,
        state=result.state,
        revision=result.revision,
        checkpoint=result.checkpoint,
    )


@router.post(
    "/workflows/mechanical/reference",
    tags=["workflows"],
    summary="Run the mechanical reference workflow to its policy checkpoint",
    response_model=RunResponse,
    status_code=status.HTTP_201_CREATED,
    responses={status.HTTP_200_OK: {"description": "An existing run was resumed."}},
)
def run_mechanical_reference(
    response: Response,
    application: ApplicationDependency,
    payload: Annotated[RunMechanicalWorkflowRequest | None, Body()] = None,
) -> RunResponse:
    """Start the mechanical reference run, or resume the one already in flight."""
    request = payload if payload is not None else RunMechanicalWorkflowRequest()
    result = application.run_mechanical_reference_workflow(
        workflow_id=request.workflow_id,
        design_id=request.design_id,
        actor_id=request.actor_id,
    )
    response.status_code = status.HTTP_201_CREATED if result.started else status.HTTP_200_OK
    response.headers["location"] = f"{API_PREFIX}/workflows/{result.workflow_id}"
    return _run_response(result)


@router.get(
    "/workflows/{workflow_id}",
    tags=["workflows"],
    summary="Read the current workflow state",
    response_model=WorkflowStateView,
)
def read_workflow_state(
    workflow_id: WorkflowIdPath,
    application: ApplicationDependency,
) -> WorkflowStateView:
    """Return the current state of one stored workflow."""
    return application.get_workflow_state(workflow_id)


@router.get(
    "/workflows/{workflow_id}/checkpoints",
    tags=["workflows"],
    summary="List the recorded policy checkpoints of one workflow",
    response_model=CheckpointListResponse,
)
def list_checkpoints(
    workflow_id: WorkflowIdPath,
    application: ApplicationDependency,
) -> CheckpointListResponse:
    """Return the stored checkpoint records and any checkpoint still pending."""
    state = application.get_workflow_state(workflow_id)
    approvals = application.list_workflow_checkpoints(workflow_id)
    return CheckpointListResponse(
        workflow_id=workflow_id,
        count=len(approvals),
        pending=state.pending_approval,
        approvals=approvals,
    )


@router.post(
    "/workflows/{workflow_id}/checkpoints/approve",
    tags=["workflows"],
    summary="Approve the pending policy checkpoint",
    response_model=RunResponse,
)
def approve_checkpoint(
    workflow_id: WorkflowIdPath,
    application: ApplicationDependency,
    payload: Annotated[CheckpointDecisionRequest | None, Body()] = None,
) -> RunResponse:
    """Approve the pending checkpoint and commit the declared revision."""
    request = payload if payload is not None else CheckpointDecisionRequest()
    return _decide(application, workflow_id, "approve", request)


@router.post(
    "/workflows/{workflow_id}/checkpoints/reject",
    tags=["workflows"],
    summary="Reject the pending policy checkpoint",
    response_model=RunResponse,
)
def reject_checkpoint(
    workflow_id: WorkflowIdPath,
    application: ApplicationDependency,
    payload: Annotated[CheckpointDecisionRequest | None, Body()] = None,
) -> RunResponse:
    """Reject the pending checkpoint and end the workflow as failed."""
    request = payload if payload is not None else CheckpointDecisionRequest()
    return _decide(application, workflow_id, "reject", request)


def _decide(
    application: KalaApplication,
    workflow_id: str,
    decision: str,
    payload: CheckpointDecisionRequest,
) -> RunResponse:
    result = application.decide_workflow_checkpoint(
        workflow_id,
        decision,
        actor_id=payload.actor_id,
        state_digest=payload.state_digest,
        action_digest=payload.action_digest,
        reason=payload.reason,
    )
    return _run_response(result)


@router.get(
    "/events",
    tags=["journal"],
    summary="Read a bounded page of journal events",
    response_model=EventPage,
)
def list_events(
    application: ApplicationDependency,
    after_sequence: AfterSequenceQuery = 0,
    limit: LimitQuery = DEFAULT_LIST_LIMIT,
    workflow_id: WorkflowIdQuery = None,
    event_type: EventTypeQuery = None,
    aggregate_id: AggregateIdQuery = None,
) -> EventPage:
    """Return journal events in sequence order, bounded by a page limit."""
    return application.list_events(
        after_sequence=after_sequence,
        limit=limit,
        workflow_id=workflow_id,
        event_type=event_type,
        aggregate_id=aggregate_id,
    )


LifespanFactory = Callable[[FastAPI], AbstractAsyncContextManager[None]]


def build_lifespan(application: KalaApplication, *, owns: bool) -> LifespanFactory:
    """Return a lifespan handler that releases an owned application."""

    @asynccontextmanager
    async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
        try:
            yield
        finally:
            if owns and not application.closed:
                application.close()

    return lifespan


def build_middleware_stack(
    *,
    api_token: str,
    max_body_bytes: int,
) -> list[Middleware]:
    """Return the request boundary, outermost first.

    The order is the policy. Security headers wrap everything, so a refusal from
    any stage is as complete as a success. The Host guard runs before a body is
    read, so a rebound name costs nothing. The size limit runs before the token
    is judged, so the limit answers identically for every method and for an
    unauthenticated caller. The token is judged before routing, so a mutating
    method can never reach a handler unauthenticated, and the cross-origin guard
    refuses a browser-originated request before it can drive a handler.
    """
    return [
        Middleware(SecurityHeadersMiddleware),
        Middleware(HostGuardMiddleware),
        Middleware(MaxBodySizeMiddleware, max_body_bytes=max_body_bytes),
        Middleware(BearerTokenMiddleware, api_token=api_token),
        Middleware(CrossOriginGuardMiddleware),
    ]


def create_app(
    application: KalaApplication | None = None,
    *,
    settings: Settings | None = None,
    api_token: str | None = None,
    max_body_bytes: int = MAX_REQUEST_BODY_BYTES,
    title: str = "Kala local agent API",
    version: str = API_VERSION,
) -> FastAPI:
    """Build the local API bound to one composition root.

    The application is created from the environment when it is not supplied, and
    is closed by the lifespan handler only when this factory created it.

    The bearer token guards every mutating method. It is injected for a caller
    that manages it elsewhere, and otherwise minted here for this launch and
    written to the local data directory, where ``kala-agent token`` reads it
    back. Either way the factory never returns, logs, or reports it.
    """
    if application is not None and settings is not None:
        raise ValueError("supply either an application or settings, not both")
    limit = validate_max_body_bytes(max_body_bytes)
    owns = application is None
    if application is not None:
        resolved = application
    else:
        resolved = KalaApplication(settings if settings is not None else Settings.from_env())
    token = (
        validate_api_token(api_token)
        if api_token is not None
        else issue_api_token(resolved.settings.data_dir)
    )
    app = FastAPI(
        title=title,
        version=version,
        description=OPENAPI_DESCRIPTION,
        lifespan=build_lifespan(resolved, owns=owns),
        openapi_tags=[{"name": name, "description": description} for name, description in TAGS],
        servers=[LOOPBACK_SERVER],
        docs_url="/docs",
        redoc_url=None,
        middleware=build_middleware_stack(api_token=token, max_body_bytes=limit),
    )
    setattr(app.state, APPLICATION_STATE_KEY, resolved)
    setattr(app.state, OWNS_APPLICATION_STATE_KEY, owns)
    app.include_router(root_router)
    app.include_router(router, prefix=API_PREFIX)
    install_error_handlers(app)
    return app


__all__ = (
    "API_PREFIX",
    "API_VERSION",
    "LOOPBACK_SERVER",
    "OPENAPI_DESCRIPTION",
    "build_lifespan",
    "build_middleware_stack",
    "create_app",
    "root_router",
    "router",
)
