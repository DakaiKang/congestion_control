"""Compare busy-spin baseline vs condvar fix on thread-count scaling."""
from pathlib import Path
import pandas as pd
import matplotlib.pyplot as plt

THREADS = [2, 4, 8, 16, 24]
COND_THREADS = [8, 16, 24]
BASE = Path("/home/ubuntu/congestion_control/pevm/thread_sweep")
COND = Path("/home/ubuntu/congestion_control/pevm/thread_sweep_condvar")
OUT = Path("/home/ubuntu/congestion_control/pevm/plot_condvar_compare.png")

def agg(p):
    df = pd.read_csv(p)
    s = df["seq_time_s"].sum()
    return {
        "par":   s / df["par_time_s"].sum(),
        "graph": s / df["graph_time_s"].sum(),
        "integ": s / df["integrated_time_s"].sum(),
    }

fig, axes = plt.subplots(1, 2, figsize=(14, 5.5), sharey=True)
COLORS = {"par": "C1", "graph": "C2", "integ": "C3"}
LABELS = {"par": "Parallel (Block-STM)", "graph": "Graph Parallel", "integ": "Integrated"}
TITLES = {"real": "Real ETH mainnet blocks", "v2": "V2 synthetic (rw_time)"}

for ax, wl in zip(axes, ["real", "v2"]):
    base = {t: agg(BASE / f"{wl}_t{t}.csv") for t in THREADS}
    cond = {t: agg(COND / f"{wl}_t{t}.csv") for t in COND_THREADS}
    for key in ("par", "graph", "integ"):
        # baseline: solid line over all 5 thread counts
        ax.plot(THREADS, [base[t][key] for t in THREADS],
                marker="o", linewidth=1.6, linestyle="-",
                color=COLORS[key], label=f"{LABELS[key]} — busy-spin")
        # condvar: dashed line on 8/16/24
        ax.plot(COND_THREADS, [cond[t][key] for t in COND_THREADS],
                marker="s", linewidth=1.8, linestyle="--",
                color=COLORS[key], alpha=0.9,
                label=f"{LABELS[key]} — condvar fix")
    ax.axhline(1.0, color="black", linewidth=0.6, alpha=0.5)
    ax.set_xlabel("NUM_THREADS")
    ax.set_xticks(THREADS)
    ax.set_title(TITLES[wl])
    ax.grid(alpha=0.3)
    ax.legend(loc="upper left", fontsize=8, ncol=2)

axes[0].set_ylabel("Speedup vs Sequential")
fig.suptitle("GraphScheduler busy-spin vs condvar: speedup scaling  "
             "(100 batches × 100 blocks, tau_cv=0.5, hot_kt=1.5)", y=1.02)
fig.tight_layout()
fig.savefig(OUT, dpi=130, bbox_inches="tight")
print(f"wrote {OUT}")
