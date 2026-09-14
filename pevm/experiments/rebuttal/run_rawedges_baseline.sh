#!/usr/bin/env bash
# Baseline (GRAPH_RAW_EDGES unset) diagnostics rerun on the same 20 rounds per
# chunk, with the cross-block abort counters. Waits for run_rawedges.sh.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=experiments/rebuttal/rawedges; mkdir -p "$OUT"
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 BATCH_SIZE=100 BLOCKS_DIR=$D/blocks_rw CHECK_STATE=1
unset GRAPH_RAW_EDGES
WAIT_LOG=${WAIT_LOG:-}
[ -n "$WAIT_LOG" ] && until grep -q "RAWEDGES_DONE" "$WAIT_LOG" 2>/dev/null; do sleep 30; done
cargo build --release --features diagnostics --test rebuttal_test 2>&1 | grep -E "^error" -A 6; echo "BUILD_EXIT=${PIPESTATUS[0]}"
for start in 16774645 18581726 19557289; do
  f="$OUT/none_diag_${start}.csv"
  echo "==> $f  $(date +%H:%M)"
  START_BLOCK=$start MAX_BATCHES=${NB:-20} OUTPUT="$f" cargo test --release --features diagnostics --test rebuttal_test \
      test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|skipping|failed \(" | tail -2
done
echo "BASELINE_DONE $(date +%H:%M)"
