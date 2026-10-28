# KALA STUDIO — MASTER UI/UX REDESIGN PROMPT
*(Optimized through 10 Rounds of Senior Product Designer & UI/UX Engineering Critique)*

---

## COPY-PASTE DESIGN PROMPT

```markdown
# Role & Objective
You are a Principal Industrial Software Product Designer and Lead CAD/UI Systems Engineer. Redesign **Kala Studio**, an AI-native mechanical engineering and CAD workbench, into **"The AS 1100 Living Drafting Sheet"** — an interface that combines the spatial purity, calm authority, and geometric precision of an Australian Standard AS 1100 / ISO 5457 vellum technical drawing sheet with tactile, reactive AI drafting mechanics.

---

## 1. Core Design Philosophy & Anti-Clutter Mandate
1. **The Blank AS 1100 Sheet First Impression:** When Kala Studio opens, it must feel as open, breathable, and disciplined as a fresh vellum drafting sheet taped to a drawing board.
2. **Zero Left Sidebar:** Completely eliminate the 340px left sidebar and all left-side text panels. The primary drawing canvas spans uninterrupted from the left border to the right-hand engineering dock.
3. **Every Line Earns Its Ink:** If an element does not directly aid spatial measurement, model inspection, active generation control, or agent/CAD telemetry, it does not exist on the sheet.

### Strictly Removed Legacy Elements (Do Not Render)
- **Removed `#2` & `#8`:** No floating HLR description paragraphs (`HLR · Preliminary...`) or floating envelope text (`ISO · FROM (+X -Y +Z)...`) over the canvas.
- **Removed `#3` & `#4`:** No bottom-bar telemetry text strip (`ENV 160x60...`) and no redundant bottom-right `Report` / `Options` button strip (the CAD connection badge moves into `#1`).
- **Removed `#7`:** No static 3-view orthographic cluster inset overlapping the main 3D canvas.
- **Removed `#9`, `#10`, `#11`, `#12`, `#15`:** No left-sidebar prompt box (`#9`), no left-sidebar geometry card (`#10` shifts into `#5`), no preset template buttons (`#11`), no top-left branding header (`#12`), and no inline sidebar API key drawer (`#15`).
- **Removed `#16` & `#17`:** No full-screen 4-card Options & Engineering Modal overlay or modal footer. All functional configuration lives inside the Engineer Profile Menu anchored in `#1`.

---

## 2. Visual Language, Color Tokens & Typography

### Surface & Ink Palette (Vellum Drafting Theme)
- **Primary Sheet Surface (`--vellum-bg`):** `#F4EFE6` (Warm Drafting Vellum with subtle 2% monochromatic paper grain).
- **Elevated Dock / Paper-Roll Surface (`--vellum-elevated`):** `#FAF6EE` with crisp hairline borders.
- **Primary Structural Linework (`--ink-primary`):** `#181614` (India Ink — `1.5px` outer AS 1100 sheet frame, `1px` dock dividers, `0.75px` title-block cell rules).
- **Secondary Technical Linework (`--ink-secondary`):** `#4A453E` (2H Graphite — dimensions, leader lines, secondary labels).
- **Muted / Construction Linework (`--ink-muted`):** `#8C857B` (4H Construction Pencil — hidden lines, inactive states).
- **Interactive Redline & Selection Accent (`--accent-redline`):** `#C83E2B` (Drafting Vermilion — vector hover/click selection, active section plane, abort action).
- **Sub-Assembly / Fit Highlight (`--accent-amber`):** `#D97706` (Amber Ink — highlights active part location inside the `#6` Assembly Mini-Map).
- **Verification Pass Stamp (`--stamp-pass`):** `#23533D` (Blueprint Forest Green — valid B-Rep & manifold checks).

