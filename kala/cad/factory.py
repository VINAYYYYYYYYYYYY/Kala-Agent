"""Factory for CAD backends."""

from __future__ import annotations

from kala.cad.protocol import CadBackend


def create_backend(name: str = "freecad") -> CadBackend:
    key = name.lower().strip()
    if key in {"freecad", "fc"}:
        try:
            from kala.cad.freecad.backend import FreeCADBackend

            return FreeCADBackend()
        except ImportError as exc:
            raise RuntimeError(
                "FreeCAD is not available. Install the FreeCAD package "
                "(e.g. `sudo pacman -S freecad`) or use `--backend mock`."
            ) from exc
    if key == "mock":
        from kala.cad.mock import MockBackend

        return MockBackend()
    if key in {"ocp", "opencascade"}:
        raise NotImplementedError(
            "OpenCASCADE backend is deferred until after the FreeCAD prototype."
        )
    raise ValueError(f"Unknown backend: {name}")
