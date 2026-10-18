# Kala screw jack — two FreeCAD builds

Target: **2,000 kg vertical manual screw jack, 120 mm lift**, Tr30×4 single-start right-hand screw. Both routes were built in the user's existing FreeCAD 1.1.3 / Kala session. Codex supplied the design reasoning. Kala's external reasoning and decision gateways were bypassed for these builds, with the gateway restored afterward. The original open document was preserved.

**Engineering prototype; not released for manufacture or vehicle lifting.** Geometry verification establishes shape and sampled motion validity. It does not establish a certified 2-tonne capacity. The most immediate unfinished mechanical detail is positive retention of the removable travel-stop collar. Production release also requires final fits, tolerances, weld and fastening details, saddle interface, independent engineering review and physical qualification. Closed height is 328 mm, so vehicle lift-point clearance must be checked.

## Open the models

- [Native Kala tool build](tool_based/screw-jack-tool.FCStd) — 386 validated calls, fully constrained sketches and editable PartDesign features. Sixteen CAD bodies represent fifteen manufacturing components because the integral spindle is split into two editable regions. The external helix is modeled; the internal nut thread uses a clearly labeled clearance envelope.
- [Direct Python script build](script_based/screw_jack_script.FCStd) — fourteen assembly components, integral spindle/head, paired external/internal native helical geometry, a rounded base and housing gussets. Edit `PARAMETERS` in the script to regenerate the main assembly. Separate editable thread construction files are in `script_based/parts/`.
- Neutral assemblies: [tool STEP](tool_based/screw-jack-tool.step) and [script STEP](script_based/screw_jack_script.step).
- [Dimensioned general arrangement](general_arrangement.svg) and [engineering calculations/report](ENGINEERING_BASIS.md).
- [Script BOM](script_based/BOM.csv), [common design basis](design_basis.json), [calculation results](engineering_results.json), and [independent geometry verification](assembly_verification.json).

The two routes share the load/stroke and principal axial/radial design controls. They are not identical manufacturing models: base corners, mounting-hole diameters, bolt-pattern orientation, grips, gussets, thread detail and stop-collar representation differ. Native thread end run-outs also differ. The general arrangement is a common reference, not a toleranced fabrication drawing.

## Rebuild and inspect

From this repository, the tool route uses the project's Python environment:

```bash
.venv/bin/python outputs/screw_jack_20261017/tool_build.py
```

The direct script uses the system Python compatible with the installed FreeCAD:

```bash
/usr/bin/python3 outputs/screw_jack_20261017/script_build.py
/usr/bin/python3 outputs/screw_jack_20261017/verify_cad.py
/usr/bin/python3 outputs/screw_jack_20261017/engineering_calculations.py
```

To run both in the existing GUI again, execute `build_in_kala.FCMacro`. It creates new documents and blocks Kala model API calls only inside a `try/finally` scope. Existing build files can be overwritten on regeneration; FreeCAD may retain backups. `present_builds.FCMacro` selects and captures the finished assembly documents without rebuilding geometry.

`calls.json`, `tool_based/request.json`, and `tool_based/tool_trace.log` preserve the native tool procedure. `script_build.py` and `script_based/construction.json` describe the Python route. `session_execution.json` records the connected process, build completion, zero model-gateway attempts and restoration of routing.

The independent verifier reopens saved FCStd and STEP files, checks single-solid components and the Ø30 screw envelope, and tests all assembly pairs at 0, 30, 60, 90 and 120 mm lift with coupled screw rotation. Sampling five poses is not a continuous collision proof. Hand calculations use 19.62 kN working load and an explicitly assumed 29.43 kN proof load; the 1.5 factor is not claimed to come from a reviewed standard.

## Production release work

Finalize the collar's positive retention, nut location and antirotation, thread tolerances/gauging/run-outs, bearing installation, saddle angular accommodation and vehicle interface, fastener locking/preload, welds, corrosion protection, lubrication and guarding. Review eccentric loading, stability, fatigue, wear and load holding. Complete toleranced drawings, traceable material/process specifications, inspection and applicable-standard compliance, then perform physical load, overload, stability and endurance tests. See `ENGINEERING_BASIS.md` for the detailed assumptions and source references.
