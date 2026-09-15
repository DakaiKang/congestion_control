#!/usr/bin/env bash
# Hot-key threshold sweep (paper Sec. 7.4, Fig. hot-sweep): tau_hot in
# {0.5, 1.0, 1.5 (paper), 3.0, 5.0} with tau_cv fixed at 0.5, full real dataset
# (150 rounds x 100 blocks). The threshold only affects Omakase's integration,
# so only sequential + Omakase are timed (ENGINES=omakase).  ~1 h.
#   VALUES="0.5 1.0 1.5 3.0 5.0"   override the swept values
. "$(dirname "$0")/common.sh"
build_timing || exit 1
for h in ${VALUES:-0.5 1.0 1.5 3.0 5.0}; do
  for start in $CHUNKS; do
    f="$RESULTS/sweeps/hot_real_h${h}_${start}.csv"; have "$f" 50 && { echo "skip $f"; continue; }
    run test_rebuttal_real "" "$f" ENGINES=omakase HOT_KEY_THRESHOLD=$h TAU_CV=0.5 START_BLOCK=$start MAX_BATCHES=50
  done
done
echo "HOT-KEY SWEEP DONE $(date +%H:%M)"
