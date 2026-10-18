"""Kala Studio standalone surface.

The studio is a Windows/Linux desktop app for parametric CAD and analysis
interoperability. The Python surface here serves a loopback-only FastAPI
process that the Electron host renders around. It is intentionally
separate from the core ``kala_agent.application`` composition root: the
two surfaces share only the contract helpers (loopback authority
predicate, error envelope shape) and nothing else.

Public entry points:

- :func:`kala_agent.studio.server.create_studio_app` builds a FastAPI
  instance for tests and in-process embedding.
- :func:`kala_agent.studio.cli.main` runs the studio HTTP server through
  ``python -m kala_agent.studio`` from the launcher.
"""

from __future__ import annotations

from kala_agent.studio.server import (
    STUDIO_PREFIX,
    STUDIO_VERSION,
    build_lifespan,
    build_middleware_stack,
    create_studio_app,
)

__all__ = (
    "STUDIO_PREFIX",
    "STUDIO_VERSION",
    "build_lifespan",
    "build_middleware_stack",
    "create_studio_app",
)
