# Kala Studio — Screen-by-Screen UI Wireframes & Layout Specification

This document provides a comprehensive, color-agnostic, structural wireframe specification of every screen, mode, panel, and state in **Kala Studio** based on the codebase (`studio/src/`).

It focuses strictly on **element placement, layout hierarchy, component coordinates, interactive controls, inputs, readouts, and state behaviors**.

---

## 1. High-Level App Layout Structure

The application operates as a desktop Electron shell or browser fallback inside a fixed full-height viewport with no root document scrolling (`overflow: hidden`). All internal sections scroll independently.

### Grid Definition
```
+-------------------------------------------------------------------------------------------------------+
| Header: [⚙ Kala Studio] [chat-first agent]                  [● Live] [Desktop shell] [v0.1.0]        |
+-------------------+-------------------------------------------------------------------+---------------+
| Sidebar (220px)   | Main Workspace (Toolbar + Center Viewport)                         | Right Rail    |
|                   |                                                                   | (360px)       |
| - CAD tab         | +---------------------------------------------------------------+ |               |
| - Analysis tab    | | Toolbar: [View: Agent|3D] [Prims] [Toggles] [Mats] [Bg] [Reset]| | [Workbench]   |
| - Providers tab   | +---------------------------------------------------------------+ | [Export]      |
|                   | |                                                               | | [Assistant]   |
| [Active Drawer:   | | Center Viewport:                                              | |               |
|  Primitives /     | |   - Agent Mode: [Mechanical Engineering Agent Chat]           | | [Docked 3D    |
|  Integrations /   | |   - 3D Focus Mode: [Full 3D WebGL Scene Canvas]               | |  Preview 190px|
|  API Keys]        | |                                                               | |  (Agent Mode)]|
|                   | |                                                               | |               |
|                   | |                                                               | | [Tab Panel    |
|                   | |                                                               | |  Content:     |
|                   | |                                                               | |  Inspector /  |
|                   | |                                                               | |  Export Form /|
|                   | |                                                               | |  Chat Thread] |
|                   | +---------------------------------------------------------------+ |               |
+-------------------+-------------------------------------------------------------------+---------------+
| Footer: Connected to http://127.0.0.1:8765 (desktop shell) | Backend v0.1.0 · step available · memory |
+-------------------------------------------------------------------------------------------------------+
```

---

## Screen 1: Boot Gate — Initial Connecting State

Displayed when the application is launched while the backend bridge (`resolveConnectionFromBridge`) is resolving and testing connectivity.

### Visual Wireframe
```
+-----------------------------------------------------------------------+
|                                                                       |
|                                                                       |
|                     +---------------------------+                     |
|                     |        Kala Studio        |                     |
|                     |                           |                     |
|                     | Connecting to Kala Studio |                     |
|                     | backend…                  |                     |
|                     |                           |                     |
|                     +---------------------------+                     |
|                                                                       |
|                                                                       |
+-----------------------------------------------------------------------+
```

### Element Breakdown
- **Container**: Centered dialog card (`max-width: 460px`, `padding: 24px 28px`).
- **Heading 1**: "Kala Studio" (18px, bold).
- **Status Text**: "Connecting to Kala Studio backend…" (aria-busy="true", role="status").

---

## Screen 2: Boot Gate — Browser Fallback / Manual Connection Form

Rendered when the desktop IPC bridge is absent (`phase === 'awaiting-browser'`), requiring explicit loopback origin and bearer token authentication.

### Visual Wireframe
```
+-----------------------------------------------------------------------+
|                                                                       |
|         +---------------------------------------------------+         |
|         | Kala Studio                                       |         |
|         |                                                   |         |
|         | Connect to Kala Studio backend                    |         |
|         |                                                   |         |
|         | The browser fallback keeps the token in this tab  |         |
|         | only. It is never persisted, logged, or echoed    |         |
|         | back to the server. Reloading the page clears it. |         |
|         |                                                   |         |
|         | Backend origin                                    |         |
|         | [ http://127.0.0.1:8765                         ] |         |
|         |                                                   |         |
|         | Bearer token                                      |         |
|         | [ ••••••••••••••••••••••••••••••••••••••••••••  ] |         |
|         |                                                   |         |
|         | [! Validation / Network Error Banner            ] |         |
|         |                                                   |         |
|         | [ Connect ]                                       |         |
|         +---------------------------------------------------+         |
|                                                                       |
+-----------------------------------------------------------------------+
```

