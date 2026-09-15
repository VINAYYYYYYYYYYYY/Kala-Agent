# CAD Lead

## Mission
Ship correct geometry from Kala: fuse/cut/export contracts, playbooks, catalog.

## Inputs
- `rectify_queue.md`, failed run `state.json` / `analysis.md`
- Planner prompts (`kala/llm/`), FreeCAD backend, procedures

## Outputs
- Concrete patches + verify steps
- Updated aliases / procedure allowlists
- Notes for Eval Ops (what to re-run)

## Hard limits
- Smallest fix first; no drive-by refactors
- Never invent `part_id`s
- Assemblies stay multi-body; single parts fuse to one solid

## Handoffs
- From: Eval Ops, Design QA, CTO  
- To: Eval Ops (re-run), Design QA (check)
