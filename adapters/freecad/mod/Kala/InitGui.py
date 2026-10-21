"""Load the Kala design box.

FreeCAD runs this file with exec(), so the dock lives in the imported module.
FreeCAD already put this folder on ``sys.path``; the adapter paths are
prepared by ``Init.py`` (which every session runs) and are prepared again
here, because the graphical add-on may be loaded on its own.
"""

try:
    import kala_paths
except Exception:  # pragma: no cover - the add-on folder is incomplete
    kala_paths = None

if kala_paths is not None:
    try:
        kala_paths.prepare()
    except Exception:  # pragma: no cover - never block the workbench
        pass

import kala_gui

kala_gui.register()