### Element Breakdown
1. **Modal Card**: Centered on background.
2. **Title**: "Kala Studio".
3. **Form Header**: "Connect to Kala Studio backend".
4. **Notice / Help Text**: Privacy policy & in-memory token notice.
5. **Input Field (URL)**:
   - Label: `Backend origin`
   - Input: `<input type="url">`, placeholder: `http://127.0.0.1:8765`, pre-filled with window origin if valid loopback.
6. **Input Field (Password/Token)**:
   - Label: `Bearer token`
   - Input: `<input type="password">`, placeholder: `Paste the per-launch KALA_STUDIO_TOKEN`.
7. **Inline Error Area**: Visible only on validation error (e.g. non-loopback origin, query strings, empty token).
8. **Action Button**: `[ Connect ]` (Submit button).

---

## Screen 3: Boot Gate — Connection Error & Recovery

Rendered when the backend probe fails, authentication is rejected (401/403), or the loopback service is unreachable.

### Visual Wireframe
```
+-----------------------------------------------------------------------+
|                                                                       |
|         +---------------------------------------------------+         |
|         | Kala Studio                                       |         |
|         |                                                   |         |
|         | [X] auth: invalid bearer token (401)              |         |
|         |                                                   |         |
|         | [ Retry ]                                         |         |
|         |                                                   |         |
|         | If you launched Kala Studio from the desktop app, |         |
|         | the token is held in the desktop process and is   |         |
|         | not visible in the browser. Re-launch the desktop |         |
|         | app to reset, or fall back to the in-page form to |         |
|         | paste a token printed elsewhere.                  |         |
|         +---------------------------------------------------+         |
|                                                                       |
+-----------------------------------------------------------------------+
```

### Element Breakdown
1. **Title**: "Kala Studio".
2. **Error Message**: Explicit backend or network diagnostic string.
3. **Action Button**: `[ Retry ]` button (clears server cache, attempts reconnect).
4. **Help Footnote**: Instruction for desktop vs browser fallback tokens.

---

## Screen 4: Global Header & Global Footer

Persistent across all application views once connected.

### Header Wireframe
```
+-------------------------------------------------------------------------------------------------------+
| [⚙] Kala Studio  chat-first mechanical engineering agent        [● Live]  [Desktop shell]  [v0.1.0]   |
+-------------------------------------------------------------------------------------------------------+
```

#### Header Elements:
- **Left**:
  - App Logo Glyph: `⚙`
  - App Name: `Kala Studio` (16px bold)
  - Tagline: `chat-first mechanical engineering agent` (12px secondary)
- **Right (Status Cluster)**:
  - Status Chip: `[● Live]` / `[● Offline]` / `[● Connecting]` (includes dot indicator + text label)
  - Mode Indicator: `Desktop shell` vs `Browser fallback (token in memory)` vs `Awaiting connection`
  - Version Tag: `v0.1.0` (monospace)

### Footer Wireframe
```
+-------------------------------------------------------------------------------------------------------+
| Connected to http://127.0.0.1:8765 (desktop shell)   |  Backend v0.1.0 · step available · memory only |
+-------------------------------------------------------------------------------------------------------+
```

#### Footer Elements:
- **Left**: Connection origin URL (`<code>http://127.0.0.1:8765</code>`) + mode.
- **Right**: Backend version status, STEP exporter availability status (`available` or `unavailable`), token memory security notice.

---

## Screen 5: Left Sidebar — Navigation & Workspace Drawers

Width: 220px. Contains top navigation buttons and an expandable context panel below.

### Visual Wireframe
```
+---------------------+
| [CAD              ] | <- Tab 1 (Active)
| 3 integrations      |
| Parametric prims... |
+---------------------+
| [Analysis         ] | <- Tab 2
| 2 integrations      |
| Handoff to FEA/CAE  |
+---------------------+
| [Providers        ] | <- Tab 3
| 2/3 configured      |
| API keys & chat     |
+---------------------+
| CONTEXT PANEL:      |
| (Depends on tab)    |
|                     |
| [Content Area]      |
+---------------------+
```

