"""FreeCAD CAD backend (primary test target) — syncs into the FreeCAD GUI."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

from kala.cad.freecad.gui_sync import (
    default_live_path,
    default_step_path,
    ensure_live_shown,
    install_kala_freecad_mod,
    publish_to_gui,
)
from kala.cad.protocol import ToolResult


def discover_freecad_lib() -> Path | None:
    """Find FreeCAD's Python lib directory on common Linux layouts."""
    candidates = [
        Path("/usr/lib/freecad/lib"),
        Path("/usr/lib64/freecad/lib"),
        Path("/usr/share/freecad/lib"),
        Path("/usr/lib/freecad-python3/lib"),
    ]
    for path in candidates:
        if (path / "FreeCAD.so").exists() or (path / "FreeCAD.py").exists():
            return path
        if path.is_dir() and any(path.glob("FreeCAD*")):
            return path
    return None


def ensure_freecad() -> Any:
    """Import FreeCAD, adding discovered lib path if needed."""
    try:
        import FreeCAD  # type: ignore

        return FreeCAD
    except ImportError:
        lib = discover_freecad_lib()
        if lib and str(lib) not in sys.path:
            sys.path.insert(0, str(lib))
        try:
            import FreeCAD  # type: ignore

            return FreeCAD
        except ImportError as exc:
            raise ImportError(
                "FreeCAD Python module not found. Install FreeCAD "
                "(e.g. `sudo pacman -S freecad`) or use `--backend mock`."
            ) from exc


