#!/usr/bin/env python3
"""Direct FreeCAD/OCCT build: 2 tonne, 120 mm vertical screw jack.

Run with the system FreeCAD Python:
    /usr/bin/python3 outputs/screw_jack_20261017/script_build.py
Or run main() in the connected FreeCAD GUI. Existing documents are preserved.

All dimensions are millimetres. This is a design review assembly, not a released
or certified lifting device. The script is the editable parametric source; saved
Part features preserve exact BREP and dimensional/manufacturing properties.
"""
from __future__ import annotations

import csv
import json
import math
import os
import sys
import time
from pathlib import Path

if "/usr/lib/freecad/lib" not in sys.path:
    sys.path.append("/usr/lib/freecad/lib")
import FreeCAD as App
import Part
import Sketcher

V = App.Vector
OUTPUT_DIR = Path(__file__).resolve().parent / "script_based"
PARAMETERS = {
    "rated_load_kg": 2000.0,
    "gravity_m_s2": 9.81,
    "design_load_factor": 1.5,
    "stroke_mm": 120.0,
    "display_stroke_mm": 60.0,
    "base_length_mm": 200.0,
    "base_width_mm": 180.0,
    "base_thickness_mm": 12.0,
    "base_corner_radius_mm": 12.0,
    "mounting_hole_diameter_mm": 13.0,
    "mounting_hole_x_mm": 75.0,
    "mounting_hole_y_mm": 65.0,
    "housing_od_mm": 70.0,
    "housing_id_mm": 56.0,
    "housing_bottom_mm": 12.0,
    "housing_top_mm": 212.0,
    "support_flange_od_mm": 90.0,
    "support_flange_bottom_mm": 204.0,
    "nut_od_mm": 54.0,
    "nut_minor_bore_mm": 26.0,
    "nut_bottom_mm": 164.0,
    "nut_top_mm": 212.0,
    "nut_flange_od_mm": 80.0,
    "nut_flange_top_mm": 220.0,
    "entry_skirt_bottom_mm": 160.0,
    "entry_skirt_bore_mm": 32.0,
    "flange_bolt_pcd_mm": 66.0,
    "flange_bolt_clearance_mm": 6.5,
    "screw_major_diameter_mm": 30.0,
    "screw_root_diameter_mm": 25.5,
    "screw_pitch_mm": 4.0,
    "thread_included_angle_deg": 30.0,
    "screw_length_mm": 230.0,
    "screw_collapsed_bottom_mm": 40.0,
    "nut_thread_radial_clearance_mm": 0.25,
    "nut_thread_axial_clearance_mm": 0.12,
    "stop_collar_od_mm": 36.0,
    "stop_collar_height_mm": 6.0,
    "stop_collar_bore_mm": 25.7,
    "head_od_mm": 48.0,
    "head_height_mm": 28.0,
    "handle_diameter_mm": 16.0,
    "handle_length_mm": 500.0,
    "handle_bore_mm": 17.0,
    "bearing_od_mm": 52.0,
    "bearing_id_mm": 25.0,
    "bearing_height_mm": 18.0,
    "bearing_C0a_kN": 61.5,
    "bearing_Ca_kN": 36.0,
    "journal_diameter_mm": 25.0,
    "journal_height_mm": 28.0,
    "saddle_od_mm": 70.0,
    "saddle_bore_mm": 25.5,
    "saddle_height_mm": 12.0,
}


def cylinder(radius, height, z=0.0, x=0.0, y=0.0):
    return Part.makeCylinder(radius, height, V(x, y, z))


def annulus(ro, ri, h, z):
    return cylinder(ro, h, z).cut(cylinder(ri, h + 2, z - 1))


def moved(shape, vector):
    result = shape.copy()
    result.translate(vector)
    return result


def fuse(shapes):
    result = shapes[0]
    for shape in shapes[1:]:
        result = result.fuse(shape)
    return result.removeSplitter()


def hex_prism(radius, height, z, x=0, y=0):
    pts = [V(x + radius * math.cos(i * math.pi / 3),
             y + radius * math.sin(i * math.pi / 3), z) for i in range(6)]
    wire = Part.makePolygon(pts + [pts[0]])
    return Part.Face(wire).extrude(V(0, 0, height))


