"""Install the Kala design box into FreeCAD's own window.

The box is a FreeCAD mod. This module only links that mod into the user
Mod directory. It does not import FreeCAD.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

MOD_NAME = "Kala"


def source_mod() -> Path:
    """Return the Kala mod shipped beside this package."""
    return Path(__file__).resolve().parents[2] / "mod" / MOD_NAME


def user_mod_dir(home: Path | None = None) -> Path:
    """Return the FreeCAD user Mod directory for this machine."""
    root = (home or Path.home()) / ".local" / "share" / "FreeCAD"
    if (root / "v1-1").is_dir():
        return root / "v1-1" / "Mod"
    return root / "Mod"


def install(home: Path | None = None) -> Path:
    """Link the Kala mod into FreeCAD so the design box loads with the window."""
    source = source_mod()
    if not (source / "InitGui.py").is_file():
        raise FileNotFoundError(f"the Kala mod is missing at {source}")
    target_root = user_mod_dir(home)
    target_root.mkdir(parents=True, exist_ok=True)
    link = target_root / MOD_NAME
    if link.is_symlink() and link.resolve() == source.resolve():
        return link
    if link.exists() or link.is_symlink():
        raise FileExistsError(f"{link} already exists and points somewhere else")
    link.symlink_to(source, target_is_directory=True)
    return link


def main(argv: list[str] | None = None) -> int:
    """Install the design box and open FreeCAD."""
    args = list(sys.argv[1:] if argv is None else argv)
    link = install()
    print(f"Kala design box linked at {link}")
    if "--no-open" in args:
        return 0
    program = shutil.which("freecad") or "/usr/bin/freecad"
    if not Path(program).is_file():
        print("FreeCAD is not installed", file=sys.stderr)
        return 1
    environment = os.environ.copy()
    environment["QT_IM_MODULE"] = "none"
    subprocess.Popen(
        [program],
        env=environment,
        start_new_session=True,
    )
    print("FreeCAD is opening. The Kala box is on the right side of the window.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
