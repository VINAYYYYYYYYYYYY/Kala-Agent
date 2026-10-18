# Independent engineering review

Engineering review only. No production release or vehicle-lifting approval is established.

The reviewed source files are `tool_build.py`, `script_build.py`, `design_basis.json` and `engineering_calculations.py`. The numerical CAD acceptance record is `assembly_verification.json` when present; its pass/fail values, generated from the final saved CAD, take precedence over any source-level expectation below. Five sampled poses are a geometric check, not a physical motion, durability or load test.

## Final arrangement and assembly logic

- The normal compressive path is saddle → thrust-bearing races → integral spindle shoulder → screw/nut flanks → bronze-nut flange → support ring → housing → base → foundation. The native-tool route represents the monolithic spindle in two touching CAD bodies; these are one physical manufacturing component, rather than an unspecified detachable head joint.
- The Ø80 bronze flange bears on the Ø90 support ring at z212. Main axial compression is not assigned to the four M6 retaining screws. Their present Ø5 tap-drill / Ø4.9 thread-root geometry is symbolic; threaded engagement, locking, preload and edge stresses remain unqualified.
- The nut can be installed from the lower screw end before the removable stop collar. The collar is a nominal envelope or split pair, with attachment hardware absent. Its apparent shoulder contact must not be interpreted as a retained or rated stop.
- Housing bore Ø56 clears the moving Ø36 collar across the stroke. No lower guide blocks that swept path. At 120 mm extension the collar top and nut-skirt underside both reach z160; the screw's thread start remains 4 mm below the first nut thread at z164. At zero extension the collar bottom z34 remains 22 mm above the base top z12.
- Bearing and saddle install from the upper journal end. The axial cap is above the journal, with nominal 0.2 mm axial and 0.5 mm radial clearance to the saddle counterbore. A transverse pin through the saddle is not used. This permits relative rotation about the screw axis; it does not establish angular articulation for an inclined vehicle lift point.
- The Ø16 handle clears the Ø17 cross-bore. Handle retention, off-center sliding positions, end grips and local cross-bore stress still require detailed design. The 250 mm lever used in calculations refers to the centered handle pose.

## Baseline controls and material assumptions

The common target is 2,000 kg, gravity 9.81 m/s², 120 mm stroke, 230 mm nominal thread length and saddle heights 328/388/448 mm. The nominal root is Ø25.5; analytical strength uses a minimum Ø25.0 assumption that requires supplier dimensional verification. The final stop-collar bore is Ø25.7.

Native and script CAD are equivalent architecture demonstrations with deliberate detail differences: base corner/holes, gussets, grip envelopes and feature representation. Their exported part lists must be used for manufacturing interpretation; this package does not define an interchangeable released drawing set.

42CrMo4 +QT, structural steel and bronze are candidate material descriptions. Analytical yield and bronze allowables are stated assumptions, not material certificates. Source-level material labels do not prove heat treatment, weldability, fatigue strength or wear resistance.

## Most consequential remaining release gates

1. **Positive travel-stop retention:** the present collar has no qualified attachment. Loss of the stop can permit excessive unscrewing and eventual thread disengagement. Collar grip, overtravel torque, attachment strength and nut-skirt bearing strength require engineering and tests.
2. **Nut location and guidance:** Ø54 nut body in Ø56 housing leaves 2 mm diametral clearance. A fitted pilot/bushing and positive antirotation/load alignment must be specified; nominal bolt holes do not establish a precision guide.
3. **Actual vehicle and foundation interface:** verify the intended lift-point height, seat shape, angular accommodation, load eccentricity, base contact and stability. The 328 mm closed height limits vehicle compatibility. No lateral-load or slope operating rating is established.
4. **Bearing and load holding:** candidate NSK51305 passes the project's basic static-rating screen, but installation fit, alignment, hardened seats, sealing, drag and life remain unqualified. Ideal self-locking depends on friction; verify actual lubrication/wear/vibration behavior and provide qualified load-holding protection from the hazard assessment.
5. **Manufacturing and physical validation:** finish thread tolerances/gauges/runouts, shoulder fillets, retainer selection, bolt preload, weld procedure, corrosion protection, drawings, traceable BOM and inspection plan; perform the applicable independent review and proof/overload, stability, endurance, wear and environmental tests. The 1.5 load multiplier remains an explicit project assumption until the applicable standard is fully reviewed.

See `ENGINEERING_BASIS.md` for equations, sensitivity results and primary-source scope. See `general_arrangement.svg` for the common dimensional arrangement; it is expressly not a fabrication drawing.