### Typography
- **Technical Telemetry, Coordinates, Dimensions & Title Block Cells:** `JetBrains Mono` or `IBM Plex Mono` (`font-variant-numeric: tabular-nums`, uppercase tracking `0.06em` for cell headers, `10px–12px`).
- **Prompt Input, Branch Trees & Interactive Controls:** `Space Grotesk` or `Inter` (`13px–14px`, crisp geometric legibility).

---

## 3. Macro Spatial Architecture (AS 1100 Sheet Grid)

Frame the entire application viewport inside an authentic **AS 1100 / ISO 5457 Double-Ruled Drawing Border**:
- **Outer Margin Ring (`20px` inset from viewport edges):** Contains alphanumeric zone reference ticks (`1` through `8` along top/bottom; `A` through `F` along left/right) and four triangular centering arrows at the midpoints of each edge.
- **Two Macro Interior Zones:**
  1. **The Main Drafting Field (Left + Center, `calc(100vw - 340px)` width, `100%` height):** Unobstructed interactive 3D/2D vellum viewport with a zoom-synchronized spatial measurement grid.
  2. **The Right Engineering Dock (`340px` fixed width, docked flush against the right and bottom inner frame borders):** Vertically stacked into three precision instruments:
     - **Top-Right (`#6`, `28%` height):** 2D Engineering Mini-Map ("Spatial & Assembly Locator").
     - **Middle-Right (`#5`, `48%` height):** Graphite Process & B-Rep Verification Trail (absorbs `#10`).
     - **Bottom-Right (`#1`, `24%` height):** AS 1100 Live Agent Title Block & Square Engineer Profile Button (absorbs `#4`, `#15`, `#16`).

---

## 4. Detailed Component & Interaction Specifications

### A. Calibrated Spatial Measurement Grid (`#0` Canvas Background)
Replace decorative background dots with a **Zoom-Synchronized Engineering Scale Grid**:
- **Major Grid Squares:** `0.6px` warm graphite lines (`rgba(24, 22, 20, 0.11)`) representing real metric units (`10 mm`, `50 mm`, or `100 mm` depending on zoom level).
- **Minor Subdivisions:** `1px` crisp graphite micro-dots (`rgba(24, 22, 20, 0.22)`) at `1 mm` / `5 mm` intervals that smoothly fade in as the engineer zooms closer.
- **Scale Reference Pill:** A minimal monochrome stamp in the bottom-left corner of the sheet margin (`SCALE 1:1 · GRID = 10 MM`) that dynamically updates as the camera zooms, giving the engineer immediate visual size intuition.

---

### B. Above-Border "Paper-Roll" Hierarchical Assembly Tabs (`#13a`) & Below-Border Tool Strip (`#13b`)
Solve the `#13` overlap by strictly separating **Model Navigation** from **Canvas Tools** across the top black border line:

1. **Above the Top Black Border (`#13a` — Hierarchical Assembly Tabs):**
   - Generated or imported models sit **on top of the upper black frame border** as physical index tabs (e.g., `SCREW JACK ASSY [6]` or `JAGDTIGER TANK [44]`).
   - **Paper-Roll Unfurl Animation:** Clicking an assembly tab's part-count badge or chevron unfurls a vertical **Paper-Roll Dropdown** (`cubic-bezier(0.22, 1, 0.36, 1)` unroll animation with subtle paper-curl shading and horizontal ruled lines) listing the full component hierarchy inside that model (e.g., `Springs`, `Hex Bolts M8`, `Base Plate`, `Threaded Spindle`).
   - **Obsidian-Style Pinned Sub-Tabs:** Clicking any component branch inside the unrolled paper list pins a compact **Mini Sub-Tab** directly underneath the parent tab along the top border edge (`└─ [ Threaded Spindle × ]`). Multiple sub-parts can be pinned side-by-side beneath their parent tab for instant switching between the full assembly and individual parts.
   - Include a clean `[ + Import / New Sheet ]` tab trigger at the end of the tab rail.

