#!/usr/bin/env python3
"""Per-phase executor cost of the live Omakase (integrated) mode.

Reads the ROUND log lines written by mysticeti-core (prep_ms / integ_ms /
exec_only_ms fields, steady-state window WARM..DUR seconds) and prints the
executor cost per committed transaction, split into the proposer-side stages
(pre-execute + graph build + reorder), integration and execution. The
prototype re-derives the graph at every validator for every block of the
round (it does not ship graphs with proposals), so the proposer-side share is
over-counted P times; the last column amortises it by 1/P for a P-validator
committee (default P = 4).
"""
import glob, re, sys
d = sys.argv[1] if len(sys.argv) > 1 else "experiments/rebuttal/live/phases_minround1000"
P = int(sys.argv[2]) if len(sys.argv) > 2 else 4
warm, dur = 20.0, 90.0
pat = re.compile(r"ROUND blocks=(\d+) txs=(\d+) exec_ms=([\d.]+) total_txs=\d+ elapsed_s=([\d.]+) "
                 r"Throughput = [\d.]+ prep_ms=([\d.]+) integ_ms=([\d.]+) exec_only_ms=([\d.]+)")
print(f"{'validator':<20}{'rounds':>7}{'blk/rnd':>8}{'tx/s':>8}{'total':>8}{'prep':>7}{'integ':>7}{'exec':>7}{'P='+str(P):>8}  (us/tx)")
agg = [0.0] * 5
for f in sorted(glob.glob(f"{d}/integrated_v*.log")):
    st = [tuple(map(float, m.groups())) for m in pat.finditer(open(f, errors="ignore").read())]
    st = [r for r in st if warm <= r[3] <= dur]
    if not st:
        continue
    blocks, txs = sum(r[0] for r in st), sum(r[1] for r in st)
    tot, prep, integ, ex = (sum(r[i] for r in st) for i in (2, 4, 5, 6))
    amort = tot - prep * (1 - 1 / P)
    print(f"{f.split('/')[-1]:<20}{len(st):>7}{blocks/len(st):>8.1f}{txs/(dur-warm):>8.0f}"
          f"{tot*1000/txs:>8.2f}{prep*1000/txs:>7.2f}{integ*1000/txs:>7.2f}{ex*1000/txs:>7.2f}{amort*1000/txs:>8.2f}")
    for i, v in enumerate((txs, tot, prep, integ, ex)):
        agg[i] += v
if agg[0]:
    txs, tot, prep, integ, ex = agg
    print(f"{'mean':<20}{'':>7}{'':>8}{'':>8}{tot*1000/txs:>8.2f}{prep*1000/txs:>7.2f}{integ*1000/txs:>7.2f}{ex*1000/txs:>7.2f}"
          f"{(tot - prep*(1-1/P))*1000/txs:>8.2f}")
