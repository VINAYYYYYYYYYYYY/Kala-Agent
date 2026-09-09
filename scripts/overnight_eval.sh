#!/usr/bin/env bash
# Overnight batch: run all 150 designs with resume, isolated per-design folders.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=outputs/eval/batch_cad1000_150
mkdir -p "$OUT"
LOG="$OUT/overnight.log"
echo "[$(date -Is)] overnight start" | tee -a "$LOG"
# Keep going until summary says done or process dies; --resume is safe.
while true; do
  echo "[$(date -Is)] batch_eval resume pass" | tee -a "$LOG"
  if uv run python scripts/batch_eval.py designs/batch_cad1000_150.jsonl --backend freecad --resume >>"$LOG" 2>&1; then
    if grep -q '"done": true' "$OUT/summary.json" 2>/dev/null; then
      echo "[$(date -Is)] ALL DESIGNS COMPLETE" | tee -a "$LOG"
      break
    fi
  else
    echo "[$(date -Is)] batch_eval exited non-zero; retry in 30s" | tee -a "$LOG"
    sleep 30
  fi
  # If results count already 150, stop
  n=$(wc -l < "$OUT/results.jsonl" 2>/dev/null || echo 0)
  if [[ "$n" -ge 150 ]]; then
    echo "[$(date -Is)] results.jsonl has $n rows — stopping" | tee -a "$LOG"
    break
  fi
done
echo "[$(date -Is)] overnight exit" | tee -a "$LOG"
