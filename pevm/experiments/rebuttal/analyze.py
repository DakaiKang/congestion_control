#!/usr/bin/env python3
"""Turn the rebuttal CSVs into the tables and figures the author feedback needs.

Robust to partial results: any missing file is reported and skipped, so this
can be re-run while the campaign is still going.

    python3 experiments/rebuttal/analyze.py [--dir experiments/rebuttal] [--out REPORT.md]
"""
import argparse, glob, os, sys
import numpy as np
import pandas as pd
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

ENGINES = [  # (csv prefix, display name)
    ("seq", "Sequential"),
    ("par", "Block-STM"),
    ("concat", "Block-STM, concatenated"),
    ("cgraph", "Graph-aware OCC, concatenated"),
    ("graph", "Graph-aware OCC"),
    ("vegeta", "Vegeta (schedule on graph OCC)"),
    ("integrated", "Omakase"),
]
DIAG = [("par", "Block-STM"), ("concat", "Block-STM, concatenated"), ("cgraph", "Graph OCC, concatenated"),
        ("graph", "Graph-aware OCC"), ("vegeta", "Vegeta"), ("integ", "Omakase")]


def load(paths):
    frames = []
    for p in paths:
        if os.path.exists(p) and os.path.getsize(p) > 0:
            d = pd.read_csv(p)
            d = d[(d.num_txs > 0) & (d.seq_time_s > 0)]
            if len(d):
                d["src"] = os.path.basename(p)
                frames.append(d)
    return pd.concat(frames, ignore_index=True) if frames else None


def md_table(rows, header):
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---:" if i else "---" for i in range(len(header))) + "|"]
    for r in rows:
        out.append("| " + " | ".join(str(x) for x in r) + " |")
    return "\n".join(out)


def throughput_table(d, title):
    """Absolute time, aggregate tx/s, per-block latency percentiles, speedup."""
    seq_tot = d.seq_time_s.sum()
    rows = []
    for k, name in ENGINES:
        col = f"{k}_time_s"
        if col not in d:
            continue
        t = d[col]
        if (t <= 0).all():
            continue
        m = t > 0
        tot = t[m].sum()
        tput = d.num_txs[m].sum() / tot
        ms_blk = (t[m] / d.num_blocks[m]) * 1000
        p50, p90, p99 = np.percentile(ms_blk, [50, 90, 99])
        rows.append([name, f"{tot:.1f}", f"{tput:,.0f}", f"{p50:.2f}", f"{p90:.2f}", f"{p99:.2f}",
                     f"{d.seq_time_s[m].sum() / tot:.2f}×"])
    hdr = ["Engine", "total s", "tx/s (aggregate)", "ms/block p50", "p90", "p99", "speedup"]
    return f"**{title}** — {len(d)} batches, {int(d.num_blocks.sum()):,} blocks, {int(d.num_txs.sum()):,} txs\n\n" + md_table(rows, hdr)


def phase_table(d, title):
    pre, gb, integ, ex = (d.phase_pre_execute_s.sum(), d.phase_graph_build_s.sum(),
                          d.phase_integrate_s.sum(), d.phase_execute_s.sum())
    seq = d.seq_time_s.sum()
    tot = pre + gb + integ + ex
    n_blocks = d.num_blocks.sum()
    rows = [
        ["Pre-execute (proposer, once per block)", f"{pre:.1f}", f"{pre / tot * 100:.0f}%", f"{pre / n_blocks * 1000:.2f}", f"{pre / seq:.2f}×"],
        ["Build conflict graph + intra-block reorder", f"{gb:.1f}", f"{gb / tot * 100:.0f}%", f"{gb / n_blocks * 1000:.2f}", f"{gb / seq:.2f}×"],
        ["Integrate (greedy, per committed round)", f"{integ:.1f}", f"{integ / tot * 100:.0f}%", f"{integ / n_blocks * 1000:.2f}", f"{integ / seq:.2f}×"],
        ["Execute (Omakase, parallel)", f"{ex:.1f}", f"{ex / tot * 100:.0f}%", f"{ex / n_blocks * 1000:.2f}", f"{ex / seq:.2f}×"],
        ["**All phases on one node**", f"**{tot:.1f}**", "100%", f"**{tot / n_blocks * 1000:.2f}**", f"**{tot / seq:.2f}×** (speedup {seq / tot:.2f}×)"],
        ["Validator-side: integrate + execute", f"{integ + ex:.1f}", "", f"{(integ + ex) / n_blocks * 1000:.2f}", f"{(integ + ex) / seq:.2f}× (speedup {seq / (integ + ex):.2f}×)"],
        ["Sequential execution (reference)", f"{seq:.1f}", "", f"{seq / n_blocks * 1000:.2f}", "1.00×"],
    ]
    hdr = ["Phase", "total s", "share", "ms/block", "cost as a fraction of sequential time"]
    veg = d.vegeta_speculate_s.sum()
    note = (f"\n\nVegeta's schedule construction (Rule-1 reorder + DAG rebuild) on the same pre-pass: "
            f"{veg:.2f} s total, {veg / n_blocks * 1000:.3f} ms/block.")
    return f"**{title}**\n\n" + md_table(rows, hdr) + note