class FreeCADBackend:
    """Script-based FreeCAD Part tools; saves + opens live doc in the GUI."""

    name = "freecad"

    def __init__(self, doc_name: str = "kala_live") -> None:
        self._FreeCAD = ensure_freecad()
        import Part  # type: ignore

        self._Part = Part
        self._live_path = default_live_path()
        self._step_path = default_step_path()
        self._mod_path = install_kala_freecad_mod(self._live_path)
        # FreeCAD Start page covers the 3D view — hide it so the model is visible
        try:
            self._FreeCAD.ParamGet(
                "User parameter:BaseApp/Preferences/Mod/Start"
            ).SetBool("ShowOnStartup", False)
        except Exception:
            pass
        self._doc_name = doc_name
        # Fresh document every run — otherwise solids accumulate across Runs
        # and the GUI looks wrong / overcrowded.
        existing = self._FreeCAD.listDocuments()
        if doc_name in existing:
            self._FreeCAD.closeDocument(doc_name)
        self._doc = self._FreeCAD.newDocument(doc_name)
        self._counter = 0

    def _next_label(self, prefix: str) -> str:
        self._counter += 1
        return f"{prefix}_{self._counter}"

    def _get_object(self, body_id: str) -> Any:
        obj = self._doc.getObject(body_id)
        if obj is None:
            raise KeyError(f"Body not found: {body_id}")
        return obj

    def _export_step_compound(self) -> None:
        """Write the best finished solid to STEP for FreeCAD GUI (not every leftover cutter)."""
        shapes = []
        preferred = None
        for obj in self._doc.Objects:
            shape = getattr(obj, "Shape", None)
            if shape is None or shape.isNull():
                continue
            name = (obj.Name or "").lower()
            label = (obj.Label or "").lower()
            # Prefer boolean results over raw cutters / duplicate stock
            if any(
                k in name or k in label
                for k in ("cut_", "fuse_", "fillet_", "kala")
            ):
                preferred = shape
            shapes.append(shape)
        if preferred is not None:
            compound = preferred
        elif not shapes:
            raise RuntimeError("No shapes to publish to FreeCAD GUI")
        elif len(shapes) == 1:
            compound = shapes[0]
        else:
            # Largest solid ≈ finished part when booleans produced one
            compound = max(shapes, key=lambda s: float(getattr(s, "Volume", 0.0) or 0.0))
        # Heal before export to ensure valid STEP files
        compound = self._heal_shape_for_export(compound)
        self._step_path.parent.mkdir(parents=True, exist_ok=True)
        compound.exportStep(str(self._step_path))

    def _sync_gui(self, *, force_open: bool = False) -> str:
        """Update live STEP quietly. Full FreeCAD GUI publish only on force_open.

        Mid-run freecadcmd + Mod reload on every tool was thrashing FreeCAD and
        contributed to crashes (Taj Mahal-scale runs especially).
        """
        try:
            self._export_step_compound()
        except Exception as exc:  # noqa: BLE001
            return f"STEP export failed: {exc}"
        if not force_open:
            return f"Live STEP updated ({self._step_path.name})"
        note = publish_to_gui(
            step_path=self._step_path,
            fcstd_path=self._live_path,
            force_open=True,
        )
        if self._mod_path:
            note += f" | bridge: {self._mod_path}"
        return note

    def show_in_gui(self) -> str:
        """Force-open the live document in FreeCAD (call at end of a run)."""
        try:
            if any(
                getattr(o, "Shape", None) is not None and not o.Shape.isNull()
                for o in self._doc.Objects
            ):
                self._export_step_compound()
        except Exception as exc:  # noqa: BLE001
            return f"Final GUI export failed: {exc}"
        # One-shot publish at end of run only
        try:
            self._export_step_compound()
        except Exception:
            pass
        return ensure_live_shown()

    def _ok(
        self,
        message: str,
        *,
        cad_api: str,
        data: dict[str, Any] | None = None,
        sync: bool = True,
    ) -> ToolResult:
        payload = dict(data or {})
        payload["cad_software"] = "FreeCAD"
        payload["cad_api"] = cad_api
        payload["live_document"] = str(self._live_path)
        gui_note = ""
        if sync:
            gui_note = self._sync_gui()
            payload["gui_sync"] = gui_note
        full = (
            f"[FreeCAD] {message}\n"
            f"  tool: {cad_api}\n"
            f"  doc: {self._live_path}"
        )
        if gui_note:
            full += f"\n  gui: {gui_note}"
        return ToolResult(ok=True, message=full, data=payload)

    def create_box(
        self, length: float, width: float, height: float, *, label: str = "Box"
    ) -> ToolResult:
        name = self._next_label(label)
        cad_api = f"Part.makeBox({length}, {width}, {height}) → Part::Feature '{name}'"
        box = self._Part.makeBox(length, width, height)
        obj = self._doc.addObject("Part::Feature", name)
        obj.Shape = box
        self._doc.recompute()
        return self._ok(
            f"Created box {obj.Name} ({length}×{width}×{height})",
            cad_api=cad_api,
            data={"body_id": obj.Name, "dims": [length, width, height]},
        )

    def create_cylinder(
        self, radius: float, height: float, *, label: str = "Cylinder"
    ) -> ToolResult:
        name = self._next_label(label)
        cad_api = f"Part.makeCylinder({radius}, {height}) → Part::Feature '{name}'"
        cyl = self._Part.makeCylinder(radius, height)
        obj = self._doc.addObject("Part::Feature", name)
        obj.Shape = cyl
        self._doc.recompute()
        return self._ok(
            f"Created cylinder {obj.Name} (r={radius}, h={height})",
            cad_api=cad_api,
            data={"body_id": obj.Name, "radius": radius, "height": height},
        )

    def create_sphere(self, radius: float, *, label: str = "Sphere") -> ToolResult:
        name = self._next_label(label)
        cad_api = f"Part.makeSphere({radius}) → Part::Feature '{name}'"
        sphere = self._Part.makeSphere(radius)
        obj = self._doc.addObject("Part::Feature", name)
        obj.Shape = sphere
        self._doc.recompute()
        return self._ok(
            f"Created sphere {obj.Name} (r={radius})",
            cad_api=cad_api,
            data={"body_id": obj.Name, "radius": radius},
        )

    def create_cone(
        self, radius1: float, radius2: float, height: float, *, label: str = "Cone"
    ) -> ToolResult:
        name = self._next_label(label)
        cad_api = (
            f"Part.makeCone({radius1}, {radius2}, {height}) → Part::Feature '{name}'"
        )
        cone = self._Part.makeCone(radius1, radius2, height)
        obj = self._doc.addObject("Part::Feature", name)
        obj.Shape = cone
        self._doc.recompute()
        return self._ok(
            f"Created cone {obj.Name} (r1={radius1}, r2={radius2}, h={height})",
            cad_api=cad_api,
            data={
                "body_id": obj.Name,
                "radius1": radius1,
                "radius2": radius2,
                "height": height,
            },
        )

    def _remove_body(self, body_id: str) -> None:
        """Drop a consumed donor so the document isn't full of duplicate leftovers."""
        obj = self._doc.getObject(body_id)
        if obj is not None:
            self._doc.removeObject(body_id)

    def boolean_fuse(self, body_a: str, body_b: str) -> ToolResult:
        a = self._get_object(body_a)
        b = self._get_object(body_b)
        name = self._next_label("Fuse")
        cad_api = f"Shape.fuse() on '{body_a}' + '{body_b}' → Part::Feature '{name}'"
        fused = a.Shape.fuse(b.Shape)
        # If parts only touched (no volume overlap), OCCT leaves multiple solids.
        n_solids = len(getattr(fused, "Solids", []) or [])
        obj = self._doc.addObject("Part::Feature", name)
        obj.Shape = fused
        # Remove donors — otherwise FreeCAD fills with every intermediate solid
        self._remove_body(body_a)
        self._remove_body(body_b)
        self._doc.recompute()
        warn = ""
        if n_solids > 1:
            warn = (
                f" WARNING: fuse produced {n_solids} solids (parts likely only "
                "touched — sink/overlap pieces so volumes intersect)."
            )
        return self._ok(
            f"Fused {body_a} + {body_b} → {obj.Name} (donors removed){warn}",
            cad_api=cad_api,
            data={
                "body_id": obj.Name,
                "removed": [body_a, body_b],
                "solid_count": n_solids,
            },
        )

    def boolean_cut(self, body_a: str, body_b: str) -> ToolResult:
        a = self._get_object(body_a)
        b = self._get_object(body_b)
        name = self._next_label("Cut")
        cad_api = f"Shape.cut() '{body_a}' - '{body_b}' → Part::Feature '{name}'"
        cut = a.Shape.cut(b.Shape)
        obj = self._doc.addObject("Part::Feature", name)
        obj.Shape = cut
        self._remove_body(body_a)
        self._remove_body(body_b)
        self._doc.recompute()
        return self._ok(
            f"Cut {body_b} from {body_a} → {obj.Name} (donors removed)",
            cad_api=cad_api,
            data={"body_id": obj.Name, "removed": [body_a, body_b]},
        )

    def translate(self, body_id: str, x: float, y: float, z: float) -> ToolResult:
        obj = self._get_object(body_id)
        cad_api = (
            f"Part::Feature '{body_id}'.Placement.Base += FreeCAD.Vector({x}, {y}, {z})"
        )
        obj.Placement.Base = self._FreeCAD.Vector(
            obj.Placement.Base.x + x,
            obj.Placement.Base.y + y,
            obj.Placement.Base.z + z,
        )
        self._doc.recompute()
        return self._ok(
            f"Translated {body_id} by ({x}, {y}, {z})",
            cad_api=cad_api,
            data={"body_id": body_id},
        )

    def rotate(
        self,
        body_id: str,
        axis: str,
        angle_deg: float,
        *,
        cx: float = 0.0,
        cy: float = 0.0,
        cz: float = 0.0,
    ) -> ToolResult:
        obj = self._get_object(body_id)
        axis_l = axis.lower().strip()
        axes = {
            "x": self._FreeCAD.Vector(1, 0, 0),
            "y": self._FreeCAD.Vector(0, 1, 0),
            "z": self._FreeCAD.Vector(0, 0, 1),
        }
        if axis_l not in axes:
            return ToolResult(
                ok=False,
                message=f"[FreeCAD] rotate axis must be x|y|z, got {axis!r}",
                data={"cad_software": "FreeCAD"},
            )
        center = self._FreeCAD.Vector(cx, cy, cz)
        cad_api = (
            f"Placement.rotate(({cx},{cy},{cz}), axis={axis_l}, angle={angle_deg})"
        )
        obj.Placement.rotate(center, axes[axis_l], float(angle_deg))
        self._doc.recompute()
        return self._ok(
            f"Rotated {body_id} by {angle_deg}° about {axis_l}",
            cad_api=cad_api,
            data={"body_id": body_id, "axis": axis_l, "angle_deg": float(angle_deg)},
        )

    def fillet(self, body_id: str, radius: float) -> ToolResult:
        obj = self._get_object(body_id)
        edges = obj.Shape.Edges
        if not edges:
            return ToolResult(
                ok=False,
                message=f"[FreeCAD] No edges to fillet on {body_id}",
                data={"cad_software": "FreeCAD", "cad_api": "Shape.makeFillet"},
            )
        name = self._next_label("Fillet")
        cad_api = (
            f"Shape.makeFillet({radius}, edges) on '{body_id}' → Part::Feature '{name}'"
        )
        filleted = obj.Shape.makeFillet(radius, edges)
        out = self._doc.addObject("Part::Feature", name)
        out.Shape = filleted
        self._remove_body(body_id)
        self._doc.recompute()
        return self._ok(
            f"Filleted {body_id} → {out.Name} (r={radius})",
            cad_api=cad_api,
            data={"body_id": out.Name, "radius": radius, "removed": [body_id]},
        )

    def list_bodies(self) -> ToolResult:
        bodies = [
            {"body_id": o.Name, "label": o.Label, "type": o.TypeId}
            for o in self._doc.Objects
            if hasattr(o, "Shape")
        ]
        return self._ok(
            f"{len(bodies)} bodies in document",
            cad_api="Document.Objects (Part::Feature)",
            data={"bodies": bodies},
            sync=False,
        )

    def _heal_shape_for_export(self, shape: Any) -> Any:
        """Apply robust healing to ensure STEP export produces valid, readable files.
        
        Matches the healing strategy used by batch_eval probe to ensure exported
        STEP files pass freecadcmd isValid() + volume>0 checks.
        """
        if shape.isNull():
            return shape
        
        # Fix invalid shapes
        if not shape.isValid():
            try:
                shape.fix()
            except Exception:
                pass
        
        # Extract and fuse multiple solids into one when possible
        solids = list(shape.Solids) if hasattr(shape, "Solids") else []
        if len(solids) == 1:
            shape = solids[0]
        elif len(solids) > 1:
            # Fuse all solids into a single compound
            try:
                fused = solids[0]
                for s in solids[1:]:
                    fused = fused.fuse(s)
                # Validate and fix the fused result
                if not fused.isValid():
                    try:
                        fused.fix()
                    except Exception:
                        pass
                # Only use fused shape if it's valid AND has volume
                if fused.isValid() and float(getattr(fused, "Volume", 0.0) or 0.0) > 0:
                    shape = fused
            except Exception:
                # Fall back to original shape if fusion fails
                pass
        
        # Final validation: fix one more time if still invalid
        if not shape.isValid():
            try:
                shape.fix()
            except Exception:
                pass
        
        return shape

    def export(self, body_id: str, path: str, fmt: str = "step") -> ToolResult:
        out = Path(path)
        root_out = Path(__file__).resolve().parents[3] / "outputs"
        try:
            out = out if out.is_absolute() else (Path.cwd() / out)
            out.resolve().relative_to(root_out.resolve())
        except Exception:
            safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in out.name) or "kala_export.step"
            if not safe.lower().endswith((".step", ".stp", ".stl", ".brep")):
                safe += ".step"
            out = root_out / safe
        out.parent.mkdir(parents=True, exist_ok=True)
        fmt_l = fmt.lower()

        # Assembly export: compound every remaining solid
        if str(body_id).upper() in {"*", "ALL", "__ALL__", "ASSEMBLY"}:
            shapes = [
                o.Shape
                for o in self._doc.Objects
                if getattr(o, "Shape", None) is not None and not o.Shape.isNull()
            ]
            if not shapes:
                return ToolResult(ok=False, message="[FreeCAD] No shapes to export")
            shape = shapes[0]
            for extra in shapes[1:]:
                shape = shape.fuse(extra)
            # Heal the assembled shape before export
            shape = self._heal_shape_for_export(shape)
            # fuse for export only — do not mutate document
            n = len(shapes)
            cad_api = f"compound/fuse of {n} bodies → heal → exportStep"
            if fmt_l in {"step", "stp"}:
                shape.exportStep(str(out))
            elif fmt_l == "stl":
                shape.exportStl(str(out))
            else:
                return ToolResult(ok=False, message=f"[FreeCAD] Unsupported format: {fmt}")
            return self._ok(
                f"Exported assembly ({n} bodies) → {out}",
                cad_api=cad_api,
                data={"path": str(out), "body_id": "ASSEMBLY", "fmt": fmt_l, "body_count": n},
            )

        obj = self._get_object(body_id)
        shape = obj.Shape
        
        # Early validation
        if shape.isNull():
            return ToolResult(ok=False, message=f"[FreeCAD] Empty shape: {body_id}")
        
        # Apply robust healing for single-body export
        shape = self._heal_shape_for_export(shape)
        
        if fmt_l in {"step", "stp"}:
            cad_api = f"Shape → heal → exportStep('{out}')"
            shape.exportStep(str(out))
        elif fmt_l == "stl":
            cad_api = f"Shape.exportStl('{out}')"
            shape.exportStl(str(out))
        elif fmt_l == "brep":
            cad_api = f"Shape.exportBrep('{out}')"
            shape.exportBrep(str(out))
        else:
            return ToolResult(
                ok=False,
                message=f"[FreeCAD] Unsupported format: {fmt}",
                data={"cad_software": "FreeCAD"},
            )
        return self._ok(
            f"Exported {body_id} → {out}",
            cad_api=cad_api,
            data={"path": str(out), "body_id": body_id, "fmt": fmt_l},
        )
