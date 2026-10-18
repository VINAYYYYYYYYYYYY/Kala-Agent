#!/usr/bin/env python3
"""Repeatable preliminary screw-jack hand calculations; N, mm, MPa.

This computes an engineering screening model, not a product rating or a
standards conformity assessment. It creates design_basis.json,
engineering_results.json and ENGINEERING_BASIS.md beside this script.
"""
from __future__ import annotations

import json
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent
PI = math.pi

BASIS = {
    "revision": "2026-10-17-B",
    "status": "ENGINEERING PROTOTYPE — NOT RELEASED FOR MANUFACTURE OR VEHICLE LIFTING",
    "architecture": "Vertical manual single-start trapezoidal screw bottle jack",
    "units": {"length": "mm", "force": "N", "stress": "MPa", "torque": "N m"},
    "target_working_mass_kg": 2000,
    "gravity_m_s2": 9.81,
    "proof_load_multiplier_project_assumption": 1.5,
    "travel_mm": 120,
    "displayed_travel_mm": 60,
    "saddle_height_collapsed_mm": 328,
    "saddle_height_displayed_mm": 388,
    "saddle_height_raised_mm": 448,
    "datum": "Base underside z=0; XY centered on screw; displayed pose is 60 mm above collapsed",
    "geometry": {
        "base": {"length_x": 200, "width_y": 180, "thickness": 12, "z_min": 0, "z_max": 12},
        "housing": {"outer_diameter": 70, "bore_diameter": 56, "z_min": 12, "z_max": 212},
        "top_support_ring": {"outer_diameter": 90, "inner_diameter": 56, "z_min": 204, "z_max": 212},
        "nut_body": {"outer_diameter": 54, "thread": "Tr30x4 RH single-start", "engagement": 48,
                     "z_min": 164, "z_max": 212},
        "nut_flange": {"outer_diameter": 80, "clearance_bore_diameter": 32,
                       "z_min": 212, "z_max": 220, "retaining_bolts": "4 x M6 at PCD66; detailed fastening pending"},
        "nut_stop_skirt": {"outer_diameter": 54, "inner_diameter": 32, "z_min": 160, "z_max": 164},
        "screw": {"major_diameter_basic": 30, "pitch_diameter_basic": 28,
                  "root_diameter_basic": 25.5, "root_diameter_strength_assumption": 25.0,
                  "thread_pitch": 4, "lead": 4, "flank_included_angle_degrees": 30,
                  "thread_length": 230, "displayed_z_min": 100, "displayed_z_max": 330,
                  "collapsed_z_min": 40, "collapsed_z_max": 270,
                  "raised_z_min": 160, "raised_z_max": 390},
        "screw_end_stop_collar": {"outer_diameter": 36, "inner_diameter": 25.7, "z_min": 94, "z_max": 100,
                                  "retention": "Removable collar envelope only; split/clamp retention and fasteners unresolved; no rated pin is claimed",
                                  "assembly": "Install nut from lower screw end before removable collar; integral head and integral collar would trap nut"},
        "drive_head": {"outer_diameter": 48, "z_min": 330, "z_max": 358,
                       "cross_bore_diameter": 17, "cross_bore_center_z": 344},
        "handle": {"diameter": 16, "length": 500, "effective_lever_arm": 250,
                   "axis": "X through drive-head cross-bore", "center_z": 344},
        "thrust_bearing": {"candidate": "NSK 51305", "outer_diameter": 52, "inner_diameter": 25,
                            "z_min": 358, "z_max": 376,
                            "candidate_selected": True, "application_qualified": False,
                            "basic_dynamic_load_rating_N": 36000,
                            "basic_static_load_rating_N": 61500,
                            "minimum_C0a_project_target_N": 58860,
                            "minimum_shaft_abutment_diameter": 41,
                            "maximum_housing_abutment_bore_diameter": 36,
                            "maximum_fillet_radius": 1,
                            "note": "Catalogue candidate with verified dimensions/rating; CAD is envelope only; application, alignment, drag and sealing still unqualified"},
        "saddle_journal": {"diameter": 25, "z_min": 358, "z_max": 386,
                           "retention": "Positive axial saddle retention required; detail pending"},
        "saddle": {"outer_diameter": 70, "bore_diameter": 25.5, "z_min": 376, "z_max": 388,
                   "interface": "Vehicle-specific load seat and angular accommodation unresolved"},
    },
    "material_requirements_are_assumptions": {
        "screw_and_handle": "42CrMo4 / AISI4140 Q&T candidate; certified minimum yield 650 MPa assumed",
        "housing_base_support_ring": "Weldable steel candidate; minimum yield 355 MPa assumed",
        "nut": "Bearing bronze candidate; project screening allowables 30 MPa thread bearing and 60 MPa shear; supplier qualification required",
        "screw_elastic_modulus_MPa": 205000,
    },
    "analysis_assumptions": {
        "thread_friction_range": [0.08, 0.12, 0.15],
        "adverse_low_thread_friction_case": 0.03,
        "rolling_thrust_equivalent_friction": 0.02,
        "strength_screw_root_min_mm": 25.0,
        "screw_unsupported_length_mm": 250,
        "buckling_effective_length_factor": 2,
        "nut_effective_engagement_fraction_sensitivity": 0.6,
        "eccentric_axial_load_sensitivity_mm": 15,
        "slope_sensitivity_degrees_not_an_operating_rating": 3,
        "horizontal_force_fraction_sensitivity_not_allowed_working_load": 0.05,
        "support": "Hard level non-slipping foundation; substrate bearing and local vehicle lift-point qualification excluded",
    },
    "standards_scope": {
        "candidate": "ASME PASE-2024, subject to intended market and classification review",
        "verified_public_scope": "Publisher lists automotive mechanical jacks",
        "full_standard_reviewed": False,
        "proof_multiplier_from_standard": False,
        "claims_of_conformity": "None",
    },
    "sources": [
        {"title": "ASME publisher PASE product and scope", "url": "https://www.asme.org/codes-standards/find-codes-standards/safety-standard-for-portable-automotive-service-equipment", "use": "Automotive mechanical-jack scope and 2024 edition; no detailed proof requirement inferred"},
        {"title": "NSK 51103 manufacturer specifications", "url": "https://www.nsk.com/engineering/products/bearings/ball-bearings/thrust-ball-bearings/single-direction-thrust-ball-bearings/51103-apn.html", "use": "Example 17x30x9 thrust bearing C0a=19500N is insufficient for this project's 29430N proof assumption; not a chosen component"},
        {"title": "NSK 51305 manufacturer specifications", "url": "https://www.nsk.com/engineering/51305-apn.html", "use": "Selected candidate dimensions 25x52x18; Ca=36000N, C0a=61500N; da>=41, Da<=36, fillet<=1mm; application qualification pending"},
        {"title": "Schaeffler trapezoidal-screw catalogue supplied as design reference", "url": "https://www.schaeffler.com/remotemedien/media/_shared_media/08_media_library/01_publications/schaeffler_2/catalogue_1/downloads_6/la1_de_en.pdf", "use": "Reference only; web retrieval failed because file exceeds tool size limit; exact thread tolerances still to verify"},
    ],
    "release_blockers": [
        "Qualify candidate NSK51305 bearing for actual load/life/static safety, races, shaft/seat tolerances, sealing, misalignment and drag; catalogue rating alone does not qualify installation",
        "Qualify actual screw/nut thread tolerance, minimum root, runout, flank contact and bronze wear/PV limits; CAD thread envelope is not a tolerance specification",
        "Resolve 2 mm diametral nut-to-housing clearance using a positive locating pilot/bushing and antirotation feature; retaining bolts alone must not be assumed a precision guide",
        "Engineer saddle angular accommodation, vehicle lift-point interface, retention and load eccentricity; flat CAD saddle is only an envelope",
        "Design and qualify end-stop collar retention, nut-skirt contact and overload behavior; CAD touching faces do not prove stop strength",
        "Provide qualified load-holding protection: verify self-locking across lubrication/wear/vibration and design an independent restraint as required by hazard analysis",
        "Validate 12 mm base under eccentric/nonuniform ground contact, welds, local ring/flange bending, bolt holes and housing stability",
        "Resolve accessible pinch points, handle end stops/retention, maximum-travel indication, corrosion, lubrication and service interval",
        "Create fully toleranced manufacturing drawings, material/heat-treatment/weld procedures, traceable BOM, inspection plan and current-market standards compliance matrix",
        "Conduct independent structural review, proof/overload, stability, load-holding, endurance, wear, misuse and environmental tests before product release",
    ],
}


