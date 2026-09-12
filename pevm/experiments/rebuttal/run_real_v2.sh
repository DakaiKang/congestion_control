#!/usr/bin/env bash
# Rebuttal runs on the paper's two main workloads. Waits until the 15 000-block
# dataset and its rw_time derivative are complete so the timing runs happen on
# a quiet machine, then runs each workload twice (timing build, diagnostics
# build). 150 batches x 100 blocks, t=8, tau_cv=0.5, hot_kt=1.5 as in the paper.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=experiments/rebuttal
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 BATCH_SIZE=100
export BLOCKS_DIR=$D/blocks_rw RW_TIME_DIR=$D/rw_time

echo "==> waiting for dataset (15000 blocks_rw, >=14900 rw_time, no generators running)"
while :; do
  b=$(ls $D/blocks_rw 2>/dev/null | wc -l); r=$(ls $D/rw_time 2>/dev/null | wc -l)
  gen=$(pgrep -fc '(^|/)(finish_download\.sh|generate_synthetic\.sh|download_rw [0-9]|tx_simulator_test-)'); gen=${gen:-0}
  if [ "$b" -ge 15000 ] && [ "$r" -ge 14900 ] && [ "$gen" -eq 0 ]; then break; fi
  # Fall back once the download driver has given up but nothing else is running.
  if [ "$gen" -eq 0 ] && grep -q "FINAL" /tmp/finish_download.log 2>/dev/null && grep -q "FINAL" /tmp/gen_synthetic.log 2>/dev/null; then
    echo "    generators finished with blocks_rw=$b rw_time=$r; proceeding"; break; fi
  sleep 120
done
echo "==> dataset ready: blocks_rw=$(ls $D/blocks_rw|wc -l) rw_time=$(ls $D/rw_time|wc -l)"

run_real() {  # $1=features flag ("" or "--features diagnostics") $2=suffix
  for start in 16774645 18581726 19557289; do
    f="$OUT/real_${start}${2}.csv"
    [ -s "$f" ] && [ "$(wc -l < "$f")" -gt 50 ] && { echo "skip $f"; continue; }
    echo "==> real chunk $start $2"
    START_BLOCK=$start MAX_BATCHES=50 OUTPUT="$f" cargo test --release $1 --test rebuttal_test \
        test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|skipping|failed \("
  done
}
run_v2() {
  f="$OUT/v2${2}.csv"
  [ -s "$f" ] && [ "$(wc -l < "$f")" -gt 150 ] && { echo "skip $f"; return; }
  echo "==> v2 $2"
  MAX_BATCHES=150 CHECK_STATE=1 OUTPUT="$f" cargo test --release $1 --test rebuttal_test \
      test_rebuttal_v2 -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|assertion|load failed"
}

cargo build --release --test rebuttal_test >/dev/null 2>&1
run_real "" ""
run_v2 "" ""
cargo build --release --features diagnostics --test rebuttal_test >/dev/null 2>&1
run_real "--features diagnostics" "_diag"
run_v2 "--features diagnostics" "_diag"
echo "==> REAL+V2 DONE"
