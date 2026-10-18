#!/usr/bin/env python3
"""Build a screw-jack engineering candidate with Kala's native CAD calls.

Codex authors the plan. No API keys, reasoning/design/decision model clients,
or automatic redesign are invoked. All geometry uses shipped Kala handlers.
Run with the project venv, whose Python must match the installed FreeCAD ABI.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path[:0] = [str(ROOT / "src"), str(ROOT / "adapters/freecad/src"), "/usr/lib/freecad/lib"]

CALLS: list[dict] = []
PARTS: dict[str, dict] = {}


def call(tool: str, **arguments: object) -> None:
    CALLS.append({"tool": tool, "arguments": arguments})


def body(name: str, z: float, *, x: float = 0, y: float = 0,
         material: str = "S355 structural steel", note: str = "") -> None:
    call("partdesign.body", name=name)
    call("partdesign.placement", name=name, x=x, y=y, z=z)
    PARTS[name] = {"material": material, "note": note}


def finish_sketch() -> None:
    call("sketch.validate")
    call("sketch.leave")


def circle_sketch(circles: list[tuple[float, float, float]], plane: str = "XY") -> None:
    call("sketch.new", plane=plane)
    for index, (radius, x, y) in enumerate(circles):
        call("sketch.circle", radius=radius, x=x, y=y)
        call("sketch.diameter", geo=index, length=2 * radius)
        if x == 0 and y == 0:
            call("sketch.coincident", geo=index, pos=3, geo2=-1, pos2=1)
        else:
            call("sketch.distance_x", geo=index, pos=3, length=x)
            call("sketch.distance_y", geo=index, pos=3, length=y)
    finish_sketch()


def pad(circles: list[tuple[float, float, float]], length: float, *,
        reversed: bool = False, plane: str = "XY") -> None:
    circle_sketch(circles, plane)
    if reversed:
        call("partdesign.pad", length=length, reversed=True)
    else:
        call("partdesign.pad", length=length)
    call("partdesign.check")


def ring(outer: float, inner: float, length: float, *, reversed: bool = False) -> None:
    pad([(outer / 2, 0, 0), (inner / 2, 0, 0)], length, reversed=reversed)


def pocket(circles: list[tuple[float, float, float]], length: float,
           plane: str = "XY") -> None:
    circle_sketch(circles, plane)
    call("partdesign.pocket", length=length)
    call("partdesign.check")


def polygon_sketch(points: list[tuple[float, float]], plane: str = "XY") -> None:
    """Explicit vertex dimensions fully constrain even non-orthogonal profiles."""
    call("sketch.new", plane=plane)
    call("sketch.polyline", closed=True, points=[list(point) for point in points])
    for index, (x, y) in enumerate(points):
        call("sketch.coincident", geo=index, pos=2, geo2=(index + 1) % len(points), pos2=1)
        call("sketch.distance_x", geo=index, pos=1, length=x)
        call("sketch.distance_y", geo=index, pos=1, length=y)
    finish_sketch()


def plan() -> list[dict]:
    CALLS.clear()
    PARTS.clear()
    body("BasePlate", 0, note="200 x 180 x 12; four diameter-12 mounting holes on 150 x 130 centres")
    polygon_sketch([(-100, -90), (100, -90), (100, 90), (-100, 90)])
    call("partdesign.pad", length=12)
    call("partdesign.check")
    pocket([(6, x, y) for x in (-75, 75) for y in (-65, 65)], 12)

    bolt_xy = [(33 / math.sqrt(2) * x, 33 / math.sqrt(2) * y)
               for x, y in ((1, 1), (-1, 1), (-1, -1), (1, -1))]
    body("Housing", 204, note="Tube OD70/ID56 z12..204; welded integral support ring OD90/ID56 z204..212; M6x1 tapped holes8mm engagement represented by diameter5 tap-drill envelope, internal thread ridges omitted")
    ring(70, 56, 192, reversed=True)
    ring(90, 56, 8)
    pocket([(2.5, x, y) for x, y in bolt_xy], 8)

    body("BronzeNut", 212, material="CuSn12 bronze",
         note="Tr30x4 RH internal thread specified; modeled diameter30.2 clearance envelope, internal helix omitted by tool limitation")
    ring(54, 30.2, 48, reversed=True)
    ring(80, 32, 8)
    ring(54, 32, 52, reversed=True)
    pocket([(3.25, x, y) for x, y in bolt_xy], 8)

    body("PowerScrew", 100, material="42CrMo4 quenched and tempered steel",
         note="Tr30x4 RH external helical ridge on diameter25.5 root; midstroke60; removable stop collar below root limits upward travel to120; PowerScrew and DriveHead are two editable CAD regions of one integral machined spindle")
    pad([(12.75, 0, 0)], 230)
    pad([(12.75, 0, 0)], 6, reversed=True)
    # 15-degree trapezoid flanks. A 0.1-mm radial overlap makes a robust union.
    polygon_sketch([(12.65, 0), (15, 0.6296806), (15, 2), (12.65, 2.6296806)], "XZ")
    call("partdesign.helix", pitch=4, height=226)
    call("partdesign.check")

    body("DriveHead", 358, x=-24, material="42CrMo4 quenched and tempered steel",
         note="OD48 head z330..358; diameter17 tommy-bar bore at344; diameter25 journal z358..386; diameter6.4 axial retainer bore; integral machined continuation of PowerScrew, separate CAD region only")
    pad([(24, 24, 0)], 28, reversed=True)
    pad([(12.5, 24, 0)], 28)
    pocket([(8.5, 0, -14)], 48, "YZ")
    pocket([(3.2, 24, 0)], 28)

    body("RemovableStopCollar", 94, material="42CrMo4 steel",
         note="OD36 ID25.7 h6 removable stop collar; split/clamp retention omitted and must be engineered before release")
    ring(36, 25.7, 6)

    body("TommyBar", 344, x=-250, material="42CrMo4 quenched and tempered steel",
         note="Diameter16 x 500; sliding removable bar; grip end caps retained separately")
    pad([(8, 0, 0)], 500, plane="YZ")
    for name, x in (("HandleGripLeft", -258), ("HandleGripRight", 250)):
        body(name, 344, x=x, material="Oil-resistant elastomer",
             note="Diameter24 retaining grip end; nominal envelope")
        pad([(12, 0, 0)], 8, plane="YZ")

    body("ThrustBearingEnvelope", 358, material="Bearing steel",
         note="NSK51305 candidate axial rolling thrust bearing envelope OD52 ID25 h18; C0a61.5kN Ca36kN; supplier specification/fit/life verification required")
    ring(52, 25, 18)

    body("SwivelSaddle", 376, material="C45 steel",
         note="OD70 h12; diameter25.5 through journal clearance; OD33 upper counterbore from385.8 to388; 0.2 axial retention clearance; vehicle-specific contact geometry remains a release gate")
    ring(70, 25.5, 9.8)
    ring(70, 33, 12)

    body("AxialRetainerEnvelope", 386, material="Property class 10.9 steel",
         note="Nominal axial retainer OD32 h2 with M6 stud10 below; fits saddle counterbore with0.5radial/0.2axial clearance; thread/drive/locking detail unresolved")
    pad([(3, 0, 0)], 10, reversed=True)
    pad([(16, 0, 0)], 2)

    for index, (x, y) in enumerate(bolt_xy, 1):
        body(f"NutBolt{index}", 220, x=x, y=y, material="Property class 10.9 steel",
             note="M6x1 bolt with modeled diameter4.9 thread-root envelope; support ring8mm tapped engagement, nominal grip clearance; external thread ridges and head socket omitted")
        pad([(2.45, 0, 0)], 16, reversed=True)
        pad([(5, 0, 0)], 6)
    return CALLS


def optimal_bounds(shape: object) -> list[float]:
    """OCC's default B-spline control-pole box overstates helical dimensions."""
    bounds = shape.optimalBoundingBox(False, False)
    return [bounds.XMin, bounds.YMin, bounds.ZMin, bounds.XMax, bounds.YMax, bounds.ZMax]


