#!/usr/bin/env bash
# Shared setup for the experiment scripts. Source this file; do not run it.
#
#   RESULTS   output root (default experiments/results, relative to pevm/);
#             analyze.py reads the same layout: <root>/{*.csv,artificial,sweeps,vegeta,live}
#   DATASET   root of the downloaded dataset (blocks_rw/, rw_time/), see README
#
# Every harness knob is an environment variable read by pevm/crates/pevm/tests/rebuttal_test.rs.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO/pevm"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

export DATASET=${DATASET:-/home/ubuntu/Omakase/eth-block-downloader/test_data}
export BLOCKS_DIR=${BLOCKS_DIR:-$DATASET/blocks_rw}
export RW_TIME_DIR=${RW_TIME_DIR:-$DATASET/rw_time}
export RESULTS=${RESULTS:-experiments/results}
mkdir -p "$RESULTS/artificial" "$RESULTS/sweeps" "$RESULTS/vegeta/artificial" "$RESULTS/live"

# Paper defaults.
export NUM_THREADS=${NUM_THREADS:-8} TAU_CV=${TAU_CV:-0.5} HOT_KEY_THRESHOLD=${HOT_KEY_THRESHOLD:-1.5}
export BATCH_SIZE=${BATCH_SIZE:-100} CHECK_STATE=${CHECK_STATE:-1}
# Real-Ethereum dataset: three 5,000-block segments (Merge, Shanghai, Cancun).
CHUNKS=${CHUNKS:-"16774645 18581726 19557289"}
CANCUN=19557289

TEST="cargo test --release --test rebuttal_test"
build_timing() { cargo build --release --test rebuttal_test 2>&1 | grep -E "^error" -A 8; [ "${PIPESTATUS[0]}" -eq 0 ] || { echo "build failed"; return 1; }; return 0; }
build_diag()   { cargo build --release --features diagnostics --test rebuttal_test 2>&1 | grep -E "^error" -A 8; [ "${PIPESTATUS[0]}" -eq 0 ] || { echo "build failed"; return 1; }; return 0; }
# have FILE MINLINES -> true if FILE exists with more than MINLINES lines (resume support)
have() { [ -s "$1" ] && [ "$(wc -l < "$1")" -gt "${2:-1}" ]; }
# run TESTNAME FEATURES OUTPUT ENV... : one harness invocation, terse output
run() {
  local test=$1 feat=$2 out=$3; shift 3
  echo "==> $out  $(date +%H:%M)"
  env "$@" OUTPUT="$out" $TEST $feat "$test" -- --nocapture --exact 2>&1 \
    | grep -aE "✓|panicked|assertion|failed \(|state !=" | tail -3
  return 0
}
