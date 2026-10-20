"""Local HTTP adapter for the Kala agent.

Import surface only. The API is a thin transport over
:mod:`kala_agent.application`; every engineering rule, every persisted fact, and
every provider interaction stays in the composition root and the components it
holds.

The transport surface is re-exported lazily. Building the ASGI application needs
the composition root, which reaches the bounded tool plane and its
platform-specific sandbox, so a bare ``import kala_agent.api`` must not load it.
Every name below resolves to the same object as before on first explicit access,
and the public ``__all__`` is unchanged.
"""

from __future__ import annotations

from importlib import import_module
from typing import TYPE_CHECKING, Final, cast

if TYPE_CHECKING:
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

_LAZY_EXPORTS: Final[dict[str, str]] = {
    "API_PREFIX": "kala_agent.api.app",
    "API_VERSION": "kala_agent.api.app",
    "LOOPBACK_SERVER": "kala_agent.api.app",
    "OPENAPI_DESCRIPTION": "kala_agent.api.app",
    "build_lifespan": "kala_agent.api.app",
    "build_middleware_stack": "kala_agent.api.app",
    "create_app": "kala_agent.api.app",
    "router": "kala_agent.api.app",
    "API_TOKEN_FILE_NAME": "kala_agent.api.boundary",
    "MUTATING_METHODS": "kala_agent.api.boundary",
    "BearerTokenMiddleware": "kala_agent.api.boundary",
    "CrossOriginGuardMiddleware": "kala_agent.api.boundary",
    "HostGuardMiddleware": "kala_agent.api.boundary",
    "is_loopback_host": "kala_agent.api.boundary",
    "is_permitted_authority": "kala_agent.api.boundary",
    "issue_api_token": "kala_agent.api.boundary",
    "read_api_token": "kala_agent.api.boundary",
    "validate_api_token": "kala_agent.api.boundary",
    "APPLICATION_STATE_KEY": "kala_agent.api.dependencies",
    "OWNS_APPLICATION_STATE_KEY": "kala_agent.api.dependencies",
    "ApplicationDependency": "kala_agent.api.dependencies",
    "RequestIdPath": "kala_agent.api.dependencies",
    "RevisionIdPath": "kala_agent.api.dependencies",
    "get_application": "kala_agent.api.dependencies",
    "install_error_handlers": "kala_agent.api.errors",
    "MAX_REQUEST_BODY_BYTES": "kala_agent.api.middleware",
    "SECURITY_HEADERS": "kala_agent.api.middleware",
    "MaxBodySizeMiddleware": "kala_agent.api.middleware",
    "SecurityHeadersMiddleware": "kala_agent.api.middleware",
    "validate_max_body_bytes": "kala_agent.api.middleware",
    "SERVICE_NAME": "kala_agent.api.schemas",
    "CheckpointDecisionRequest": "kala_agent.api.schemas",
    "CheckpointListResponse": "kala_agent.api.schemas",
    "CreateRevisionRequest": "kala_agent.api.schemas",
    "DomainListResponse": "kala_agent.api.schemas",
    "EvaluateDomainRequest": "kala_agent.api.schemas",
    "EvaluationResponse": "kala_agent.api.schemas",
    "HealthResponse": "kala_agent.api.schemas",
    "PrepareRevisionRequest": "kala_agent.api.schemas",
    "RevisionListResponse": "kala_agent.api.schemas",
    "RunMechanicalWorkflowRequest": "kala_agent.api.schemas",
    "RunResponse": "kala_agent.api.schemas",
    "error_response": "kala_agent.api.envelope",
}


def __getattr__(name: str) -> object:
    """Resolve a deferred public export on first access.

    Only the names in :data:`_LAZY_EXPORTS` are deferred; every other name is an
    ordinary missing attribute. The resolved object is cached in the module
    namespace, so repeated lookups do not re-enter this function and the
    exported identity is stable.
    """
    module_name = _LAZY_EXPORTS.get(name)
    if module_name is None:
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
    value = getattr(import_module(module_name), name)
    globals()[name] = value
    return cast(object, value)


def __dir__() -> list[str]:
    """Return the public surface, including exports that are not yet resolved."""
    return sorted({*globals(), *__all__})