def thread_dimensions(body: object) -> dict:
    shape = body.Shape
    bounds = shape.optimalBoundingBox(False, False)
    points, _triangles = shape.tessellate(0.03)
    radius = max(math.hypot(point.x, point.y) for point in points)
    if not shape.isValid() or bounds.XLength > 30.001 or bounds.YLength > 30.001 or radius > 15.001:
        raise RuntimeError(f"External thread exceeds Tr30 envelope: {bounds}; tessellated radius {radius}")
    return {"valid": shape.isValid(), "optimal_bounds_mm": optimal_bounds(shape),
            "optimal_xy_size_mm": [bounds.XLength, bounds.YLength],
            "tessellation_deflection_mm": 0.03, "tessellation_vertex_count": len(points),
            "maximum_tessellated_radius_mm": radius, "nominal_major_diameter_mm": 30,
            "passed_30_001_mm_envelope": True,
            "bound_method_note": "optimalBoundingBox(False, False) measures analytic geometry without cached triangulation; default BoundBox overstates the long helical B-spline control-pole enclosure"}


def refresh_validation_dimensions(document: object, path: Path = HERE / "tool_based/validation.json") -> dict:
    """Refresh an already-built report without rebuilding or changing its CAD."""
    report = json.loads(Path(path).read_text())
    by_label = {obj.Label: obj for obj in document.Objects if obj.TypeId == "PartDesign::Body"}
    for record in report["parts"]:
        record["bounds_mm"] = optimal_bounds(by_label[record["name"]].Shape)
    report["external_thread_dimension_check"] = thread_dimensions(by_label["PowerScrew"])
    report["bounding_box_method"] = "OpenCascade optimalBoundingBox (geometric extrema)"
    Path(path).write_text(json.dumps(report, indent=2) + "\n")
    return report