### Sidebar Context Panel Variations:

#### Variation 5A: "CAD" Tab Active
```
+------------------------------------------+
| CAD PRIMITIVES                           |
| Current shape: [box]                     |
|                                          |
| The OpenCascade workbench in the right   |
| rail previews the shape, sizes it        |
| parametrically, and reports nominal      |
| volume, surface area, and bounding       |
| envelope.                                |
|                                          |
| [ Reset primitive ]                      |
|                                          |
| - FreeCAD          [Neutral file]        |
|   Capabilities: stl, step                |
| - SOLIDWORKS       [Neutral file]        |
|   Capabilities: stl, step                |
| - Fusion 360       [Neutral file]        |
|   Capabilities: stl, step                |
+------------------------------------------+
```
- Shape label: `Current shape: <code>box</code>`
- Action: `[ Reset primitive ]` button
- Embedded CAD integrations list with capability tags

#### Variation 5B: "Analysis" Tab Active
```
+------------------------------------------+
| ANALYSIS REGISTRY                        |
|                                          |
| Native solver integration is not run     |
| from Kala Studio; this list shows the    |
| analysis targets that can consume a      |
| neutral artefact.                        |
|                                          |
| - ANSYS Mechanical [Neutral file]        |
|   Capabilities: stl, step                |
| - Abaqus           [Neutral file]        |
|   Capabilities: stl, step                |
+------------------------------------------+
```
- Informational notice on neutral FEA handoff
- Embedded Analysis integrations list with status pills

#### Variation 5C: "Providers" Tab Active
```
+------------------------------------------+
| PROVIDER & API KEY SETTINGS              |
|                                          |
| Save your OpenAI or OpenRouter API key   |
| to local settings store. Keys are stored |
| server-side only.                        |
|                                          |
| - Local Helper      [Configured]         |
| - OpenAI            [Not configured]     |
|   Set OPENAI_API_KEY                     |
|   [ sk-................... ]             |
|   [Save key] [Clear]                     |
| - OpenRouter        [Configured]         |
|   Key stored in Studio settings          |
|   [ sk-or-................ ]             |
|   [Save key] [Clear]                     |
+------------------------------------------+
```
- Compact provider registry list
- Secure key entry fields (`type="password"`), status confirmation text

---

## Screen 6: Viewport Controls Toolbar

Spans the top of the main workspace across both the center viewport and the right rail.

### Visual Wireframe
```
+-----------------------------------------------------------------------------------------------------------------------------------+
| View: [ Agent | 3D Focus ] | Shape: [ Box | Cyl | Sph ] | [x] Grid [x] Axes [x] Dims | Mat: [ Def | Matte | Wire ] | Bg: [ Studio | Paper | Dark ] | [ Reset ] | Verifying… |
+-----------------------------------------------------------------------------------------------------------------------------------+
```

### Element Breakdown (Left to Right):
1. **Workspace View Switch**:
   - Label: `VIEW`
   - Radio buttons: `[ Agent ]` (Chat-first mode) | `[ 3D Canvas Focus ]` (Full WebGL mode)
2. **Primitive Kind Picker**:
   - Radio buttons: `[ Box ]` | `[ Cylinder ]` | `[ Sphere ]`
3. **Viewport Display Toggles**:
   - Toggle button: `[ Grid ]` (`aria-checked="true/false"`)
   - Toggle button: `[ Axes ]` (`aria-checked="true/false"`)
   - Toggle button: `[ Dimensions ]` (`aria-checked="true/false"`)
4. **Material Mode Picker**:
   - Radio buttons: `[ default ]` | `[ matte ]` | `[ wireframe ]`
5. **Background Selector**:
   - Radio buttons: `[ studio ]` | `[ paper ]` | `[ dark ]`
6. **Action Button**:
   - `[ Reset ]` (warning styled, resets primitive parameters to defaults)
7. **Status Indicator (Right-aligned)**:
   - Text display: `Verifying…` / `Timeout: …` / `Cancelled` / `Error: …` / `[Empty when idle]`

---

## Screen 7: Center Viewport — View Mode A: Agent Chat (Default)

