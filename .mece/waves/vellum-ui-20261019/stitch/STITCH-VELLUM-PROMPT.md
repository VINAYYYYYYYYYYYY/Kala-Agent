# Kala Studio — Stitch Design Prompt Pack ("The Vellum Drafting Board")

> **Design North Star:** *"The user should feel the nostalgia and joy of using a pencil and drawing board again when using our product."*

---

## 1. Architectural & UX Synthesis

From [ui-screens-wireframes.md](file:///home/lad00/Downloads/ui-screens-wireframes.md) and [architecture-report.md](file:///home/lad00/Projects/Kala-Agent/docs/architecture-report.md), **Kala Studio** is a local-first, single-user mechanical engineering agent (`KalaApplication`) backed by an append-only SQLite hash-chained event journal, a content-addressed SHA-256 artifact store, an OpenCascade (`STEP`/`STL`) parametric geometry & nominal estimation workbench, and an optional FreeCAD native bridge (`cad.freecad-geometry`, `cad.freecad-fem`).

To honor your spatial and aesthetic requirements, the existing multi-tab clutter is distilled into a **pure, zero-distraction Drafting Board layout** where every panel maps to an authentic physical drafting instrument:

```
+----------------------------------+---------------------------------------------------+-------------------------------+
| LEFT COLUMN (340px)              | CENTER VIEWPORT (Flex 1 — NO TOP BAR)             | RIGHT COLUMN (320px)          |
|AGENT CHAT & INPUT                | THE VELLUM DRAFTING SHEET                         |                               |
|                                  | (Full height from top window edge to bottom bar)  | TOP-RIGHT: SECONDARY DISPLAY  |
| - Conversational Engineering     |                                                   | (220px tall)                  |
|   Thread (Graphite notes on      |   - Hand-drawn 2H pencil isometric/ortho grid     | - 2D Cross-Section / Cutaway  |
|   ruled vellum margin)           |   - Center 2D/3D Model (Bracket / Shaft / Housing)|   with 45° ANSI hatching      |
| - Quick Sketch Starters          |   - Mode Switch:                                  | - Creative Aux Views (Ortho   |
|   [Size Bracket] [Size Shaft]    |     [ 2D Pencil Vector  |  3D Solid Clay Model ]  |   Projection / Tolerance Map) |
| - Multi-modal Input Composer:    |   - Hand-lettered redline callouts & leader lines |                               |
|   Text prompt, Voice/Sketch      |     with arrowheads (← 40.0 mm →)                 +-------------------------------+
|   input icons, Remote Egress     |                                                   | BOTTOM-RIGHT: AGENT LOGS      |
|   Consent stamp                  |                                                   | (Flex 1 to bottom)            |
|                                  +---------------------------------------------------+ - Live Kernel & CAD Event Log |
|                                  | BOTTOM END BAR (48px — Under Main Viewport)       |   (sketch.new, pad, BRepGProp,|
|                                  | [2D Vector|3D Solid] | 40×20×30mm · 24.0cm³ |     |   PRE_COMMIT, SHA-256 digest) |
|                                  | [Grid][Axes][Dims][Cut] | [STEP][FreeCAD] [⚙ Opt] |                               |
+----------------------------------+---------------------------------------------------+-------------------------------+
```

---

## 2. Master Stitch Prompt (Copy & Paste Directly into Stitch)

Use this prompt in **Google Stitch (Standard Mode)** to generate the primary desktop workspace:

```text
Desktop CAD & Mechanical Engineering Agent workspace (1440x900), themed as a tactile vintage architectural and mechanical drafting board ("Pencil & Vellum Theme"). The user should immediately feel the warmth, nostalgia, and tactile joy of drawing with a mechanical pencil on translucent vellum paper pinned to an architect's drafting table.

AESTHETIC & COLOR TOKENS (USE EXACTLY):
- Primary Canvas Surface (Vellum Sheet): Warm aged cream vellum (#F6F3EB) with subtle paper grain and a double-line drafted border around the sheet perimeter.
- Secondary Panel Surface (Tracing Paper / Margin): Warm alabaster (#EFECE4) with crisp 0.5px graphite hairlines (#C8C2B6) instead of drop shadows.
- Primary Ink (2B Graphite): Deep charcoal graphite (#23211E) for main geometry strokes, primary headings, and active icons.
- Construction Lines (2H Hard Lead): Muted silver-graphite (#7A756C) for construction lines, extension lines, and secondary labels.
- Grid Lines: Delicate hand-drawn pencil grid (#DFDBD0 major 50mm squares, #EAE6DC minor 10mm subdivisions) with tiny hand-ticked crosshair intersections (+).
- Dimension & Callout Accent (Drafting Redline / Carmine Pencil): Terracotta drafting red (#C84B31) for dimension arrows, section cut planes (A–A), and tolerance callouts.
- Blueprint / Verification Accent (Prussian Blue Ink): Technical fountain-pen blue (#2B5B84) for verified B-Rep badges, active toggles, and selection highlights.
- Approval / Live Stamp (Forest Stamp Ink): Vintage rubber-stamp green (#2A6B4C) for "SERVER VERIFIED", "PRE_COMMIT READY", and live connection dots.
- Typography: Technical drafting lettering (Space Grotesk or Architect's Daughter feel for headers, Inter for clean body, JetBrains Mono / IBM Plex Mono for all dimensions, tolerances, volumes, and logs). Zero glossy gradients, zero neon, zero dark-mode cyberpunk.

STRICT 4-ZONE SPATIAL LAYOUT (1440x900, overflow hidden):

1. LEFT SIDEBAR (340px wide, full height) — AGENT CHAT & INPUT MARGIN:
- Styled like a bound engineer's calculation pad on the left edge of the drawing board.
- Minimal header stamp at top: "KALA // MECHANICAL DESIGN AGENT" with a small green ink dot "LOCAL LOOPBACK".
- Quick starter chips styled as taped paper tabs: "[Size L-Bracket]", "[Size Stepped Shaft]", "[Housing Envelope]", "[STEP Handoff]".
- Scrollable conversation thread:
  - User message: "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes."
  - Agent response: Clean technical note with a mini inline dimension table (Envelope: 60.0 × 45.0 × 40.0 mm | Nominal Volume: 28.16 cm³ | Material: 6061-T6 Surrogate) and a subtle Prussian-blue annotation: "Parametric B-Rep ready. Section A-A updated in auxiliary view."
- Bottom Input Composer:
  - Multi-modal input bar at the bottom of the left column with a tactile pencil-sketch text area ("Describe a part, tolerance, or revision..."), small mechanical-pencil and microphone/attachment icons for input methods, a compact "Remote Egress Consent" checkbox stamp, and a charcoal button "[ Draft Part ↵ ]".

2. CENTER MAIN VIEWPORT (800px wide, starts directly at the VERY TOP edge of the window — NO top header bar above the viewport):
- Full-bleed warm vellum drafting sheet (#F6F3EB) framed by a classic 1px technical drawing border with zone coordinate ticks (A–F, 1–8) along the outer margin.
- Background features a delicate pencil-drawn isometric/orthographic grid.
- Centerpiece: A precision mechanical L-bracket rendered in a hybrid "2D Pencil Vector <-> 3D Solid Model" view:
  - Show the part mid-transition or with a crisp toggle pill floating subtly on the vellum: "[ ✎ 2D Pencil Vector (Active) | ⬡ 3D Shaded Clay ]".
  - The model displays hand-drafted 2B graphite visible edges, dashed 2H hidden-line bore pass-throughs, redline centerline axes (— · —), and hand-lettered dimension callouts (`← 60.00 mm →`, `Ø 10.00 H7`, `R 4.0`, cutting plane line `A ——— A` in terracotta red #C84B31).
- Bottom-left corner of the vellum sheet has a classic drafted Title Block watermark: "PART: KALA-BRKT-01 | SCALE: 1:1 | UNITS: MM | REV: 03".

3. BOTTOM END BAR (48px tall, directly underneath the Center Main Viewport only):
- Styled like a brass-and-boxwood engineer's scale ruler docked along the bottom edge of the drawing board.
- Left cluster (View & Canvas Toggles): Icon buttons with crisp graphite line art for `[2D/3D Switch]`, `[Pencil Grid]`, `[Center Axes]`, `[Redline Dims]`, `[Section Cut A-A]`, `[Reset View]`.
- Center cluster (Live Nominal Readout in Monospace): `ENVELOPE: 60.0 × 45.0 × 40.0 mm` · `VOL: 28,160 mm³ (28.16 cm³)` · `AREA: 9,420 mm²` · `[✓ B-Rep Verified]`.
- Right cluster (Quick Actions & Options Menu Trigger): `[Export STEP]`, `[FreeCAD Live]`, and a prominent mechanical gear/compass icon button `[ ⚙ Options & Tools ]`.

4. RIGHT COLUMN (300px wide, full height, split vertically into two stacked panels):
- TOP-RIGHT PANEL (250px tall) — SECONDARY AUXILIARY DISPLAY (CROSS-SECTION & CREATIVE SCOPE):
  - Framed like a pinned detail loupe / auxiliary drafting detail card.
  - Header bar with mode selector tabs: `[ Section A-A (Active) | Ortho 3-View | Tolerance Map ]`.
  - Inside the display: When the main viewport is in 2D/3D mode, this shows the live **2D Cross-Section Cut (Section A–A)** through the 10 mm central bore and 8 mm wall, complete with classic 45-degree diagonal ANSI pencil cross-hatching on the cut faces, wall-thickness redline arrows (`8.00 mm`), and bore depth callout.
- BOTTOM-RIGHT PANEL (Remaining 650px height) — AGENT & KERNEL EXECUTION LOGS:
  - Styled like an engineer's punched-tape / monospace ledger sheet (`#EFECE4`).
  - Header: `AGENT & KERNEL JOURNAL` with a small status stamp `[HASH-CHAIN OK]`.
  - Scrollable monospace log entries showing real Kala Agent & FreeCAD steps:
    - `16:04:12 [plan.created] Parsed bracket spec (60x45x40 mm)`
    - `16:04:13 [sketch.new] Sketch001 on XY_Plane (6 constraints)`
    - `16:04:14 [partdesign.pad] Pad001 length=8.00 mm`
    - `16:04:14 [partdesign.pocket] Bore Ø10.00 mm ThroughAll`
    - `16:04:15 [ocp.brepgprop] Vol=28160.0 mm³ | Drift=0.000 mm`
    - `16:04:16 [policy.checkpoint] PRE_COMMIT approved (sha256: 8f3a…c91b)`
    - `16:04:17 [freecad.bridge] Synced to FreeCAD 1.1.3 (Workspace 3)`
```

---

## 3. Secondary Stitch Prompt — Options Menu & Modal Wireframes

Since all secondary functionalities (Parametric Sliders, CAD/CAE Target Registry, Export & Handoff, Provider API Keys, and Loopback Boot Gate) live inside the **Options Menu (`[ ⚙ Options & Tools ]`)** triggered from the bottom end bar, run this second prompt in Stitch to generate the wireframe overlay sheet:

```text
Desktop wireframe & UI kit overlay sheet (1440x900) for "Kala Studio" in the exact same "Pencil & Vellum Drafting Board" theme (warm cream vellum #F6F3EB, alabaster panels #EFECE4, 2B charcoal graphite ink #23211E, terracotta redline #C84B31, Prussian blue #2B5B84, monospace numerals, zero drop shadows, 1px drafted borders).

Show the "OPTIONS & ENGINEERING DRAWER" modal overlay triggered from the bottom bar [⚙ Options] button, organized into 4 clean drafted wireframe cards on a single vellum sheet:

CARD 1: PARAMETRIC INSPECTOR & PRIMITIVE CONTROLS
- Shape selector tabs: [ Box | Cylinder | Sphere | Custom Script ]
- Tactile Vernier-style numeric inputs & ruler sliders (in mm):
  - Width (size_x): [ 60.0 mm ] | Height (size_y): [ 45.0 mm ] | Depth (size_z): [ 40.0 mm ] | Wall / Bore: [ 8.0 mm ]
- "OPENCASCADE NOMINAL ESTIMATE" stamped box:
  - Volume: 28,160 mm³ (28.16 cm³) | Surface Area: 9,420 mm² | Bounds Drift Warning indicator (< 0.01 mm).
  - Honesty footnote in italic pencil script: "Closed-form nominal geometry for sizing & handoff. Not a certified FEA solver result."

CARD 2: NEUTRAL-FILE EXPORT & CAD/CAE HANDOFF
- Target Suite Dropdown: [ FreeCAD 1.1 (Connected) | SOLIDWORKS (Neutral File) | Fusion 360 | ANSYS Mechanical | Abaqus ]
- Format Radio Pills: [ STEP (ISO-10303-21) ] [ STL Mesh ] [ Native FCStd ]
- Primary Stamp Button: "[ Export & Stamp STEP Artefact ]"
- Receipt line: "✓ Saved bracket-60x45x40.step · 14,820 bytes · sha256 verified"

CARD 3: MODEL GATEWAY & API PROVIDERS (SERVER-SIDE VAULT)
- Provider rows with status stamps:
  - Local Deterministic Helper: [ ACTIVE · OFFLINE ]
  - OpenRouter (Reasoning & Decision Models): [ CONFIGURED ] — Masked key input `[ sk-or-v1-•••••••••••• ]` + `[Save]` `[Clear]`
  - OpenAI Compatible: [ NOT CONFIGURED ] — Masked key input + Egress allowlist toggle.

CARD 4: LOOPBACK BOOT GATE & CONNECTION RECOVERY (MINI WIREFRAME)
- Compact drafted card showing the initial connection state:
  - Backend Origin input: `[ http://127.0.0.1:8765 ]`
  - Per-launch Bearer Token input: `[ •••••••••••••••••••••••• ]`
  - Status stamp: `[ Connect Loopback ]` and `[ Retry Bridge ]` with in-memory token privacy notice.
```

---

## 4. Quick Follow-Up Refinement Prompts for Stitch
If Stitch drifts toward a modern SaaS look on the first generation, paste any of these 1-line corrections into Stitch's chat:
- *"Remove any top navbar above the center viewport; the center vellum drawing canvas must touch the very top edge of the window, with only the 48px scale-ruler bar at the bottom."*
- *"Replace all flat digital cards with drafted technical title-block borders (1px solid #23211E outer frame, 0.5px #C8C2B6 inner dividers, corner tick marks, and warm #F6F3EB vellum texture)."*
- *"Make the top-right secondary display clearly show a 2D cross-section cut (Section A-A) with 45-degree diagonal hatching lines and redline dimension arrows."*
- *"Ensure the bottom-right panel is a dedicated monospace Agent & Kernel Log stream right underneath the top-right cross-section display."*