2. **Below the Top Black Border (`#13b` — Vertical Drafting Tool Strip):**
   - Anchored **inside** the canvas (`top: 16px`, `left: 16px`), completely clear of the tabs above.
   - Houses tactile `32px × 32px` monochrome drafting tool buttons:
     - `Reset Isometric View`
     - `Toggle Dimensions & Leaders`
     - `Point-to-Point Caliper Measure`
     - `Interactive Section Plane (A-A)`
     - `Grid Snap & Scale Lock`
     - `[ + ]` **Extensibility Slot** for future engineering add-ons (FEA Stress Overlay, DFM Undercut Check, Tolerance Stackup).

---

### C. View & Finish Switcher (`#14`) + Clickable 2D Orthographic Vectors (Replaces `#7` & `#14`)
Anchored at the top-right of the main canvas (`top: 16px`, `right: 16px` inside the drafting field):

1. **Primary View Mode Segmented Switch:**
   - **`[ 3D Model ]`** *(Default)*
   - **`[ 2D Orthographic ]`** *(Full-sheet interactive 3rd-angle drafting layout: `TOP`, `FRONT`, `RIGHT` views — replacing the deleted `#7` inset)*

2. **3D Surface Finish Sub-Toggle (Visible when `3D Model` is active):**
   - **`◐ Graphite Shade`** *(Default)*: Renders the 3D B-Rep geometry with a custom **technical pencil / graphite illustration shader** (warm paper highlights, cross-hatched shadow troughs, crisp `1.2px` ink silhouette and crease edges).
   - **`● CAD Color`**: Switches the 3D viewport to distinct metallic/engineering material colors per assembly component.

3. **Interactive Clickable Vectors (Active when `2D Orthographic` is selected):**
   - In `2D Orthographic` mode, **every single projected vector edge, arc, bore circle, and hidden line is an interactive, clickable SVG element**.
   - **Hover Behavior:** Hovering any vector highlights the segment in **Drafting Vermilion (`#C83E2B`)**, displays a live dimension badge (`L = 60.0 mm` / `Ø 12.0 mm`), and simultaneously highlights that exact geometric feature inside the **Top-Right Mini-Map (`#6`)**.
   - **Click Behavior (Contextual Vector Edit Popover):** Clicking any vector opens a compact, floating **Feature Edit Popover** anchored directly to the vector with two editing speeds:
     - **Fast Low-Compute Parametric Nudge:** Instant stepper controls (`[- 1.0] [- 0.1] [ 60.00 mm ] [+ 0.1] [+ 1.0]`) and one-click feature modifiers (`+ Fillet`, `+ Chamfer`, `Offset`, `Delete Feature`) that patch the parametric construction step directly without waiting for a full LLM regeneration.
     - **Scoped Micro-Prompt:** A focused single-line input (`"Change this edge/hole..."`) that scopes the AI agent's modification strictly to the selected vector/feature.

---

### D. Center-Bottom "Drafting Pencil" Prompt Bar & The Living Graphite String (Replaces `#3` & `#9`)
Anchored floating `20px` above the bottom sheet border in the horizontal center of the main drafting field (`width: min(660px, 60vw)`, `height: 44px`):

1. **The Pencil Silhouette:**
   - Designed as an iconic, architectural **horizontal drafting pencil**:
     - **Left Ferrule & Eraser Cap (`48px`):** Knurled metallic band housing a `[ ↺ Prompt History ]` trigger and `[ Clear ]` action.
     - **Center Hexagonal Barrel (`flex: 1`):** Vellum-tinted input chamber bordered by twin horizontal bevel hairlines representing a hexagonal pencil body. Placeholder: *"Describe a part, assembly, or modification to draft..."*
     - **Right Conical Nib (`52px`):** Tapered wood-and-graphite cone ending in a sharp graphite point (`▶`) on the far right edge, acting as the `[ Draft / Submit ↵ ]` button.

