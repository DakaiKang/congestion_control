#!/usr/bin/env bash
# Full rerun with the paper's Algorithm 1 graph construction (GRAPH_EDGES, see
# dependency_graph.rs) and account-level reads in the predicted read set
# (TRACK_BASIC_READS=1). Results go to experiments/rebuttal_v3/ so the
# submission-code-path (WAW-only) results in experiments/rebuttal/ stay as the
# ablation. Serial queue: no two timing runs share the machine.
#   MODE=raw | raw+waw    (default raw = Algorithm 1 as written)
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export GRAPH_EDGES=${MODE:-raw} TRACK_BASIC_READS=1
export REB_ROOT=experiments/rebuttal_v3
S=experiments/rebuttal
mkdir -p $REB_ROOT/sweeps $REB_ROOT/artificial $REB_ROOT/live
[ -e $REB_ROOT/vegeta ] || ln -s ../rebuttal/vegeta $REB_ROOT/vegeta   # Vegeta does not use the graph
echo "==> CAMPAIGN v3 start $(date +%F_%H:%M)  GRAPH_EDGES=$GRAPH_EDGES TRACK_BASIC_READS=$TRACK_BASIC_READS -> $REB_ROOT"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data

$S/run_real_v2.sh                       # real x3 + V2, timing + diagnostics
$S/run_artificial.sh                    # K x M grid, timing + diagnostics
$S/run_sweeps.sh                        # threads / round size (2k sample) / merge cap / tx cost / aggressive
DELAYS="0 1000 2000 5000 10000 20000 50000" $S/run_state_latency.sh
SUFFIX=_t16 NUM_THREADS=16 DELAYS="20000 50000" $S/run_state_latency.sh
SUFFIX=_t32 NUM_THREADS=32 DELAYS="20000 50000" $S/run_state_latency.sh
$S/run_roundsize_full.sh                # round size 20 / 50 on the full datasets

# Aggressive integration on the full real dataset (tau_cv ~ 0, cap 10)
export NUM_THREADS=8 HOT_KEY_THRESHOLD=1.5 BATCH_SIZE=100 BLOCKS_DIR=$D/blocks_rw CHECK_STATE=1
for start in 16774645 18581726 19557289; do
  f=$REB_ROOT/sweeps/aggressive_full_real_${start}.csv
  [ -s "$f" ] && [ "$(wc -l < "$f")" -gt 50 ] && { echo "skip $f"; continue; }
  echo "==> $f  $(date +%H:%M)"
  TAU_CV=0.01 MERGE_CAP=10 START_BLOCK=$start MAX_BATCHES=50 OUTPUT="$f" cargo test --release --test rebuttal_test \
      test_rebuttal_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked" | tail -1
done
# Pipelined integration
export TAU_CV=0.5
for b in 100 20; do
  f=$REB_ROOT/sweeps/pipeline_real_b${b}.csv; nr=10; [ $b = 20 ] && nr=20
  [ -s "$f" ] && { echo "skip $f"; continue; }
  echo "==> $f  $(date +%H:%M)"
  START_BLOCK=19557289 BATCH_SIZE=$b NUM_ROUNDS=$nr MERGE_CAP=10 OUTPUT="$f" cargo test --release --test rebuttal_test \
      test_rebuttal_pipeline_real -- --nocapture --exact 2>&1 | grep -aE "✓|panicked|pipelined" | tail -2
done
# Live 4-validator Mysticeti (rebuild picks up the new graph code; env passes through)
( cd mysticeti && cargo build --release -p mysticeti 2>&1 | grep -E "^error" -A 6; echo "MYST_BUILD_EXIT=${PIPESTATUS[0]}" )
OUT=$PWD/$REB_ROOT/live PEVM_MIN_ROUND_TXS=1000 PEVM_LOAD=150000 $S/run_live_mysticeti.sh
[ -f $REB_ROOT/live/summary.csv ] && mv $REB_ROOT/live/summary.csv $REB_ROOT/live/summary_minround1000.csv
python3 $S/analyze.py --dir $REB_ROOT > /dev/null 2>&1 && echo "REPORT: $REB_ROOT/REPORT.md"
echo "==> CAMPAIGN v3 DONE $(date +%F_%H:%M)"
