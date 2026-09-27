"""Annotated FastAPI dependencies for the local Kala HTTP adapter.

The composition root is resolved from application state rather than from a
module global, so a test can inject an application and two applications can
coexist in one process. Path and query parameters are declared with Annotated
bounds here, so every route inherits the same limits.
"""

from __future__ import annotations

from typing import Annotated, Final

from fastapi import Depends, Path, Query, Request

from kala_agent.api.schemas import (
    DESIGN_ID_PATTERN,
    IDENTIFIER_PATTERN,
    MAX_AFTER_SEQUENCE,
    MAX_DESIGN_ID_LENGTH,
    MAX_IDENTIFIER_LENGTH,
    MAX_LIST_LIMIT,
)
from kala_agent.application import ClosedApplicationError, KalaApplication

APPLICATION_STATE_KEY: Final[str] = "kala_application"
OWNS_APPLICATION_STATE_KEY: Final[str] = "kala_owns_application"


def get_application(request: Request) -> KalaApplication:
    """Return the application bound to the running ASGI application.

    A closed application is refused before any route runs, so a released data
    directory is reported as unavailable rather than read.
    """
    application = getattr(request.app.state, APPLICATION_STATE_KEY, None)
    if not isinstance(application, KalaApplication):
        raise ClosedApplicationError("the local application is not available")
    if application.closed:
        raise ClosedApplicationError("the local application is closed")
    return application


def owns_application(request: Request) -> bool:
    """Return whether the lifespan handler will close the application."""
    return bool(getattr(request.app.state, OWNS_APPLICATION_STATE_KEY, False))


ApplicationDependency = Annotated[KalaApplication, Depends(get_application)]
DomainPath = Annotated[
    str,
    Path(
        min_length=1,
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Registered domain package name.",
    ),
]
DesignIdPath = Annotated[
    str,
    Path(
        min_length=1,
        max_length=MAX_DESIGN_ID_LENGTH,
        pattern=DESIGN_ID_PATTERN,
        description="Stable design identifier.",
    ),
]
WorkflowIdPath = Annotated[
    str,
    Path(
        min_length=1,
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Workflow identifier.",
    ),
]
RevisionIdPath = Annotated[
    str,
    Path(
        min_length=1,
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Immutable revision identifier.",
    ),
]
RequestIdPath = Annotated[
    str,
    Path(
        min_length=1,
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Prepared revision commit identifier.",
    ),
]
DesignIdQuery = Annotated[
    str,
    Query(
        min_length=1,
        max_length=MAX_DESIGN_ID_LENGTH,
        pattern=DESIGN_ID_PATTERN,
        description="Design whose revisions should be listed.",
    ),
]
LimitQuery = Annotated[
    int,
    Query(
        ge=1,
        le=MAX_LIST_LIMIT,
        description="Maximum number of records to return.",
    ),
]
AfterSequenceQuery = Annotated[
    int,
    Query(
        ge=0,
        le=MAX_AFTER_SEQUENCE,
        description="Return events after this sequence.",
    ),
]
WorkflowIdQuery = Annotated[
    str | None,
    Query(
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Restrict events to one workflow.",
    ),
]
EventTypeQuery = Annotated[
    str | None,
    Query(
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Restrict events to one event type.",
    ),
]
AggregateIdQuery = Annotated[
    str | None,
    Query(
        max_length=MAX_IDENTIFIER_LENGTH,
        pattern=IDENTIFIER_PATTERN,
        description="Restrict events to one aggregate.",
    ),
]

__all__ = (
    "APPLICATION_STATE_KEY",
    "OWNS_APPLICATION_STATE_KEY",
    "AfterSequenceQuery",
    "AggregateIdQuery",
    "ApplicationDependency",
    "DesignIdQuery",
    "DomainPath",
    "EventTypeQuery",
    "LimitQuery",
    "RequestIdPath",
    "RevisionIdPath",
    "WorkflowIdPath",
    "WorkflowIdQuery",
    "get_application",
    "owns_application",
)
