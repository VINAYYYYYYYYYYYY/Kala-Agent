# Element audit: Stitch design vs. the built Kala Studio UI (READ-ONLY)

Goal: list EVERY UI element in your assigned Stitch screens and decide whether the built UI in `/home/v9i/Projects/Kala-Agent/studio/src` has it.
Context: read `../WAVE.md` (invariant 2: no fake data) and the four REPORTs in `cells/{shell,viewport,chat-journal,qa}/REPORT.md`. Screenshots of the built UI are in `cells/qa/shots/*.png`.

Rules: READ-ONLY. Do not edit `studio/**` or anything else except your own `AUDIT.md`. No commit, delegation, or credential reads. You may run `npm test` or the build if useful, but do not change files.

## Method (per assigned screen)
1. Parse `stitch/<screen>/code.html` (and view `screen.png` if you can view images) and enumerate the elements: panels/regions, headings/labels, buttons, tabs, toggles, inputs/sliders, stamps/badges/status pills, tables/data fields, icons, decorative drafting elements (zone ticks, title blocks, hatching, grids, corner marks), and interaction states (hover/active/empty/error/loading).
2. For each element, search `studio/src` (grep component/class/label text) and the QA shots, then classify it:
   - **PRESENT**: built and functional. Give the evidence: `file:line`.
   - **PARTIAL**: exists but differs (say how: missing state, wrong placement, static vs live).
   - **MISSING-BUILDABLE**: not built, but the data or function already exists in the frontend store or backend API (name the source: store field, API route in `studio/src/api/client.ts`, or backend `src/kala_agent/**`).
   - **MISSING-NEEDS-BACKEND**: not built, and honest data for it does not exist yet (say what backend capability would be needed).
   - **OMITTED-FAKE**: intentionally left out because the Stitch value is placeholder data (for example a fixed sha, PID, latency, FIPS, Llama).
   - **N/A**: purely mock decoration that has no meaning for this product (justify in one line).
3. Do not guess. If unsure, open the file and check it.

## Output
`cells/audit-<you>/AUDIT.md` containing:
- One markdown table per screen with columns `# | Element | Stitch detail | Status | Evidence / source | Notes`.
- Per-screen counts and a total per status.
- "Top gaps", ranked: the MISSING-BUILDABLE and PARTIAL items worth fixing first (impact on look and usability), each with the concrete file that would change.
- "Needs backend": a short list of the MISSING-NEEDS-BACKEND items, grouped by backend capability.

## Assignment
- **audit-minimax**: `kala_studio_the_vellum_drafting_board_master_workspace`, `kala_studio_master_engineering_design_workbench_day_vellum`, `kala_studio_master_engineering_design_workbench_night_slate`, `kala_agent_drafting_kernel_desktop_1600x1000`, `kala_agent_floating_overlay_system_3_states`.
- **audit-deepseek**: `kala_studio_options_engineering_drawer_wireframe_sheet`, `drawing_sheets_title_blocks`, `hash_chain_ledger_audit`, `parametric_constraint_matrix`, `drafting_canvas_workbench`, plus the element lists in `STITCH-VELLUM-PROMPT.md` §2–3 and `tactile_drafting_graphite_workbench/DESIGN.md` (check that the design tokens, typography scale and spacing are applied). `a_realistic_high_fidelity_desktop_screenshot_of_siemens_solid_edge_cad_software` is an image-only reference: list any CAD-workbench conventions it shows that the UI lacks, marked as suggestions only.
