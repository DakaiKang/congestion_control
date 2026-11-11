#!/usr/bin/env bash
cd "$(dirname "$0")/../.."
experiments/rebuttal/run_real_v2.sh
experiments/rebuttal/run_sweeps.sh
echo "==> CAMPAIGN DONE"
