#!/usr/bin/env bash
# Emulated per-read state latency (spin-wait of d ns on every account/slot read,
# every engine), 20 Cancun rounds: d = 0..50 us at 8 workers, plus 20/50 us at
# 16 and 32 workers.  ~1 h.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
lat() { # threads suffix delays...
  local t=$1 suf=$2; shift 2
  for d in "$@"; do
    f="$RESULTS/sweeps/statelat_real_d${d}${suf}.csv"; have "$f" 20 && { echo "skip $f"; continue; }
    run test_rebuttal_state_latency_real "" "$f" NUM_THREADS=$t STATE_DELAY_NS=$d START_BLOCK=$CANCUN MAX_BATCHES=20
  done
}
lat 8  ""     0 1000 2000 5000 10000 20000 50000
lat 16 "_t16" 20000 50000
lat 32 "_t32" 20000 50000
echo "STATE LATENCY DONE $(date +%H:%M)"