def native_helical_feature(blank, profile_points, pitch, height, z, subtractive, key):
    """Use FreeCAD's native helix builder, which preserves the helix section.

    A long generic OCCT pipe can remain BREP-valid despite section distortion.
    Native PartDesign helix construction avoids that failure and also saves a
    fully editable native sketch/helix construction document for each thread.
    """
    doc = App.newDocument("Kala_Thread_" + key.replace("-", "_") + "_" + str(time.time_ns()))
    body = doc.addObject("PartDesign::Body", "ThreadedBody")
    base = body.newObject("PartDesign::Feature", "MachinedBlank")
    base.Shape = blank
    sketch = body.newObject("Sketcher::SketchObject", "TrapezoidalProfile")
    sketch.Placement = App.Placement(V(0, 0, z), App.Rotation(V(1, 0, 0), 90))
    for i, point in enumerate(profile_points):
        following = profile_points[(i + 1) % len(profile_points)]
        sketch.addGeometry(Part.LineSegment(V(point[0], point[1], 0), V(following[0], following[1], 0)), False)
    # Block constraints preserve the generated production-review section while
    # leaving each dimension visible and editable in native Sketcher geometry.
    for i in range(len(profile_points)):
        sketch.addConstraint(Sketcher.Constraint("Block", i))
    kind = "PartDesign::SubtractiveHelix" if subtractive else "PartDesign::AdditiveHelix"
    feature = body.newObject(kind, "NativeTr30x4Helix")
    feature.Profile = (sketch, [""])
    feature.ReferenceAxis = (sketch, ["V_Axis"])
    feature.Pitch = pitch
    feature.Height = height
    feature.Mode = "pitch-height-angle"
    feature.LeftHanded = False
    doc.recompute()
    result = feature.Shape.copy()
    if result.isNull() or not result.isValid() or len(result.Solids) != 1:
        App.closeDocument(doc.Name)
        raise RuntimeError(key + " native helical feature failed: " + feature.getStatusString())
    if "NATIVE_DOCUMENT_OUTPUT_DIR" in globals():
        doc.saveAs(str(NATIVE_DOCUMENT_OUTPUT_DIR / (key + "_native_thread.FCStd")))
    App.closeDocument(doc.Name)
    return result