Primary screen where user converses with the Mechanical Engineering Agent.

### Visual Wireframe
```
+---------------------------------------------------------------------------------------------------+
| Mechanical Engineering Agent                                         [ API Keys ] [ Clear Thread ]|
| Conversational mechanical sizing, parametric geometry estimation, and STEP/STL workflow planning. |
+---------------------------------------------------------------------------------------------------+
| [Optional Dropdown Settings Area for Provider API Keys when toggled open]                         |
+---------------------------------------------------------------------------------------------------+
| ACTIVE GEOMETRY: [ BOX ]  40.0 mm × 20.0 mm × 30.0 mm | envelope 40×20×30 mm | volume 24,000 mm³   |
+---------------------------------------------------------------------------------------------------+
| Quick Starters: [ Size a bracket ] [ Size a shaft ] [ Estimate a housing ] [ Plan STEP/STL handoff]|
+---------------------------------------------------------------------------------------------------+
| CHAT THREAD (Scrollable):                                                                         |
|                                                                                                   |
| +-----------------------------------------------------------------------------------------------+ |
| | YOU                                                                                           | |
| | Size an L-bracket for a 40 mm mounting plate with 8 mm wall thickness.                        | |
| +-----------------------------------------------------------------------------------------------+ |
|                                                                                                   |
| +-----------------------------------------------------------------------------------------------+ |
| | ASSISTANT                                                                                     | |
| | For a 40 mm plate with 8 mm walls, the nominal bounding envelope is 60 × 45 × 40 mm.          | |
| | Analytical volume is approximately 28,160 mm³ (28.16 cm³). STEP export is ready.              | |
| +-----------------------------------------------------------------------------------------------+ |
|                                                                                                   |
+---------------------------------------------------------------------------------------------------+
| COMPOSER (Fixed Bottom Form):                                                                     |
| Provider: [ OpenAI              ▼ ]     Model: [ gpt-4o-mini                                    ] |
| [x] I consent to this prompt being sent to a remote provider. Without this tick the request       |
|     will not leave the loopback backend.                                                          |
| Prompt:                                                                                           |
| [ Ask a question about the active primitive, its envelope, or the STEP/STL handoff...           ] |
| [ Prompt text continues here...                                                                 ] |
|                                                                                                   |
| [ Send ]  [ Cancel ]                                            [ Provider not configured warning]|
+---------------------------------------------------------------------------------------------------+
```

### Element Breakdown
1. **Header**:
   - Title: `Mechanical Engineering Agent`
   - Subtitle: `Conversational mechanical sizing, parametric geometry estimation, and STEP/STL workflow planning.`
   - Top-right buttons: `[ API Keys ]` (expands drawer), `[ Clear ]` (disabled when thread is empty).
2. **Active Geometry Context Banner**:
   - Label: `ACTIVE GEOMETRY`
   - Kind Pill: `BOX` / `CYLINDER` / `SPHERE`
   - Sizing readout: `40.0 mm × 20.0 mm × 30.0 mm`
   - Envelope readout: `envelope 40.0 × 20.0 × 30.0 mm`
   - Volume readout: `volume 24,000 mm³ (24.00 cm³)`
3. **Engineering Starter Prompts**:
   - Horizontal wrap pill buttons:
     - `[ Size a bracket ]`
     - `[ Size a shaft ]`
     - `[ Estimate a housing ]`
     - `[ Plan a STEP/STL handoff ]`
4. **Chat Conversation Area**:
   - Scrollable message thread.
   - Message bubble structure:
     - Author label: `YOU` or `ASSISTANT`
     - Message body: Raw monospace/prose text
   - In-flight indicator: `Awaiting response…`
   - Error banner: `[!] Error text`
5. **Message Composer Form**:
   - **Provider Selector**: `<select>` dropdown (`Local`, `Local Helper`, `OpenAI`, `OpenRouter`).
   - **Model Input**: `<input type="text">` (required for remote, optional for local helper).
   - **Remote Consent Checkbox**: Visible when remote provider selected. Mandatory consent gate before send is permitted.
   - **Prompt Textarea**: Multiline text area (`rows=3`).
   - **Form Actions**:
     - `[ Send ]` button (enabled only when valid, configured, consent given).
     - `[ Cancel ]` button (visible during in-flight requests).
     - Warning message (shown if chosen provider is unconfigured).

