---
name: Tactile Drafting & Graphite Workbench
colors:
  surface: '#fcf9f2'
  surface-dim: '#dcdad3'
  surface-bright: '#fcf9f2'
  surface-container-lowest: '#ffffff'
  surface-container-low: '#f6f3ec'
  surface-container: '#f1eee7'
  surface-container-high: '#ebe8e1'
  surface-container-highest: '#e5e2db'
  on-surface: '#1c1c18'
  on-surface-variant: '#474741'
  inverse-surface: '#31312c'
  inverse-on-surface: '#f3f0e9'
  outline: '#777770'
  outline-variant: '#c8c7bf'
  surface-tint: '#5f5e5b'
  primary: '#000000'
  on-primary: '#ffffff'
  primary-container: '#1c1c1a'
  on-primary-container: '#858481'
  inverse-primary: '#c9c6c3'
  secondary: '#33628b'
  on-secondary: '#ffffff'
  secondary-container: '#a4d0ff'
  on-secondary-container: '#295982'
  tertiary: '#000000'
  on-tertiary: '#ffffff'
  tertiary-container: '#3e0500'
  on-tertiary-container: '#dd5a3e'
  error: '#ba1a1a'
  on-error: '#ffffff'
  error-container: '#ffdad6'
  on-error-container: '#93000a'
  primary-fixed: '#e5e2de'
  primary-fixed-dim: '#c9c6c3'
  on-primary-fixed: '#1c1c1a'
  on-primary-fixed-variant: '#474744'
  secondary-fixed: '#cfe5ff'
  secondary-fixed-dim: '#9ecbfa'
  on-secondary-fixed: '#001d33'
  on-secondary-fixed-variant: '#154a72'
  tertiary-fixed: '#ffdad3'
  tertiary-fixed-dim: '#ffb4a4'
  on-tertiary-fixed: '#3e0500'
  on-tertiary-fixed-variant: '#891e07'
  background: '#fcf9f2'
  on-background: '#1c1c18'
  surface-variant: '#e5e2db'
typography:
  headline-xl:
    fontFamily: Newsreader
    fontSize: 38px
    fontWeight: '400'
    lineHeight: 44px
    letterSpacing: -0.02em
  headline-xl-mobile:
    fontFamily: Newsreader
    fontSize: 30px
    fontWeight: '400'
    lineHeight: 36px
    letterSpacing: -0.01em
  headline-lg:
    fontFamily: Newsreader
    fontSize: 28px
    fontWeight: '400'
    lineHeight: 34px
    letterSpacing: -0.01em
  headline-md:
    fontFamily: Newsreader
    fontSize: 22px
    fontWeight: '500'
    lineHeight: 28px
  body-lg:
    fontFamily: Space Grotesk
    fontSize: 16px
    fontWeight: '400'
    lineHeight: 24px
  body-md:
    fontFamily: Space Grotesk
    fontSize: 14px
    fontWeight: '400'
    lineHeight: 20px
  body-sm:
    fontFamily: Space Grotesk
    fontSize: 12px
    fontWeight: '400'
    lineHeight: 16px
  label-lg:
    fontFamily: JetBrains Mono
    fontSize: 13px
    fontWeight: '600'
    lineHeight: 16px
    letterSpacing: 0.04em
  label-md:
    fontFamily: JetBrains Mono
    fontSize: 11px
    fontWeight: '500'
    lineHeight: 14px
    letterSpacing: 0.06em
  label-sm:
    fontFamily: JetBrains Mono
    fontSize: 9px
    fontWeight: '500'
    lineHeight: 12px
    letterSpacing: 0.08em
spacing:
  gutter: 0.75rem
  gutter-mobile: 0.5rem
  margin: 1rem
  margin-mobile: 0.75rem
  space-xs: 0.25rem
  space-sm: 0.5rem
  space-md: 0.75rem
  space-lg: 1.25rem
  space-xl: 2rem
---

## Brand & Style

This design system channels the uncompromising rigor of aerospace, mechanical, and architectural drafting boards. It rejects generic software tropes—eschewing rounded pill surfaces, iridescent blurs, and floating neon states in favor of physical drafting instruments, cotton vellum sheets, brass calipers, and precision graphite grades (2H, HB, 4B).