def compute() -> dict:
    a = BASIS["analysis_assumptions"]
    g = BASIS["geometry"]
    working = BASIS["target_working_mass_kg"] * BASIS["gravity_m_s2"]
    proof = working * BASIS["proof_load_multiplier_project_assumption"]
    d = a["strength_screw_root_min_mm"]
    dp = g["screw"]["pitch_diameter_basic"]
    lead = g["screw"]["lead"]
    area = PI * d**2 / 4
    inertia = PI * d**4 / 64
    tan_lam = lead / (PI * dp)
    alpha = math.radians(g["screw"]["flank_included_angle_degrees"] / 2)
    bearing_ri = g["thrust_bearing"]["inner_diameter"] / 2
    bearing_ro = g["thrust_bearing"]["outer_diameter"] / 2
    # Uniform-pressure annular mean radius; only a friction screening surrogate.
    bearing_rm = (2 / 3) * (bearing_ro**3 - bearing_ri**3) / (bearing_ro**2 - bearing_ri**2)
    torque_cases = []
    for mu in [*a["thread_friction_range"], a["adverse_low_thread_friction_case"]]:
        mu_eq = mu / math.cos(alpha)
        for load_name, f in [("working", working), ("proof_assumption", proof)]:
            raise_thread = f * dp / 2 * (tan_lam + mu_eq) / (1 - mu_eq * tan_lam) / 1000
            lower_thread = f * dp / 2 * (mu_eq - tan_lam) / (1 + mu_eq * tan_lam) / 1000
            bearing_torque = a["rolling_thrust_equivalent_friction"] * f * bearing_rm / 1000
            torque_cases.append({"load": load_name, "thread_mu": mu,
                                 "thread_raise_Nm": raise_thread,
                                 "thread_lower_resisting_Nm": lower_thread,
                                 "bearing_surrogate_Nm": bearing_torque,
                                 "total_raise_Nm": raise_thread + bearing_torque,
                                 "handle_force_N": (raise_thread + bearing_torque) * 1000 / g["handle"]["effective_lever_arm"],
                                 "thread_only_self_locking": mu_eq > tan_lam})
    worst = max(c["total_raise_Nm"] for c in torque_cases if c["load"] == "proof_assumption")
    sigma = proof / area
    tau = 16 * worst * 1000 / (PI * d**3)
    vm = math.sqrt(sigma**2 + 3 * tau**2)
    fy = 650
    ecc = a["eccentric_axial_load_sensitivity_mm"]
    bend = 32 * proof * ecc / (PI * d**3)
    vm_ecc = math.sqrt((sigma + bend)**2 + 3 * tau**2)
    e_mod = BASIS["material_requirements_are_assumptions"]["screw_elastic_modulus_MPa"]
    effective_length = a["buckling_effective_length_factor"] * a["screw_unsupported_length_mm"]
    slenderness = effective_length / math.sqrt(inertia / area)
    euler = PI**2 * e_mod * inertia / effective_length**2
    johnson = area * fy * (1 - fy * slenderness**2 / (4 * PI**2 * e_mod))
    engage = g["nut_body"]["engagement"]
    bearing_area = PI * dp * engage / 2
    thread_shear_area = PI * d * engage / 2
    flange_area = PI / 4 * (g["nut_flange"]["outer_diameter"]**2 - g["top_support_ring"]["inner_diameter"]**2)
    housing_area = PI / 4 * (g["housing"]["outer_diameter"]**2 - g["housing"]["bore_diameter"]**2)
    base = g["base"]
    q_avg = proof / (base["length_x"] * base["width_y"])
    overhang = (max(base["length_x"], base["width_y"]) - g["housing"]["outer_diameter"]) / 2
    strip_stress = 3 * q_avg * overhang**2 / base["thickness"]**2
    h = BASIS["saddle_height_raised_mm"]
    slope_e = h * math.tan(math.radians(a["slope_sensitivity_degrees_not_an_operating_rating"]))
    side_e = h * a["horizontal_force_fraction_sensitivity_not_allowed_working_load"]
    scenarios = []
    for name, e in [("vertical_centered", 0), ("vertical_with_15mm_offset", ecc),
                    ("15mm_offset_plus_3deg_slope_sensitivity", ecc + slope_e),
                    ("offset_slope_plus_5pct_horizontal_sensitivity", ecc + slope_e + side_e)]:
        b = base["width_y"]
        length = base["length_x"]
        if e <= b / 6:
            pmax = q_avg * (1 + 6 * e / b)
            contact_width = b
        elif e < b / 2:
            contact_width = 3 * (b / 2 - e)
            pmax = 2 * proof / (length * contact_width)
        else:
            contact_width, pmax = 0.0, float("inf")
        scenarios.append({"scenario": name, "resultant_offset_mm": e,
                          "distance_to_tipping_edge_mm": b / 2 - e,
                          "geometric_tipping_ratio": None if e == 0 else (b / 2) / e,
                          "contact_width_mm": contact_width, "peak_ground_pressure_MPa": pmax,
                          "plate_strip_stress_scaled_by_peak_pressure_MPa": strip_stress * pmax / q_avg})
    handle_sigma = 32 * worst * 1000 / (PI * g["handle"]["diameter"]**3)
    stop_contact_area = PI / 4 * (g["screw_end_stop_collar"]["outer_diameter"]**2 - g["nut_stop_skirt"]["inner_diameter"]**2)
    return {
        "working_axial_load_N": working, "assumed_proof_axial_load_N": proof,
        "turns_for_full_travel": BASIS["travel_mm"] / lead,
        "thread_lead_angle_degrees": math.degrees(math.atan(tan_lam)),
        "critical_thread_friction_coefficient_for_self_locking": tan_lam * math.cos(alpha),
        "torque_cases": torque_cases,
        "screw": {"strength_diameter_mm": d, "area_mm2": area, "second_moment_mm4": inertia,
                  "proof_axial_stress_MPa": sigma, "conservative_proof_torsional_stress_MPa": tau,
                  "proof_von_mises_centered_MPa": vm, "assumed_yield_margin_centered": fy / vm,
                  "eccentricity_sensitivity_mm": ecc, "proof_bending_stress_MPa": bend,
                  "proof_von_mises_with_eccentricity_MPa": vm_ecc, "assumed_yield_margin_with_eccentricity": fy / vm_ecc,
                  "effective_length_mm": effective_length, "slenderness": slenderness,
                  "Euler_load_N": euler, "Johnson_load_N": johnson,
                  "screening_buckling_load_N": min(euler, johnson),
                  "screening_buckling_margin_to_proof": min(euler, johnson) / proof,
                  "note": "No thread-root concentration, residual stress, corrosion, fatigue or column imperfection included"},
        "nut": {"engaged_turns": engage / lead,
                "mean_proof_thread_bearing_pressure_MPa": proof / bearing_area,
                "effective_60pct_proof_thread_bearing_pressure_MPa": proof / (bearing_area * 0.6),
                "mean_proof_screw_root_thread_shear_MPa": proof / thread_shear_area,
                "effective_60pct_proof_thread_shear_MPa": proof / (thread_shear_area * 0.6),
                "proof_flange_support_bearing_pressure_MPa": proof / flange_area,
                "raised_thread_start_reserve_mm": g["nut_body"]["z_min"] - g["screw"]["raised_z_min"],
                "note": "Mean stresses do not qualify first-thread concentration, flank bending, wear, fit or actual bronze grade"},
        "housing": {"proof_nominal_compression_MPa": proof / housing_area},
        "bearing_candidate": {"part": "NSK 51305", "basic_dynamic_rating_N": 36000,
                              "basic_static_rating_N": 61500,
                              "static_rating_to_working_axial_load": 61500 / working,
                              "static_rating_to_assumed_proof_axial_load": 61500 / proof,
                              "shaft_abutment_diameter_mm": 48, "minimum_required_shaft_abutment_mm": 41,
                              "housing_abutment_bore_mm": 25.5, "maximum_required_housing_abutment_bore_mm": 36,
                              "nominal_head_seat_bearing_pressure_proof_MPa": proof / (PI / 4 * (48**2 - 25**2)),
                              "nominal_saddle_seat_bearing_pressure_proof_MPa": proof / (PI / 4 * (52**2 - 27**2)),
                              "journal_all_axial_load_conservative_stress_MPa": proof / (PI / 4 * 25**2),
                              "note": "Normal thrust bypasses journal via bearing races; nominal seat pressures exclude race flexibility and edge loading; candidate has no radial load capacity"},
        "base": {"average_ground_pressure_proof_MPa": q_avg,
                 "centered_strip_model_plate_bending_MPa": strip_stress,
                 "strip_model_note": "Uniform foundation pressure and an unsupported plate strip from housing edge; not a validated plate/ground FEA",
                 "stability_and_pressure_sensitivities": scenarios},
        "handle": {"worst_assumed_proof_input_torque_Nm": worst,
                   "proof_nominal_bending_stress_MPa": handle_sigma,
                   "assumed_yield_margin": fy / handle_sigma,
                   "note": "Drive-head cross-bore stress concentration and sliding-handle position require separate checks"},
        "stop": {"nominal_stop_annular_contact_area_mm2": stop_contact_area,
                 "stress_if_full_proof_were_transferred_through_stop_MPa": proof / stop_contact_area,
                 "note": "This is an adverse sensitivity, not the normal lifting load path; stop attachment/contact must be qualified for overtravel force"},
        "limitations": BASIS["release_blockers"],
    }