---

## Screen 8: Right Rail — OpenCascade Workbench (In Agent Mode)

Width: 360px. In "Agent" view mode, it houses a docked 3D canvas on top and tabbed panels below.

### Visual Wireframe
```
+------------------------------------------+
| [ Workbench ]   [ Export ]   [ Assistant]| <- Rail Tab Bar
+------------------------------------------+
| DOCKED 3D PREVIEW (Height: 190px)        |
|                                          |
|             [ 3D Mesh View ]             |
|                                          |
|                   [ Server verified mesh]| <- Top-Right Source Pill
+------------------------------------------+
| SCROLLABLE TAB PANEL AREA:               |
|                                          |
| (Content of active tab:                  |
|  8A: Workbench / Inspector               |
|  8B: Export & Handoff                    |
|  8C: Assistant Thread)                   |
|                                          |
+------------------------------------------+
```

---

## Screen 8A: Right Rail — "Workbench" Tab (Inspector & Estimates)

Contains primitive parametric controls, nominal analytical calculations, and bounding box validation.

### Visual Wireframe
```
+------------------------------------------+
| Inspector              [Server verified] |
+------------------------------------------+
| PARAMETERS:                              |
| Width (size_x) (mm)                      |
| [ 40.0                                 ] |
| Height (size_y) (mm)                     |
| [ 20.0                                 ] |
| Depth (size_z) (mm)                      |
| [ 30.0                                 ] |
| Subdivisions                             |
| [ 1                                    ] |
+------------------------------------------+
| OPENCASCADE ESTIMATE (NOMINAL)           |
| Volume:                                  |
|   24,000 mm³ (24.00 cm³)                 |
| Surface area:                            |
|   5,200 mm² (52.00 cm²)                  |
| Bounding envelope:                       |
|   40.0 × 20.0 × 30.0 mm                  |
| Envelope volume:                         |
|   24,000 mm³                             |
|                                          |
| Closed-form nominal geometry for the     |
| ideal solid, for sizing and STEP/STL     |
| handoff. No stress/deflection computed.  |
+------------------------------------------+
| BOUNDS (MM)                              |
| Width:  40.000 mm                        |
| Height: 20.000 mm                        |
| Depth:  30.000 mm                        |
|                                          |
| [! Drift warning if server/local differ] |
+------------------------------------------+
```

### Parameter Controls by Shape:
- **Box**:
  - `Width (size_x)`: 0.1 mm to 2000 mm (step 0.5)
  - `Height (size_y)`: 0.1 mm to 2000 mm (step 0.5)
  - `Depth (size_z)`: 0.1 mm to 2000 mm (step 0.5)
  - `Subdivisions`: 1 to 64 (integer step 1)
- **Cylinder**:
  - `Radius`: 0.1 mm to 2000 mm (step 0.5)
  - `Height`: 0.1 mm to 2000 mm (step 0.5)
  - `Radial segments`: 6 to 256 (integer step 1)
  - `Height segments`: 1 to 64 (integer step 1)
- **Sphere**:
  - `Radius`: 0.1 mm to 2000 mm (step 0.5)
  - `Rings`: 3 to 96 (integer step 1)
  - `Sectors`: 4 to 128 (integer step 1)

### Estimates & Readouts:
- **Nominal Estimates**: Closed-form mathematical volume (`mm³` + `cm³`), surface area (`mm²` + `cm²`), bounding envelope dimensions, and envelope volume.
- **Bounds (mm)**: Derived from actual rendered mesh vertices.
- **Drift Warning**: Displayed automatically if server-verified B-rep bounds differ from local mesh by > 0.01 mm.

---

## Screen 8B: Right Rail — "Export" Tab (Export & Handoff)

Enables generating and downloading neutral STEP or STL files for CAD/CAE tools.

