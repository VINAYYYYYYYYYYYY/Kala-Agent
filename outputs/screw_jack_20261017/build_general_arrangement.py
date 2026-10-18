#!/usr/bin/env python3
"""Create a dimensioned vector general arrangement from the shared JSON basis."""
from pathlib import Path
import html
import json
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
b = json.loads((HERE / "design_basis.json").read_text())
g = b["geometry"]
S = 1.8
CX = 500
GROUND = 1020
items = []


def y(z):
    return GROUND - S * z


def line(x1, y1, x2, y2, cls="edge", extra=""):
    items.append(f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" class="{cls}" {extra}/>')


def text(x, yy, value, cls="label", anchor="start", extra=""):
    items.append(f'<text x="{x:.2f}" y="{yy:.2f}" class="{cls}" text-anchor="{anchor}" {extra}>{html.escape(str(value))}</text>')


def rect(x, yy, w, h, fill, cls="edge", extra=""):
    items.append(f'<rect x="{x:.2f}" y="{yy:.2f}" width="{w:.2f}" height="{h:.2f}" fill="{fill}" class="{cls}" {extra}/>')


def part(d, z0, z1, fill, inner=0):
    rect(CX - d * S / 2, y(z1), d * S, (z1 - z0) * S, fill)
    if inner:
        rect(CX - inner * S / 2, y(z1), inner * S, (z1 - z0) * S, "#ffffff")


def hdim(x1, x2, yy, label, extension_y=None):
    if extension_y is not None:
        line(x1, extension_y, x1, yy + 8, "ext")
        line(x2, extension_y, x2, yy + 8, "ext")
    line(x1, yy, x2, yy, "dim", 'marker-start="url(#arrow)" marker-end="url(#arrow)"')
    text((x1 + x2) / 2, yy - 9, label, "dimtext", "middle")


def vdim(xx, z0, z1, label, extension_x):
    line(extension_x, y(z0), xx + 8, y(z0), "ext")
    line(extension_x, y(z1), xx + 8, y(z1), "ext")
    line(xx, y(z0), xx, y(z1), "dim", 'marker-start="url(#arrow)" marker-end="url(#arrow)"')
    mid = (y(z0) + y(z1)) / 2
    text(xx - 9, mid, label, "dimtext", "middle", f'transform="rotate(-90 {xx - 9} {mid})"')


def leader(x0, z0, xx, yy, label):
    elbow = xx - 22
    line(x0, y(z0), elbow, yy - 6, "lead")
    line(elbow, yy - 6, xx - 7, yy - 6, "lead")
    items.append(f'<circle cx="{x0}" cy="{y(z0)}" r="3" fill="#294c6d"/>')
    text(xx, yy, label)


items.append('''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1800 1280" role="img" aria-labelledby="title desc">
<title id="title">Preliminary screw-jack general arrangement, 2,000 kg target, 120 mm travel</title>
<desc id="desc">Dimensioned common general arrangement with section elevation, base plan, and release notes. Engineering review only, not a fabrication drawing or load rating.</desc>
<defs>
<marker id="arrow" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#294c6d"/></marker>
<pattern id="steel" width="8" height="8" patternUnits="userSpaceOnUse"><rect width="8" height="8" fill="#eef2f5"/><path d="M-2,8 L8,-2 M6,10 L10,6" stroke="#b5c1cb" stroke-width="1"/></pattern>
<pattern id="bronze" width="9" height="9" patternUnits="userSpaceOnUse"><rect width="9" height="9" fill="#f4dfaa"/><path d="M0,9 L9,0" stroke="#c09a45" stroke-width="1"/></pattern>
</defs>
<style>
text{font-family:Arial,Helvetica,sans-serif;fill:#172c3d}.title{font-size:30px;font-weight:700}.subtitle{font-size:19px}.warn{font-size:22px;font-weight:700;fill:#a13b22}.label{font-size:19px}.small{font-size:17px}.tiny{font-size:15px}.dimtext{font-size:18px;fill:#294c6d}.edge{stroke:#243847;stroke-width:1.7}.ext{stroke:#708795;stroke-width:1}.dim{stroke:#294c6d;stroke-width:1.2}.lead{stroke:#294c6d;stroke-width:1.1;fill:none}.center{stroke:#7091a6;stroke-width:1;stroke-dasharray:12 4 2 4}.ghost{stroke:#8d9398;stroke-width:1.2;stroke-dasharray:7 5;fill:none}
</style><rect width="1800" height="1280" fill="#ffffff"/>
''')
text(52, 55, "KALA · SCREW JACK · COMMON GENERAL ARRANGEMENT", "title")
text(52, 88, "Target 2,000 kg / 120 mm stroke · Tr30×4 RH · dimensions mm · 17 October 2026", "subtitle")
text(52, 124, "PRELIMINARY — NOT RELEASED FOR FABRICATION OR VEHICLE LIFTING", "warn")
line(52, 145, 1745, 145, "ext")
text(285, 193, "SECTION ELEVATION · shown at 60 mm extension", "subtitle")

# Dashed positions establish the stroke while avoiding a false detailed assembly.
for z, title in ((328, "collapsed top 328"), (448, "raised top 448")):
    line(410, y(z), 750, y(z), "ghost")
    text(755, y(z) - 9, title, "small")

# Housing cut section, nominal load-bearing pieces and moving spindle.
part(200, 0, 12, "url(#steel)")
part(70, 12, 212, "url(#steel)", 56)
part(90, 204, 212, "url(#steel)", 56)
part(54, 164, 212, "url(#bronze)", 30.2)
part(54, 160, 164, "url(#bronze)", 32)
part(80, 212, 220, "url(#bronze)", 32)
part(25.5, 100, 330, "#d3dbe1")
part(36, 94, 100, "#8f9ca5", 25.7)
part(48, 330, 358, "url(#steel)")
part(25, 358, 386, "#d3dbe1")
part(52, 358, 376, "#bcc6ce", 25)
part(70, 376, 388, "url(#steel)", 25.5)
line(CX, y(410), CX, y(-10), "center")

# External thread is a drawing symbol rather than an invented tolerance section.
for z in range(102, 329, 4):
    line(CX - 15 * S, y(z), CX - 12.75 * S, y(z + 1.3), "ext")
    line(CX + 12.75 * S, y(z + 1.3), CX + 15 * S, y(z + 2.6), "ext")
line(CX - 15 * S, y(100), CX - 15 * S, y(330), "ext")
line(CX + 15 * S, y(100), CX + 15 * S, y(330), "ext")
rect(CX - 250 * S, y(352), 500 * S, 16 * S, "#8c9ba5")
line(CX - 255 * S, y(344), CX + 255 * S, y(344), "center")
hdim(CX - 250 * S, CX + 250 * S, 250, "HANDLE Ø16 × 500", y(352))
hdim(CX - 100 * S, CX + 100 * S, 1070, "200", GROUND)
vdim(1020, 0, 388, "388 shown", CX + 100 * S)
vdim(1100, 328, 448, "120 travel / 30 turns", 750)
vdim(190, 100, 330, "230 threaded length", CX - 15 * S)
vdim(275, 164, 212, "48 thread engagement", CX - 27 * S)
leader(CX + 35 * S, 25, 750, 945, "Housing Ø70 / Ø56")
leader(CX + 27 * S, 185, 750, 746, "Bronze nut Ø54 · thread length 48")
leader(CX + 40 * S, 216, 750, 595, "Flange Ø80 × 8 on support ring Ø90 × 8")
leader(CX + 18 * S, 97, 750, 827, "Stop collar Ø36 / Ø25.7 × 6 · retention TBD")
leader(CX + 24 * S, 355, 1180, 291, "Integral drive head Ø48 × 28 / Ø17 bore")
leader(CX + 26 * S, 367, 1180, 325, "NSK51305 candidate · Ø25 / Ø52 × 18")
leader(CX + 35 * S, 385, 1180, 359, "Saddle Ø70 × 12 · journal Ø25 / bore Ø25.5")

text(1180, 416, "COMMON DESIGN CONTROLS", "subtitle")
notes = [
    "• Base 200 × 180 × 12; housing top z212.",
    "• Tr30×4 single-start RH; basic root Ø25.5.",
    "• Strength screening uses minimum root Ø25.0.",
    "• Working target 19.62 kN; assumed proof 29.43 kN.",
    "• Nut engagement remains 48 throughout travel.",
    "• Raised screw starts z160; nut thread starts z164.",
    "• Collar top contacts nut-skirt underside at z160.",
    "• Stop clamp/attachment is NOT designed or rated.",
    "• Ø54 nut in Ø56 housing needs fitted location.",
    "• M6 flange screws nominal; fit/preload not released.",
    "• Seat articulation / vehicle interface unresolved.",
    "• Closed height 328 limits vehicle applicability.",
]
for i, note in enumerate(notes):
    text(1180, 451 + 29 * i, note, "small")

text(1180, 832, "BASE PLAN · common envelope", "subtitle")
PX, PY = 1370, 1028
rect(PX - 100 * S, PY - 90 * S, 200 * S, 180 * S, "url(#steel)")
for diameter, col in ((90, "#d3dbe1"), (70, "#e4e8eb"), (56, "#ffffff")):
    items.append(f'<circle cx="{PX}" cy="{PY}" r="{diameter * S / 2}" fill="{col}" class="edge"/>')
line(PX - 210, PY, PX + 210, PY, "center")
line(PX, PY - 185, PX, PY + 185, "center")
hdim(PX - 100 * S, PX + 100 * S, 1230, "200", PY + 90 * S)
line(PX + 100 * S, PY - 90 * S, 1610, PY - 90 * S, "ext")
line(PX + 100 * S, PY + 90 * S, 1610, PY + 90 * S, "ext")
line(1600, PY - 90 * S, 1600, PY + 90 * S, "dim", 'marker-start="url(#arrow)" marker-end="url(#arrow)"')
text(1622, PY + 6, "180", "dimtext")

text(52, 1136, "CAD-route details differ: native call build uses editable feature regions; direct script adds gussets and rounded base corners.", "small")
text(52, 1166, "Mounting holes / grip details / retention hardware are excluded here. Use each CAD validation and BOM to inspect its actual geometry.", "small")
text(52, 1196, "General arrangement only. No dimensional tolerances, weld symbols, fits, heat-treatment process or product rating are established.", "small")
text(52, 1226, "Source: design_basis.json revision " + b["revision"] + " · regenerate with build_general_arrangement.py", "tiny")
items.append("</svg>")
svg = "\n".join(items)
ET.fromstring(svg)
(HERE / "general_arrangement.svg").write_text(svg, encoding="utf-8")
print(HERE / "general_arrangement.svg")
