#!/usr/bin/env python3
"""Compare GRAPH_RAW_EDGES=none (baseline: first NB rounds of real_*.csv and
real_*_diag.csv) with =all and =cross (rawedges/*.csv): speedup, re-executions,
validation aborts, cascade aborts per tx for the graph-driven engines."""
import glob, os, re, sys
import pandas as pd
D = os.path.dirname(os.path.abspath(__file__)); NB = int(sys.argv[1]) if len(sys.argv) > 1 else 20
chunks = ["16774645", "18581726", "19557289"]
def load(files, nb):
    fs = [pd.read_csv(f).head(nb) for f in files if os.path.exists(f) and os.path.getsize(f) > 0]
    return pd.concat(fs, ignore_index=True) if fs else None
sets = {"none (baseline)": (load([f"{D}/real_{c}.csv" for c in chunks], NB), load([f"{D}/real_{c}_diag.csv" for c in chunks], NB))}
for mode in ("all", "cross"):
    sets[mode] = (load([f"{D}/rawedges/{mode}_timing_{c}.csv" for c in chunks], NB), load([f"{D}/rawedges/{mode}_diag_{c}.csv" for c in chunks], NB))
eng = [("par", "par", "Block-STM"), ("concat", "concat", "Concat"), ("cgraph", "cgraph", "Concat+graph"), ("graph", "graph", "Graph OCC"), ("integrated", "integ", "Omakase")]
print(f"{'RAW edges':16s}{'engine':14s}{'rounds':>7}{'speedup':>9}{'re-exec/tx':>11}{'val/tx':>8}{'cascade/tx':>11}{'blocking/tx':>12}")
for mode, (t, d) in sets.items():
    for kt, kd, name in eng:
        sp = f"{t.seq_time_s.sum() / t[kt + '_time_s'].sum():.2f}x" if t is not None else "-"
        if d is not None and f"{kd}_cascade_aborts" in d:
            n = d.num_txs.sum(); r, v, c = d[f"{kd}_re_exec"].sum() / n, d[f"{kd}_validation_aborts"].sum() / n, d[f"{kd}_cascade_aborts"].sum() / n
            print(f"{mode:16s}{name:14s}{len(d):>7}{sp:>9}{r:>11.3f}{v:>8.3f}{c:>11.3f}{r - v:>12.3f}")
        else:
            print(f"{mode:16s}{name:14s}{len(t) if t is not None else 0:>7}{sp:>9}{'-':>11}{'-':>8}{'-':>11}{'-':>12}")
    print()
