#!/usr/bin/env bash
# Omakase's group cap (blocks per integrated group): 10 (paper), 25, 50, 1000
# (= whole round). Artificial (K=4, M=40%) and real (20 Cancun rounds).  ~30 min.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
export K=4 M=40 HOT_SET_SIZE=5
for c in 10 25 50 1000; do
  f="$RESULTS/sweeps/mergecap_artificial_c${c}.csv"; have "$f" 50 || run test_rebuttal_artificial "" "$f" MERGE_CAP=$c NUM_BATCHES=50 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
  f="$RESULTS/sweeps/mergecap_real_c${c}.csv";       have "$f" 20 || run test_rebuttal_real "" "$f" MERGE_CAP=$c START_BLOCK=$CANCUN MAX_BATCHES=20
done
echo "MERGE CAP DONE $(date +%H:%M)"
