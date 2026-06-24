"""Plots for the fully-artificial workload sweep (see EXPERIMENTS.md).

Reads:
  artificial_sweep/k<K>_m<M>.csv      (HOT_SET_SIZE=10, full: par/graph/integ)
  artificial_sweep_h5/k<K>_m<M>.csv   (HOT_SET_SIZE=5; M=30 full, others integ-only)

Writes:
  plot_artificial_perK.png    3-strategy speedup vs M, one panel per K (hot_set=10)
  plot_artificial_h5.png      6 lines: par/graph (K-independent) + integrated per K (hot_set=5)

Parallel/Graph speedup depends only on M (blocks run independently), so it is
K- and hot_set-independent; the integ-only h5 runs therefore reuse the hot_set=10
par/graph values for those curves.
"""
from pathlib import Path
import pandas as pd
import matplotlib.pyplot as plt

EXP = Path(__file__).parent
KS = [1, 2, 3, 4]
MS = [10, 20, 30, 40]

def speedups(d):
    return ((d.par_tput / d.seq_tput).mean(),
            (d.graph_tput / d.seq_tput).mean(),
            (d.integrated_tput / d.seq_tput).mean())

def load(dirname, k, m):
    return pd.read_csv(EXP / dirname / f"k{k}_m{m}.csv")

# ── Plot 1: hot_set=10, 3 strategies vs M, one panel per K ───────────────────
fig, axes = plt.subplots(2, 2, figsize=(12, 9), sharex=True, sharey=True)
for ax, k in zip(axes.flat, KS):
    par, graph, integ = [], [], []
    for m in MS:
        p, g, i = speedups(load("artificial_sweep", k, m))
        par.append(p); graph.append(g); integ.append(i)
    ax.plot(MS, par, 'o-', color='C1', label='Parallel (Block-STM)')
    ax.plot(MS, graph, 's-', color='C2', label='Graph Parallel')
    ax.plot(MS, integ, '^-', color='C3', label='Integrated')
    ax.axhline(1, color='k', lw=.6, alpha=.5)
    ax.set_title(f'K = {k}  (hot-group size {9 + k})')
    ax.set_xticks(MS); ax.grid(alpha=.3); ax.legend()
    ax.set_xlabel('M (% txs touching hot key)'); ax.set_ylabel('Speedup vs Sequential')
fig.suptitle('Artificial workload (HOT_SET_SIZE=10): strategies vs intra-block M, per K '
             '(100×50×100, t=8, $\\tau_{cv}$=0.5, $h_{kt}$=1.5)', y=1.01)
fig.tight_layout(); fig.savefig(EXP / "plot_artificial_perK.png", dpi=130, bbox_inches='tight')
print("wrote plot_artificial_perK.png")

# ── Plot 2: hot_set=5, 6 lines (par/graph + integrated per K) vs M ────────────
# par/graph are K-independent -> average the hot_set=10 full runs per M.
par_m, graph_m = [], []
for m in MS:
    ps = [speedups(load("artificial_sweep", k, m))[0] for k in KS]
    gs = [speedups(load("artificial_sweep", k, m))[1] for k in KS]
    par_m.append(sum(ps) / len(ps)); graph_m.append(sum(gs) / len(gs))

fig, ax = plt.subplots(figsize=(8.5, 5.5))
ax.plot(MS, par_m, 'o--', color='C1', lw=2, label='Parallel (Block-STM)')
ax.plot(MS, graph_m, 's--', color='C2', lw=2, label='Graph Parallel')
cmap = plt.get_cmap('viridis')
for i, k in enumerate(KS):
    integ = [speedups(load("artificial_sweep_h5", k, m))[2] for m in MS]
    ax.plot(MS, integ, '^-', color=cmap(i / 3), label=f'Integrated K={k}')
ax.set_xlabel('M (% txs touching hot key)'); ax.set_ylabel('Speedup vs Sequential')
ax.set_xticks(MS); ax.grid(alpha=.3); ax.legend(fontsize=9)
ax.set_title('Artificial workload, HOT_SET_SIZE=5  (100×50×100, t=8, $\\tau_{cv}$=0.5, $h_{kt}$=1.5)\n'
             'Parallel/Graph are K-independent; Integrated shown per K')
fig.tight_layout(); fig.savefig(EXP / "plot_artificial_h5.png", dpi=130, bbox_inches='tight')
print("wrote plot_artificial_h5.png")
