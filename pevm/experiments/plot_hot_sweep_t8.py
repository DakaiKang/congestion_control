"""hot_cv/threshold sweep at NUM_THREADS=8 over 200 batches (real ETH).
Integrated speedup = integrated_tput / seq_tput, one line per value.
Skipped chunk-2 batches (empty rows) become NaN and are dropped by the plot.
"""
from pathlib import Path
import numpy as np, pandas as pd, matplotlib.pyplot as plt

VALUES = [0.5, 1.0, 1.5, 3.0, 5.0]
SWEEP_DIR = Path("/home/ubuntu/congestion_control/pevm/experiments/hot_sweep_t8")
OUT = Path("/home/ubuntu/congestion_control/pevm/experiments/plot_hot_sweep_t8.png")
cmap = plt.get_cmap("viridis")
colors = [cmap(i/(len(VALUES)-1)) for i in range(len(VALUES))]

fig, ax = plt.subplots(figsize=(11, 5.5))
print("value  mean   median  min    max")
for v, color in zip(VALUES, colors):
    csv = SWEEP_DIR / f"batches_hot_{v}.csv"
    if not csv.exists(): print(f"  missing {csv}"); continue
    df = pd.read_csv(csv)
    sp = df["integrated_tput"] / df["seq_tput"]
    ax.plot(df["batch_idx"], sp, label=f"hot_key_threshold = {v}", color=color, linewidth=1.3)
    print(f"{v:>5}  {sp.mean():.3f}  {sp.median():.3f}  {sp.min():.2f}  {sp.max():.2f}")

ax.axhline(1.0, color="black", linewidth=0.7, alpha=0.5)
ax.set_xlabel("Batch index (100 blocks each)")
ax.set_ylabel("Integrated speedup (vs Sequential)")
ax.set_title("Integrated speedup across hot_key_threshold sweep  (real ETH, 200 batches, NUM_THREADS=8, tau_cv=0.5)")
ax.grid(alpha=0.3); ax.legend(loc="upper right", fontsize=9)
allv = np.concatenate([(pd.read_csv(SWEEP_DIR/f"batches_hot_{v}.csv")["integrated_tput"]/
        pd.read_csv(SWEEP_DIR/f"batches_hot_{v}.csv")["seq_tput"]).dropna().values
        for v in VALUES if (SWEEP_DIR/f"batches_hot_{v}.csv").exists()])
ax.set_ylim(max(0, np.percentile(allv,1)-0.1), np.percentile(allv,99)*1.05)
fig.tight_layout(); fig.savefig(OUT, dpi=130)
print(f"wrote {OUT}")
