# Conductor's real OCP smoke test

October 18, 2026: project `.venv` has cadquery-ocp 8.0.1.1.0. All three native primitive shapes were exported and reimported successfully using these actual binding calls, not a tessellated shell:

- Box: `BRepPrimAPI_MakeBox(gp_Pnt(-10,-15,-20),20,30,40).Shape()`.
- Cylinder aligned Z and centered: `BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0,0,-15),gp_Dir(0,0,1)),10,30).Shape()`.
- Sphere: `BRepPrimAPI_MakeSphere(gp_Pnt(0,0,0),10).Shape()`.
- `STEPControl_Writer().Transfer(shape, STEPControl_AsIs)` returns `IFSelect_RetDone`; same writer `.Write(str(tempfile))` returns `IFSelect_RetDone`.
- `STEPControl_Reader().ReadFile(str(tempfile))` returns `IFSelect_RetDone`; `.TransferRoots()>0`; `.OneShape()` passes `BRepCheck_Analyzer(shape).IsValid()`.
- `properties=GProp_GProps(); BRepGProp.VolumeProperties_s(shape, properties); properties.Mass()` gives 24,000 mm^3 box, 9,424.777960769381 mm^3 cylinder and 4,188.790204786392 mm^3 sphere after STEP roundtrip, within 1e-7 relative of analytic volume.
- STEP output sizes were 15,518 / 5,674 / 2,123 bytes respectively.

Imports come from `OCP.BRepPrimAPI`, `OCP.gp`, `OCP.STEPControl`, `OCP.IFSelect`, `OCP.BRepCheck`, `OCP.BRepGProp`, `OCP.GProp`.

This verifies upstream API usability. It does NOT yet verify Studio export route uses it. The backend should wire these supported primitive paths, validate before dispatch, use isolated temporary file names and bounded readback, and test route output through the reader. Primitive parameters must be preserved to export exact B-rep rather than a faceted STEP from preview triangles.