def generate_shapes(p):
    """Build complete parts at the displayed stroke. Return shapes and BOM data."""
    s = p["display_stroke_mm"]
    lower = p["screw_collapsed_bottom_mm"] + s
    top = lower + p["screw_length_mm"]
    head_top = top + p["head_height_mm"]
    bearing_top = head_top + p["bearing_height_mm"]
    saddle_top = bearing_top + p["saddle_height_mm"]
    root = p["screw_root_diameter_mm"] / 2
    major = p["screw_major_diameter_mm"] / 2
    pitch = p["screw_pitch_mm"]
    angle = math.radians(p["thread_included_angle_deg"] / 2)
    pitch_radius = (p["screw_major_diameter_mm"] - pitch / 2) / 2
    half_outer = pitch / 4 - (major - pitch_radius) * math.tan(angle)
    # The section slightly overlaps the core to make a reliable manifold union.
    inner_r = root - 0.05
    half_inner = pitch / 4 + (pitch_radius - inner_r) * math.tan(angle)
    radial_clearance = p["nut_thread_radial_clearance_mm"]
    axial_clearance = p["nut_thread_axial_clearance_mm"]
    parts = []

    def add(key, label, shape, material, process, note="", color=(0.72, 0.74, 0.77), motion="fixed", quantity=1):
        parts.append(dict(key=key, label=label, shape=shape.removeSplitter(),
                          material=material, process=process, note=note,
                          color=color, motion=motion, quantity=quantity))

    # Rounded base, with a real four-hole mounting pattern.
    length, width, thick, cr = (p[k] for k in (
        "base_length_mm", "base_width_mm", "base_thickness_mm", "base_corner_radius_mm"))
    base_pieces = [Part.makeBox(length - 2 * cr, width, thick, V(-length / 2 + cr, -width / 2, 0)),
                   Part.makeBox(length, width - 2 * cr, thick, V(-length / 2, -width / 2 + cr, 0))]
    for x in (-length / 2 + cr, length / 2 - cr):
        for y in (-width / 2 + cr, width / 2 - cr):
            base_pieces.append(cylinder(cr, thick, x=x, y=y))
    base = fuse(base_pieces)
    for x in (-p["mounting_hole_x_mm"], p["mounting_hole_x_mm"]):
        for y in (-p["mounting_hole_y_mm"], p["mounting_hole_y_mm"]):
            base = base.cut(cylinder(p["mounting_hole_diameter_mm"] / 2, thick + 2, -1, x, y))
    add("SJ-001", "Base plate — 200 × 180 × 12", base, "S355 steel", "Laser cut / mill / weld",
        "Four Ø13 mounting holes on 150 × 130 pattern. Ground stability and weld sizing require validation.", (0.16, 0.24, 0.32))

    housing = annulus(p["housing_od_mm"] / 2, p["housing_id_mm"] / 2,
                      p["housing_top_mm"] - p["housing_bottom_mm"], p["housing_bottom_mm"])
    support = annulus(p["support_flange_od_mm"] / 2, p["housing_id_mm"] / 2,
                      p["housing_top_mm"] - p["support_flange_bottom_mm"], p["support_flange_bottom_mm"])
    gusset_face = Part.Face(Part.makePolygon([
        V(32, -3, 12), V(75, -3, 12), V(32, -3, 72), V(32, -3, 12)]))
    gusset = gusset_face.extrude(V(0, 6, 0))
    housing_bits = [housing, support]
    for deg in (0, 90, 180, 270):
        g = gusset.copy()
        g.rotate(V(0, 0, 0), V(0, 0, 1), deg)
        housing_bits.append(g)
    housing = fuse(housing_bits)
    bolt_positions = [(p["flange_bolt_pcd_mm"] / 2 * math.cos(i * math.pi / 2),
                       p["flange_bolt_pcd_mm"] / 2 * math.sin(i * math.pi / 2)) for i in range(4)]
    for x, y in bolt_positions:
        housing = housing.cut(cylinder(2.5, 10, 203, x, y))
    add("SJ-002", "Housing, flange and four gussets", housing, "S355 steel",
        "Tube / machined flange / welded fabrication",
        "OD70 × ID56 body. Ø90 support flange. Weld geometry and qualified weld procedure remain to be released.", (0.16, 0.24, 0.32))

    nut = fuse([
        annulus(p["nut_od_mm"] / 2, p["nut_minor_bore_mm"] / 2,
                p["nut_top_mm"] - p["nut_bottom_mm"], p["nut_bottom_mm"]),
        annulus(p["nut_flange_od_mm"] / 2, p["entry_skirt_bore_mm"] / 2,
                p["nut_flange_top_mm"] - p["nut_top_mm"], p["nut_top_mm"]),
        annulus(p["nut_od_mm"] / 2, p["entry_skirt_bore_mm"] / 2,
                p["nut_bottom_mm"] - p["entry_skirt_bottom_mm"], p["entry_skirt_bottom_mm"]),
    ])
    # Align the nut helix with the moving screw phase at the mid-stroke pose.
    # 160 mm and the collapsed thread origin 40 mm differ by 30 complete turns.
    cutter_inner_half = half_inner + axial_clearance + (radial_clearance + .1) * math.tan(angle)
    cutter_outer_half = half_outer + axial_clearance
    nut_profile = [(inner_r - .1, half_inner - cutter_inner_half),
                   (major + radial_clearance, half_inner - cutter_outer_half),
                   (major + radial_clearance, half_inner + cutter_outer_half),
                   (inner_r - .1, half_inner + cutter_inner_half)]
    nut = native_helical_feature(nut, nut_profile, pitch,
        p["nut_flange_top_mm"] - p["entry_skirt_bottom_mm"], p["entry_skirt_bottom_mm"], True, "SJ-003")
    for x, y in bolt_positions:
        nut = nut.cut(cylinder(p["flange_bolt_clearance_mm"] / 2, 10, 211, x, y))
    add("SJ-003", "Flanged bronze nut — true Tr30 × 4 internal helix", nut,
        "CuSn12 bearing bronze", "Turn / single-point internal trapezoidal thread / drill",
        "48 mm threaded body; entry skirt and top flange have Ø32 clearance bore. Thread fit is preliminary, not a released tolerance class.", (0.79, 0.59, 0.20))

    screw_profile = [(inner_r, 0), (major, half_inner - half_outer),
                     (major, half_inner + half_outer), (inner_r, 2 * half_inner)]
    threaded_core = native_helical_feature(cylinder(root, p["screw_length_mm"] + p["stop_collar_height_mm"], lower - p["stop_collar_height_mm"]),
        screw_profile, pitch, p["screw_length_mm"] - pitch, lower, False, "SJ-004")
    spindle = fuse([
        threaded_core,
        cylinder(p["head_od_mm"] / 2, p["head_height_mm"], top),
        cylinder(p["journal_diameter_mm"] / 2, p["journal_height_mm"], head_top),
    ])
    bore_z = top + p["head_height_mm"] / 2
    handle_hole = Part.makeCylinder(p["handle_bore_mm"] / 2, p["head_od_mm"] + 2,
                                    V(-p["head_od_mm"] / 2 - 1, 0, bore_z), V(1, 0, 0))
    spindle = spindle.cut(handle_hole)
    # Axial retainer bore is deliberately clear of the independently swivelling
    # saddle. Supplier thread/locking details are still a release item.
    journal_top = head_top + p["journal_height_mm"]
    spindle = spindle.cut(cylinder(3.2, 12, journal_top - 10))
    add("SJ-004", "Spindle and integral drive head — true Tr30 × 4", spindle,
        "42CrMo4 +QT alloy steel", "Forge / turn / thread whirl or roll / finish grind",
        "Single-start right-hand 4 mm lead, Ø25.5 root; Ø48 integral head with Ø17 handle bore. Lower removable collar permits nut assembly.",
        (0.75, 0.77, 0.81), "screw")

    # The stop is split so the bronze nut can be installed before the collar.
    # Clamp screws are deliberately not invented; release requires verified
    # retention hardware. Their absence is a blocking release item in validation.
    collar = annulus(p["stop_collar_od_mm"] / 2, p["stop_collar_bore_mm"] / 2,
                     p["stop_collar_height_mm"], lower - p["stop_collar_height_mm"])
    for suffix, y0 in (("A", .15), ("B", -40)):
        clip = collar.common(Part.makeBox(80, 39.85, 10, V(-40, y0, lower - 8)))
        add("SJ-005" + suffix, "Travel stop collar half " + suffix, clip,
            "42CrMo4 +QT alloy steel", "Turn matched pair / split / clamp hardware TBD",
            "Preliminary envelope only. Positive retention and clamp hardware MUST be engineered before any lifting use.",
            (0.35, 0.38, 0.42), "screw")

    bearing = annulus(p["bearing_od_mm"] / 2, p["bearing_id_mm"] / 2,
                      p["bearing_height_mm"], head_top)
    add("SJ-006", "NSK 51305 thrust bearing — supplier envelope", bearing,
        "Bearing steel / purchased", "Purchase matched thrust bearing assembly",
        "Ø25 × Ø52 × 18 supplier envelope only, without rolling internals. C0a 61.5 kN / Ca 36 kN candidate; finalize fit, retention, grease and supplier availability.",
        (0.43, 0.45, 0.49), "vertical")

    saddle = cylinder(p["saddle_od_mm"] / 2, p["saddle_height_mm"], bearing_top)
    saddle = saddle.cut(cylinder(p["saddle_bore_mm"] / 2, p["saddle_height_mm"] + 2, bearing_top - 1))
    saddle = saddle.cut(cylinder(16.5, 3.2, journal_top - .2))
    # Four shallow radial grip grooves do not cross the bearing abutment.
    for deg in (0, 90, 180, 270):
        slot = Part.makeBox(14, 4, 1.5, V(20, -2, saddle_top - 1.5))
        slot.rotate(V(0, 0, 0), V(0, 0, 1), deg)
        saddle = saddle.cut(slot)
    add("SJ-007", "Swivel load saddle — Ø70", saddle, "C45 steel",
        "Turn / mill grip grooves / surface treatment",
        "Ø25.5 central bore, Ø33 axial cap access recess. Vehicle-specific contact geometry and articulation need verification.",
        (0.27, 0.30, 0.34), "vertical")

    cap = fuse([cylinder(16, 2, journal_top), cylinder(3, 10, journal_top - 10)])
    # Hex socket in the exposed retainer, without crossing the journal or saddle.
    cap = cap.cut(hex_prism(2.31, 2, journal_top + .6))
    add("SJ-008", "Axial saddle retaining cap — preliminary envelope", cap,
        "Class 8.8 steel / purchased", "Finalize axial threaded retainer and positive locking",
        "Ø32 cap / M6 nominal stud. 0.2 mm axial and 0.5 mm radial saddle clearances allow rotation; thread, locking and retention load remain unverified.",
        (0.15, 0.16, 0.18), "screw")

    handle = Part.makeCylinder(p["handle_diameter_mm"] / 2, p["handle_length_mm"],
                               V(-p["handle_length_mm"] / 2, 0, bore_z), V(1, 0, 0))
    add("SJ-009", "Sliding tommy bar — Ø16 × 500", handle, "42CrMo4 +QT alloy steel",
        "Cut bar / deburr / surface treatment",
        "Shown centered. Effective hand lever 250 mm maximum; allowable effort and pinch clearance require validation.",
        (0.55, 0.58, 0.61), "screw")

    for i, (x, y) in enumerate(bolt_positions, 1):
        bolt = fuse([cylinder(2.45, 8, 204, x, y), cylinder(3, 8, 212, x, y), hex_prism(5.7735, 4, 220, x, y)])
        add("SJ-010-" + str(i), "M6 nut-flange retention bolt " + str(i), bolt,
            "ISO 898-1 class 8.8 steel", "Purchase M6 bolt / finalize length and locking",
            "Ø4.9 thread-root envelope in Ø5 M6×1 tap-drill, 8 mm steel engagement. Thread ridges omitted. Upper Ø6 grip in Ø6.5 bronze clearance. Fit, preload and locking remain unverified.",
            (0.40, 0.42, 0.45))
    return parts, dict(screw_bottom=lower, screw_top=top, head_top=head_top,
                       bearing_top=bearing_top, saddle_top=saddle_top,
                       handle_axis_z=bore_z, journal_top=journal_top)


