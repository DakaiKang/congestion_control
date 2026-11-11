#!/usr/bin/env bash
# Vegeta (NSDI'25) reimplementation: speculate-order-replay with Rule 1/2 and a
# serial tail, timed against sequential only (ENGINES=vegeta skips the other
# engines). Real Ethereum, synthetic V2 and the artificial grid.  ~1 h.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
export ENGINES=vegeta
for start in $CHUNKS; do
  f="$RESULTS/vegeta/real_${start}.csv"; have "$f" 50 && { echo "skip $f"; continue; }
  run test_rebuttal_real "" "$f" START_BLOCK=$start MAX_BATCHES=50
done
f="$RESULTS/vegeta/v2.csv"; have "$f" 150 || run test_rebuttal_v2 "" "$f" MAX_BATCHES=150
export NUM_BATCHES=${NUM_BATCHES:-100} BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100 HOT_SET_SIZE=5
for K in 2 4 6 8; do for M in 10 20 30 40; do
  f="$RESULTS/vegeta/artificial/k${K}_m${M}.csv"; have "$f" "$NUM_BATCHES" && { echo "skip $f"; continue; }
  run test_rebuttal_artificial "" "$f" K=$K M=$M
done; done
echo "VEGETA DONE $(date +%H:%M)"
