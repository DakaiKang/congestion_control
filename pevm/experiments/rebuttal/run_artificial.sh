#!/usr/bin/env bash
# Rebuttal sweep on the fully-artificial workload (no dataset needed).
# Same grid as the paper's Figure 7: H=5, rho_inter=K in {2,4,6,8},
# rho_intra=M in {10,20,30,40}%, 100 batches x 50 blocks x 100 txs, t=8.
#
# Two passes per point: a plain build for timing, and a `--features
# diagnostics` build for the abort breakdown (the counters add overhead, so
# they are never read from the timing run).
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
OUT=experiments/rebuttal/artificial
mkdir -p "$OUT"
export NUM_BATCHES=${NUM_BATCHES:-100} BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100 HOT_SET_SIZE=5
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 CHECK_STATE=1

cargo build --release --test rebuttal_test >/dev/null 2>&1
for K in 2 4 6 8; do for M in 10 20 30 40; do
  f="$OUT/k${K}_m${M}.csv"
  [ -s "$f" ] && [ "$(wc -l < "$f")" -gt "$NUM_BATCHES" ] && { echo "skip $f"; continue; }
  echo "==> timing K=$K M=$M"
  K=$K M=$M OUTPUT="$f" cargo test --release --test rebuttal_test test_rebuttal_artificial \
      -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|assertion" 
done; done

cargo build --release --features diagnostics --test rebuttal_test >/dev/null 2>&1
for K in 2 4 6 8; do for M in 10 20 30 40; do
  f="$OUT/k${K}_m${M}_diag.csv"
  [ -s "$f" ] && [ "$(wc -l < "$f")" -gt "$NUM_BATCHES" ] && { echo "skip $f"; continue; }
  echo "==> diagnostics K=$K M=$M"
  K=$K M=$M OUTPUT="$f" cargo test --release --features diagnostics --test rebuttal_test \
      test_rebuttal_artificial -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|assertion"
done; done
echo "==> ARTIFICIAL SWEEP DONE"
