"""FreeCadFemCalculiXBackend — FEM analysis using FreeCAD Fem module and CalculiX solver."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from kala.analysis.base import AnalysisReport, AnalysisRequest


class FreeCadFemCalculiXBackend:
    """FEM analysis backend using FreeCAD Fem workbench and CalculiX solver.
    
    Performs structural analysis on CAD bodies when FreeCAD Fem module is available.
    Soft-fails gracefully when dependencies are missing — never raises exceptions.
    """

    def analyze(self, request: AnalysisRequest) -> AnalysisReport:
        """Run FEM analysis on a CAD body.
        
        Args:
            request: Analysis request with body_id and backend_handle
            
        Returns:
            AnalysisReport with FEM results or soft-fail status
        """
        body_id = request.body_id
        backend_handle = request.backend_handle

        if backend_handle is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message="No backend handle provided",
                metrics={},
                kind="freecad_fem_calculix",
                solver_status="no_backend",
            )

        try:
            return self._run_fem_analysis(body_id, backend_handle)
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"FEM analysis failed: {exc}",
                metrics={},
                kind="freecad_fem_calculix",
                solver_status="error",
            )

    def _run_fem_analysis(self, body_id: str, backend: Any) -> AnalysisReport:
        """Execute FEM analysis workflow."""
        doc = getattr(backend, "_doc", None)
        if doc is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message="Backend has no _doc attribute",
                metrics={},
                kind="freecad_fem_calculix",
                solver_status="no_doc",
            )

        obj = doc.getObject(body_id)
        if obj is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Body not found: {body_id}",
                metrics={},
                kind="freecad_fem_calculix",
                solver_status="body_not_found",
            )

        shape = getattr(obj, "Shape", None)
        if shape is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Body {body_id} has no Shape attribute",
                metrics={},
                kind="freecad_fem_calculix",
                solver_status="no_shape",
            )

        # Preflight geometry checks
        try:
            is_valid = shape.isValid()
            volume = float(shape.Volume) if hasattr(shape, "Volume") else None
            
            if not is_valid:
                return AnalysisReport(
                    body_id=body_id,
                    ok=False,
                    message=f"Geometry is invalid for {body_id}",
                    metrics={"is_valid": is_valid, "volume": volume},
                    kind="freecad_fem_calculix",
                    solver_status="invalid_geometry",
                )
            
            if volume is not None and volume <= 0.0:
                return AnalysisReport(
                    body_id=body_id,
                    ok=False,
                    message=f"Geometry has zero or negative volume: {volume}",
                    metrics={"is_valid": is_valid, "volume": volume},
                    kind="freecad_fem_calculix",
                    solver_status="zero_volume",
                )
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Preflight geometry check failed: {exc}",
                metrics={},
                kind="freecad_fem_calculix",
                solver_status="preflight_error",
            )

        # Check for FreeCAD Fem module availability
        try:
            import Fem  # noqa: F401
            import ObjectsFem  # noqa: F401
        except ImportError:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message="FreeCAD Fem module not available (skipped_no_wb)",
                metrics={"is_valid": True, "volume": volume},
                kind="freecad_fem_calculix",
                solver_status="skipped_no_wb",
            )

        # Attempt to run FEM analysis
        try:
            return self._execute_fem(doc, obj, body_id, volume)
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"FEM execution failed: {exc}",
                metrics={"is_valid": True, "volume": volume},
                kind="freecad_fem_calculix",
                solver_status="execution_error",
            )

    def _execute_fem(
        self, doc: Any, solid_obj: Any, body_id: str, volume: float | None
    ) -> AnalysisReport:
        """Execute the FEM analysis using FreeCAD Fem objects."""
        import ObjectsFem

        metrics: dict[str, Any] = {
            "is_valid": True,
            "volume": volume,
        }

        # Create FEM analysis container
        analysis = ObjectsFem.makeAnalysis(doc, f"FemAnalysis_{body_id}")
        
        # Create solver object (CalculiX)
        try:
            solver = ObjectsFem.makeSolverCalculix(doc, f"SolverCalculiX_{body_id}")
            output_dir = Path("outputs") / "fem" / body_id
            output_dir.mkdir(parents=True, exist_ok=True)
            solver.WorkingDir = str(output_dir)
            analysis.addObject(solver)
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Failed to create CalculiX solver: {exc}",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="solver_creation_failed",
            )

        # Add minimal default material (Solids) required by CalculiX
        try:
            material = ObjectsFem.makeMaterialSolid(doc, f"Solids_{body_id}")
            material_dict = {
                "Name": "Solids",
                "YoungsModulus": "210000 MPa",
                "PoissonRatio": "0.30",
                "Density": "7900 kg/m^3",
            }
            material.Material = material_dict
            # Soft bind: empty refs = all geometry (OK for single material).
            # When Shape has Solids, bind Solid1..SolidN like FreeCAD FEM examples.
            try:
                solids = getattr(solid_obj.Shape, "Solids", None) or []
                if solids:
                    material.References = [
                        (solid_obj, f"Solid{i + 1}") for i in range(len(solids))
                    ]
            except Exception:  # noqa: BLE001
                pass
            analysis.addObject(material)
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Failed to create material: {exc}",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="material_creation_failed",
            )

        # Create mesh
        try:
            mesh_obj = ObjectsFem.makeMeshGmsh(doc, f"FemMesh_{body_id}")
            mesh_obj.Part = solid_obj
            mesh_obj.CharacteristicLengthMax = "10.0 mm"
            analysis.addObject(mesh_obj)
            doc.recompute()
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Failed to create mesh: {exc}",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="mesh_creation_failed",
            )

        # Add minimal constraints (fixed constraint at base + self-weight load)
        # Constraints remain incomplete without actual face References
        constraints_incomplete = True
        try:
            fixed = ObjectsFem.makeConstraintFixed(doc, f"ConstraintFixed_{body_id}")
            face_name = self._pick_fixed_face(solid_obj)
            if face_name:
                fixed.References = [(solid_obj, face_name)]
            analysis.addObject(fixed)

            if hasattr(fixed, "References") and fixed.References:
                constraints_incomplete = False

            gravity = ObjectsFem.makeConstraintSelfWeight(doc, f"ConstraintGravity_{body_id}")
            analysis.addObject(gravity)

            doc.recompute()
        except Exception:  # noqa: BLE001
            pass

        # Write FEM report
        report_path = output_dir / "fem_report.json"
        report_data = {
            "body_id": body_id,
            "analysis_type": "freecad_fem_calculix",
            "solver": "CalculiX",
            "mesh_created": True,
            "constraints_complete": not constraints_incomplete,
            "volume": volume,
        }

        # Export STEP file for reproducibility (optional)
        try:
            step_path = output_dir / "solid.step"
            solid_obj.Shape.exportStep(str(step_path))
            report_data["step_export"] = str(step_path)
        except Exception:  # noqa: BLE001
            pass

        # Check if solver can actually run
        solver_available = self._check_solver_available()
        
        if not solver_available:
            report_data["solver_available"] = False
            report_data["note"] = "CalculiX solver binary not available"
            
            try:
                with open(report_path, "w") as f:
                    json.dump(report_data, f, indent=2)
            except Exception:  # noqa: BLE001
                pass

            message = "FEM setup complete but CalculiX solver not available (skipped_no_wb)"
            if constraints_incomplete:
                message += " - constraints incomplete (needs face selection)"

            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=message,
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="skipped_no_wb",
            )

        # Attempt to run real CalculiX solver
        try:
            from femtools import ccxtools
            
            fea = ccxtools.FemToolsCcx(solver)
            fea.update_objects()
            
            check_msg = fea.check_prerequisites()
            if check_msg:
                report_data["solver_prerequisites_failed"] = check_msg
                try:
                    with open(report_path, "w") as f:
                        json.dump(report_data, f, indent=2)
                except Exception:  # noqa: BLE001
                    pass
                
                status = "constraints_incomplete" if constraints_incomplete else "setup_only"
                return AnalysisReport(
                    body_id=body_id,
                    ok=False,
                    message=f"FEM prerequisites failed: {check_msg}",
                    metrics=metrics,
                    kind="freecad_fem_calculix",
                    solver_status=status,
                )
            
            fea.purge_results()
            fea.write_inp_file()
            fea.ccx_run()
            fea.load_results()
            
            report_data["solver_run"] = "completed"
            
            if hasattr(fea, "ccx_stdout") and fea.ccx_stdout:
                report_data["ccx_has_output"] = True
            
            try:
                with open(report_path, "w") as f:
                    json.dump(report_data, f, indent=2)
            except Exception:  # noqa: BLE001
                pass
            
            return self._extract_results(doc, analysis, body_id, metrics, report_path, fea)
            
        except Exception as exc:  # noqa: BLE001
            report_data["solver_run_error"] = str(exc)
            
            try:
                with open(report_path, "w") as f:
                    json.dump(report_data, f, indent=2)
            except Exception:  # noqa: BLE001
                pass

            status = "constraints_incomplete" if constraints_incomplete else "setup_only"
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Solver execution failed: {exc}",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status=status,
            )


    def _pick_fixed_face(self, solid_obj: Any) -> str | None:
        """Pick FaceN with lowest CenterOfMass.z as a soft gravity-base fixed face.

        FreeCAD face names are 1-based (Face1 == Shape.Faces[0]). Returns None when
        no usable face exists so callers keep constraints_incomplete=True.
        """
        try:
            shape = getattr(solid_obj, "Shape", None)
            faces = getattr(shape, "Faces", None) if shape is not None else None
            if not faces:
                return None
            best_i: int | None = None
            best_z: float | None = None
            for i, face in enumerate(faces):
                try:
                    z = float(face.CenterOfMass.z)
                except Exception:  # noqa: BLE001
                    continue
                if best_z is None or z < best_z:
                    best_z = z
                    best_i = i
            if best_i is None:
                return None
            return f"Face{best_i + 1}"
        except Exception:  # noqa: BLE001
            return None

    def _extract_results(
        self, doc: Any, analysis: Any, body_id: str, metrics: dict[str, Any], report_path: Path, fea: Any = None
    ) -> AnalysisReport:
        """Extract real post-process metrics from FEM result objects.
        
        vonMises values are in MPa (CalculiX default units, verified in FreeCAD FEM source).
        """
        result_objects = [obj for obj in analysis.Group if hasattr(obj, "Mesh") and hasattr(obj, "vonMises")]
        
        if not result_objects:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message="No result objects found after solver run",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="no_results",
            )
        
        result_obj = result_objects[0]
        
        try:
            von_mises_values = result_obj.vonMises if hasattr(result_obj, "vonMises") else []
            
            if not von_mises_values or len(von_mises_values) == 0:
                return AnalysisReport(
                    body_id=body_id,
                    ok=False,
                    message="Result object has no von Mises stress data",
                    metrics=metrics,
                    kind="freecad_fem_calculix",
                    solver_status="no_stress_data",
                )
            
            max_von_mises = max(von_mises_values)
            min_von_mises = min(von_mises_values)
            avg_von_mises = sum(von_mises_values) / len(von_mises_values)
            
            metrics["max_von_mises_stress_mpa"] = float(max_von_mises)
            metrics["min_von_mises_stress_mpa"] = float(min_von_mises)
            metrics["avg_von_mises_stress_mpa"] = float(avg_von_mises)
            metrics["num_nodes"] = len(von_mises_values)
            metrics["fem_report_path"] = str(report_path)
            
            if hasattr(result_obj, "DisplacementLengths"):
                disp_values = result_obj.DisplacementLengths
                if disp_values and len(disp_values) > 0:
                    metrics["max_displacement_mm"] = float(max(disp_values))
            
            if fea and hasattr(fea, "ccx_stdout"):
                metrics["ccx_stdout_available"] = bool(fea.ccx_stdout)
            
            return AnalysisReport(
                body_id=body_id,
                ok=True,
                message=f"FEM analysis completed for {body_id}",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="completed",
            )
            
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Failed to extract stress metrics: {exc}",
                metrics=metrics,
                kind="freecad_fem_calculix",
                solver_status="result_parse_failed",
            )

    def _check_solver_available(self) -> bool:
        """Check if CalculiX solver binary is available."""
        try:
            import subprocess
            
            result = subprocess.run(
                ["which", "ccx_static"],
                capture_output=True,
                timeout=2,
            )
            if result.returncode == 0:
                return True
            
            result = subprocess.run(
                ["which", "ccx"],
                capture_output=True,
                timeout=2,
            )
            return result.returncode == 0
            
        except Exception:  # noqa: BLE001
            return False
