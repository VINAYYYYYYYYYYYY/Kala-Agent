"""Load the Kala design box.

FreeCAD runs this file with exec(), so the dock lives in the imported module.
"""

import os
import sys

import FreeCAD

_mod = os.path.join(FreeCAD.getUserAppDataDir(), "Mod", "Kala")
if _mod not in sys.path:
    sys.path.insert(0, _mod)

import kala_gui  # noqa: E402

kala_gui.register()
