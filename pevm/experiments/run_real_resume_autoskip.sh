#!/usr/bin/env bash
# Resume the REAL sweep (batches 150..199) for every thread count, appending to
# the existing thread_sweep/real_t<t>.csv. Handles two failure modes that the
# in-code error-skip cannot:
#   - STALL: a tx loops to its gas limit; process alive but no new CSV row.
#   - CRASH: an unwrap panic (panic=abort) exits the process early.
# In both cases the offending batch = (last written batch)+1: add it to
# SKIP_BATCHES and resume. Resumes each thread from where its CSV left off, so
# re-runs never duplicate rows. The skip set accumulates across thread counts.
set -u

WS=/home/ubuntu/congestion_control/pevm
DATA=/home/ubuntu/eth-block-downloader/test_data
OUT=$WS/experiments/thread_sweep
cd "$WS" || exit 1

THREADS=(2 4 8 12 16 20 24)
LAST=199          # final batch index (20000 blocks / 100)
export BLOCKS_DIR=$DATA/blocks_rw
export TAU_CV=0.5
export HOT_KEY_THRESHOLD=1.5
STALL=60          # seconds of no new CSV row => current batch is hung
POLL=15

SKIP="159,168,171"   # known hangs so far; grows as more are found
overall=$(date +%s)

max_batch() { awk -F, 'NR>1 && $1+0>=150 {print $1}' "$1" 2>/dev/null | sort -n | tail -1; }

for t in "${THREADS[@]}"; do
  CSV=$OUT/real_t${t}.csv
  echo "############ NUM_THREADS=$t ############"
  while :; do
    cur=$(max_batch "$CSV"); cur=${cur:-149}
    if [ "$cur" -ge "$LAST" ]; then
      echo ">>> t=$t COMPLETE (last batch $cur) | SKIP=$SKIP"; break
    fi
    start=$((cur+1))
    echo ">>> run t=$t START_BATCH=$start SKIP_BATCHES=$SKIP"
    NUM_THREADS=$t START_BATCH=$start SKIP_BATCHES=$SKIP OUTPUT=$CSV \
      cargo test --release --test bench_test test_eth_block_data_all_batches \
      -- --nocapture --exact > /tmp/autoskip_t${t}.log 2>&1 &
    pid=$!
    last_max=$cur; last_change=$(date +%s)
    while kill -0 $pid 2>/dev/null; do
      sleep $POLL
      m=$(max_batch "$CSV"); m=${m:-$cur}
      now=$(date +%s)
      if [ "$m" -ne "$last_max" ]; then last_max=$m; last_change=$now; fi
      if [ $((now-last_change)) -ge $STALL ]; then
        stalled=$((m+1))
        echo ">>> STALL: batch $stalled no progress >${STALL}s -> kill + skip"
        kill -9 $pid 2>/dev/null; wait $pid 2>/dev/null
        SKIP="$SKIP,$stalled"
        break
      fi
    done
    wait $pid 2>/dev/null
    # If the process exited on its own before finishing, it crashed (panic);
    # the next unwritten batch is the culprit -> skip it and resume.
    m=$(max_batch "$CSV"); m=${m:-$cur}
    if kill -0 $pid 2>/dev/null; then :; fi
    if [ "$m" -lt "$LAST" ]; then
      crashed=$((m+1))
      case ",$SKIP," in *",$crashed,"*) : ;; *) echo ">>> CRASH/early-exit at batch $crashed -> skip"; SKIP="$SKIP,$crashed";; esac
    fi
  done
  echo ">>> t=$t rows: $(wc -l < $CSV) | succeeded: $(awk -F, 'NR>1 && $6!=""' $CSV|wc -l)"
done

echo "ALL DONE in $(( $(date +%s) - overall ))s | FINAL SKIP_BATCHES=$SKIP"
