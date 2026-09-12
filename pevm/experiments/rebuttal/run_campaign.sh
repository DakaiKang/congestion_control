#!/usr/bin/env bash
# Serial queue so no two timing runs share the machine:
#   1. artificial K x M grid (no dataset needed)
#   2. real x3 chunks + V2, timing and diagnostics (waits for the dataset)
#   3. thread / round-size / merge-cap sweeps
cd "$(dirname "$0")/../.."
experiments/rebuttal/run_artificial.sh
experiments/rebuttal/run_real_v2.sh
experiments/rebuttal/run_sweeps.sh
echo "==> CAMPAIGN DONE"
