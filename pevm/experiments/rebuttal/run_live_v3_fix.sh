#!/usr/bin/env bash
# Rebuild mysticeti against the current pevm (paper graph construction) and
# redo the live run of the v3 campaign, whose first attempt used a stale binary.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export GRAPH_EDGES=raw TRACK_BASIC_READS=1 REB_ROOT=experiments/rebuttal_v3
until grep -q "CAMPAIGN v3 DONE" "${WAIT_LOG:-/dev/null}" 2>/dev/null; do sleep 20; done
( cd ../mysticeti && cargo build --release -p mysticeti 2>&1 | grep -E "^error" -A 6; echo "MYST_BUILD_EXIT=${PIPESTATUS[0]}" )
strings ../mysticeti/target/release/mysticeti | grep -c "GRAPH_EDGES" | sed 's/^/binary has GRAPH_EDGES strings: /'
rm -f $REB_ROOT/live/*.log $REB_ROOT/live/summary*.csv
OUT=$PWD/$REB_ROOT/live PEVM_MIN_ROUND_TXS=1000 PEVM_LOAD=150000 experiments/rebuttal/run_live_mysticeti.sh
[ -f $REB_ROOT/live/summary.csv ] && mv $REB_ROOT/live/summary.csv $REB_ROOT/live/summary_minround1000.csv
python3 experiments/rebuttal/analyze.py --dir $REB_ROOT > /dev/null 2>&1 && echo "REPORT regenerated"
echo "LIVE_V3_DONE $(date +%H:%M)"
