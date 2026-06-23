#!/usr/bin/env bash
# Thread-count sweep over all 20000 blocks (200 batches × 100 blocks).
# real workload: test_eth_block_data_all_batches  (BLOCKS_DIR)
# v2   workload: test_v2_all_batches              (RW_TIME_DIR)
# NUM_THREADS unifies BOTH the actual parallel execution AND the
# dependency-graph / greedy CV simulation (verified in source).
# Params held fixed: tau_cv=0.5, hot_kt=1.5.
set -u

WS=/home/ubuntu/congestion_control/pevm
DATA=/home/ubuntu/eth-block-downloader/test_data
OUT=$WS/experiments/thread_sweep
mkdir -p "$OUT"
cd "$WS" || exit 1

THREADS=(2 4 8 12 16 20 24)
export BLOCKS_DIR=$DATA/blocks_rw
export RW_TIME_DIR=$DATA/rw_time
export TAU_CV=0.5
export HOT_KEY_THRESHOLD=1.5

overall=$(date +%s)
for t in "${THREADS[@]}"; do
  echo "############ NUM_THREADS=$t  (real) ############"
  st=$(date +%s)
  NUM_THREADS=$t OUTPUT=$OUT/real_t${t}.csv \
    cargo test --release --test bench_test test_eth_block_data_all_batches \
    -- --nocapture --exact 2>&1 | grep -E 'done in|^✓|Found [0-9]|batch .* failed'
  echo ">>> real t=$t done in $(( $(date +%s) - st ))s"

  echo "############ NUM_THREADS=$t  (v2) ############"
  st=$(date +%s)
  NUM_THREADS=$t OUTPUT=$OUT/v2_t${t}.csv \
    cargo test --release --test tx_simulator_test test_v2_all_batches \
    -- --nocapture --exact 2>&1 | grep -E 'done in|^✓|files=|batch .* load failed'
  echo ">>> v2 t=$t done in $(( $(date +%s) - st ))s"
done
echo "ALL DONE in $(( $(date +%s) - overall ))s"
ls -la "$OUT"
