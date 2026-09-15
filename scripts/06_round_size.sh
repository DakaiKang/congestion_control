#!/usr/bin/env bash
# Blocks per round (= proposers per round, one block each): 5..100.
#  (a) 2,000-block Cancun sample and 2,500-block artificial sample, all sizes;
#  (b) full real dataset at 5/10/20/50 blocks and synthetic at 20/50 (100 = script 01/02).
# ~3.5 h.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
export K=4 M=40 HOT_SET_SIZE=5
for b in 5 10 20 50 100; do
  f="$RESULTS/sweeps/roundsize_artificial_b${b}.csv"; have "$f" 5 || run test_rebuttal_artificial "" "$f" NUM_BATCHES=$((2500 / b)) BLOCKS_PER_BATCH=$b TXNS_PER_BLOCK=100 GREEDY_BATCH=$b
  f="$RESULTS/sweeps/roundsize_real_b${b}.csv";       have "$f" 5 || run test_rebuttal_real "" "$f" START_BLOCK=$CANCUN BATCH_SIZE=$b MAX_BATCHES=$((2000 / b))
done
for b in 5 10 20 50; do
  for start in $CHUNKS; do
    f="$RESULTS/sweeps/roundsize_full_real_b${b}_${start}.csv"; have "$f" 5 || run test_rebuttal_real "" "$f" START_BLOCK=$start BATCH_SIZE=$b MAX_BATCHES=$((5000 / b))
  done
done
for b in 20 50; do
  f="$RESULTS/sweeps/roundsize_full_v2_b${b}.csv"; have "$f" 5 || run test_rebuttal_v2 "" "$f" BATCH_SIZE=$b MAX_BATCHES=$((15000 / b))
done
echo "ROUND SIZE DONE $(date +%H:%M)"
