#!/usr/bin/env bash
# Live multi-proposer end-to-end run: a 4-validator Mysticeti committee on one
# machine, each validator executing every committed round with pevm in one of
# four modes. Throughput = committed transactions executed per second, parsed
# from the executor's ROUND log lines over the steady-state window.
#
#   PEVM_EXECUTION_MODE  sequential | parallel (Block-STM) | concatenated | integrated (Omakase)
#   PEVM_WORKLOAD        ERC20 shape "clusters,families,people" (snapshot must exist; see
#                        tests/erc20_snapshot.rs)
#   PEVM_THREADS         execution workers per validator (8, as in the paper)
set -uo pipefail
MYST=/home/ubuntu/Omakase/congestion_control/mysticeti
OUT=/home/ubuntu/Omakase/congestion_control/pevm/experiments/rebuttal/live
DUR=${DUR:-90}          # seconds per run
WARM=${WARM:-20}        # seconds discarded at the start
COMMITTEE=${COMMITTEE:-4}
export PEVM_WORKLOAD=${PEVM_WORKLOAD:-8,4,8} PEVM_THREADS=${PEVM_THREADS:-8} RUST_LOG=error
mkdir -p "$OUT"; cd "$MYST"

for mode in ${MODES:-sequential parallel concatenated integrated}; do
  echo "==> mode=$mode workload=$PEVM_WORKLOAD threads=$PEVM_THREADS committee=$COMMITTEE dur=${DUR}s"
  rm -rf dryrun-validator-* 2>/dev/null
  pids=()
  for a in $(seq 0 $((COMMITTEE-1))); do
    PEVM_EXECUTION_MODE=$mode timeout $DUR ./target/release/mysticeti dry-run --committee-size $COMMITTEE --authority $a \
        > "$OUT/${mode}_v$a.log" 2>&1 &
    pids+=($!)
  done
  wait "${pids[@]}" 2>/dev/null
  # Steady-state throughput per validator: txs executed between WARM and DUR seconds.
  python3 - "$OUT" "$mode" "$WARM" "$DUR" <<'PY'
import re, sys, glob, statistics
out, mode, warm, dur = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4])
rows = []
for f in sorted(glob.glob(f"{out}/{mode}_v*.log")):
    pts = [(float(m.group(3)), int(m.group(1)), int(m.group(2)), float(m.group(4)))
           for m in re.finditer(r"ROUND blocks=(\d+) txs=(\d+) exec_ms=([\d.]+) total_txs=\d+ elapsed_s=([\d.]+)", open(f, errors="ignore").read())]
    steady = [p for p in pts if p[3] >= warm]
    if not steady:
        rows.append((f, 0, 0, 0.0, 0.0, 0.0)); continue
    txs = sum(p[2] for p in steady); blocks = sum(p[1] for p in steady)
    span = steady[-1][3] - warm
    exec_share = sum(p[0] for p in steady) / 1000.0 / max(span, 1e-9)
    rows.append((f, len(steady), blocks, txs / max(span, 1e-9), exec_share, blocks / len(steady)))
with open(f"{out}/summary.csv", "a") as w:
    for f, n, b, tps, share, bpr in rows:
        w.write(f"{mode},{f.split('/')[-1]},{n},{b},{tps:.1f},{share:.3f},{bpr:.2f}\n")
tps = [r[3] for r in rows if r[1] > 0]
print(f"    {mode}: validators={len(tps)} steady tx/s per validator mean={statistics.mean(tps) if tps else 0:.0f} "
      f"min={min(tps) if tps else 0:.0f} max={max(tps) if tps else 0:.0f} | rounds/validator={rows[0][1]} blocks/round={rows[0][5]:.1f} | exec busy={rows[0][4]*100:.0f}%")
PY
done
echo "==> LIVE DONE  ($OUT/summary.csv: mode,log,rounds,blocks,tx_per_s,exec_busy_share,blocks_per_round)"