def bandwidth_table(d, title):
    cd, og, vg = d.calldata_bytes.sum(), d.omakase_graph_bytes.sum(), d.vegeta_sched_bytes.sum()
    n_blocks = d.num_blocks.sum()
    rows = [
        ["Transaction calldata (what a block already carries)", f"{cd / 1e6:.1f}", f"{cd / n_blocks / 1e3:.1f}", "100%"],
        ["Omakase per-block conflict graph (access-set hashes + WAW edges)", f"{og / 1e6:.1f}", f"{og / n_blocks / 1e3:.1f}", f"{og / cd * 100:.1f}%"],
        ["Vegeta schedule (same encoding)", f"{vg / 1e6:.1f}", f"{vg / n_blocks / 1e3:.1f}", f"{vg / cd * 100:.1f}%"],
    ]
    return f"**{title}**\n\n" + md_table(rows, ["Payload", "MB total", "KB/block", "vs calldata"])


def abort_table(d, title):
    n = d.num_txs.sum()
    rows = []
    for k, name in DIAG:
        if f"{k}_re_exec" not in d:
            continue
        re, va, ca, wn = (d[f"{k}_re_exec"].sum(), d[f"{k}_validation_aborts"].sum(),
                          d[f"{k}_cascade_aborts"].sum(), d[f"{k}_wrote_new_loc"].sum())
        rows.append([name, f"{re / n:.3f}", f"{va / n:.3f}", f"{ca / n:.4f}", f"{wn / n:.4f}",
                     f"{(ca / va * 100) if va else 0:.0f}%"])
    hdr = ["Engine", "re-executions / tx", "validation aborts / tx", "cascade aborts / tx",
           "re-exec writing a new location / tx", "cascade share of aborts"]
    return (f"**{title}** — {int(n):,} txs (diagnostics build)\n\n" + md_table(rows, hdr) +
            "\n\n*cascade abort* = validation failure because a lower-indexed writer appeared after the read; "
            "*re-exec writing a new location* = a re-execution whose write set differs from its previous *recorded* incarnation "
            "(post-blocking first executions excluded). On real Ethereum this is ~0 for every engine: cascades propagate through values, not access sets.")


def artificial_grid(dirpath):
    files = sorted(glob.glob(os.path.join(dirpath, "artificial", "k*_m*.csv")))
    files = [f for f in files if not f.endswith("_diag.csv")]
    if not files:
        return None, None
    rec = []
    for f in files:
        d = pd.read_csv(f); d = d[d.seq_time_s > 0]
        if not len(d):
            continue
        base = os.path.basename(f)[:-4]
        K = int(base.split("_")[0][1:]); M = int(base.split("_")[1][1:])
        seq = d.seq_time_s.sum()
        row = {"K": K, "M": M}
        for k, _ in ENGINES[1:]:
            t = d.get(f"{k}_time_s"); row[k] = seq / t.sum() if t is not None and (t > 0).all() else np.nan
        dg = f[:-4] + "_diag.csv"
        if os.path.exists(dg):
            dd = pd.read_csv(dg); n = dd.num_txs.sum()
            for k, _ in DIAG:
                if f"{k}_re_exec" in dd:
                    row[f"{k}_re"] = dd[f"{k}_re_exec"].sum() / n
        rec.append(row)
    g = pd.DataFrame(rec).sort_values(["M", "K"])
    lines = ["**Artificial workload — speedup over sequential (t=8, H=5)**\n"]
    hdr = ["ρ_inter K", "ρ_intra M", "Block-STM", "Concat", "Concat+graph", "Graph OCC", "Vegeta", "Omakase"]
    rows = [[r.K, f"{r.M}%", f"{r.par:.2f}", f"{r.concat:.2f}", f"{r.cgraph:.2f}", f"{r.graph:.2f}", f"{r.vegeta:.2f}", f"{r.integrated:.2f}"]
            for r in g.itertuples()]
    lines.append(md_table(rows, hdr))
    if "par_re" in g:
        lines.append("\n**Artificial workload — re-executions per transaction**\n")
        rows = [[r.K, f"{r.M}%", f"{r.par_re:.3f}", f"{r.concat_re:.3f}", f"{r.cgraph_re:.3f}", f"{r.graph_re:.3f}", f"{r.vegeta_re:.3f}", f"{r.integ_re:.3f}"]
                for r in g.dropna(subset=["par_re"]).itertuples()]
        lines.append(md_table(rows, ["K", "M", "Block-STM", "Concat", "Concat+graph", "Graph OCC", "Vegeta", "Omakase"]))
    return "\n".join(lines), g


