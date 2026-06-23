#!/usr/bin/env bash
# Resume the REAL thread sweep for batches 150..199 and append to the existing
# thread_sweep/real_t<t>.csv (which already hold batches 0..149). v2 is already
# complete at 200 batches, so it is not re-run here.
#   - START_BATCH=150 skips batches 0..149 and opens the CSV in append mode.
#   - run_one_batch now skips (empty row) any batch whose seq/par execution
#     errors (e.g. cross-boundary nonce), instead of unwrap-panicking.
# Same fixed params as the main sweep: tau_cv=0.5, hot_kt=1.5.
set -u

WS=/home/ubuntu/congestion_control/pevm
DATA=/home/ubuntu/eth-block-downloader/test_data
OUT=$WS/experiments/thread_sweep
cd "$WS" || exit 1

THREADS=(2 4 8 12 16 20 24)
export BLOCKS_DIR=$DATA/blocks_rw
export TAU_CV=0.5
export HOT_KEY_THRESHOLD=1.5
export START_BATCH=150

overall=$(date +%s)
for t in "${THREADS[@]}"; do
  echo "############ NUM_THREADS=$t  (real resume 150..199) ############"
  st=$(date +%s)
  NUM_THREADS=$t OUTPUT=$OUT/real_t${t}.csv \
    cargo test --release --test bench_test test_eth_block_data_all_batches \
    -- --nocapture --exact 2>&1 | grep -E 'done in|^✓|Found [0-9]|Resuming|skipping'
  echo ">>> real t=$t resume done in $(( $(date +%s) - st ))s | rows now: $(wc -l < $OUT/real_t${t}.csv)"
done
echo "ALL DONE in $(( $(date +%s) - overall ))s"
