#!/usr/bin/env bash
# Round size = number of proposers (one block per proposer per round) on the
# FULL datasets, so that the "every stage on one node, 1/P of the proposer
# stages" rows of the stage-cost table use integration and execution costs
# measured at round size P rather than at 100 blocks. Real: 3 chunks x 5,000
# blocks; synthetic V2: 15,000 blocks. Round size 100 is the main campaign
# (real_*.csv, v2.csv).
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=${REB_ROOT:-experiments/rebuttal}/sweeps; mkdir -p "$OUT"
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 CHECK_STATE=1 BLOCKS_DIR=$D/blocks_rw
have() { [ -s "$1" ] && [ "$(wc -l < "$1")" -gt 2 ]; }
cargo build --release --test rebuttal_test >/dev/null 2>&1 || { echo "build failed"; exit 1; }
for b in ${SIZES:-20 50}; do
  for start in 16774645 18581726 19557289; do
    f="$OUT/roundsize_full_real_b${b}_${start}.csv"; have "$f" && { echo "skip $f"; continue; }
    echo "==> $f  $(date +%H:%M)"
    START_BLOCK=$start BATCH_SIZE=$b MAX_BATCHES=$((5000 / b)) OUTPUT="$f" cargo test --release --test rebuttal_test \
        test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|skipping|failed \(" | tail -3
  done
  f="$OUT/roundsize_full_v2_b${b}.csv"; have "$f" && { echo "skip $f"; continue; }
  echo "==> $f  $(date +%H:%M)"
  BATCH_SIZE=$b MAX_BATCHES=$((15000 / b)) OUTPUT="$f" cargo test --release --test rebuttal_test \
      test_rebuttal_v2 -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|assertion|load failed" | tail -3
done
echo "ROUNDSIZE_FULL_DONE $(date +%H:%M)"
