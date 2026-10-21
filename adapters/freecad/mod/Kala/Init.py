"""Kala is a graphical FreeCAD add-on.

FreeCAD runs this file in every session, console sessions included, before
any other add-on code. The adapter paths are prepared here so that a console
session finds the adapter exactly like the graphical one does.
"""

try:
    import kala_paths
except Exception:  # pragma: no cover - the add-on folder is incomplete
    kala_paths = None

if kala_paths is not None:
    try:
        kala_paths.prepare()
    except Exception:  # pragma: no cover - never block FreeCAD's startup
        pass
