#!/usr/bin/env bash
# Artificial workload with tunable conflicts (paper Fig. 7 grid): H=5 hot keys,
# rho_inter = K in {2,4,6,8}, rho_intra = M in {10,20,30,40}%, 100 rounds x 50
# blocks x 100 txs. Timing and diagnostics passes.  ~3 h.
. "$(dirname "$0")/common.sh"
export NUM_BATCHES=${NUM_BATCHES:-100} BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100 HOT_SET_SIZE=5
for feat in "" "--features diagnostics"; do
  if [ -z "$feat" ]; then build_timing || exit 1; else build_diag || exit 1; fi
  suf=""; [ -n "$feat" ] && suf="_diag"
  for K in 2 4 6 8; do for M in 10 20 30 40; do
    f="$RESULTS/artificial/k${K}_m${M}${suf}.csv"; have "$f" "$NUM_BATCHES" && { echo "skip $f"; continue; }
    run test_rebuttal_artificial "$feat" "$f" K=$K M=$M
  done; done
done
echo "ARTIFICIAL DONE $(date +%H:%M)"
