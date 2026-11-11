#!/usr/bin/env bash
# Work per transaction (artificial TARGET = 100 paper, 500 = 5x, 2000 = 20x):
# does the constant integration cost pay off once transactions are expensive?  ~30 min.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
export K=4 M=40 HOT_SET_SIZE=5
for tg in 100 500 2000; do
  f="$RESULTS/sweeps/txcost_artificial_g${tg}.csv"; have "$f" 30 || run test_rebuttal_artificial "" "$f" TARGET=$tg NUM_BATCHES=30 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
done
echo "TX COST DONE $(date +%H:%M)"
