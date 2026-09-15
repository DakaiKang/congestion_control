#!/usr/bin/env bash
# Ablation of the conflict-graph construction on 20 rounds per real segment:
# the paper's Algorithm 1 (default: RAW edges from the last writer, account-level
# reads in the read set) against the write-after-write-only variant
# (GRAPH_EDGES=waw TRACK_BASIC_READS=0). Timing and diagnostics passes.  ~40 min.
. "$(dirname "$0")/common.sh"
OUT="$RESULTS/graph_ablation"; mkdir -p "$OUT"
for feat in "" "--features diagnostics"; do
  if [ -z "$feat" ]; then build_timing || exit 1; else build_diag || exit 1; fi
  suf="timing"; [ -n "$feat" ] && suf="diag"
  for start in $CHUNKS; do
    f="$OUT/algorithm1_${suf}_${start}.csv"; have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20
    f="$OUT/waw_only_${suf}_${start}.csv";   have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20 GRAPH_EDGES=waw TRACK_BASIC_READS=0
  done
done
echo "GRAPH ABLATION DONE $(date +%H:%M)"
