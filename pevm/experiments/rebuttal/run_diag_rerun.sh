#!/usr/bin/env bash
# Re-run the diagnostics passes (real x3 chunks, synthetic V2) after the
# cascade-abort classification changed (2026-09-14): a cascade abort is now a
# validation failure caused by a lower-indexed transaction's *re-execution*.
# Old CSVs (previous definition) are kept as *_diag_v1.csv.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=experiments/rebuttal
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 BATCH_SIZE=100 BLOCKS_DIR=$D/blocks_rw
WAIT_LOG=${WAIT_LOG:-}
if [ -n "$WAIT_LOG" ]; then
  echo "waiting for ROUNDSIZE_FULL_DONE in $WAIT_LOG"
  until grep -q "ROUNDSIZE_FULL_DONE" "$WAIT_LOG" 2>/dev/null; do sleep 30; done
fi
for f in $OUT/real_*_diag.csv $OUT/v2_diag.csv; do
  [ -f "$f" ] && [ ! -f "${f%.csv}_v1.csv" ] && mv "$f" "${f%.csv}_v1.csv" && echo "kept old $f as ${f%.csv}_v1.csv"
done
cargo build --release --features diagnostics --test rebuttal_test 2>&1 | grep -E "^error" -A 8; echo "BUILD_EXIT=${PIPESTATUS[0]}"
for start in 16774645 18581726 19557289; do
  f="$OUT/real_${start}_diag.csv"
  echo "==> real chunk $start diag  $(date +%H:%M)"
  START_BLOCK=$start MAX_BATCHES=50 OUTPUT="$f" cargo test --release --features diagnostics --test rebuttal_test \
      test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|skipping|failed \(" | tail -2
done
f="$OUT/v2_diag.csv"
echo "==> v2 diag  $(date +%H:%M)"
MAX_BATCHES=150 CHECK_STATE=1 OUTPUT="$f" cargo test --release --features diagnostics --test rebuttal_test \
    test_rebuttal_v2 -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|assertion|load failed" | tail -2
echo "DIAG_RERUN_DONE $(date +%H:%M)"
