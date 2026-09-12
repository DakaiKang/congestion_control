#!/usr/bin/env bash
# Where does the concatenated-block baseline stop winning?  Three sweeps that
# vary the one thing each hypothesis is about, everything else at paper defaults.
#
#   threads    — Block-STM's abort/validation contention grows with workers and
#                with window size; concat has the largest window.
#   roundsize  — a real multi-proposer round is 5–20 blocks, not 100; concat's
#                fixed-cost amortisation shrinks with the round.
#   mergecap   — Omakase's group size is capped (paper: 10 blocks). Lifting it
#                moves Omakase toward "concat + graph"; if speedup follows, the
#                gap was a calibration choice, not a method limit.
#
# Artificial points use the highest-contention grid corner (K=4, M=40%). Real
# points use the first 20 batches of the Cancun chunk. Timing builds only; the
# main campaign's diagnostics runs cover abort counts.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
D=/home/ubuntu/Omakase/eth-block-downloader/test_data
OUT=experiments/rebuttal/sweeps; mkdir -p "$OUT"
export TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 HOT_SET_SIZE=5 K=4 M=40 CHECK_STATE=1
export BLOCKS_DIR=$D/blocks_rw
REAL_START=19557289 REAL_BATCHES=20
ART="cargo test --release --test rebuttal_test test_rebuttal_artificial -- --nocapture --exact"
REAL="cargo test --release --test rebuttal_test test_rebuttal_real -- --nocapture --exact"
have() { [ -s "$1" ] && [ "$(wc -l < "$1")" -gt 2 ]; }
run() { # $1 label $2 file $3... env assignments ; runs $CMD
  local f=$2; shift 2; have "$f" && { echo "skip $f"; return; }
  echo "==> $f"; env "$@" OUTPUT="$f" $CMD 2>&1 | grep -aE "✓|panicked|assertion|failed \("
}

cargo build --release --test rebuttal_test >/dev/null 2>&1

# 1. threads
for t in 4 8 16 24 32; do
  CMD=$ART  run thr "$OUT/threads_artificial_t${t}.csv" NUM_THREADS=$t NUM_BATCHES=50 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
  CMD=$REAL run thr "$OUT/threads_real_t${t}.csv" NUM_THREADS=$t START_BLOCK=$REAL_START BATCH_SIZE=100 MAX_BATCHES=$REAL_BATCHES
done
# 2. round size (blocks per batch); keep total txs comparable by scaling batches
for b in 5 10 20 50 100; do
  nb=$(( 2500 / b ))
  CMD=$ART  run rs "$OUT/roundsize_artificial_b${b}.csv" NUM_THREADS=8 NUM_BATCHES=$nb BLOCKS_PER_BATCH=$b TXNS_PER_BLOCK=100 GREEDY_BATCH=$b
  rb=$(( 2000 / b ))
  CMD=$REAL run rs "$OUT/roundsize_real_b${b}.csv" NUM_THREADS=8 START_BLOCK=$REAL_START BATCH_SIZE=$b MAX_BATCHES=$rb
done
# 3. merge cap (only Omakase changes; other engines are re-measured as controls)
for c in 10 25 50 1000; do
  CMD=$ART  run mc "$OUT/mergecap_artificial_c${c}.csv" NUM_THREADS=8 MERGE_CAP=$c NUM_BATCHES=50 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
  CMD=$REAL run mc "$OUT/mergecap_real_c${c}.csv" NUM_THREADS=8 MERGE_CAP=$c START_BLOCK=$REAL_START BATCH_SIZE=100 MAX_BATCHES=$REAL_BATCHES
done
# 4. per-transaction cost: does avoiding a re-execution pay once transactions are
#    expensive? (artificial only — TARGET is the simulated work per tx)
for tg in 100 500 2000; do
  CMD=$ART run cost "$OUT/txcost_artificial_g${tg}.csv" NUM_THREADS=8 TARGET=$tg NUM_BATCHES=30 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
done
# 5. aggressive integration: merge whenever no hot-key conflict, up to the whole
#    round (TAU_CV~0 disables the "already decent, stop" early-exit). Omakase
#    then differs from concat+graph only by hot-key isolation and reordering.
for c in 10 50 1000; do
  CMD=$ART  run agg "$OUT/aggressive_artificial_c${c}.csv" NUM_THREADS=8 TAU_CV=0.01 MERGE_CAP=$c NUM_BATCHES=50 BLOCKS_PER_BATCH=50 TXNS_PER_BLOCK=100
  CMD=$REAL run agg "$OUT/aggressive_real_c${c}.csv" NUM_THREADS=8 TAU_CV=0.01 MERGE_CAP=$c START_BLOCK=$REAL_START BATCH_SIZE=100 MAX_BATCHES=$REAL_BATCHES
done
echo "==> SWEEPS DONE"