2. **The Living Graphite String (`Pencil Tip` → `Window #5`):**
   - An SVG overlay layer connects the exact pixel coordinates of the **right-pointing Pencil Tip** to the **active entry port of Window `#5`** (Middle-Right Process Dock).
   - **Idle State:** The pencil tip is crisp and stationary.
   - **Generating / Loading State ("The Pencil Scribble"):**
     - As soon as the user submits a prompt, a **continuous hand-drawn graphite line ("string")** shoots out from the sharp tip of the pencil bar, curving gracefully across the right margin into **Window `#5`**.
     - Inside `#5`, the end of this continuous line animates in an **endless, organic circular pencil scribble** (like a drafter actively shading/sketching on paper) while the kernel compiles the geometry.
     - **Inline Generation Controls:** While the pencil string is actively scribbling, two crisp redline control pills appear right at the pencil collar: **`[ ✕ Abort ]`** (immediately halts generation) and **`[ ✎ Edit Prompt ]`** (pulls the prompt back into the barrel for tweaking).
   - **Completed State ("Crystallized Dotted Trail"):**
     - The moment generation finishes, the circular scribble pulls taut and snaps into a **clean, straight drafted dotted line (`• ── • ── • ── ◉`)** inside `#5`, where each dot represents a verified build milestone.

---

### E. Top-Right Window (`#6`): The 2D Engineering Mini-Map ("Spatial & Assembly Locator")
Located at the top of the right dock (`340px` wide, `28%` height):
- Acts as a **"Video-Game Mini-Map for Mechanical Engineering"** with three context-aware lenses selectable via micro-tabs in its header:
  1. **`[ PART LOCATOR ]` (Single-Part Mode):** Displays a clean 2D wireframe/axonometric overview of the active part with a live **Vermilion Redline highlight box** indicating which specific face, bore, or orthographic vector the user is currently hovering, inspecting, or editing on the main sheet.
  2. **`[ ASSEMBLY FIT ]` (Assembly / Sub-Part Mode):** When viewing a sub-part branch (from the `#13a` Paper-Roll tabs), the Mini-Map renders the **ghosted 2D schematic of the entire parent assembly** and highlights in **Amber/Vermilion ink** the exact spatial position where the active part fits, mates, and interlocks. For nested multi-assemblies, shows how sub-assemblies fit into the master assembly.
  3. **`[ SECTION A-A ]` (Cross-Section Lens):** Displays the hatched 2D cross-section cut at the active section plane with wall-thickness callouts.

---

### F. Middle-Right Window (`#5`): Graphite Process & Verification Trail (Absorbs `#10`)
Located in the middle of the right dock (`340px` wide, `48%` height), directly receiving the **Living Graphite String** from the Pencil Prompt Bar:
- Replaces the old raw cryptographic hex table and absorbs the geometry verification card from `#10`.
- **Visual Structure:**
  - **Active Header:** Shows current build status (`IDLE · READY` / `SCRIBBLING GEOMETRY...` / `VERIFIED · MANIFOLD SOLID`).
  - **The Dotted Milestone Trail (connected to the Pencil String):**
    - **Dot 1 — Intent & Parametric Script:** Displays parsed engineering intent, key parameter pills, and a `[ </> Script JSON ]` download/inspect chip.
    - **Dot 2 — B-Rep Kernel & Metrology Verification (Shifted from `#10`):**
      - Stamp badges: `[ ✓ B-REP VALID ]` `[ ✓ SOLIDS: 1 ]`
      - Tabular metrology grid: `VOLUME (cm³)`, `SURFACE AREA (mm²)`, `ENVELOPE (X × Y × Z mm)`.
    - **Dot 3 — CAD Artifacts & Direct Export:**
      - One-click artifact pills: **`[ ↓ STEP (.stp) ]`** and **`[ ↓ STL (.stl) ]`**.
  - **Collapsible Footer:** A single quiet accordion toggle (`[ ▸ Audit Hash-Chain · 229 entries · Verified ]`) keeps the cryptographic journal accessible on demand without visual noise.

---

