"""Thread-count scaling under the condvar-based GraphScheduler.

Full sweep over all 20000 blocks (200 batches × 100 blocks) for
t ∈ {2,4,8,12,16,20,24}. NUM_THREADS unifies BOTH the actual parallel
execution and the dependency-graph / greedy CV simulation. All data is the
current (condvar-fix) code, regenerated fresh into `thread_sweep/`.
"""
from pathlib import Path

import pandas as pd
import matplotlib.pyplot as plt

THREADS = [2, 4, 8, 12, 16, 20, 24]
WORKLOADS = ["real", "v2"]
BASE  = Path("/home/ubuntu/congestion_control/pevm/experiments/thread_sweep")
SUMMARY = Path("/home/ubuntu/congestion_control/pevm/experiments/thread_sweep_summary.csv")
PLOT  = Path("/home/ubuntu/congestion_control/pevm/experiments/plot_thread_sweep.png")

def source(wl: str, t: int) -> Path:
    return BASE / f"{wl}_t{t}.csv"

def aggregate(p: Path) -> dict:
    df = pd.read_csv(p)
    s = df["seq_time_s"].sum()
    return {
        "seq_time_s": s,
        "par_speedup":   s / df["par_time_s"].sum(),
        "graph_speedup": s / df["graph_time_s"].sum(),
        "integ_speedup": s / df["integrated_time_s"].sum(),
    }

rows = []
for wl in WORKLOADS:
    for t in THREADS:
        f = source(wl, t)
        if not f.exists():
            continue
        rows.append({"workload": wl, "threads": t, **aggregate(f)})
summary = pd.DataFrame(rows)
summary.to_csv(SUMMARY, index=False)
print(summary.to_string(index=False))

fig, axes = plt.subplots(1, 2, figsize=(13, 5), sharey=True)
COLORS = {"par": "C1", "graph": "C2", "integ": "C3"}
LABELS = {"par": "Parallel (Block-STM)",
          "graph": "Graph Parallel",
          "integ": "Integrated"}
TITLES = {"real": "Real ETH mainnet blocks",
          "v2":   "V2 synthetic (rw_time)"}

for ax, wl in zip(axes, WORKLOADS):
    sub = summary[summary["workload"] == wl].sort_values("threads")
    for key in ("par", "graph", "integ"):
        ax.plot(sub["threads"], sub[f"{key}_speedup"],
                marker="o", linewidth=1.8,
                color=COLORS[key], label=LABELS[key])
    ax.axhline(1.0, color="black", linewidth=0.7, alpha=0.5)
    ax.set_xlabel("NUM_THREADS")
    ax.set_xticks(THREADS)
    ax.set_title(TITLES[wl])
    ax.grid(alpha=0.3)
    ax.legend(loc="upper left", fontsize=9)

axes[0].set_ylabel("Speedup vs Sequential")
fig.suptitle("Speedup scaling with thread count  "
             "(200 batches × 100 blocks, tau_cv=0.5, hot_kt=1.5, GraphScheduler condvar fix)",
             y=1.02)
fig.tight_layout()
fig.savefig(PLOT, dpi=130, bbox_inches="tight")
print(f"\nwrote {PLOT}")
