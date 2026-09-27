"""Local HTTP adapter for the Kala agent.

Import surface only. The API is a thin transport over
:mod:`kala_agent.application`; every engineering rule, every persisted fact, and
every provider interaction stays in the composition root and the components it
holds.
"""

from kala_agent.api.app import (
    API_PREFIX,
    API_VERSION,
    LOOPBACK_SERVER,
    OPENAPI_DESCRIPTION,
    build_lifespan,
    build_middleware_stack,
    create_app,
    router,
)
from kala_agent.api.boundary import (
    API_TOKEN_FILE_NAME,
    MUTATING_METHODS,
    BearerTokenMiddleware,
    CrossOriginGuardMiddleware,
    HostGuardMiddleware,
    is_loopback_host,
    is_permitted_authority,
    issue_api_token,
    read_api_token,
    validate_api_token,
)
from kala_agent.api.dependencies import (
    APPLICATION_STATE_KEY,
    OWNS_APPLICATION_STATE_KEY,
    ApplicationDependency,
    RequestIdPath,
    RevisionIdPath,
    get_application,
)
from kala_agent.api.errors import install_error_handlers
from kala_agent.api.middleware import (
    MAX_REQUEST_BODY_BYTES,
    SECURITY_HEADERS,
    MaxBodySizeMiddleware,
    SecurityHeadersMiddleware,
    validate_max_body_bytes,
)
from kala_agent.api.schemas import (
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
    error_response,
)

__all__ = (
    "API_PREFIX",
    "API_TOKEN_FILE_NAME",
    "API_VERSION",
    "APPLICATION_STATE_KEY",
    "LOOPBACK_SERVER",
    "MAX_REQUEST_BODY_BYTES",
    "MUTATING_METHODS",
    "OPENAPI_DESCRIPTION",
    "OWNS_APPLICATION_STATE_KEY",
    "SECURITY_HEADERS",
    "SERVICE_NAME",
    "ApplicationDependency",
    "BearerTokenMiddleware",
    "CheckpointDecisionRequest",
    "CheckpointListResponse",
    "CreateRevisionRequest",
    "CrossOriginGuardMiddleware",
    "DomainListResponse",
    "EvaluateDomainRequest",
    "EvaluationResponse",
    "HealthResponse",
    "HostGuardMiddleware",
    "MaxBodySizeMiddleware",
    "PrepareRevisionRequest",
    "RequestIdPath",
    "RevisionIdPath",
    "RevisionListResponse",
    "RunMechanicalWorkflowRequest",
    "RunResponse",
    "SecurityHeadersMiddleware",
    "build_lifespan",
    "build_middleware_stack",
    "create_app",
    "error_response",
    "get_application",
    "install_error_handlers",
    "is_loopback_host",
    "is_permitted_authority",
    "issue_api_token",
    "read_api_token",
    "router",
    "validate_api_token",
    "validate_max_body_bytes",
)
