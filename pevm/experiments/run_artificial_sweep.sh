#!/usr/bin/env bash
# Fully-artificial workload sweep: controllable inter-block (K) and
# intra-block (M) conflict. 100 batches x 50 blocks x 100 txs, TxSimulatorV2,
# integration defaults tau_cv=0.5 / hot_kt=1.5 at NUM_THREADS=8.
#   K in {1,2,3,4}   (inter-block: hot-group size 9+K, first K blocks share h_1)
#   M in {10,20,30,40}%  (intra-block: first M% of a block's txs touch hot key)
# One CSV per (K,M) into artificial_sweep/k<K>_m<M>.csv.
# No real blocks => no hangs/panics, so a plain loop suffices.
set -u

WS=/home/ubuntu/congestion_control/pevm
OUT=$WS/experiments/artificial_sweep
mkdir -p "$OUT"
cd "$WS" || exit 1

export NUM_BATCHES=100
export BLOCKS_PER_BATCH=50
export TXNS_PER_BLOCK=100
export TARGET=100
export WRITES_PER_TX=3
export GREEDY_BATCH=50
export NUM_THREADS=8
export TAU_CV=0.5
export HOT_KEY_THRESHOLD=1.5

overall=$(date +%s)
for k in 1 2 3 4; do
  for m in 10 20 30 40; do
    echo "############ K=$k M=$m ############"
    st=$(date +%s)
    K=$k M=$m OUTPUT=$OUT/k${k}_m${m}.csv \
      cargo test --release --test tx_simulator_test test_artificial_all_batches \
      -- --nocapture --exact 2>&1 | grep -E 'artificial:|^✓'
    echo ">>> K=$k M=$m done in $(( $(date +%s) - st ))s"
  done
done
echo "ALL DONE in $(( $(date +%s) - overall ))s"
ls -la "$OUT"