def build(output: Path = HERE / "tool_based") -> dict:
    from kala_agent.tools.protocol import validate_against_schema
    from kala_agent_freecad.cad.design import parse_request, run_open
    from kala_agent_freecad.cad.runtime import FreeCADModules
    from kala_agent_freecad.contracts import DESIGN_INPUT_SCHEMA
    import FreeCAD as App
    import Part
    import ObjectsFem

    output.mkdir(parents=True, exist_ok=True)
    calls = plan()
    compact = json.dumps(calls, separators=(",", ":"))
    arguments = {"calls_json": compact, "document_name": "KalaScrewJackTool",
                 "artifact_stem": "screw-jack-tool", "export_formats": ["step", "fcstd"]}
    validate_against_schema(arguments, DESIGN_INPUT_SCHEMA, path="$")
    (HERE / "calls.json").write_text(json.dumps(calls, indent=2) + "\n")
    (output / "request.json").write_text(json.dumps(arguments, indent=2) + "\n")
    modules = FreeCADModules(App, Part, ObjectsFem, None, ".".join(App.Version()[:3]),
                             "/usr/lib/freecad/lib", ("FreeCAD", "Part", "ObjectsFem"), "not requested")
    steps: list[str] = []
    trace_path = output / "tool_trace.log"
    trace_path.write_text("Codex-authored Kala calls; local reasoning and decision gateways bypassed.\n")
    def step(tool: str, sketch: str) -> None:
        steps.append(tool)
        message = f"[{len(steps)}/{len(calls)}] {tool} {sketch}"
        print(message, flush=True)
        with trace_path.open("a") as trace:
            trace.write(message + "\n")
        if getattr(App, "GuiUp", False):
            import FreeCADGui as Gui
            Gui.updateGui()
    result = run_open(parse_request(arguments), modules, on_step=step,
                      automatic_redesign=False, reuse_document=False)
    document = App.ActiveDocument
    bodies = [obj for obj in document.Objects if obj.TypeId == "PartDesign::Body"]
    records = []
    colors = {"BronzeNut": (0.76, 0.51, 0.19), "GuideBush": (0.7, 0.49, 0.19),
              "BasePlate": (0.19, 0.24, 0.31), "Housing": (0.23, 0.29, 0.36),
              "ThrustBearingEnvelope": (0.55, 0.58, 0.62),
              "HandleGripLeft": (0.08, 0.08, 0.09), "HandleGripRight": (0.08, 0.08, 0.09)}
    for obj in bodies:
        info = PARTS.get(obj.Label, {})
        for prop, value in (("MaterialSpecification", info.get("material", "")),
                            ("DesignDisclosure", info.get("note", ""))):
            obj.addProperty("App::PropertyString", prop, "KalaEngineering")
            setattr(obj, prop, value)
        obj.addProperty("App::PropertyString", "BuildMethod", "KalaEngineering")
        obj.BuildMethod = "Codex authored Kala native tool calls; all local models bypassed"
        if hasattr(obj, "ViewObject") and obj.ViewObject:
            obj.ViewObject.ShapeColor = colors.get(obj.Label, (0.66, 0.69, 0.73))
        shape = obj.Shape
        record = {"name": obj.Label, "valid": shape.isValid(), "solid_count": len(shape.Solids),
                  "volume_mm3": shape.Volume,
                  "bounds_mm": optimal_bounds(shape),
                  **info}
        records.append(record)
        if not record["valid"] or record["solid_count"] != 1 or record["volume_mm3"] <= 0:
            raise RuntimeError(f"Invalid native component: {record}")
        for child in obj.Group:
            if getattr(child, "TypeId", "") == "Sketcher::SketchObject":
                if child.DoF != 0:
                    raise RuntimeError(f"Unconstrained sketch {child.Name}: {child.DoF}")
            if hasattr(child, "ViewObject") and child.ViewObject:
                child.ViewObject.Visibility = child == obj.Tip
        if hasattr(obj, "ViewObject") and obj.ViewObject:
            obj.ViewObject.Visibility = True
    document.recompute()
    by_label = {obj.Label: obj for obj in bodies}
    spindle_gap = by_label["PowerScrew"].Shape.distToShape(by_label["DriveHead"].Shape)[0]
    if spindle_gap > 1e-6:
        raise RuntimeError(f"Integral spindle CAD regions have a {spindle_gap}-mm gap")
    native = output / "screw-jack-tool.FCStd"
    step_file = output / "screw-jack-tool.step"
    document.saveAs(str(native))
    Part.export(bodies, str(step_file))
    # Re-read neutral geometry rather than assuming the exporter succeeded.
    reread = Part.Shape()
    reread.read(str(step_file))
    if not reread.isValid() or len(reread.Solids) != len(bodies):
        raise RuntimeError("STEP round-trip lost component solids")
    files = [{"path": str(path), "bytes": path.stat().st_size,
              "sha256": hashlib.sha256(path.read_bytes()).hexdigest()} for path in (native, step_file)]
    report = {"build_method": "native Kala tool calls", "reasoning_model": "Codex in this chat",
              "local_models_called": [], "automatic_redesign": False, "call_count": len(calls),
              "calls_json_bytes": len(compact.encode()), "native_body_count": len(bodies),
              "manufacturing_part_count": len(bodies) - 1,
              "integral_spindle": {"cad_regions": ["PowerScrew", "DriveHead"],
                                   "face_gap_mm": spindle_gap, "manufacture": "one integral machined component"},
              "external_thread_dimension_check": thread_dimensions(by_label["PowerScrew"]),
              "bounding_box_method": "OpenCascade optimalBoundingBox (geometric extrema)",
              "all_sketches_fully_constrained": True, "parts": records, "files": files,
              "step_roundtrip": {"valid": reread.isValid(), "solid_count": len(reread.Solids)},
              "midstroke_mm": 60, "stroke_range_mm": [0, 120],
              "saddle_top_mm": {"collapsed": 328, "midstroke": 388, "raised": 448},
              "fem": result, "release_status": "engineering candidate; not certified production-ready",
              "limitations": ["Internal nut helical thread is specified but represented by clearance envelope.",
                              "Bearing is a purchasing envelope for NSK51305; installation fits and bearing life require verification.",
                              "No load test, fatigue assessment, welding qualification, tolerance validation or safety certification completed.",
                              "Axial saddle retainer is nominal; thread, drive, locking and vehicle interface require detailed design.",
                              "M6 bolts omit thread/socket detail; base attachment and fastener preload require design approval.",
                              "Removable collar is a nominal ring; its split/clamp retention is not modeled or validated."]}
    (output / "validation.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main(argv: list[str] | None = None) -> dict | None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=HERE / "tool_based")
    parser.add_argument("--plan-only", action="store_true")
    args = parser.parse_args(argv)
    if args.plan_only:
        calls = plan()
        (HERE / "calls.json").write_text(json.dumps(calls, indent=2) + "\n")
        print(json.dumps({"calls": len(calls), "compact_json_bytes": len(json.dumps(calls, separators=(",", ":")).encode())}))
    else:
        report = build(args.output)
        print(json.dumps(report, indent=2))
        return report


if __name__ == "__main__":
    main()
