#!/usr/bin/env bash
# Aggressive integration: merge whenever no hot key is shared (TAU_CV ~ 0) with
# caps 10/50/1000 on the 20-round samples, plus cap 10 over the full real dataset.  ~1 h.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
export K=4 M=40 HOT_SET_SIZE=5
for c in 10 50 1000; do
  f="$RESULTS/sweeps/aggressive_artificial_c${c}.csv"; have "$f" 50 || run test_rebuttal_artificial "" "$f" TAU_CV=0.01 MERGE_CAP=$c NUM_BATCHES=50 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
  f="$RESULTS/sweeps/aggressive_real_c${c}.csv";       have "$f" 20 || run test_rebuttal_real "" "$f" TAU_CV=0.01 MERGE_CAP=$c START_BLOCK=$CANCUN MAX_BATCHES=20
done
for start in $CHUNKS; do
  f="$RESULTS/sweeps/aggressive_full_real_${start}.csv"; have "$f" 50 || run test_rebuttal_real "" "$f" TAU_CV=0.01 MERGE_CAP=10 START_BLOCK=$start MAX_BATCHES=50
done
echo "AGGRESSIVE DONE $(date +%H:%M)"
