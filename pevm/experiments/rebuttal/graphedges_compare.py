#!/usr/bin/env python3
"""GRAPH_EDGES=waw (submission code path) vs raw (paper Algorithm 1) vs raw+waw.
Baseline timing = first NB rounds of real_*.csv; diagnostics from graphedges/."""
import glob, os, sys
import pandas as pd
D = os.path.dirname(os.path.abspath(__file__)); NB = int(sys.argv[1]) if len(sys.argv) > 1 else 20
chunks = ["16774645", "18581726", "19557289"]
def load(files, nb):
    fs = [pd.read_csv(f).head(nb) for f in files if os.path.exists(f) and os.path.getsize(f) > 0]
    return pd.concat(fs, ignore_index=True) if fs else None
sets = {"waw (submission)": (load([f"{D}/real_{c}.csv" for c in chunks], NB), load([f"{D}/graphedges/waw_diag_{c}.csv" for c in chunks], NB))}
for mode in ("raw", "raw+waw", "rawB", "raw+wawB"):
    sets[mode] = (load([f"{D}/graphedges/{mode}_timing_{c}.csv" for c in chunks], NB), load([f"{D}/graphedges/{mode}_diag_{c}.csv" for c in chunks], NB))
eng = [("par", "par", "Block-STM"), ("concat", "concat", "Concat"), ("cgraph", "cgraph", "Concat+graph"), ("graph", "graph", "Graph OCC"), ("integrated", "integ", "Omakase")]
print(f"{'GRAPH_EDGES':18s}{'engine':14s}{'rnds':>5}{'speedup':>9}{'p99 ms/blk':>11}{'re-exec':>9}{'val':>7}{'cascade':>9}{'blocking':>10}{'x-blk val':>11}{'x-blk casc':>12}{'groups':>8}")
for mode, (t, d) in sets.items():
    for kt, kd, name in eng:
        if t is not None:
            sp = f"{t.seq_time_s.sum() / t[kt + '_time_s'].sum():.2f}x"; p99 = f"{(t[kt + '_time_s'] / t.num_blocks * 1000).quantile(0.99):.2f}"
        else:
            sp = p99 = "-"
        g = f"{t.num_integrated_groups.mean():.1f}" if (t is not None and kt == "integrated") else ""
        if d is not None and f"{kd}_cascade_aborts" in d:
            n = d.num_txs.sum(); r, v, c = d[f"{kd}_re_exec"].sum() / n, d[f"{kd}_validation_aborts"].sum() / n, d[f"{kd}_cascade_aborts"].sum() / n
            xb = (f"{d[f'{kd}_cross_block_aborts'].sum() / n:>11.3f}{d[f'{kd}_cross_block_cascade'].sum() / n:>12.3f}"
                  if f"{kd}_cross_block_aborts" in d else f"{'-':>11}{'-':>12}")
            print(f"{mode:18s}{name:14s}{len(d):>5}{sp:>9}{p99:>11}{r:>9.3f}{v:>7.3f}{c:>9.3f}{r - v:>10.3f}{xb}{g:>8}")
        else:
            print(f"{mode:18s}{name:14s}{len(t) if t is not None else 0:>5}{sp:>9}{p99:>11}{'-':>9}{'-':>7}{'-':>9}{'-':>10}{'-':>11}{'-':>12}{g:>8}")
    print()
