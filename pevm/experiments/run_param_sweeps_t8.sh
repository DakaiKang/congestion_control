#!/usr/bin/env bash
# hot_key_threshold and tau_cv sweeps at NUM_THREADS=8 over all 20000 blocks
# (200 batches x 100 blocks), real ETH workload (test_eth_block_data_all_batches).
#   hot sweep: HOT_KEY_THRESHOLD in {0.5,1.0,1.5,3.0,5.0}, tau_cv fixed 0.5
#   tau sweep: TAU_CV in {0.1,0.3,0.5,1.0,2.0}, hot_key_threshold fixed 1.5
# Output: one CSV per value into hot_sweep_t8/ and tau_sweep_t8/.
#
# Both params only affect the INTEGRATED path, so SWEEP_INTEG_ONLY=1 skips the
# Parallel and Graph-Parallel timed runs (their tput is reported 0). We only need
# integrated_tput / seq_tput.
#
# Chunk-2 (Pectra) batches 159/168/171/182 hang or panic; they are seeded into
# SKIP_BATCHES. The driver also auto-detects new stalls/crashes, adds the
# offending batch, and resumes that config (append). A longer STARTUP grace
# avoids false-flagging batch 0 (block discovery + first batch can exceed STALL).
#
# Usage: run_param_sweeps_t8.sh [hot|tau|both]   (default both)
set -u

WHICH=${1:-both}
WS=/home/ubuntu/congestion_control/pevm
DATA=/home/ubuntu/eth-block-downloader/test_data
EXP=$WS/experiments
cd "$WS" || exit 1
mkdir -p "$EXP/hot_sweep_t8" "$EXP/tau_sweep_t8"

export BLOCKS_DIR=$DATA/blocks_rw
export NUM_THREADS=8
export SWEEP_INTEG_ONLY=1
LAST=199
STALL=60          # no new row > this (after first row) => hung batch
STARTUP=300       # grace for the first row (discovery + first batch)
POLL=15
SKIP="159,168,171,182"

max_batch() { awk -F, 'NR>1 && $1+0>=0 {print $1}' "$1" 2>/dev/null | sort -n | tail -1; }

# run_config <tau_cv> <hot_kt> <output_csv> <log>
run_config() {
  local tau=$1 hot=$2 CSV=$3 LOG=$4
  rm -f "$CSV"
  while :; do
    cur=$(max_batch "$CSV"); cur=${cur:--1}
    if [ "$cur" -ge "$LAST" ]; then echo ">>> DONE $(basename $CSV) (last $cur) SKIP=$SKIP"; break; fi
    start=$((cur+1))
    echo ">>> run tau=$tau hot=$hot START_BATCH=$start SKIP=$SKIP -> $(basename $CSV)"
    TAU_CV=$tau HOT_KEY_THRESHOLD=$hot START_BATCH=$start SKIP_BATCHES=$SKIP OUTPUT=$CSV \
      cargo test --release --test bench_test test_eth_block_data_all_batches \
      -- --nocapture --exact > "$LOG" 2>&1 &
    pid=$!
    last_max=$cur; last_change=$(date +%s)
    while kill -0 $pid 2>/dev/null; do
      sleep $POLL
      m=$(max_batch "$CSV"); m=${m:-$cur}
      now=$(date +%s)
      if [ "$m" -ne "$last_max" ]; then last_max=$m; last_change=$now; fi
      # use the longer grace until the first new row appears past the resume point
      thr=$STALL; [ "$m" -le "$cur" ] && thr=$STARTUP
      if [ $((now-last_change)) -ge $thr ]; then
        s=$((m+1)); echo ">>> STALL batch $s (>${thr}s) -> kill+skip"
        kill -9 $pid 2>/dev/null; wait $pid 2>/dev/null
        SKIP="$SKIP,$s"; break
      fi
    done
    wait $pid 2>/dev/null
    m=$(max_batch "$CSV"); m=${m:-$cur}
    if [ "$m" -lt "$LAST" ]; then
      c=$((m+1))
      case ",$SKIP," in *",$c,"*) : ;; *) echo ">>> CRASH at batch $c -> skip"; SKIP="$SKIP,$c";; esac
    fi
  done
}

overall=$(date +%s)
if [ "$WHICH" = hot ] || [ "$WHICH" = both ]; then
  echo "######## HOT_KEY_THRESHOLD sweep (tau_cv=0.5) ########"
  for hot in 0.5 1.0 1.5 3.0 5.0; do
    run_config 0.5 "$hot" "$EXP/hot_sweep_t8/batches_hot_${hot}.csv" "/tmp/sweep_hot_${hot}.log"
  done
fi
if [ "$WHICH" = tau ] || [ "$WHICH" = both ]; then
  echo "######## TAU_CV sweep (hot_key_threshold=1.5) ########"
  for tau in 0.1 0.3 0.5 1.0 2.0; do
    run_config "$tau" 1.5 "$EXP/tau_sweep_t8/batches_tau_${tau}.csv" "/tmp/sweep_tau_${tau}.log"
  done
fi
echo "ALL DONE in $(( $(date +%s) - overall ))s | FINAL SKIP=$SKIP"
