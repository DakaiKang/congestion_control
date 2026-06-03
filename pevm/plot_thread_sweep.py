"""Compute aggregate speedups from per-thread CSVs and plot scaling.

Inputs:  pevm/thread_sweep/{real,v2}_t{N}.csv  for N in {2,4,8,16,24}
Outputs: pevm/plot_thread_sweep.png
         pevm/thread_sweep_summary.csv (Real + V2, per-thread speedups)
"""
from pathlib import Path

import pandas as pd
import matplotlib.pyplot as plt

THREADS = [2, 4, 8, 16, 24]
WORKLOADS = ["real", "v2"]
SWEEP = Path("/home/ubuntu/congestion_control/pevm/thread_sweep")
SUMMARY = Path("/home/ubuntu/congestion_control/pevm/thread_sweep_summary.csv")
PLOT = Path("/home/ubuntu/congestion_control/pevm/plot_thread_sweep.png")


def aggregate(csv_path: Path) -> dict:
    """Return aggregate speedups vs sequential, computed as totals."""
    df = pd.read_csv(csv_path)
    st = df["seq_time_s"].sum()
    pt = df["par_time_s"].sum()
    gt = df["graph_time_s"].sum()
    it = df["integrated_time_s"].sum()
    return {
        "seq_time_s": st,
        "par_time_s": pt,
        "graph_time_s": gt,
        "integrated_time_s": it,
        "par_speedup":   st / pt if pt > 0 else float("nan"),
        "graph_speedup": st / gt if gt > 0 else float("nan"),
        "integ_speedup": st / it if it > 0 else float("nan"),
    }


# Build summary
rows = []
for wl in WORKLOADS:
    for t in THREADS:
        f = SWEEP / f"{wl}_t{t}.csv"
        if not f.exists():
            print(f"  missing: {f}")
            continue
        a = aggregate(f)
        rows.append({"workload": wl, "threads": t, **a})

summary = pd.DataFrame(rows)
summary.to_csv(SUMMARY, index=False)
print(f"summary → {SUMMARY}\n")
print(summary.to_string(index=False))

# Plot: 2 subplots (real | v2), each with 3 lines (par, graph, integ)
fig, axes = plt.subplots(1, 2, figsize=(13, 5), sharey=True)
COLORS = {"par": "C1", "graph": "C2", "integ": "C3"}
LABELS = {"par": "Parallel (Block-STM)",
          "graph": "Graph Parallel",
          "integ": "Integrated"}
TITLES = {"real": "Real ETH mainnet blocks",
          "v2":   "V2 synthetic (rw_time)"}

for ax, wl in zip(axes, WORKLOADS):
    sub = summary[summary["workload"] == wl].sort_values("threads")
    if sub.empty:
        ax.set_title(f"{TITLES[wl]} (no data)")
        continue
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
             "(100 batches × 100 blocks each, 1 515 015 txs, tau_cv=0.5, hot_kt=1.5)",
             y=1.02)
fig.tight_layout()
fig.savefig(PLOT, dpi=130, bbox_inches="tight")
print(f"\nplot → {PLOT}")
