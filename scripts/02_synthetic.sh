#!/usr/bin/env bash
# Synthetic V2 workload (paper Sec. 7.2.2; exact hints), 150 rounds x 100 blocks,
# timing and diagnostics passes.  ~45 min.
. "$(dirname "$0")/common.sh"
for feat in "" "--features diagnostics"; do
  if [ -z "$feat" ]; then build_timing || exit 1; else build_diag || exit 1; fi
  suf=""; [ -n "$feat" ] && suf="_diag"
  f="$RESULTS/v2${suf}.csv"; have "$f" 150 && { echo "skip $f"; continue; }
  run test_rebuttal_v2 "$feat" "$f" MAX_BATCHES=150
done
echo "SYNTHETIC DONE $(date +%H:%M)"
