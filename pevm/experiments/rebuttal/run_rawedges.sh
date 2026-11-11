#!/usr/bin/env bash
# Can Omakase's cascade aborts be reduced by gating readers on preceding
# writers?  GRAPH_RAW_EDGES=all adds a read-after-write edge for every key a
# transaction reads (intra- and cross-block); =cross adds only cross-block ones
# (the dependencies a block boundary resolved before integration). Applies to
# every graph-driven engine (Graph OCC, concat+graph, Omakase). Real Ethereum,
# first 20 rounds of each chunk; baseline = the same rounds of real_*.csv /
# real_*_diag.csv (GRAPH_RAW_EDGES unset).
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=experiments/rebuttal/rawedges; mkdir -p "$OUT"
export NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 BATCH_SIZE=100 BLOCKS_DIR=$D/blocks_rw CHECK_STATE=1
NB=${NB:-20}
for build in timing diag; do
  feat=""; [ $build = diag ] && feat="--features diagnostics"
  cargo build --release $feat --test rebuttal_test 2>&1 | grep -E "^error" -A 6; echo "BUILD_${build}_EXIT=${PIPESTATUS[0]}"
  for mode in all cross; do
    for start in 16774645 18581726 19557289; do
      f="$OUT/${mode}_${build}_${start}.csv"
      [ -s "$f" ] && [ "$(wc -l < "$f")" -gt "$NB" ] && { echo "skip $f"; continue; }
      echo "==> $f  $(date +%H:%M)"
      GRAPH_RAW_EDGES=$mode START_BLOCK=$start MAX_BATCHES=$NB OUTPUT="$f" cargo test --release $feat --test rebuttal_test \
          test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|skipping|failed \(|!=" | tail -3
    done
  done
done
echo "RAWEDGES_DONE $(date +%H:%M)"
