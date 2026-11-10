#!/usr/bin/env bash
# Ablation of the conflict graph on 20 rounds per real segment, timing and
# diagnostics passes (~1.5 h):
#   algorithm1   the paper's Algorithm 1 (default: RAW, WAW and WAR edges; the
#                executor gates on RAW parents only)
#   raw_only     RAW edges only (GRAPH_EDGES=raw): the order may flip WAR/WAW
#                pairs, the graph is rebuilt in the new order
#   gate_all     Algorithm 1's graph, executor waits for every parent
#                (GATE_EDGES=same)
#   no_reorder   Algorithm 1's graph, block order kept (REORDER=0)
#   waw_only     write-after-write-only graph (GRAPH_EDGES=waw TRACK_BASIC_READS=0)
. "$(dirname "$0")/common.sh"
OUT="$RESULTS/graph_ablation"; mkdir -p "$OUT"
for feat in "" "--features diagnostics"; do
  if [ -z "$feat" ]; then build_timing || exit 1; else build_diag || exit 1; fi
  suf="timing"; [ -n "$feat" ] && suf="diag"
  for start in $CHUNKS; do
    f="$OUT/algorithm1_${suf}_${start}.csv"; have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20
    f="$OUT/raw_only_${suf}_${start}.csv";   have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20 GRAPH_EDGES=raw
    f="$OUT/gate_all_${suf}_${start}.csv";   have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20 GATE_EDGES=same
    f="$OUT/no_reorder_${suf}_${start}.csv"; have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20 REORDER=0
    f="$OUT/waw_only_${suf}_${start}.csv";   have "$f" 20 || run test_rebuttal_real "$feat" "$f" START_BLOCK=$start MAX_BATCHES=20 GRAPH_EDGES=waw TRACK_BASIC_READS=0
  done
done
echo "GRAPH ABLATION DONE $(date +%H:%M)"
