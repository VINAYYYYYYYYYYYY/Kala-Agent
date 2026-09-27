"""CAD work that runs inside the FreeCAD worker.

The subpackage holds the geometry and FEM work, the FreeCAD loader that keeps
every native import inside a call, the argument readers, and the artifact
publication helpers. No module here imports FreeCAD at import time.
"""

from kala_agent_freecad.cad.runtime import (
    FREECAD_IMPORT_ORDER,
    FreeCADModules,
    freecad_session,
)

__all__ = (
    "FREECAD_IMPORT_ORDER",
    "FreeCADModules",
    "freecad_session",
)