### Visual Wireframe
```
+------------------------------------------+
| Export & handoff                         |
|                                          |
| Renders the active primitive as a        |
| neutral artefact (STL, STEP) for the     |
| picked target — either to finish build   |
| or hand geometry to an external suite.   |
|                                          |
| Target                                   |
| [ FreeCAD (neutral file)               ▼]|
|                                          |
| Format                                   |
| [ STEP                                 ▼]|
|                                          |
| [!] Native automation is not available   |
|     for FreeCAD from Kala Studio.        |
|     Exporting writes a neutral STEP file |
|     for manual import.                   |
|                                          |
| [ Download STEP ]                        |
|                                          |
| [✓] Saved box-40x20x30.step · 4,821 bytes|
+------------------------------------------+
```

### Element Breakdown:
1. **Header**: "Export & handoff".
2. **Help Description**: Neutral file export guidance.
3. **Target Dropdown**:
   - `Direct export (no handoff)`
   - Registered CAD packages (e.g. `FreeCAD`, `SOLIDWORKS`, `Fusion 360`) with status tag.
4. **Format Dropdown**:
   - `STL`
   - `STEP` (disabled if target doesn't support it).
5. **Contextual Status Notes / Warnings**:
   - Informs user if native automation is unavailable and neutral file will be supplied.
   - Informs if integration is unconfigured.
6. **Action Button**: `[ Download STEP ]` / `[ Download STL ]` (shows `Working…` when busy).
7. **Result Notification**: `Saved <filename> · <N> bytes`.
8. **Error Notification**: Red alert text if API/network fails.

---

## Screen 9: Center Viewport — View Mode B: 3D Canvas Focus

Activated when user selects "3D Focus" in the Toolbar. The 3D scene expands to fill the entire main center viewport.

### Visual Wireframe
```
+-------------------------------------------------------------------+---------------+
| FULL-BLEED 3D WEBGL VIEWPORT                                      | RIGHT RAIL    |
|                                                                   |               |
|                                                                   | [Workbench]   |
|                               [ Server verified mesh ]            | [Export]      |
|                                                                   | [Assistant]   |
|                                                                   |               |
|                     +-----------------+                           | (Note: Docked |
|                    /                 /|                           |  3D preview   |
|                   +-----------------+ |                           |  is hidden    |
|                   |                 | |                           |  in this mode |
|                   |   [3D MESH]     | +                           |  to keep one  |
|                   |                 |/                            |  single WebGL |
|                   +-----------------+                             |  context)     |
|                                                                   |               |
|                                                                   | [Inspector /  |
|                                                                   |  Export Panel |
|                                                                   |  fills full   |
| [ 40.0 × 20.0 × 30.0 mm ]                                         |  rail height] |
+-------------------------------------------------------------------+---------------+
```

### HUD Elements in 3D Canvas:
1. **Interactive Canvas**: OrbitControls (Left click = rotate, Right click = pan, Scroll = zoom).
2. **Grid**: 1000 mm × 1000 mm ground grid with 50 subdivisions (toggleable).
3. **Axes**: 3D RGB axes indicator (toggleable).
4. **Bottom-Left Dimensions HUD**: Monospace card displaying `Width × Height × Depth mm` (toggleable).
5. **Top-Right Source Tag**: `Server verified mesh` or `Local preview — pending server verification`.
6. **Right Rail Behavior**: The docked 190px mini-preview is unmounted, allowing Inspector or Export controls to utilize the full height of the sidebar.

---

## Screen 10: Responsive & Mobile Adaptations

The application layout adapts at specific screen width breakpoints:

### Breakpoint <= 1100px (Medium / Narrow Screen)
Main grid stacks vertically into a single column:
```
+---------------------------------------------------+
| Toolbar                                           |
+---------------------------------------------------+
| Center Viewport (Agent Chat or 3D Canvas)         |
+---------------------------------------------------+
| Right Rail (Inspector / Export / Workbench)       |
+---------------------------------------------------+
```

### Breakpoint <= 720px (Compact / Mobile Screen)
Sidebar stacks on top and is capped at 42vh with internal scroll:
```
+---------------------------------------------------+
| Header                                            |
+---------------------------------------------------+
| Sidebar (Max 42vh, scrollable)                    |
+---------------------------------------------------+
| Toolbar                                           |
+---------------------------------------------------+
| Viewport                                          |
+---------------------------------------------------+
| Right Rail                                        |
+---------------------------------------------------+
| Footer                                            |
+---------------------------------------------------+
```
