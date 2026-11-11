#!/usr/bin/env bash
# Pipelined integration: integrate round r+1 on a spare thread while round r
# executes; consecutive Cancun rounds of 100 and 20 blocks.  ~5 min.
. "$(dirname "$0")/common.sh"
build_timing || exit 1
f="$RESULTS/sweeps/pipeline_real_b100.csv"; [ -s "$f" ] || run test_rebuttal_pipeline_real "" "$f" START_BLOCK=$CANCUN BATCH_SIZE=100 NUM_ROUNDS=10 MERGE_CAP=10
f="$RESULTS/sweeps/pipeline_real_b20.csv";  [ -s "$f" ] || run test_rebuttal_pipeline_real "" "$f" START_BLOCK=$CANCUN BATCH_SIZE=20  NUM_ROUNDS=20 MERGE_CAP=10
echo "PIPELINE DONE $(date +%H:%M)"
