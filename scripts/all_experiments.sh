#!/usr/bin/env bash
# Run every experiment of the paper's evaluation in sequence (no two timing runs
# share the machine), then generate REPORT.md. Each step skips outputs that
# already exist, so an interrupted run can simply be restarted.
#
#   RESULTS=experiments/results (default)   output root under pevm/
#   Total wall time on a c4.8xlarge: about 15 hours.
#
# Individual experiments can be run alone: scripts/01_real_ethereum.sh, ...
S="$(cd "$(dirname "$0")" && pwd)"
echo "==> ALL EXPERIMENTS start $(date +%F_%H:%M)"
for step in 01_real_ethereum 02_synthetic 03_vegeta 04_artificial_grid 05_threads 06_round_size \
            07_merge_cap 08_tx_cost 09_aggressive_integration 10_pipeline 11_state_latency \
            12_live_mysticeti 13_graph_construction_ablation 14_hot_key_sweep 15_tau_cv_sweep; do
  echo "==> $step  $(date +%H:%M)"
  bash "$S/$step.sh" || echo "!! $step exited with status $?"
done
. "$S/common.sh"
python3 experiments/rebuttal/analyze.py --dir "$RESULTS" && echo "REPORT: pevm/$RESULTS/REPORT.md"
echo "==> ALL EXPERIMENTS DONE $(date +%F_%H:%M)"
