# Preliminary engineering basis — 2,000 kg screw jack

**Engineering prototype. Not released for manufacture or vehicle lifting.**

The two CAD approaches share one dimensional basis. Neither valid CAD solids nor these hand calculations establish a production rating. The nominal working mass is a project target, not a certified capacity. ASME's public PASE-2024 scope includes automotive mechanical jacks; the full standard has not been reviewed. The 1.5 load multiplier is a project assumption and is not attributed to a standard.

## Common geometry

All lengths are in millimetres. The base underside is z=0. Displayed CAD is at 60 mm extension. Details and axial coordinates are in `design_basis.json`.

- Base 200 × 180 × 12; housing Ø70/Ø56, z12–212; top support ring Ø90/Ø56, z204–212.
- Bronze candidate nut Ø54 with 48 mm thread, z164–212, flange Ø80 with clearance bore Ø32, z212–220. Retaining screws are four M6 on PCD66. A fitted locating/antirotation solution must resolve the present 2 mm diametral clearance.
- Single-start right-hand Tr30×4; Ø30 major, Ø28 pitch and Ø25.5 **basic nominal** root. Strength calculations assume a certified minimum Ø25.0 root. Thread length 230, displayed z100–330, collapsed z40–270, raised z160–390.
- Travel 120 mm, 30 turns. Saddle top height 328 collapsed / 388 displayed / 448 raised. This high closed height limits vehicle applicability; check the intended vehicle's lift point clearance.
- Drive head Ø48 × 28, cross-bore Ø17; handle Ø16 × 500, nominal maximum centered lever arm 250. Candidate NSK51305 bearing envelope Ø52/Ø25 × 18. Journal Ø25, z358–386, with saddle clearance bore Ø25.5. Saddle Ø70 × 12, z376–388, requires a qualified vehicle contact and angular seat.
- Removable end collar Ø36/Ø25.7 × 6 beneath screw, displayed z94–100. Split/clamp retention and fasteners are unresolved; no rated cross-pin is claimed. At raised travel its top z160 meets the nut skirt underside z160. Screw thread start remains 4 mm below the nut's first thread. Nut must be threaded on before collar installation. Attachment, runout, retention and stop strength still need verification.

## Loads, screw and nut screening

Working axial load = 2,000 × 9.81 = **19.62 kN**. Assumed proof load = 1.5 × working = **29.43 kN**.

For root diameter d=25.0, A=πd²/4 and I=πd⁴/64. Nominal axial stress F/A at proof is 60.0 MPa. With conservative full input torque, torsion τ=16T/(πd³), combined stress √(σ²+3τ²) is 80.5 MPa. Against the **assumed certified** minimum yield 650 MPa this is a nominal margin of 8.08. It excludes root concentration, fatigue and manufacturing effects.

A 15 mm load eccentricity produces bending 32Fe/(πd³). Combined proof stress then rises to **352 MPa**, giving only 1.85 nominal yield margin. No lateral working-load rating is established.

Buckling uses a deliberately conservative 250 mm unsupported column and K=2, so effective length is 500 mm. Euler π²EI/(KL)² gives 155.2 kN. The lower Euler/Johnson screening value is 155.1 kN, or 5.27 times the assumed proof load. Column imperfections and guide compliance are not resolved.

Twelve nut turns engage. Mean projected flank pressure 2F/(πd₂L) is **13.9 MPa** at proof; reducing effective engagement to 60% raises it to 23.2 MPa. The screening bronze bearing allowable 30 MPa is an unqualified material assumption. Nominal root-thread shear 2F/(πdL) is 15.6 MPa, rising to 26.0 MPa with 60% engagement. First-thread loading, flank bending and wear remain unresolved.

## Torque and self-locking

Lead angle λ=atan(lead/(πd₂))=2.604°. For the 15° half-flank angle, μ′=μ/cos15°, raising torque is Fd₂/2 × (tanλ+μ′)/(1−μ′tanλ). A separate rolling-bearing friction surrogate adds μ_bFr_m with μ_b=0.02; it does not qualify a real bearing.

At assumed thread μ=0.15, total raising torque is 63.4 N·m at working load and 95.1 N·m at assumed proof. Corresponding hand forces at 250 mm are **254 N working / 380 N proof**. Ergonomics and actual torque must be tested. The nominal proof handle bending stress is 236 MPa before contact/concentration effects.

Self-locking requires μ > cos15° × tanλ = **0.0439**. The assumed 0.08–0.15 range passes that ideal static criterion, but the adverse μ=0.03 case backdrives. Lubricant, vibration, wear and dynamic lowering invalidate any unconditional self-locking claim. A hazard review and qualified load-holding protection remain required.

