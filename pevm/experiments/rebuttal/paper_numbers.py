"""Print every number Sec. VII of the paper quotes, from one results directory.
Usage: python3 -I paper_numbers.py <results_dir> [out_dir_for_coords]
Conventions follow analyze.py: rounds where any engine fell back are dropped
(analyze.py monkey-patches pd.read_csv), Vegeta columns are overlaid from the
vegeta/ re-measurement when present."""
import glob, os, re, sys
import numpy as np, pandas as pd

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import analyze as A  # noqa: E402  (patches pd.read_csv, provides load/overlay_vegeta)

D = sys.argv[1]
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(D, "paper_coords")
os.makedirs(OUT, exist_ok=True)
ENG = [("par", "Block-STM"), ("concat", "Concat B-STM"), ("cgraph", "Concat gOCC"),
       ("graph", "Graph OCC"), ("vegeta", "Vegeta"), ("integrated", "Omakase")]


def banner(t):
    print("\n" + "=" * 8 + " " + t)


def load_main(tag):
    if tag == "real":
        d = A.load([f for f in sorted(glob.glob(os.path.join(D, "real_*.csv")))
                    if re.fullmatch(r"real_\d+\.csv", os.path.basename(f))])
        d = A.overlay_vegeta(d, sorted(glob.glob(os.path.join(D, "vegeta", "real_*.csv"))))
        dd = A.load(sorted(glob.glob(os.path.join(D, "real_*_diag.csv"))))
    else:
        d = A.load([os.path.join(D, "v2.csv")])
        d = A.overlay_vegeta(d, [os.path.join(D, "vegeta", "v2.csv")])
        dd = A.load([os.path.join(D, "v2_diag.csv")])
    return d, dd


for tag in ("real", "v2"):
    d, dd = load_main(tag)
    if d is None:
        print(f"[{tag}] no data"); continue
    banner(f"{tag}: {len(d)} rounds, {int(d.num_txs.sum())} txs, {int(d.num_blocks.sum())} blocks")
    seq = d.seq_time_s.sum()
    print(f"sequential: {d.num_txs.sum() / seq:,.0f} tx/s, {seq / d.num_blocks.sum() * 1000:.2f} ms/block")
    rows = []
    for k, n in ENG:
        col = f"{k}_time_s"
        if col not in d or not (d[col] > 0).all():
            continue
        sp = d.seq_time_s / d[col]
        ms = d[col] / d.num_blocks * 1000
        r = dict(engine=n, agg=seq / d[col].sum(), tput=d.num_txs.sum() / d[col].sum(),
                 median=sp.median(), p10=np.percentile(sp, 10), min=sp.min(), max=sp.max(),
                 p99=np.percentile(sp, 99), ms_p50=np.percentile(ms, 50), ms_p99=np.percentile(ms, 99),
                 ms_mean=ms.mean())
        if k != "integrated":
            r["omakase_faster_rounds"] = int((d.integrated_time_s < d[col]).sum())
        if dd is not None:
            dk = "integ" if k == "integrated" else k
            if f"{dk}_re_exec" in dd:
                r["re_exec_per_tx"] = dd[f"{dk}_re_exec"].sum() / dd.num_txs.sum()
                for c in ("validation_aborts", "cascade_aborts"):
                    if f"{dk}_{c}" in dd:
                        r[c[:4] + "_per_tx"] = dd[f"{dk}_{c}"].sum() / dd.num_txs.sum()
        rows.append(r)
    t = pd.DataFrame(rows).set_index("engine")
    pd.set_option("display.width", 250); pd.set_option("display.max_columns", 30)
    print(t.round(3).to_string())
    both = ((d.integrated_time_s < d.par_time_s) & (d.integrated_time_s < d.concat_time_s)).sum()
    print(f"Omakase above both Block-STM and concat in {both}/{len(d)} rounds")
    # phase costs (Table VI rows), ms/block
    nb = d.num_blocks.sum()
    for c, lab in [("phase_pre_execute_s", "pre-execute"), ("phase_graph_build_s", "build graph + reorder"),
                   ("phase_integrate_s", "integrate"), ("phase_execute_s", "execute (Omakase)")]:
        if c in d:
            print(f"  {lab:24s} {d[c].sum() / nb * 1000:6.2f} ms/block  = {d[c].sum() / seq:.2f} of seq")
    if {"phase_integrate_s", "phase_execute_s"} <= set(d.columns):
        ie = d.phase_integrate_s.sum() + d.phase_execute_s.sum()
        print(f"  validator integrate+execute {ie / nb * 1000:.2f} ms/block, speedup {seq / ie:.2f}x; "
              f"integration share of validator time {d.phase_integrate_s.sum() / ie * 100:.0f}%")
        for P in (20, 50, 100):
            pre = d.phase_pre_execute_s.sum() + d.phase_graph_build_s.sum()
            tot = ie + pre / P
            print(f"  every stage on one node, P={P}: {tot / nb * 1000:.2f} ms/block, {seq / tot:.2f}x")
    if "omakase_graph_bytes" in d:
        print(f"  graph bytes/block {d.omakase_graph_bytes.sum() / nb:,.0f}; calldata bytes/block "
              f"{d.calldata_bytes.sum() / nb:,.0f}" + (f"; vegeta sched bytes/block {d.vegeta_sched_bytes.sum() / nb:,.0f}" if "vegeta_sched_bytes" in d else ""))
    # Fig. 6 coordinates: per-round speedups, chronological and sorted
    dd_ = d.reset_index(drop=True)
    for k in ("par", "graph", "integrated"):
        sp = (dd_.seq_time_s / dd_[f"{k}_time_s"]).values
        with open(os.path.join(OUT, f"coords_{tag.upper()}_{k}_chron.txt"), "w") as f:
            f.write(" ".join(f"({i},{v:.2f})" for i, v in enumerate(sp)))
        srt = np.sort(sp)
        with open(os.path.join(OUT, f"coords_{tag.upper()}_{k}_pct.txt"), "w") as f:
            f.write(" ".join(f"({100 * (i + 0.5) / len(srt):.1f},{v:.2f})" for i, v in enumerate(srt)))
    print(f"  Fig. 6 coordinates written to {OUT}")

