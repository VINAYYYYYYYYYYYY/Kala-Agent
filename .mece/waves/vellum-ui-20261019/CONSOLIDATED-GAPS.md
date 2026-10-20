# Consolidated element audit (Opus adjudication), 2026-10-19

Sources: `cells/audit-minimax/AUDIT.md` (5 workspace screens, 197 rows) and `cells/audit-deepseek/AUDIT.md` (drawer, title blocks, ledger, constraint matrix, canvas, prompt §2–3, DESIGN.md; 275 rows).

| Status | MiniMax | DeepSeek | Total |
|---|---:|---:|---:|
| PRESENT | 36 | 87 | **123** |
| PARTIAL | 90 | 90 | **180** |
| MISSING-BUILDABLE | 17 | 4 | **21** |
| MISSING-NEEDS-BACKEND | 27 | 46 | **73** |
| OMITTED-FAKE (intentional, invariant 2) | 21 | 27 | **48** |
| N/A | 6 | 21 | **27** |
| **Rows** | 197 | 275 | **472** |

303 of 472 elements (64 %) are built in some form. Excluding fake and N/A rows, 303 of 397 (76 %).

Opus verification of the top DeepSeek claim: confirmed. `styles/*.css` has 125 font-size declarations below 9 px (39 at 7.5 px or smaller), no `--space-*` tokens, and `--radius: 2px` in a zero-radius system.

## Tier 1: UI only, highest impact
| # | Gap | Owner |
|---|---|---|
| 1 | Type scale per DESIGN.md (body 12–14 px, labels 9–13 px mono; nothing under 9 px) | C0b |
| 2 | Spacing tokens (`--space-xs…xl`, 4 px rhythm, 12/16 px gutters) and `--radius: 0` | C0b |
| 3 | Options drawer as a drafted 2×2 sheet: all 4 cards visible at 1440×900, card 01 not stretched, no duplicate nested headings, corner ticks, card icons, real-data footer title block | C0b |
| 4 | Vernier slider component: tick rail, triangular thumb, visible min/max/step, steppers, mono readout | C0b |
| 5 | AuxView bounds, SCALE readout, CUT header, Material control, Measure tool, centre lines, chat model/elapsed badge | C0 (running) |

## Tier 2: unlocked by the ADR-0009 API (B1), built in C1
Sheet tabs (one per design) · 6-column ledger (SEQ, UTC, TYPE, MESSAGE, DIGEST + copy, CHAIN) · real HASH-CHAIN OK · B-Rep Verified stamp · title block material, declared tolerances, and design hash · Tolerance tab from declared tolerances · main-sheet section hatching from `/section` · HLR pencil lines · jump-to-Section pill on design results · export receipt with digest and copy · agent card measured-values and checks table.

## Tier 3: out of scope (needs capabilities ADR-0009 does not provide)
FEA and constraint-matrix solver values · approval and sign-off checkpoints (core kernel workflow, not Studio) · runtime telemetry (RAM, GPU, latency) · plotter and print queue · Mark/Eraser markup · named cameras · live FreeCAD/.IGES bridge state. These stay absent or honestly "unavailable"; none may be faked.
