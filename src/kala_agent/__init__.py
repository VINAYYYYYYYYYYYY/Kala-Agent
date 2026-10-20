"""A local-first engineering design agent foundation.

The package root exposes the validated settings and the composition root. The
local HTTP adapter is imported from :mod:`kala_agent.api` and the command line
adapter from :mod:`kala_agent.cli`, so importing this package never pulls in
either presentation layer.

The composition root is resolved lazily. Importing ``kala_agent`` reads only
:mod:`kala_agent.config`, which is portable; the application graph behind it
reaches the tool plane and its platform-specific sandbox, so it is imported on
the first explicit access to :class:`KalaApplication` instead of at package
import time. The public names and their identities are unchanged.
"""

from __future__ import annotations

from importlib import import_module
from typing import TYPE_CHECKING, Final, cast

from kala_agent.config import Settings

if TYPE_CHECKING:
    from kala_agent.application import KalaApplication

__all__ = ("KalaApplication", "Settings")

_LAZY_EXPORTS: Final[dict[str, str]] = {
    "KalaApplication": "kala_agent.application",
}


def __getattr__(name: str) -> object:
    """Resolve a deferred public export on first access.

    Only the names in :data:`_LAZY_EXPORTS` are deferred; everything else is an
    ordinary missing attribute. The resolved object is cached in the module
    namespace so later lookups do not re-enter this function.
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
