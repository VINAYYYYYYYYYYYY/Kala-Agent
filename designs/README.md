# Design batch format

Put briefs in JSONL (one JSON object per line). Example:

```json
{"id": "voron-001", "goal": "…human brief…", "standard_parts": true, "procedure": "machine_assembly"}
```

## CAD 1000 Hours (external HDD)

Mounted at `/run/media/v9i/july2020-/cad-1000-hours`  
([markov-ai/cad-1000-hours](https://huggingface.co/datasets/markov-ai/cad-1000-hours) — 597 workflows / ~1022 h).

**On disk now:** `task_desc.json`, `rubrics.json`, `narration.json`, some PDFs.  
**Missing (LFS not pulled):** `clip.mp4`, `input_files/*`, `output_files/*` (gold CAD / STEP).

Extracted eval suite:

- `batch_cad1000_150.jsonl` — 50 simple / 50 complex / 50 extreme from SW + CATIA + NX
- `cad1000_index.json` — counts + sample titles

To fetch gold outputs later (large):

```bash
cd /run/media/v9i/july2020-/cad-1000-hours
git lfs pull --include="solidworks/*/output_files/**"
```

## Run the loop

```bash
# smoke (mock, fast)
uv run python scripts/batch_eval.py designs/batch_sample.jsonl --backend mock

# CAD1000 suite — simple tier first
uv run python scripts/batch_eval.py designs/batch_cad1000_150.jsonl --backend freecad --limit 50

# resume / chunks
uv run python scripts/batch_eval.py designs/batch_cad1000_150.jsonl --resume --offset 50 --limit 50
```

Outputs land in `outputs/eval/<file_stem>/`:

- `results.jsonl` — per-design score + metrics  
- `summary.json` — pass rate  
- `rectify_queue.md` — recurring failure patterns to fix next  

After a chunk finishes, we read `rectify_queue.md`, patch the agent, then re-run failed ids.

## Procedure audit (chore/designs-procedure-audit)
- Source of truth (library): `kala/procedures/library/` — all 5 ids: housing_cover, machine_assembly, plate_with_holes, simple_bracket, stepped_shaft
- JSONL usage referencing only machine_assembly+simple_bracket is USAGE, not the library definition
- Audited: batch_sample.jsonl, batch_cad1000_150.jsonl, batch_cad1000_rerun4.jsonl (tracked files)
- All procedure refs valid; no mappings needed.
- Untracked slices (batch_cad1000_slice*.jsonl, slice4_extreme, slice8_sc, slice12) left untouched (not tracked, risky size).
- No exceptions.