The target audience comprises aerospace systems engineers, computational architects, and parametric modelers who manipulate high-stakes topological parameters on field-grade devices. The interface must feel physical, structural, and unyielding—like handling an archival engineering logbook bound with pressed charcoal boards.

The visual style is **Tactile Industrial Brutalism**:
- High-density information architecture arranged in rigorous orthographic modules.
- Hairline registration marks, corner crosses (+), coordinate borders (A–F, 1–8), and standardized DIN/ISO drawing blocks.
- Tactile material contrast: crisp archival cotton sheets framed by matte pressed charcoal docks, struck through with Prussian ink and vermilion ink stamps.

## Colors

The palette is derived directly from classic drafting media, physical inks, and graphite chemistry:

- **Archival Canvas (`#F4F1EA`)**: Warm, unbleached cotton vellum providing a low-strain drafting plane.
- **Card & Surface (`#EAE5D9`)**: Denser layered vellum surface for data containers, framed with `#C8C2B4`.
- **Charcoal Dock & Housing (`#1E1E1C`)**: Pressed matte charcoal board reserved for technical docks, terminal sheets, and high-density command strips.
- **4B Dark Graphite (`#181816`)**: The primary ink/graphite color for title lettering, primary geometry lines, and structural boundaries.
- **HB Medium Lead (`#4A4843`)**: Secondary text, dimensional callouts, and minor axes.
- **2H Hard Lead (`#B5AFA2`)**: Hairline construction grids, alignment crosses, and non-printing layout guides.
- **Prussian Drafting Ink (`#2B5B84`)**: Blue structural annotations, active selection strokes, and vector dimensioning.
- **Vermilion Stamp (`#C84B31`)**: Inspection stamps, safety thresholds, and formal commit validations.
- **Verified Forest (`#2E6F40`)**: Cryptographic hashes, pass criteria, and immutable chain checks.

Avoid smooth, synthetic CSS drop shadows. Depth is achieved strictly through sharp hairline borders and physical tonal stepping.

## Typography

The typographic hierarchy establishes tension between the elegance of archival annotations, mechanical stencil typesetting, and dense coordinate data:

- **Serif Display Titles (`Newsreader`)**: Set with italic or roman styles for sheet headers, major module titles, and drawing set identities. Evokes physical architectural plates and manual lettering manuals.
- **Technical UI & Controls (`Space Grotesk`)**: Monoline, geometric sans-serif for high-legibility touch targets, control panels, parameter labels, and instructional prompts.
- **Precision Monospace (`JetBrains Mono`)**: Strict character widths for coordinate readouts (X/Y/Z), floating-point tolerance bands, commit hashes, ISO sheet indices, and caliper dimensions. All numeric values must use tabular figures (`tnum`).

## Layout & Spacing

Layouts adhere to an orthographic engineering sheet paradigm. Every view is framed within a standardized technical margin reminiscent of DIN EN ISO 5457 drawing formats:

- **Perimeter Coordinate Grids**: A 12px outer perimeter houses alphanumeric coordinates (`A–F` along vertical axes, `1–8` along horizontal axes) divided by 2H (`#B5AFA2`) tick marks.
- **Grid Layout**: On mobile viewports, the grid uses a 4-column structure with 8px gutters; on tablet/desktop viewports, it shifts to an 8 or 12-column matrix with 12px gutters.
- **Rhythm**: Spacing follows a strict 4px base increment. Spacing tokens govern element distribution between structural lines. Padding must be compact to support data-dense engineering screens without scrolling latency.
- **Orthographic Registration**: Critical containers are anchored with corner crosshairs (`+`) rendered 4px from each container boundary.

## Elevation & Depth

This system discards standard diffuse shadows (`box-shadow: 0 10px 30px rgba(...)`). Depth is achieved purely through drafting linework, layered material opacity, and pressed surface offsets:

