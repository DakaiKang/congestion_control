#!/usr/bin/env bash
# Live four-validator Mysticeti committee on one machine, each validator executing
# every committed round with one of four executors (sequential, Block-STM,
# concatenated Block-STM, Omakase). ERC20 workload, PEVM_LOAD=150k tx/s per
# validator, rounds accumulated to >= 1,000 txs, 90 s per mode.  ~10 min.
. "$(dirname "$0")/common.sh"
( cd ../mysticeti && cargo build --release -p mysticeti 2>&1 | grep -E "^error" -A 8; echo "mysticeti build exit ${PIPESTATUS[0]}" )
OUT="$PWD/$RESULTS/live" PEVM_MIN_ROUND_TXS=${PEVM_MIN_ROUND_TXS:-1000} PEVM_LOAD=${PEVM_LOAD:-150000} \
  bash experiments/rebuttal/run_live_mysticeti.sh
[ -f "$RESULTS/live/summary.csv" ] && mv "$RESULTS/live/summary.csv" "$RESULTS/live/summary_minround1000.csv"
python3 experiments/rebuttal/live_phases.py "$RESULTS/live" 4 | tail -2
echo "LIVE DONE $(date +%H:%M)"
