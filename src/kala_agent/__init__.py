"""A local-first engineering design agent foundation.

The package root exposes the validated settings and the composition root. The
local HTTP adapter is imported from :mod:`kala_agent.api` and the command line
adapter from :mod:`kala_agent.cli`, so importing this package never pulls in
either presentation layer.
"""

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

__all__ = ("KalaApplication", "Settings")