- **Level 0 (Archival Canvas)**: Flat `#F4F1EA` vellum substrate displaying subtle hairline millimeter grids (`rgba(181, 175, 162, 0.35)`).
- **Level 1 (Drafting Plates / Cards)**: `#EAE5D9` panels bound by a continuous 1px `#C8C2B4` graphite border. Highlighting is rendered using a single 1px inset line in `#FFFFFF` on the top and left edges to evoke beveled card stock.
- **Level 2 (Active Precision Toolbars / Calipers)**: Crisp floating drafting rules using `#F4F1EA` bounded by 1px solid `#181816` (4B Dark Graphite) with a solid, hard-offset 2px drop stroke: `2px 2px 0px #181816`.
- **Level 3 (Dark Dock & Inspection Terminal)**: `#1E1E1C` matte charcoal baseline panel with high-contrast hairline dividers (`#4A4843`) and monospaced telemetry in `#F4F1EA`.

## Shapes

The shape system is strictly rectilinear (`0` - Sharp). Curvature contradicts the technical precision of drafting rulers and T-squares.

- **Zero Radius**: All cards, buttons, tabs, docks, and input frames feature sharp 90-degree corners.
- **Technical Chamfer Option**: For specialized inspection tags or interactive chips, a 2px 45-degree corner cut (chamfer) via CSS `clip-path` may be used to replicate physical punched cards and brass gauges.
- **Linework Gauges**: Borders are always 1px hairline strokes. Dividers, dimensional lines, and tick marks never exceed 1px, preserving optical alignment with 2H/HB pencil grades.

## Components

### Buttons & Trigger Latches
- **Primary Action (Commit / Render)**: Background `#181816`, text `#F4F1EA` in `Space Grotesk` medium, sharp corners, 1px solid `#181816`. On press, background shifts to `#2B5B84` (Prussian Ink) with an instantaneous 1px inset shift.
- **Secondary (Calibrate / Measure)**: Background `#EAE5D9`, text `#181816`, 1px border `#4A4843`.
- **Destructive / Reject**: Background transparent, 1px solid `#C84B31`, text `#C84B31` in uppercase `JetBrains Mono`.

### Architectural Title Blocks
Located at the base or header of major engineering screens:
- Divided into structured rectangular quadrants containing sheet revision number, project code, scale ratio (e.g., `1:25.4`), and date.
- Header set in `Newsreader` italic, metadata in `JetBrains Mono` label-sm.
- Bound by a heavy 1.5px `#181816` bounding box.

### Input Fields & Caliper Sliders
- **Numeric Fields**: Inset `#F4F1EA` surface framed by `#C8C2B4`. Values rendered in `JetBrains Mono` body-md, right-aligned with hard unit tags (`mm`, `deg`, `kN`) in `#4A4843`.
- **Caliper Slider**: Track drawn as a 1px `#B5AFA2` line with vertical tick marks at regular intervals. The scrub thumb is an inverted solid brass/graphite triangle (8px width, 6px height) with a persistent real-time value tooltip anchored above.

### Checkboxes, Radios, & Calibration Toggles
- **Checkbox**: 14x14px sharp square, 1px border `#181816`. Checked state displays a solid diagonal graphite cross (`X`) rather than a rounded checkmark.
- **Radio Selector**: 14x14px sharp square with a centered solid 6x6px square fill when selected.
- **Toggle**: Segmented horizontal pill alternative; a split two-cell rectangle (`[OFF | ON]`) with active cell inverted to `#181816` background and white text.

### Inspection Stamps & Chain Badges
- **Approval Stamp**: Bordered with an irregular 1.5px dashed border in `#C84B31`, set at a -2deg tilt. Typography is uppercase `Space Grotesk` with letter spacing `0.1em` containing `APPROVED // PRE_COMMIT`.
- **Hash-Chain Audit Badge**: Monospaced `#2E6F40` tag with a verified icon glyph and truncated SHA-256 string, framed by a 1px hairline border in `#2E6F40`.

### Cards & Dimensioned Viewports
- Canvas cards contain corner crosshairs (`+`) at all four outer vertices.
- Dimension lines with exterior extension tick marks extend outside card edges to communicate pixel boundaries directly on the interface plane.