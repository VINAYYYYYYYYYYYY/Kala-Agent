#!/usr/bin/env bash
# Overnight batch: run all 150 designs with resume, isolated per-design folders.
# Hardened: retries on crashes, tracks progress, respects KALA_* env vars.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=outputs/eval/batch_cad1000_150
mkdir -p "$OUT"
LOG="$OUT/overnight.log"

# Fidelity env vars for Eval Ops (override from env if set)
export KALA_MIN_TOOLS="${KALA_MIN_TOOLS:-4}"
# Auto-load stub hashes from prior runs if present
if [[ -f "$OUT/stub_hashes.json" && -z "${KALA_KNOWN_STUB_HASHES:-}" ]]; then
  STUB_HASHES=$(python3 -c "import json; print(','.join(json.load(open('$OUT/stub_hashes.json')).get('hashes', [])[:10]))" 2>/dev/null || echo "")
  if [[ -n "$STUB_HASHES" ]]; then
    export KALA_KNOWN_STUB_HASHES="$STUB_HASHES"
    echo "[$(date -Is)] Loaded KALA_KNOWN_STUB_HASHES from stub_hashes.json" | tee -a "$LOG"
  fi
fi

echo "[$(date -Is)] overnight start (KALA_MIN_TOOLS=$KALA_MIN_TOOLS)" | tee -a "$LOG"

# Keep going until summary says done or results reach target count
FAIL_COUNT=0
MAX_CONSECUTIVE_FAILS=5
while true; do
  echo "[$(date -Is)] batch_eval resume pass" | tee -a "$LOG"
  if uv run python scripts/batch_eval.py designs/batch_cad1000_150.jsonl --backend freecad --resume >>"$LOG" 2>&1; then
    FAIL_COUNT=0
    if grep -q '"done": true' "$OUT/summary.json" 2>/dev/null; then
      echo "[$(date -Is)] ALL DESIGNS COMPLETE" | tee -a "$LOG"
      break
    fi
  else
    EXIT_CODE=$?
    FAIL_COUNT=$((FAIL_COUNT + 1))
    echo "[$(date -Is)] batch_eval exited $EXIT_CODE (fail $FAIL_COUNT/$MAX_CONSECUTIVE_FAILS); retry in 30s" | tee -a "$LOG"
    if [[ "$FAIL_COUNT" -ge "$MAX_CONSECUTIVE_FAILS" ]]; then
      echo "[$(date -Is)] ERROR: $MAX_CONSECUTIVE_FAILS consecutive failures, aborting" | tee -a "$LOG"
      exit 1
    fi
    sleep 30
  fi
  
  # If results count already 150, stop
  n=$(wc -l < "$OUT/results.jsonl" 2>/dev/null || echo 0)
  if [[ "$n" -ge 150 ]]; then
    echo "[$(date -Is)] results.jsonl has $n rows — stopping" | tee -a "$LOG"
    break
  fi
  
  # Progress update
  PASSED=$(grep -c '"ok": true' "$OUT/summary.json" 2>/dev/null || echo 0)
  echo "[$(date -Is)] Progress: $n/150 designs, $PASSED passed" | tee -a "$LOG"
done
echo "[$(date -Is)] overnight exit" | tee -a "$LOG"