def write_report(r: dict) -> None:
    s = r["screw"]
    n = r["nut"]
    c = next(c for c in r["torque_cases"] if c["load"] == "working" and c["thread_mu"] == 0.15)
    p = next(c for c in r["torque_cases"] if c["load"] == "proof_assumption" and c["thread_mu"] == 0.15)
    text = f"""# Preliminary engineering basis — 2,000 kg screw jack

**Engineering prototype. Not released for manufacture or vehicle lifting.**

The two CAD approaches share one dimensional basis. Neither valid CAD solids nor these hand calculations establish a production rating. The nominal working mass is a project target, not a certified capacity. ASME's public PASE-2024 scope includes automotive mechanical jacks; the full standard has not been reviewed. The 1.5 load multiplier is a project assumption and is not attributed to a standard.

## Common geometry

All lengths are in millimetres. The base underside is z=0. Displayed CAD is at 60 mm extension. Details and axial coordinates are in `design_basis.json`.

- Base 200 × 180 × 12; housing Ø70/Ø56, z12–212; top support ring Ø90/Ø56, z204–212.
- Bronze candidate nut Ø54 with 48 mm thread, z164–212, flange Ø80 with clearance bore Ø32, z212–220. Retaining screws are four M6 on PCD66. A fitted locating/antirotation solution must resolve the present 2 mm diametral clearance.
- Single-start right-hand Tr30×4; Ø30 major, Ø28 pitch and Ø25.5 **basic nominal** root. Strength calculations assume a certified minimum Ø25.0 root. Thread length 230, displayed z100–330, collapsed z40–270, raised z160–390.
- Travel 120 mm, {r['turns_for_full_travel']:.0f} turns. Saddle top height 328 collapsed / 388 displayed / 448 raised. This high closed height limits vehicle applicability; check the intended vehicle's lift point clearance.
- Drive head Ø48 × 28, cross-bore Ø17; handle Ø16 × 500, nominal maximum centered lever arm 250. Candidate NSK51305 bearing envelope Ø52/Ø25 × 18. Journal Ø25, z358–386, with saddle clearance bore Ø25.5. Saddle Ø70 × 12, z376–388, requires a qualified vehicle contact and angular seat.
- Removable end collar Ø36/Ø25.7 × 6 beneath screw, displayed z94–100. Split/clamp retention and fasteners are unresolved; no rated cross-pin is claimed. At raised travel its top z160 meets the nut skirt underside z160. Screw thread start remains 4 mm below the nut's first thread. Nut must be threaded on before collar installation. Attachment, runout, retention and stop strength still need verification.

## Loads, screw and nut screening

Working axial load = 2,000 × 9.81 = **{r['working_axial_load_N']/1000:.2f} kN**. Assumed proof load = 1.5 × working = **{r['assumed_proof_axial_load_N']/1000:.2f} kN**.

For root diameter d=25.0, A=πd²/4 and I=πd⁴/64. Nominal axial stress F/A at proof is {s['proof_axial_stress_MPa']:.1f} MPa. With conservative full input torque, torsion τ=16T/(πd³), combined stress √(σ²+3τ²) is {s['proof_von_mises_centered_MPa']:.1f} MPa. Against the **assumed certified** minimum yield 650 MPa this is a nominal margin of {s['assumed_yield_margin_centered']:.2f}. It excludes root concentration, fatigue and manufacturing effects.

A 15 mm load eccentricity produces bending 32Fe/(πd³). Combined proof stress then rises to **{s['proof_von_mises_with_eccentricity_MPa']:.0f} MPa**, giving only {s['assumed_yield_margin_with_eccentricity']:.2f} nominal yield margin. No lateral working-load rating is established.

Buckling uses a deliberately conservative 250 mm unsupported column and K=2, so effective length is 500 mm. Euler π²EI/(KL)² gives {s['Euler_load_N']/1000:.1f} kN. The lower Euler/Johnson screening value is {s['screening_buckling_load_N']/1000:.1f} kN, or {s['screening_buckling_margin_to_proof']:.2f} times the assumed proof load. Column imperfections and guide compliance are not resolved.

Twelve nut turns engage. Mean projected flank pressure 2F/(πd₂L) is **{n['mean_proof_thread_bearing_pressure_MPa']:.1f} MPa** at proof; reducing effective engagement to 60% raises it to {n['effective_60pct_proof_thread_bearing_pressure_MPa']:.1f} MPa. The screening bronze bearing allowable 30 MPa is an unqualified material assumption. Nominal root-thread shear 2F/(πdL) is {n['mean_proof_screw_root_thread_shear_MPa']:.1f} MPa, rising to {n['effective_60pct_proof_thread_shear_MPa']:.1f} MPa with 60% engagement. First-thread loading, flank bending and wear remain unresolved.

## Torque and self-locking

Lead angle λ=atan(lead/(πd₂))={r['thread_lead_angle_degrees']:.3f}°. For the 15° half-flank angle, μ′=μ/cos15°, raising torque is Fd₂/2 × (tanλ+μ′)/(1−μ′tanλ). A separate rolling-bearing friction surrogate adds μ_bFr_m with μ_b=0.02; it does not qualify a real bearing.

At assumed thread μ=0.15, total raising torque is {c['total_raise_Nm']:.1f} N·m at working load and {p['total_raise_Nm']:.1f} N·m at assumed proof. Corresponding hand forces at 250 mm are **{c['handle_force_N']:.0f} N working / {p['handle_force_N']:.0f} N proof**. Ergonomics and actual torque must be tested. The nominal proof handle bending stress is {r['handle']['proof_nominal_bending_stress_MPa']:.0f} MPa before contact/concentration effects.

Self-locking requires μ > cos15° × tanλ = **{r['critical_thread_friction_coefficient_for_self_locking']:.4f}**. The assumed 0.08–0.15 range passes that ideal static criterion, but the adverse μ=0.03 case backdrives. Lubricant, vibration, wear and dynamic lowering invalidate any unconditional self-locking claim. A hazard review and qualified load-holding protection remain required.

The selected catalogue candidate **NSK51305** is 25/52/18 with dynamic Ca=36 kN and static C0a=61.5 kN. C0a/proof={r['bearing_candidate']['static_rating_to_assumed_proof_axial_load']:.2f}, meeting the project's preliminary 2×proof screening target. Manufacturer abutment requirements are shaft diameter≥41, housing abutment bore≤36 and fillet radius≤1. The Ø48 drive head and Ø25.5 saddle bore meet those nominal diameter bounds; detailed seats and fit remain unqualified. Nominal proof bearing pressures on head and saddle seat are {r['bearing_candidate']['nominal_head_seat_bearing_pressure_proof_MPa']:.1f} / {r['bearing_candidate']['nominal_saddle_seat_bearing_pressure_proof_MPa']:.1f} MPa. Normal thrust passes through the races and shoulder rather than the locating journal; a conservative all-axial-load journal stress bound is {r['bearing_candidate']['journal_all_axial_load_conservative_stress_MPa']:.1f} MPa. The bearing cannot carry radial load. Supplier life, hardened seats, alignment, sealing, retention and measured drag still require verification. A plain sliding washer requires a different torque calculation.

## Base, load path and stop concerns

Average centered foundation pressure at assumed proof is {r['base']['average_ground_pressure_proof_MPa']:.3f} MPa. A basic uniformly loaded plate-strip model gives {r['base']['centered_strip_model_plate_bending_MPa']:.1f} MPa bending stress in the 12 mm base. Actual corner uplift, concentrated ground contact, weld distortion and eccentricity can dominate; this is not a plate qualification.

At full height, a 15 mm offset plus a 3° slope sensitivity shifts the resultant approximately {r['base']['stability_and_pressure_sensitivities'][2]['resultant_offset_mm']:.1f} mm toward the 90 mm tipping edge. Adding a horizontal force equal to 5% of load shifts it to approximately {r['base']['stability_and_pressure_sensitivities'][3]['resultant_offset_mm']:.1f} mm and sharply increases contact pressure. These are **sensitivity cases, not permitted operating conditions or a stability rating**. Require a hard level foundation and qualified lift-point alignment. The present high closed height and base dimensions need a vehicle-specific stability review.

The end stop contact annulus has only {r['stop']['nominal_stop_annular_contact_area_mm2']:.1f} mm² area. If an adverse misuse transferred the complete proof load through it, nominal contact stress would reach {r['stop']['stress_if_full_proof_were_transferred_through_stop_MPa']:.0f} MPa. Therefore a modeled stop does not establish a rated stop: its retention, bronze skirt, overtravel torque and assembly sequence must be qualified.

The intended axial load path is vehicle saddle → selected thrust bearing → screw → bronze nut → flange/top support ring → housing → base → foundation. Retaining M6 screws are not credited as the main compressive load path. Joint, weld, flange/ring bending and hole stresses need detailed review.

## Release work

"""
    text += "\n".join(f"- {x}." for x in BASIS["release_blockers"])
    text += "\n\n## Sources and reproducibility\n\n"
    for source in BASIS["sources"]:
        text += f"- [{source['title']}]({source['url']}): {source['use']}.\n"
    text += "\nRun `python3 engineering_calculations.py` to regenerate this report and both JSON records. Equations are visible in the script. Results are analytical screening values and are separate from the CAD kernel's geometry checks.\n"
    (HERE / "ENGINEERING_BASIS.md").write_text(text, encoding="utf-8")


if __name__ == "__main__":
    results = compute()
    (HERE / "design_basis.json").write_text(json.dumps(BASIS, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (HERE / "engineering_results.json").write_text(json.dumps(results, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    write_report(results)
    print(json.dumps({"working_kN": results["working_axial_load_N"] / 1000,
                      "proof_assumption_kN": results["assumed_proof_axial_load_N"] / 1000,
                      "root_diameter_strength_mm": results["screw"]["strength_diameter_mm"],
                      "proof_centered_vm_MPa": results["screw"]["proof_von_mises_centered_MPa"],
                      "buckling_to_proof": results["screw"]["screening_buckling_margin_to_proof"],
                      "bearing_candidate": "NSK 51305", "bearing_application_qualified": False,
                      "production_released": False}, indent=2))
