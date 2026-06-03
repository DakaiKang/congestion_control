"""Combine the 5 per-tau_cv batch CSVs into a single plot.

X-axis: batch_idx (0..99)
Y-axis: integrated_speedup = integrated_tput / seq_tput
One line per tau_cv value.
"""
from pathlib import Path

import numpy as np
import pandas as pd
import matplotlib.pyplot as plt

TAU_VALUES = [0.1, 0.3, 0.5, 1.0, 2.0]
SWEEP_DIR = Path("/home/ubuntu/congestion_control/pevm/experiments/tau_sweep")
OUT = Path("/home/ubuntu/congestion_control/pevm/experiments/plot_tau_sweep.png")

cmap = plt.get_cmap("viridis")
colors = [cmap(i / (len(TAU_VALUES) - 1)) for i in range(len(TAU_VALUES))]

fig, ax = plt.subplots(figsize=(11, 5.5))

for tau, color in zip(TAU_VALUES, colors):
    csv = SWEEP_DIR / f"batches_tau_{tau}.csv"
    if not csv.exists():
        print(f"  missing: {csv}")
        continue
    df = pd.read_csv(csv)
    speedup = df["integrated_tput"] / df["seq_tput"]
    ax.plot(df["batch_idx"], speedup,
            label=f"tau_cv = {tau}",
            color=color, linewidth=1.4)
    print(f"tau={tau}: mean speedup = {speedup.mean():.2f}x  median = {speedup.median():.2f}x  "
          f"(min {speedup.min():.2f}, max {speedup.max():.2f})")

ax.axhline(1.0, color="black", linewidth=0.7, alpha=0.5)
ax.set_xlabel("Batch index (100 blocks each)")
ax.set_ylabel("Integrated speedup (vs Sequential)")
ax.set_title("Integrated speedup across tau_cv sweep  "
             "(real ETH blocks, NUM_THREADS=16, hot_key_threshold=1.5)")
ax.grid(alpha=0.3)
ax.legend(loc="upper right", fontsize=9)

all_vals = np.concatenate([
    (pd.read_csv(SWEEP_DIR / f"batches_tau_{t}.csv")["integrated_tput"] /
     pd.read_csv(SWEEP_DIR / f"batches_tau_{t}.csv")["seq_tput"]).values
    for t in TAU_VALUES
    if (SWEEP_DIR / f"batches_tau_{t}.csv").exists()
])
lo = np.percentile(all_vals, 1)
hi = np.percentile(all_vals, 99)
ax.set_ylim(max(0, lo - 0.1), hi * 1.05)

fig.tight_layout()
fig.savefig(OUT, dpi=130)
print(f"\nWrote {OUT}")