def sweep_table(dirpath, pattern, key, label):
    """Speedup of each engine as `key` varies (thread / round-size / merge-cap sweeps)."""
    files = sorted(glob.glob(os.path.join(dirpath, pattern)))
    if not files:
        return None
    rec = []
    for f in files:
        d = pd.read_csv(f); d = d[d.seq_time_s > 0]
        if not len(d):
            continue
        seq = d.seq_time_s.sum(); n = d.num_txs.sum()
        if key in d:
            kv = int(d[key].iloc[0])
        else:  # e.g. tx_target: encoded in the filename as ..._g<TARGET>.csv
            import re as _re
            kv = int(_re.search(r"_g(\d+)\.csv$", f).group(1))
        row = {key: kv, "src": os.path.basename(f)}
        for k, _ in ENGINES[1:]:
            t = d.get(f"{k}_time_s"); row[k] = seq / t.sum() if t is not None and (t > 0).all() else np.nan
        row["groups"] = d.num_integrated_groups.mean()
        for k, _ in DIAG:
            if f"{k}_re_exec" in d:
                row[f"{k}_re"] = d[f"{k}_re_exec"].sum() / n
        rec.append(row)
    if not rec:
        return None
    g = pd.DataFrame(rec).sort_values(key)
    hdr = [label, "Block-STM", "Concat", "Concat+graph", "Graph OCC", "Vegeta", "Omakase", "groups/batch", "re-exec/tx: Concat", "Omakase"]
    rows = [[r[key], f"{r.par:.2f}", f"{r.concat:.2f}", f"{r.cgraph:.2f}", f"{r.graph:.2f}", f"{r.vegeta:.2f}", f"{r.integrated:.2f}",
             f"{r.groups:.1f}", f"{r.get('concat_re', np.nan):.3f}", f"{r.get('integ_re', np.nan):.3f}"] for _, r in g.iterrows()]
    return md_table(rows, hdr)


def plot_engines(d, title, path):
    fig, ax = plt.subplots(figsize=(7, 3.6))
    seq = d.seq_time_s.sum()
    names, vals = [], []
    for k, name in ENGINES[1:]:
        t = d[f"{k}_time_s"]
        if (t > 0).all():
            names.append(name.replace(" (schedule on graph OCC)", "").replace(", concatenated", "\n(concat)")); vals.append(seq / t.sum())
    ax.bar(names, vals, color=["#888", "#b8860b", "#daa520", "#4682b4", "#8a2be2", "#2e8b57"][:len(vals)])
    for i, v in enumerate(vals):
        ax.text(i, v + 0.03, f"{v:.2f}×", ha="center", fontsize=9)
    ax.set_ylabel("speedup over sequential"); ax.set_title(title); ax.tick_params(axis="x", labelsize=8)
    fig.tight_layout(); fig.savefig(path, dpi=140); plt.close(fig)


def plot_phases(d, title, path):
    n = d.num_blocks.sum()
    parts = [("pre-execute", d.phase_pre_execute_s.sum()), ("graph build", d.phase_graph_build_s.sum()),
             ("integrate", d.phase_integrate_s.sum()), ("execute", d.phase_execute_s.sum())]
    fig, ax = plt.subplots(figsize=(7, 2.4))
    left = 0
    for name, v in parts:
        ax.barh([title], [v / n * 1000], left=left, label=name); left += v / n * 1000
    ax.barh(["sequential exec"], [d.seq_time_s.sum() / n * 1000], color="#bbb")
    ax.set_xlabel("ms per block"); ax.legend(ncol=4, fontsize=8, loc="lower right")
    fig.tight_layout(); fig.savefig(path, dpi=140); plt.close(fig)


