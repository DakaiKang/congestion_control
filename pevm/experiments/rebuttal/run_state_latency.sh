#!/usr/bin/env bash
# Sweep the emulated per-read state-access latency on 20 real (Cancun) batches.
# 0 = the in-memory setting of every other experiment; 2-20 us brackets the
# cost of a cached trie-node / SSD lookup in a production client.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export BLOCKS_DIR=/home/ubuntu/Omakase/eth-block-downloader/test_data/blocks_rw
export START_BLOCK=19557289 BATCH_SIZE=100 MAX_BATCHES=${MAX_BATCHES:-20} NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5
cargo build --release --test rebuttal_test >/dev/null 2>&1
for d in ${DELAYS:-0 1000 2000 5000 10000 20000}; do
  f=experiments/rebuttal/sweeps/statelat_real_d${d}.csv
  [ -s "$f" ] && [ "$(wc -l < "$f")" -gt "$MAX_BATCHES" ] && { echo "skip $f"; continue; }
  echo "==> STATE_DELAY_NS=$d"
  STATE_DELAY_NS=$d OUTPUT=$f cargo test --release --test rebuttal_test test_rebuttal_state_latency_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked"
done
echo "==> STATE LATENCY SWEEP DONE"