The selected catalogue candidate **NSK51305** is 25/52/18 with dynamic Ca=36 kN and static C0a=61.5 kN. C0a/proof=2.09, meeting the project's preliminary 2×proof screening target. Manufacturer abutment requirements are shaft diameter≥41, housing abutment bore≤36 and fillet radius≤1. The Ø48 drive head and Ø25.5 saddle bore meet those nominal diameter bounds; detailed seats and fit remain unqualified. Nominal proof bearing pressures on head and saddle seat are 22.3 / 19.0 MPa. Normal thrust passes through the races and shoulder rather than the locating journal; a conservative all-axial-load journal stress bound is 60.0 MPa. The bearing cannot carry radial load. Supplier life, hardened seats, alignment, sealing, retention and measured drag still require verification. A plain sliding washer requires a different torque calculation.

## Base, load path and stop concerns

Average centered foundation pressure at assumed proof is 0.818 MPa. A basic uniformly loaded plate-strip model gives 72.0 MPa bending stress in the 12 mm base. Actual corner uplift, concentrated ground contact, weld distortion and eccentricity can dominate; this is not a plate qualification.

At full height, a 15 mm offset plus a 3° slope sensitivity shifts the resultant approximately 38.5 mm toward the 90 mm tipping edge. Adding a horizontal force equal to 5% of load shifts it to approximately 60.9 mm and sharply increases contact pressure. These are **sensitivity cases, not permitted operating conditions or a stability rating**. Require a hard level foundation and qualified lift-point alignment. The present high closed height and base dimensions need a vehicle-specific stability review.

The end stop contact annulus has only 213.6 mm² area. If an adverse misuse transferred the complete proof load through it, nominal contact stress would reach 138 MPa. Therefore a modeled stop does not establish a rated stop: its retention, bronze skirt, overtravel torque and assembly sequence must be qualified.

The intended axial load path is vehicle saddle → selected thrust bearing → screw → bronze nut → flange/top support ring → housing → base → foundation. Retaining M6 screws are not credited as the main compressive load path. Joint, weld, flange/ring bending and hole stresses need detailed review.

## Release work

- Qualify candidate NSK51305 bearing for actual load/life/static safety, races, shaft/seat tolerances, sealing, misalignment and drag; catalogue rating alone does not qualify installation.
- Qualify actual screw/nut thread tolerance, minimum root, runout, flank contact and bronze wear/PV limits; CAD thread envelope is not a tolerance specification.
- Resolve 2 mm diametral nut-to-housing clearance using a positive locating pilot/bushing and antirotation feature; retaining bolts alone must not be assumed a precision guide.
- Engineer saddle angular accommodation, vehicle lift-point interface, retention and load eccentricity; flat CAD saddle is only an envelope.
- Design and qualify end-stop collar retention, nut-skirt contact and overload behavior; CAD touching faces do not prove stop strength.
- Provide qualified load-holding protection: verify self-locking across lubrication/wear/vibration and design an independent restraint as required by hazard analysis.
- Validate 12 mm base under eccentric/nonuniform ground contact, welds, local ring/flange bending, bolt holes and housing stability.
- Resolve accessible pinch points, handle end stops/retention, maximum-travel indication, corrosion, lubrication and service interval.
- Create fully toleranced manufacturing drawings, material/heat-treatment/weld procedures, traceable BOM, inspection plan and current-market standards compliance matrix.
- Conduct independent structural review, proof/overload, stability, load-holding, endurance, wear, misuse and environmental tests before product release.

## Sources and reproducibility

- [ASME publisher PASE product and scope](https://www.asme.org/codes-standards/find-codes-standards/safety-standard-for-portable-automotive-service-equipment): Automotive mechanical-jack scope and 2024 edition; no detailed proof requirement inferred.
- [NSK 51103 manufacturer specifications](https://www.nsk.com/engineering/products/bearings/ball-bearings/thrust-ball-bearings/single-direction-thrust-ball-bearings/51103-apn.html): Example 17x30x9 thrust bearing C0a=19500N is insufficient for this project's 29430N proof assumption; not a chosen component.
- [NSK 51305 manufacturer specifications](https://www.nsk.com/engineering/51305-apn.html): Selected candidate dimensions 25x52x18; Ca=36000N, C0a=61500N; da>=41, Da<=36, fillet<=1mm; application qualification pending.
- [Schaeffler trapezoidal-screw catalogue supplied as design reference](https://www.schaeffler.com/remotemedien/media/_shared_media/08_media_library/01_publications/schaeffler_2/catalogue_1/downloads_6/la1_de_en.pdf): Reference only; web retrieval failed because file exceeds tool size limit; exact thread tolerances still to verify.

Run `python3 engineering_calculations.py` to regenerate this report and both JSON records. Equations are visible in the script. Results are analytical screening values and are separate from the CAD kernel's geometry checks.