# ---------------- state latency (Table V) ----------------
banner("state latency (Table V), 8 workers")
files = sorted(glob.glob(os.path.join(D, "sweeps", "statelat_real_d*.csv")),
               key=lambda f: int(re.search(r"_d(\d+)", f).group(1)))
for f in files:
    m = re.search(r"statelat_real_d(\d+)(?:_t(\d+))?\.csv", os.path.basename(f))
    d = A.load([f])
    if d is None:
        continue
    seq = d.seq_time_s.sum(); nb = d.num_blocks.sum()
    cols = {"par": "Block-STM", "concat": "Concat B-STM", "cgraph": "Concat gOCC", "graph": "Graph OCC",
            "vegeta": "Vegeta", "integrated": "Omakase"}
    parts = []
    for k, n in cols.items():
        c = f"{k}_time_s"
        if c in d and (d[c] > 0).all():
            parts.append(f"{n} {seq / d[c].sum():.2f}")
    integ, ex = d.phase_integrate_s.sum(), d.integrated_time_s.sum()
    parts.append(f"Omakase+integ {seq / (ex + integ):.2f}")
    parts.append(f"integ share {integ / (ex + integ) * 100:.0f}%")
    print(f"delta={int(m.group(1)) / 1000:g}us t={m.group(2) or 8} ({len(d)} rounds, seq {seq / nb * 1000:.2f} ms/blk): " + ", ".join(parts))

# ---------------- thread sweeps ----------------
banner("thread sweeps (aggregate speedup)")
for wl in ("real", "v2"):
    series = {}
    for t_ in (4, 8, 16, 24, 32):
        f = os.path.join(D, "sweeps", f"threads_{wl}_t{t_}.csv")
        d = A.load([f])
        if d is None:
            continue
        seq = d.seq_time_s.sum()
        for k, n in ENG:
            c = f"{k}_time_s"
            if c in d and (d[c] > 0).all():
                series.setdefault(n, []).append((t_, seq / d[c].sum()))
    for n, pts in series.items():
        print(f"  {wl:4s} {n:13s} " + "  ".join(f"t{t_}={v:.2f}" for t_, v in pts))