def plot_artificial(g, path):
    fig, axes = plt.subplots(1, 4, figsize=(13, 3.2), sharey=True)
    for ax, M in zip(axes, sorted(g.M.unique())):
        sub = g[g.M == M].sort_values("K")
        for k, name, mk in [("par", "Block-STM", "o"), ("concat", "Concat", "s"), ("cgraph", "Concat+graph", "P"),
                            ("graph", "Graph OCC", "^"), ("vegeta", "Vegeta", "v"), ("integrated", "Omakase", "D")]:
            if k in sub:
                ax.plot(sub.K, sub[k], marker=mk, label=name)
        ax.set_title(f"ρ_intra = {M}%"); ax.set_xlabel("ρ_inter (K)")
    axes[0].set_ylabel("speedup over sequential"); axes[0].legend(fontsize=7)
    fig.tight_layout(); fig.savefig(path, dpi=140); plt.close(fig)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default=os.path.join(os.path.dirname(__file__)))
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    D = a.dir
    out = a.out or os.path.join(D, "REPORT.md")
    sections = ["# Rebuttal experiments — generated tables\n",
                "Machine: c4.8xlarge (18 cores / 36 threads, 58 GiB), t = 8 workers, τ_CV = 0.5, τ_hot = 1.5, InMemoryStorage.\n"]
    missing = []

    real = load(sorted(glob.glob(os.path.join(D, "real_*.csv"))) and
                [f for f in sorted(glob.glob(os.path.join(D, "real_*.csv"))) if not f.endswith("_diag.csv")])
    real_d = load([f for f in sorted(glob.glob(os.path.join(D, "real_*_diag.csv")))])
    v2 = load([os.path.join(D, "v2.csv")])
    v2_d = load([os.path.join(D, "v2_diag.csv")])

    for label, d, dd in [("Real Ethereum (15 000 mainnet blocks)", real, real_d),
                         ("Synthetic V2 (paper §7.2.2)", v2, v2_d)]:
        sections.append(f"\n## {label}\n")
        if d is None:
            missing.append(label); sections.append("_no results yet_\n"); continue
        sections.append(throughput_table(d, "Absolute throughput and per-block latency") + "\n")
        sections.append(phase_table(d, "Where the time goes — full pipeline including the preparatory phases") + "\n")
        sections.append(bandwidth_table(d, "Metadata shipped in a proposal") + "\n")
        if dd is not None:
            sections.append(abort_table(dd, "Aborts and re-executions") + "\n")
        else:
            missing.append(label + " (diagnostics)")
        tag = "real" if d is real else "v2"
        plot_engines(d, label, os.path.join(D, f"plot_{tag}_speedup.png"))
        plot_phases(d, "Omakase pipeline", os.path.join(D, f"plot_{tag}_phases.png"))
        sections.append(f"![]({os.path.basename(os.path.join(D, f'plot_{tag}_speedup.png'))}) ![](plot_{tag}_phases.png)\n")

    sections.append("\n## Artificial workload (tunable conflict)\n")
    txt, g = artificial_grid(D)
    if txt is None:
        missing.append("artificial"); sections.append("_no results yet_\n")
    else:
        sections.append(txt + "\n")
        plot_artificial(g, os.path.join(D, "plot_artificial_grid.png"))
        sections.append("![](plot_artificial_grid.png)\n")

    for sub, pattern, key, label, title in [
        ("sweeps", "threads_*_t*.csv", "threads", "worker threads", "Thread-count sweep"),
        ("sweeps", "roundsize_*_b*.csv", "num_blocks", "blocks per round", "Round-size sweep"),
        ("sweeps", "mergecap_*_c*.csv", "merge_cap", "merge cap (blocks/group)", "Merge-cap sweep (Omakase only varies)"),
        ("sweeps", "aggressive_*_c*.csv", "merge_cap", "merge cap, tau_cv=0.01", "Aggressive integration (merge unless hot-key conflict)"),
        ("sweeps", "txcost_*_g*.csv", "tx_target", "simulated work per tx (TARGET)", "Per-transaction cost sweep"),
    ]:
        for wl in ["artificial", "real", "v2"]:
            t = sweep_table(os.path.join(D, sub), pattern.replace("*_", f"{wl}_", 1), key, label)
            if t:
                sections.append(f"\n## {title} — {wl}\n\n{t}\n")

    pipes = sorted(glob.glob(os.path.join(D, "sweeps", "pipeline_real_b*.csv")))
    if pipes:
        rows = []
        for f in pipes:
            d = pd.read_csv(f).iloc[0]
            sp = lambda x: d.seq_time_s / x
            rows.append([int(d.blocks_per_round), int(d.rounds), int(d.num_txs), f"{d.seq_time_s:.2f}",
                         f"{d.par_time_s:.2f} ({sp(d.par_time_s):.2f}×)",
                         f"{d.omakase_serial_s:.2f} ({sp(d.omakase_serial_s):.2f}×)",
                         f"{d.integrate_only_s:.2f}",
                         f"{d.omakase_pipelined_s:.2f} ({sp(d.omakase_pipelined_s):.2f}×)",
                         f"{(d.omakase_serial_s - d.omakase_pipelined_s) / d.integrate_only_s * 100:.0f}%"])
        sections.append("\n## Pipelined integration — real\n\n" + md_table(rows,
            ["blocks/round", "rounds", "txs", "seq s", "Block-STM s", "Omakase serial s", "integration s",
             "Omakase pipelined s", "integration hidden"]) + "\n")

    lat = sorted(glob.glob(os.path.join(D, "sweeps", "statelat_real_d*.csv")),
                 key=lambda f: int(os.path.basename(f)[len("statelat_real_d"):-4]))
    if lat:
        rows = []
        for f in lat:
            d = pd.read_csv(f); d = d[d.seq_time_s > 0]
            if not len(d):
                continue
            nb = d.num_blocks.sum(); seq = d.seq_time_s.sum(); ms = lambda c: d[c].sum() / nb * 1000
            integ, ex, par, con = d.phase_integrate_s.sum(), d.integrated_time_s.sum(), d.par_time_s.sum(), d.concat_time_s.sum()
            rows.append([f"{int(d.delay_ns.iloc[0]) / 1000:g}", f"{ms('seq_time_s'):.1f}", f"{seq / par:.2f}", f"{seq / con:.2f}",
                         f"{seq / ex:.2f}", f"{seq / (ex + integ):.2f}", f"{ms('phase_integrate_s'):.2f}",
                         f"{integ / (ex + integ) * 100:.0f}%", f"{ms('par_time_s') - ms('integrated_time_s'):.2f}",
                         f"{d.par_re_exec.sum() / d.num_txs.sum():.2f}", f"{d.integ_re_exec.sum() / d.num_txs.sum():.2f}"])
        sections.append("\n## Emulated state-access latency — real Ethereum (20 Cancun batches, t=8)\n\n" + md_table(rows,
            ["read latency µs", "seq ms/block", "Block-STM ×", "Concat ×", "Omakase exec ×", "Omakase exec+integrate ×",
             "integrate ms/block", "integrate share of validator time", "exec saved vs Block-STM ms/block", "re-exec/tx Block-STM", "Omakase"]) +
            "\n\nEvery account/slot read pays the latency in every engine; graph construction and integration never touch state and stay constant.\n")

    lives = sorted(glob.glob(os.path.join(D, "live", "summary*.csv")))
    for live in lives:
        d = pd.read_csv(live, header=None, names=["mode", "log", "rounds", "blocks", "tps", "busy", "bpr"])
        d["step"] = (d.groupby("mode").cumcount() // 4)  # each driver invocation appends 4 validators per mode
        rows = []
        for (mode, step), g in d.groupby(["mode", "step"], sort=False):
            cap = g.tps.mean() / max(g.busy.mean(), 1e-9)
            rows.append([mode, int(step), f"{g.tps.mean():,.0f}", f"{g.busy.mean() * 100:.0f}%", f"{cap:,.0f}",
                         f"{g.bpr.mean():.1f}", f"{g.blocks.mean() * 1.0 / max(g.rounds.mean(), 1):.1f}" if False else f"{g.rounds.mean():,.0f}"])
        tag = os.path.basename(live).replace("summary", "").replace(".csv", "").strip("_") or "default"
        sections.append(f"\n## Live 4-validator Mysticeti deployment — {tag} (ERC20 8x4x8, steady state, per validator)\n\n" + md_table(rows,
            ["execution mode", "load step", "committed tx/s", "executor busy", "implied executor capacity tx/s", "blocks/round", "rounds"]) +
            "\n\nThe generator caps committed throughput at ~40k tx/s per validator in every mode, so the executor is never the bottleneck here; "
            "*executor busy* is the share of wall-clock the executor spends on its round (all stages, pre-execution and integration included for Omakase), "
            "and *implied capacity* = tx/s ÷ busy. Load step 0/1 = successive PEVM_LOAD settings.\n")

    if missing:
        sections.append("\n---\n_Still missing: " + ", ".join(missing) + "_\n")
    with open(out, "w") as f:
        f.write("\n".join(sections))
    print("\n".join(sections))
    print(f"\n[written {out}]", file=sys.stderr)


if __name__ == "__main__":
    main()
