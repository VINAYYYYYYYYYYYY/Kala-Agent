#!/usr/bin/env python3
"""Independent BREP/STEP and coupled-motion checks of saved jack assemblies."""
from __future__ import annotations

import json
import sys
from itertools import combinations
from pathlib import Path

sys.path.insert(0, '/usr/lib/freecad/lib')
import FreeCAD as App
import Part

HERE = Path(__file__).resolve().parent
TOLERANCE_MM3 = 0.01


def mode(obj):
    if hasattr(obj, 'MotionGroup'):
        return obj.MotionGroup
    if obj.Label in {'BasePlate', 'Housing', 'BronzeNut'} or obj.Label.startswith('NutBolt'):
        return 'fixed'
    if 'Bearing' in obj.Label or 'Saddle' in obj.Label and 'Retain' not in obj.Label:
        return 'vertical'
    return 'screw'


def verify(native, neutral):
    doc = App.openDocument(str(native))
    objects = [obj for obj in doc.Objects if obj.TypeId == 'PartDesign::Body' or hasattr(obj, 'PartNumber')]
    checks = {'native_file': str(native), 'part_count': len(objects), 'solid_checks': [], 'dimensional_checks': [], 'poses': []}
    try:
        for obj in objects:
            shape = obj.Shape
            checks['solid_checks'].append({'name': obj.Label, 'valid': shape.isValid(),
                                          'solid_count': len(shape.Solids), 'volume_mm3': shape.Volume})
            if obj.Label == 'PowerScrew' or getattr(obj, 'PartNumber', '') == 'SJ-004':
                # Clip out the larger integral drive head before testing Ø30.
                region = shape.common(Part.makeBox(100, 100, 229.8, App.Vector(-50, -50, 100.1)))
                envelope = Part.makeCylinder(15.001, 230, App.Vector(0, 0, 100))
                excess = region.cut(envelope).Volume
                checks['dimensional_checks'].append({'part': obj.Label, 'maximum_thread_diameter_mm': 30.002,
                                                     'volume_outside_thread_envelope_mm3': excess,
                                                     'passed': excess < TOLERANCE_MM3})
        shape = Part.Shape()
        shape.read(str(neutral))
        checks['step_roundtrip'] = {'valid': shape.isValid(), 'solid_count': len(shape.Solids),
                                    'same_solid_count': len(shape.Solids) == len(objects)}
        for lift in (0.0, 30.0, 60.0, 90.0, 120.0):
            delta = lift - 60
            shapes = []
            for obj in objects:
                part = obj.Shape.copy()
                motion = mode(obj)
                if motion == 'screw':
                    part.rotate(App.Vector(), App.Vector(0, 0, 1), delta * 360 / 4)
                if motion != 'fixed':
                    part.translate(App.Vector(0, 0, delta))
                shapes.append((obj.Label, part))
            interference = []
            for (left, a), (right, b) in combinations(shapes, 2):
                if not a.BoundBox.intersect(b.BoundBox):
                    continue
                volume = a.common(b).Volume
                if volume > TOLERANCE_MM3:
                    interference.append({'a': left, 'b': right, 'volume_mm3': volume})
            checks['poses'].append({'lift_mm': lift, 'screw_rotation_from_mid_deg': delta * 90,
                                    'interferences': interference, 'clear': not interference})
        checks['passed'] = (
            bool(objects)
            and all(item['valid'] and item['solid_count'] == 1 and item['volume_mm3'] > 0 for item in checks['solid_checks'])
            and checks['step_roundtrip']['valid']
            and checks['step_roundtrip']['same_solid_count']
            and bool(checks['dimensional_checks'])
            and all(item['passed'] for item in checks['dimensional_checks'])
            and all(pose['clear'] for pose in checks['poses'])
        )
        return checks
    finally:
        App.closeDocument(doc.Name)


def main():
    reports = []
    for folder, stem in [('tool_based', 'screw-jack-tool'), ('script_based', 'screw_jack_script')]:
        print('Verifying ' + folder, flush=True)
        reports.append(verify(HERE / folder / (stem + '.FCStd'), HERE / folder / (stem + '.step')))
    report = {'kernel': 'FreeCAD ' + '.'.join(App.Version()[:3]),
              'intersection_threshold_mm3': TOLERANCE_MM3,
              'scope': 'Geometry and five coupled-motion samples, not structural or production qualification',
              'builds': reports, 'passed': all(item['passed'] for item in reports)}
    (HERE / 'assembly_verification.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
