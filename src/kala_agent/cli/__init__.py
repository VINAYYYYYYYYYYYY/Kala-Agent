"""Local command line adapter for the Kala agent.

Import surface only. The command line is a presentation layer over
:mod:`kala_agent.application`: it reads settings, calls the composition root,
prints JSON, and maps outcomes onto exit codes.
"""

from kala_agent.cli.main import (
    DISCLAIMER,
    EXIT_ERROR,
    EXIT_FAILED_OUTCOME,
    EXIT_OK,
    EXIT_USAGE,
    PROGRAM_NAME,
    SERVE_HOST,
    SERVE_PORT,
    ServePlan,
    ServeTarget,
    build_parser,
    build_serve_plan,
    build_serve_target,
    load_uvicorn,
    main,
)

__all__ = (
    "DISCLAIMER",
    "EXIT_ERROR",
    "EXIT_FAILED_OUTCOME",
    "EXIT_OK",
    "EXIT_USAGE",
    "PROGRAM_NAME",
    "SERVE_HOST",
    "SERVE_PORT",
    "ServePlan",
    "ServeTarget",
    "build_parser",
    "build_serve_plan",
    "build_serve_target",
    "load_uvicorn",
    "main",
)