# ---------------- hot-key / tau_cv sweeps (Omakase only) ----------------
banner("parameter sweeps (Omakase aggregate speedup; main-campaign fallback rounds excluded)")
excl = set()
for f in glob.glob(os.path.join(D, "real_*.csv")):
    if re.fullmatch(r"real_\d+\.csv", os.path.basename(f)):
        start = re.search(r"real_(\d+)\.csv", f).group(1)
        raw = pd.read_csv.__wrapped__(f) if hasattr(pd.read_csv, "__wrapped__") else A._pd_read_csv(f)
        fb = [c for c in raw.columns if c.endswith("_fallbacks")]
        for b in raw.loc[raw[fb].sum(axis=1) > 0, "batch_idx"]:
            excl.add((start, int(b)))
for prefix, tagc, vals in (("hot", "h", ["0.5", "1.0", "1.5", "3.0", "5.0"]), ("tau", "t", ["0.1", "0.3", "0.5", "1.0", "2.0"])):
    out = []
    for v in vals:
        fr = []
        for f in sorted(glob.glob(os.path.join(D, "sweeps", f"{prefix}_real_{tagc}{v}_*.csv"))):
            d = A.load([f])
            if d is None:
                continue
            start = re.search(r"_(\d{8})\.csv$", f).group(1)
            d = d[~d.batch_idx.apply(lambda b: (start, int(b)) in excl)]
            fr.append(d)
        if fr:
            d = pd.concat(fr)
            out.append(f"{prefix}={v}: {d.seq_time_s.sum() / d.integrated_time_s.sum():.2f}x ({len(d)} rounds)")
    print("  " + "; ".join(out))

# ---------------- round-size P rows, pipeline, tx cost, merge cap, aggressive ----------------
banner("round-size: integration and execution per block at P blocks/round (full dataset)")
for wl, pat in (("real", "roundsize_full_real_b{P}_*.csv"), ("v2", "roundsize_full_v2_b{P}.csv")):
    for P in (5, 10, 20, 50):
        fs = sorted(glob.glob(os.path.join(D, "sweeps", pat.format(P=P))))
        d = A.load(fs)
        if d is None:
            continue
        seq = d.seq_time_s.sum(); nb = d.num_blocks.sum()
        ie = d.phase_integrate_s.sum() + d.phase_execute_s.sum()
        pre = d.phase_pre_execute_s.sum() + d.phase_graph_build_s.sum()
        print(f"  {wl} P={P}: integ {d.phase_integrate_s.sum() / nb * 1000:.2f} ms/blk, exec {d.phase_execute_s.sum() / nb * 1000:.2f}, "
              f"Omakase {seq / d.integrated_time_s.sum():.2f}x, all-stage {seq / (ie + pre / P):.2f}x ({len(d)} rounds)")
banner("pipeline")
for f in sorted(glob.glob(os.path.join(D, "sweeps", "pipeline_real_b*.csv"))):
    d = pd.read_csv(f)
    if len(d) and "omakase_pipelined_s" in d:
        d = d.sum(numeric_only=True)
        print(f"  {os.path.basename(f)}: serial {d.omakase_serial_s:.2f}s, pipelined {d.omakase_pipelined_s:.2f}s, "
              f"integration hidden {(d.omakase_serial_s - d.omakase_pipelined_s) / d.integrate_only_s * 100:.0f}%")
banner("tx cost / merge cap / aggressive (Omakase and baselines)")
for pat in ("txcost_artificial_g*.csv", "mergecap_real_c*.csv", "mergecap_artificial_c*.csv", "aggressive_real_c*.csv", "aggressive_full_real_*.csv"):
    for f in sorted(glob.glob(os.path.join(D, "sweeps", pat))):
        d = A.load([f])
        if d is None:
            continue
        seq = d.seq_time_s.sum()
        parts = [f"{n} {seq / d[f'{k}_time_s'].sum():.2f}" for k, n in ENG if f"{k}_time_s" in d and (d[f"{k}_time_s"] > 0).all()]
        print(f"  {os.path.basename(f)} ({len(d)} rounds): " + ", ".join(parts))
banner("graph ablation (script 13)")
for f in sorted(glob.glob(os.path.join(D, "graph_ablation", "*_timing_*.csv"))):
    d = A.load([f])
    if d is None:
        continue
    seq = d.seq_time_s.sum()
    print(f"  {os.path.basename(f)} ({len(d)} rounds): Graph OCC {seq / d.graph_time_s.sum():.2f}, Omakase {seq / d.integrated_time_s.sum():.2f}, "
          f"groups/round {d.num_integrated_groups.mean():.1f}")
