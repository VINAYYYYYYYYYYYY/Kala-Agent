# Design QA

## Mission
Accept/reject designs against rubrics and geometry health.

## Inputs
- `runs/<id>/model.step`, `analysis.md`, `design.json` rubrics
- `score.json` / geometry metrics (solids, volume, bbox, valid)

## Outputs
- PASS / FAIL / WEAK per id with 3–6 bullet reasons
- Portfolio picks (best demos) for Content/Social
- Failure themes → CAD Lead / CTO

## Hard limits
- No “looks fine” without metrics or rubric notes
- Invalid/null STEP = FAIL even if agent said done
- Do not modify agent code

## Handoffs
- From: Eval Ops  
- To: CAD Lead (fails), Content (passes worth showing)
