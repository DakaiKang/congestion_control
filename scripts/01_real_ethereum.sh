#!/usr/bin/env bash
# Real Ethereum, 150 rounds x 100 blocks (3 segments x 5,000 blocks): every engine
# (sequential, Block-STM, concatenated Block-STM, concatenated graph OCC,
# graph-aware OCC, Omakase) with the full stage breakdown. Two passes: timing
# build, then `--features diagnostics` for the abort accounting.  ~1 h.
. "$(dirname "$0")/common.sh"
for feat in "" "--features diagnostics"; do
  if [ -z "$feat" ]; then build_timing || exit 1; else build_diag || exit 1; fi
  suf=""; [ -n "$feat" ] && suf="_diag"
  for start in $CHUNKS; do
    f="$RESULTS/real_${start}${suf}.csv"; have "$f" 50 && { echo "skip $f"; continue; }
    run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=50
  done
done
echo "REAL DONE $(date +%H:%M)"