def add_property(obj, name, value, group="Manufacturing"):
    if isinstance(value, bool):
        kind = "App::PropertyBool"
    elif isinstance(value, (int, float)):
        kind = "App::PropertyFloat"
    else:
        kind = "App::PropertyString"
    obj.addProperty(kind, name, group)
    setattr(obj, name, value)


def validate(parts, p, stack):
    result = {"status": "DESIGN_REVIEW_ONLY_NOT_RELEASED", "parts": [], "intrusions": [],
              "thread_model": "True BREP external and mating internal 30-degree trapezoidal helical sweeps",
              "parameter_editing": "Edit this Python source PARAMETERS and regenerate; feature properties are reference metadata, not shape-driving expressions.",
              "supplier_bearing": {"model": "NSK 51305", "url": "https://www.nsk.com/engineering/51305-apn.html",
                                   "geometry_mm": [25, 52, 18], "C0a_kN": 61.5, "Ca_kN": 36},
              "release_blockers": [
                  "Travel-stop collar retention and its clamp fasteners are not designed; envelope only.",
                  "Material specifications, heat treatment, fatigue, buckling, thread wear and load test validation remain required.",
                  "Production tolerances, thread fit/gauging, run-outs, fillets, weld sizes/procedure and corrosion protection remain to be released.",
                  "Bearing supplier fit, axial cap/retainer and mounting hardware selection/locking remain to be finalized.",
                  "Vehicle saddle interface, lateral-load stability, ground support and applicable lifting-device compliance are not validated.",
              ]}
    by_key = {item["key"]: item for item in parts}
    for item in parts:
        shape = item["shape"]
        valid = shape.isValid() and len(shape.Solids) == 1 and shape.Volume > 0
        result["parts"].append({"part_number": item["key"], "valid": valid,
                                "solid_count": len(shape.Solids), "volume_mm3": shape.Volume})
        if not valid:
            raise RuntimeError("Invalid component " + item["key"])
    # A closed BREP intersection is the collision criterion. Surface contacts
    # have zero common volume and are legitimate support or assembly contacts.
    for i, a in enumerate(parts):
        for b in parts[i + 1:]:
            if not a["shape"].BoundBox.intersect(b["shape"].BoundBox):
                continue
            overlap = a["shape"].common(b["shape"]).Volume
            if overlap > .01:
                result["intrusions"].append({"part_a": a["key"], "part_b": b["key"], "volume_mm3": overlap})
    result["all_parts_valid"] = True
    result["midstroke_no_solid_interference"] = len(result["intrusions"]) == 0
    fixed_nut = by_key["SJ-003"]["shape"]
    screw = by_key["SJ-004"]["shape"]
    result["stroke_checks"] = []
    for stroke in (0.0, 30.0, 60.0, 90.0, 120.0):
        delta = stroke - p["display_stroke_mm"]
        posed_screw = screw.copy()
        posed_screw.rotate(V(0, 0, 0), V(0, 0, 1), 360 * delta / p["screw_pitch_mm"])
        posed_screw.translate(V(0, 0, delta))
        collision = posed_screw.common(fixed_nut).Volume
        collar_top = p["screw_collapsed_bottom_mm"] + stroke
        checks = {"stroke_mm": stroke, "screw_rotation_deg_from_mid": 360 * delta / p["screw_pitch_mm"],
                  "saddle_top_mm": stack["saddle_top"] + delta,
                  "thread_to_nut_intersection_mm3": collision,
                  "nut_thread_engagement_mm": min(p["nut_top_mm"], stack["screw_top"] + delta) - max(p["nut_bottom_mm"], stack["screw_bottom"] + delta),
                  "stop_gap_mm": p["entry_skirt_bottom_mm"] - collar_top,
                  "collar_bottom_ground_clearance_mm": collar_top - p["stop_collar_height_mm"],
                  "collar_bottom_base_clearance_mm": collar_top - p["stop_collar_height_mm"] - p["base_thickness_mm"]}
        checks["pass_geometric_check"] = collision < .01 and checks["nut_thread_engagement_mm"] >= 48 - .01 and checks["stop_gap_mm"] >= 0
        result["stroke_checks"].append(checks)
    result["rated_load_N"] = p["rated_load_kg"] * p["gravity_m_s2"]
    result["factored_design_load_N"] = result["rated_load_N"] * p["design_load_factor"]
    result["design_load_factor_is_assumption_not_certified_test_factor"] = True
    result["true_thread_fit_test_passed"] = all(x["pass_geometric_check"] for x in result["stroke_checks"])
    return result


