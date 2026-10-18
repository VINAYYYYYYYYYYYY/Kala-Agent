"""Standalone backend entry for the PyInstaller-frozen Kala Studio desktop.

The desktop bundle launches a single executable that imports this
module. PyInstaller reads ``packaging/studio-backend.spec`` and
collects ``kala_agent.studio.__main__:main`` as the entry point.

This module is intentionally thin. It forwards to the real studio
``__main__`` so that the desktop bundle and the developer-mode
``kala-studio`` script resolve to the same code; nothing about the
packaged binary is privileged over the source distribution.

Why a separate entry script and not the package's ``__main__``?

- The PyInstaller spec needs a module path that is unambiguous under
  ``--onefile`` collection. ``kala_agent.studio.__main__`` is the
  canonical entry, but PyInstaller treats an entry-point script with
  a stable file path more reliably than a ``__main__`` module.
- The packaging layer is delivery-owned. The studio backend is
  backend-owned. This module is the seam: it lives in ``packaging/``
  and imports from the studio package without modifying it.

The PyInstaller hook enables ``uvicorn`` to use the imported ``app``
instance rather than the ``"module:app"`` string form, so the frozen
process does not need to import ``studio.server`` twice. The
backend-owned ``create_studio_app`` factory is the only entry.
"""

from __future__ import annotations

import sys

from kala_agent.studio.__main__ import main

if __name__ == "__main__":  # pragma: no cover - exercised by PyInstaller
    sys.exit(main())