### G. Bottom-Right Window (`#1`): AS 1100 Agent Title Block & Square Engineer Profile Button (Absorbs `#4`, `#15`, `#16`, `#17`)
Anchored flush in the **bottom-right corner** of the drawing frame (`340px` wide, `24%` height), structured with authentic AS 1100 ruled title-block compartments:

1. **Live Agent & Project Telemetry Cells (Left & Top compartments of `#1`):**
   - **`PROJECT // SHEET`:** Editable project title & active revision tag (e.g., `SCREW-JACK-ASSY · REV 03`).
   - **`AI ENGINE // MODEL`:** Active provider & model identifier (`OPENROUTER · MINIMAX-M3.1` or `OPENAI · GPT-5`) with API health dot (`● READY`).
   - **`CAD BRIDGE` (Shifted from `#4`):** Live bidirectional CAD link status (`● FREECAD 1.1 CONNECTED` / `SOLIDEDGE` / `OPENCASCADE`) with one-click `[ Push to CAD ↗ ]`.
   - **`TELEMETRY // TOKENS`:** Session token consumption & turn counter (`14,280 TOK · 4 TURNS`).

2. **Bottom-Right Corner Cell — The Square Punchy Engineer Profile Button (`56px × 56px`):**
   - Flush in the extreme bottom-right cell of the Title Block sits a **bold, square, high-contrast black-and-white woodcut/linocut Engineer Portrait Button** with a crisp `1.5px` ink border and tactile press shadow.
   - **8 Built-In Legendary Engineer Avatars (B&W Woodcut/Engraving Style):**
     1. **Nikola Tesla** *(AC Polyphase & Electromechanical Pioneer)*
     2. **Dr. A.P.J. Abdul Kalam** *(Aerospace & Launch Vehicle Systems Architect)*
     3. **Sir M. Visvesvaraya** *(Precision Hydraulic & Civil Engineering Master)*
     4. **Leonardo da Vinci** *(Codex Atlanticus Mechanical Drafter)*
     5. **Ada Lovelace** *(Analytical Engine & Symbolic Computation Pioneer)*
     6. **Isambard Kingdom Brunel** *(Heavy Mechanical & Railway Chief Engineer)*
     7. **James Watt** *(Centrifugal Governor & Steam Kinematics Pioneer)*
     8. **Margaret Hamilton** *(Apollo Guidance Systems Engineering Director)*
   - **Click Interaction — The Engineer Command Popover (Replaces `#15`, `#16`, `#17`):**
     - Clicking the square Engineer Avatar opens a crisp, ruled **AS 1100 Engineer Command Card** anchored directly above the Title Block with four clean sections:
       1. **Engineer Profile & Avatar Selector:** Choose among the 8 B&W engineer portraits, edit drafter initials (stamped into the title block), and toggle sheet surface (`Warm Vellum` vs `Blueprint Slate`).
       2. **API Keys & LLM Routing (from `#15` & `#16`):** Configure OpenAI / OpenRouter / Local Endpoint keys, select active model, and toggle remote API consent.
       3. **CAD Bridge & Kernel Target (from `#4` & `#16`):** Select target CAD environment (`FreeCAD`, `SolidEdge`, `OpenCascade`), check loopback port health, and trigger direct CAD handoff.
       4. **Batch Export & Sheet Standards:** Export full assembly STEP/STL bundles or DXF/SVG 2D orthographic sheets.

---

## 5. Future Extensibility & Add-On Architecture
Ensure the layout natively accommodates future engineering modules without adding new panels or clutter:
1. **Tool Strip Slot (`#13b`):** Expandable vertical icon rail below the top border for future simulation/inspection tools (FEA, Kinematics, GD&T).
2. **Mini-Map Lens Slot (`#6`):** Tabbed header inside the top-right Mini-Map for additional spatial overlays (Thermal Map, Interference Check, BOM Explode).
3. **Orthographic Vector Action Slot (`#14`):** Plugin hook inside the Clickable Vector Popover for domain-specific edge operations (Weld Symbols, Surface Finish Marks, Thread Callouts).
```