def build(output_dir=None, parameter_overrides=None):
    global NATIVE_DOCUMENT_OUTPUT_DIR
    started = time.time()
    out = Path(output_dir) if output_dir else OUTPUT_DIR
    out.mkdir(parents=True, exist_ok=True)
    (out / "parts").mkdir(exist_ok=True)
    NATIVE_DOCUMENT_OUTPUT_DIR = out / "parts"
    p = dict(PARAMETERS)
    if parameter_overrides:
        p.update(parameter_overrides)
    if not 0 <= p["display_stroke_mm"] <= p["stroke_mm"]:
        raise ValueError("Display stroke must lie inside 0..120 mm")
    print("[script-build] Generating true helical thread and component solids", flush=True)
    parts, stack = generate_shapes(p)
    print("[script-build] Validating component solids, thread fit and five stroke poses", flush=True)
    result = validate(parts, p, stack)
    if not result["midstroke_no_solid_interference"] or not result["true_thread_fit_test_passed"]:
        with (out / "failed_validation.json").open("w") as f:
            json.dump(result, f, indent=2)
        raise RuntimeError("Script CAD geometric validation failed; inspect failed_validation.json")
    doc = App.newDocument("Kala_ScrewJack_Script_" + time.strftime("%H%M%S"))
    doc.Label = "Kala · Direct script · 2 t / 120 mm screw jack"
    assembly = doc.addObject("App::Part", "ScrewJackAssembly")
    assembly.Label = "SCREW JACK — SCRIPT BUILD — DESIGN REVIEW"
    add_property(assembly, "DesignStatus", "PRELIMINARY / NOT RELEASED / NOT CERTIFIED", "Design")
    add_property(assembly, "RebuildScript", str(Path(__file__).resolve()), "Design")
    add_property(assembly, "MotionConstraint", "0 <= lift <= 120 mm; spindle rotation = lift × 360/4 deg; saddle swivels on thrust bearing", "Design")
    add_property(assembly, "ReleaseBlockers", " | ".join(result["release_blockers"]), "Design")
    objects = []
    for item in parts:
        obj = doc.addObject("Part::Feature", item["key"].replace("-", "_"))
        obj.Label = item["label"]
        obj.Shape = item["shape"]
        assembly.addObject(obj)
        for key, value in (("PartNumber", item["key"]), ("Material", item["material"]),
                           ("ManufacturingProcess", item["process"]), ("DesignNotes", item["note"]),
                           ("MotionGroup", item["motion"]), ("Quantity", item["quantity"]),
                           ("Volume_mm3", item["shape"].Volume)):
            add_property(obj, key, value)
        if App.GuiUp:
            obj.ViewObject.ShapeColor = item["color"]
            obj.ViewObject.LineColor = (0.10, 0.11, 0.12)
        objects.append(obj)
    # Include the full numerical input ledger inside the editable FCStd file.
    ledger = doc.addObject("Spreadsheet::Sheet", "DesignParameters")
    ledger.Label = "Design parameters — edit script and regenerate geometry"
    ledger.set("A1", "Parameter")
    ledger.set("B1", "Value")
    for row, (key, value) in enumerate(p.items(), 2):
        ledger.set("A" + str(row), key)
        ledger.set("B" + str(row), str(value))
    ledger.setColumnWidth("A", 290)
    ledger.setColumnWidth("B", 120)
    doc.recompute()
    fcstd = out / "screw_jack_script.FCStd"
    step = out / "screw_jack_script.step"
    doc.saveAs(str(fcstd))
    Part.export(objects, str(step))
    for item, obj in zip(parts, objects):
        Part.export([obj], str(out / "parts" / (item["key"] + ".step")))
    result.update({"build_elapsed_seconds": time.time() - started,
                   "document": str(fcstd), "assembly_step": str(step), "part_count": len(parts),
                   "parameters": p, "stack_at_display_stroke_mm": stack})
    with (out / "validation.json").open("w") as f:
        json.dump(result, f, indent=2)
    with (out / "parameters.json").open("w") as f:
        json.dump(p, f, indent=2)
    construction = {"route": "Direct FreeCAD Python / OCCT geometry, with Codex reasoning",
                    "decision_model": "No separate decision model used by this build script",
                    "source": str(Path(__file__).resolve()),
                    "operations": ["Rounded plate union and four through-hole cuts", "Tubular housing / support flange / four gusset union",
                                   "True Tr30×4 external helical sweep and root/head/journal union",
                                   "True mating internal trapezoidal helical subtraction in bronze nut",
                                   "Handle, bearing supplier envelope, saddle, stop collar halves, axial retaining cap and four flange bolts",
                                   "Exact BREP solid validation, pairwise intersection checks and five coupled screw-motion poses"],
                    "not_released": True}
    with (out / "construction.json").open("w") as f:
        json.dump(construction, f, indent=2)
    with (out / "BOM.csv").open("w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["Part number", "Description", "Quantity", "Material", "Process", "Design/release note", "Volume mm3"])
        for item in parts:
            writer.writerow([item["key"], item["label"], item["quantity"], item["material"],
                             item["process"], item["note"], round(item["shape"].Volume, 3)])
    if App.GuiUp:
        import FreeCADGui as Gui
        Gui.activeDocument().activeView().viewAxonometric()
        Gui.activeDocument().activeView().fitAll()
    print("[script-build] " + json.dumps({"FCStd": str(fcstd), "STEP": str(step),
        "parts": len(parts), "all_valid": result["all_parts_valid"],
        "midstroke_intrusions": result["intrusions"], "stroke_thread_fit_passed": result["true_thread_fit_test_passed"],
        "elapsed_seconds": round(result["build_elapsed_seconds"], 2)}), flush=True)
    return doc, result


def main():
    return build()


if __name__ == "__main__":
    main()
