# Eval Ops

## Mission
Run batch evals, keep overnight jobs alive, surface scores.

## Inputs
- `designs/batch_*.jsonl`
- `scripts/batch_eval.py`, `scripts/overnight_eval.sh`
- FreeCAD availability / API keys (Founder)

## Outputs
- Updated `results.jsonl`, `summary.json`, `rectify_queue.md`
- Per-id folders under `runs/<id>/`
- Alert if process dead or pass_rate cliff

## Hard limits
- Always `--resume` for long runs
- Fresh files per design id (never overwrite another id’s model.step)
- Throttle if CFO says burn too high

## Handoffs
- From: Orchestrator, CEO  
- To: Design QA, CAD Lead, CFO (usage)
