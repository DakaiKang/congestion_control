#!/usr/bin/env bash
# CV threshold sweep (paper Sec. 7.4, Fig. tau-sweep): tau_cv in
# {0.1, 0.3, 0.5 (paper), 1.0, 2.0} with the hot-key threshold fixed at 1.5,
# full real dataset (150 rounds x 100 blocks). Only sequential + Omakase are
# timed (ENGINES=omakase); the threshold affects no other engine.  ~1 h.
#   VALUES="0.1 0.3 0.5 1.0 2.0"   override the swept values
. "$(dirname "$0")/common.sh"
build_timing || exit 1
for t in ${VALUES:-0.1 0.3 0.5 1.0 2.0}; do
  for start in $CHUNKS; do
    f="$RESULTS/sweeps/tau_real_t${t}_${start}.csv"; have "$f" 50 && { echo "skip $f"; continue; }
    run test_rebuttal_real "" "$f" ENGINES=omakase TAU_CV=$t HOT_KEY_THRESHOLD=1.5 START_BLOCK=$start MAX_BATCHES=50
  done
done
echo "TAU_CV SWEEP DONE $(date +%H:%M)"
