#!/usr/bin/env bash
# Worker-count sweep (4..32 threads): artificial (K=4, M=40%) and real Ethereum
# (20 Cancun rounds).  ~30 min.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
export K=4 M=40 HOT_SET_SIZE=5
for t in 4 8 16 24 32; do
  f="$RESULTS/sweeps/threads_artificial_t${t}.csv"; have "$f" 50 || run test_rebuttal_artificial "" "$f" NUM_THREADS=$t NUM_BATCHES=50 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
  f="$RESULTS/sweeps/threads_real_t${t}.csv";       have "$f" 20 || run test_rebuttal_real "" "$f" NUM_THREADS=$t START_BLOCK=$CANCUN MAX_BATCHES=20
done
echo "THREADS DONE $(date +%H:%M)"
