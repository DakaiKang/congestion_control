#!/usr/bin/env bash
# Generate the rw_time dataset (the synthetic workload's per-transaction timing
# + access sets) incrementally, in parallel with the block download.
#
# test_generate_rw_time is driven by whatever rw_gas_<N>.json files exist and
# honours SKIP_EXISTING=1, so repeated passes pick up blocks as they land
# instead of waiting for the whole 15 000-block download to finish.
#
# Caveat recorded deliberately: `executionTime` is a wall-clock measurement, so
# blocks timed during the download are measured on a slightly busier machine
# than blocks timed after it. The generator already defends against this by
# taking a trimmed mean over ROUNDS passes visited round-first; the box is
# 36-core and the download is network-bound (<1 core), so a single-threaded
# replay is effectively unaffected.
set -uo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

D=/home/ubuntu/Omakase/eth-block-downloader/test_data
export RW_GAS_DIR=$D/rw_gas
export BLOCKS_DIR=$D/blocks_rw
export RW_TIME_DIR=$D/rw_time
export SKIP_EXISTING=1
export ROUNDS=${ROUNDS:-5}
export TRIM=${TRIM:-1}
export PROGRESS_EVERY=${PROGRESS_EVERY:-500}

mkdir -p "$RW_TIME_DIR"
TARGET=${TARGET_BLOCKS:-15000}

for pass in $(seq 1 40); do
  have_gas=$(ls "$RW_GAS_DIR" 2>/dev/null | wc -l)
  have_time=$(ls "$RW_TIME_DIR" 2>/dev/null | wc -l)
  dl_running=$(pgrep -fc 'release/download_rw|finish_download' 2>/dev/null || echo 0)

  echo "==> pass $pass: rw_gas=$have_gas rw_time=$have_time (target $TARGET) download_running=$dl_running"

  if [ "$have_time" -ge "$TARGET" ]; then
    echo "==> rw_time complete ($have_time files)"
    break
  fi

  # Nothing new to do and the download has finished: we are as done as we can be.
  if [ "$have_time" -ge "$have_gas" ] && [ "$dl_running" -eq 0 ]; then
    echo "==> no remaining work and download finished; stopping at $have_time files"
    break
  fi

  # Nothing new yet but the download is still going: wait for more input.
  if [ "$have_time" -ge "$have_gas" ]; then
    sleep 120
    continue
  fi

  cargo test --release --test tx_simulator_test test_generate_rw_time \
      -- --nocapture --exact 2>&1 | tail -5
done

echo "==> FINAL rw_time files: $(ls "$RW_TIME_DIR" 2>/dev/null | wc -l)"
