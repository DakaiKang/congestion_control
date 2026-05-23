"""Plot per-batch metrics from a batches CSV.

Usage:
  python3 plot_batches.py [CSV] [PREFIX] [TITLE_TAG]

Defaults:
  CSV       = eth_block_data_batches.csv
  PREFIX    = plot          (outputs plot_tput.png, plot_speedup.png, plot_integ_vs_seq.png)
  TITLE_TAG = "ETH mainnet blocks"
"""
import sys
from pathlib import Path

import numpy as np
import pandas as pd
import matplotlib.pyplot as plt


def robust_ylim(series_list, lo_pct=1, hi_pct=99, pad=0.05, floor=None):
    """Compute a (lo, hi) y-range that ignores the tails."""
    vals = np.concatenate([np.asarray(s) for s in series_list])
    lo = np.percentile(vals, lo_pct)
    hi = np.percentile(vals, hi_pct)
    span = hi - lo
    lo -= span * pad
    hi += span * pad
    if floor is not None:
        lo = max(lo, floor)
    return lo, hi

CSV = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).with_name("eth_block_data_batches.csv")
PREFIX = sys.argv[2] if len(sys.argv) > 2 else "plot"
TITLE_TAG = sys.argv[3] if len(sys.argv) > 3 else "ETH mainnet blocks"
OUTDIR = Path(__file__).parent

df = pd.read_csv(CSV)
df["par_speedup"] = df["par_tput"] / df["seq_tput"]
df["graph_speedup"] = df["graph_tput"] / df["seq_tput"]
df["integ_speedup"] = df["integrated_tput"] / df["seq_tput"]

# Segment boundary: when the block-number jumps (sparse dataset).
gap_idx = None
for i in range(1, len(df)):
    if df.loc[i, "start_block"] - df.loc[i - 1, "end_block"] > 1:
        gap_idx = i
        break

def mark_gap(ax):
    if gap_idx is not None:
        ax.axvline(gap_idx - 0.5, color="gray", linestyle=":", linewidth=1, alpha=0.7,
                   label=f"block-range gap (after batch {gap_idx - 1})")

# Shared colors so the same strategy is the same color across plots.
COLOR = {
    "seq":   "C0",
    "par":   "C1",
    "graph": "C2",
    "integ": "C3",
}

# ─── 1. Per-batch throughput ────────────────────────────────────────────────
fig, ax = plt.subplots(figsize=(11, 5))
ax.plot(df["batch_idx"], df["seq_tput"],         label="Sequential",          color=COLOR["seq"],   linewidth=1.2)
ax.plot(df["batch_idx"], df["par_tput"],         label="Parallel (Block-STM)", color=COLOR["par"],   linewidth=1.2)
ax.plot(df["batch_idx"], df["graph_tput"],       label="Graph Parallel",      color=COLOR["graph"], linewidth=1.2)
ax.plot(df["batch_idx"], df["integrated_tput"],  label="Integrated",          color=COLOR["integ"], linewidth=1.6)
mark_gap(ax)
ax.set_xlabel("Batch index (100 blocks each)")
ax.set_ylabel("Throughput  (tx / s)")
ax.set_title(f"Throughput per batch — 100 batches × 100 {TITLE_TAG}, NUM_THREADS=16")
ax.grid(alpha=0.3)
ax.legend(loc="upper right", fontsize=9)
ax.set_ylim(*robust_ylim(
    [df["seq_tput"], df["par_tput"], df["graph_tput"], df["integrated_tput"]],
    lo_pct=1, hi_pct=99, floor=0,
))
fig.tight_layout()
fig.savefig(OUTDIR / f"{PREFIX}_tput.png", dpi=130)
plt.close(fig)

# ─── 2. Per-batch speedup vs Sequential ─────────────────────────────────────
fig, ax = plt.subplots(figsize=(11, 5))
ax.axhline(1.0, color="black", linewidth=0.8, alpha=0.7)
ax.plot(df["batch_idx"], df["par_speedup"],    label="Parallel / Seq",    color=COLOR["par"],   linewidth=1.2)
ax.plot(df["batch_idx"], df["graph_speedup"],  label="Graph / Seq",       color=COLOR["graph"], linewidth=1.2)
ax.plot(df["batch_idx"], df["integ_speedup"],  label="Integrated / Seq",  color=COLOR["integ"], linewidth=1.6)
mark_gap(ax)
ax.set_xlabel("Batch index")
ax.set_ylabel("Speedup vs Sequential")
ax.set_title(f"Per-batch speedup over Sequential — {TITLE_TAG}")
ax.grid(alpha=0.3)
ax.legend(loc="upper right", fontsize=9)
ax.set_ylim(*robust_ylim(
    [df["par_speedup"], df["graph_speedup"], df["integ_speedup"]],
    lo_pct=1, hi_pct=99,
))
fig.tight_layout()
fig.savefig(OUTDIR / f"{PREFIX}_speedup.png", dpi=130)
plt.close(fig)

# ─── 3. Integrated vs Sequential scatter ────────────────────────────────────
fig, ax = plt.subplots(figsize=(7.5, 7))
seg1 = df[df.index < (gap_idx if gap_idx else len(df))]
seg2 = df[df.index >= (gap_idx if gap_idx else len(df))]
ax.scatter(seg1["seq_tput"], seg1["integrated_tput"], s=18, alpha=0.75,
           label=f"segment 1 (n={len(seg1)})", color="C0")
if len(seg2):
    ax.scatter(seg2["seq_tput"], seg2["integrated_tput"], s=18, alpha=0.75,
               label=f"segment 2 (n={len(seg2)})", color="C3")

lo = 0
hi = max(df["seq_tput"].max(), df["integrated_tput"].max()) * 1.05
xs = [lo, hi]
ax.plot(xs, xs, "k--", linewidth=0.8, label="y = x (no speedup)")
ax.plot(xs, [v * 2 for v in xs], color="gray", linestyle=":", linewidth=0.8, label="y = 2x")
ax.plot(xs, [v * 3 for v in xs], color="gray", linestyle=":", linewidth=0.8, label="y = 3x")
ax.set_xlim(lo, hi)
ax.set_ylim(lo, hi)
ax.set_aspect("equal")
ax.set_xlabel("Sequential throughput (tx/s)")
ax.set_ylabel("Integrated throughput (tx/s)")
ax.set_title(f"Integrated vs Sequential — each point = one batch ({TITLE_TAG})")
ax.grid(alpha=0.3)
ax.legend(loc="upper left", fontsize=9)
fig.tight_layout()
fig.savefig(OUTDIR / f"{PREFIX}_integ_vs_seq.png", dpi=130)
plt.close(fig)

print("Wrote:")
for p in (f"{PREFIX}_tput.png", f"{PREFIX}_speedup.png", f"{PREFIX}_integ_vs_seq.png"):
    print(" ", OUTDIR / p)
