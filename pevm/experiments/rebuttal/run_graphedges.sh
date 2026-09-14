#!/usr/bin/env bash
# Paper Algorithm 1 (GRAPH_EDGES=raw: tail = last writer, RAW edges writer ->
# reader, head sets) and raw+waw, against the WAW-only construction the
# submission's experiments used (baseline timing = real_*.csv first NB rounds;
# baseline diagnostics rerun here for the cross-block counters).
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=experiments/rebuttal/graphedges; mkdir -p "$OUT"
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 BATCH_SIZE=100 BLOCKS_DIR=$D/blocks_rw CHECK_STATE=1
unset GRAPH_RAW_EDGES
NB=${NB:-20}
run() { # $1 mode $2 build
  local mode=$1 build=$2 feat=""; [ $build = diag ] && feat="--features diagnostics"
  for start in 16774645 18581726 19557289; do
    f="$OUT/${mode}_${build}_${start}.csv"
    [ -s "$f" ] && [ "$(wc -l < "$f")" -gt "$NB" ] && { echo "skip $f"; continue; }
    echo "==> $f  $(date +%H:%M)"
    GRAPH_EDGES=$mode START_BLOCK=$start MAX_BATCHES=$NB OUTPUT="$f" cargo test --release $feat --test rebuttal_test \
        test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|skipping|failed \(|state !=" | tail -3
  done
}
cargo build --release --test rebuttal_test 2>&1 | grep -E "^error" -A 6; echo "BUILD_timing_EXIT=${PIPESTATUS[0]}"
for mode in raw raw+waw; do run $mode timing; done
cargo build --release --features diagnostics --test rebuttal_test 2>&1 | grep -E "^error" -A 6; echo "BUILD_diag_EXIT=${PIPESTATUS[0]}"
for mode in waw raw raw+waw; do run $mode diag; done
echo "GRAPHEDGES_DONE $(date +%H:%M)"
