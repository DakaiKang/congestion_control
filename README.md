# Omakase: Execution Congestion Control through Conflict-Graph Integration in Multi-Proposer Blockchains

This repository holds the implementation and the experiment harness behind the
paper. It has two parts:

- `pevm/` — the execution engine (a fork of [pevm](https://github.com/risechain/pevm)):
  Block-STM, the graph-aware OCC engine, conflict-graph construction
  (`dependency_graph.rs`), the greedy integrator, and reimplemented baselines
  (Vegeta, concatenated-round Block-STM). The experiment harness lives in
  `pevm/crates/pevm/tests/rebuttal_test.rs`; every knob is an environment variable.
- `mysticeti/` — the Mysticeti consensus prototype the executors plug into, used
  for the live four-validator deployment.

`EXPERIMENTS.md` is the research log: datasets, exact invocations, parameter
sweeps and their results. `CLAUDE.md` describes the code layout.

## What is ours and what is upstream

`pevm/` started as a fork of [risechain/pevm](https://github.com/risechain/pevm),
which provides the Block-STM engine (`pevm.rs`, `scheduler.rs`, `mv_memory.rs`,
`vm.rs`, `chain/`, `storage/`). Everything the paper describes is ours, in
`pevm/crates/pevm/src/` unless noted:

| Paper section | Component | File(s) |
|---|---|---|
| §5.1 | Pre-execution and conflict-graph construction (Algorithm 1), hot-resource detection | `dependency_graph.rs` |
| §5.2–5.5 | Simulation, reordering, balance test and greedy integration (Algorithms 2–4) | `greedy_integrator.rs`, `graph_scheduler.rs` |
| §5.6 | Graph-aware OCC engine (Algorithm 5) | `graph_pevm.rs`; multi-version memory extensions and abort accounting in `mv_memory.rs`, `exec_diagnostics.rs` |
| §6 | Nonce tracking for reordered execution, graph serialization shipped with a block | `utils/nonce_tracker.rs`, `serialization/` |
| §7.2 | Baselines reimplemented on the same engine: Vegeta, concatenated-round Block-STM and graph-aware OCC | `vegeta.rs`, `tests/rebuttal_test.rs` |
| §7.3 | Synthetic and artificial workload generators, real-block loader | `tests/tx_simulator/`, `storage/block_loader.rs` |
| §7 | Experiment harness, scripts and report generator | `tests/rebuttal_test.rs`, `scripts/`, `experiments/rebuttal/analyze.py` |
| §7.7 | Executor integration into the Mysticeti prototype | `mysticeti/crates/mysticeti-core/src/{core,scheduler,schedule_fetcher,validator,transactions_generator,config}.rs`, `pevm/crates/pevm/src/api.rs` |

`mysticeti/` is the upstream Mysticeti prototype with the files above modified;
the consensus protocol itself is unchanged.

## Prerequisites

- Rust (stable) and `cmake` (for the snmalloc allocator); build from `pevm/`.
- The real-Ethereum dataset: 15,000 mainnet blocks in three 5,000-block segments
  starting at blocks 16,774,645 (Merge), 18,581,726 (Shanghai) and 19,557,289
  (Cancun), downloaded with the companion `eth-block-downloader` into
  `test_data/{blocks_rw,rw_time}`. Point `DATASET` at that directory (default
  `/home/ubuntu/Omakase/eth-block-downloader/test_data`).
- Python 3 with `pandas`, `numpy` and `matplotlib` for `analyze.py`.

## Running the experiments

Every experiment has its own script under `scripts/`; `scripts/all_experiments.sh`
runs all of them in sequence (no two timing runs share the machine) and then
generates the report:

```bash
scripts/all_experiments.sh          # ~15 h on a c4.8xlarge (18 cores / 36 threads)
```

Results go to `pevm/experiments/results/` (override with `RESULTS=...`), and
`pevm/experiments/rebuttal/analyze.py --dir <results>` turns them into
`REPORT.md` with every table of the evaluation. Each script skips outputs that
already exist, so an interrupted run can be restarted, and each can be run alone:

| Script | What it measures | Time |
|---|---|---|
| `01_real_ethereum.sh` | All engines on 150 rounds × 100 real blocks; stage breakdown; abort diagnostics | 1 h |
| `02_synthetic.sh` | Same on the synthetic workload (exact hints) | 45 min |
| `03_vegeta.sh` | Vegeta reimplementation on real, synthetic and artificial workloads | 1 h |
| `04_artificial_grid.sh` | Tunable-conflict grid (ρ_inter × ρ_intra), timing and diagnostics | 3 h |
| `05_threads.sh` | Worker count 4–32 | 30 min |
| `06_round_size.sh` | Blocks per round 5–100 (samples and full dataset) | 3.5 h |
| `07_merge_cap.sh` | Omakase group cap 10–1000 | 30 min |
| `08_tx_cost.sh` | Work per transaction 1×/5×/20× | 30 min |
| `09_aggressive_integration.sh` | Merge unless a hot key is shared | 1 h |
| `10_pipeline.sh` | Integration of round r+1 overlapped with execution of round r | 5 min |
| `11_state_latency.sh` | Emulated per-read state latency 0–50 µs, 8/16/32 workers | 1 h |
| `12_live_mysticeti.sh` | Four-validator Mysticeti deployment, four executors | 10 min |
| `13_graph_construction_ablation.sh` | Algorithm 1 graph vs. write-after-write-only variant | 40 min |
| `14_hot_key_sweep.sh` | Hot-key threshold τ_hot 0.5–5.0 (paper §7.4), full real dataset, Omakase only | 1 h |
| `15_tau_cv_sweep.sh` | CV threshold τ_CV 0.1–2.0 (paper §7.4), full real dataset, Omakase only | 1 h |

Scripts 01–13 are the SIGMOD author-feedback experiments; together with 14–15
they cover every figure and table of the paper's evaluation (§7.4 parameter
sweeps, §7.5 speedup, §7.6 absolute throughput and stronger baselines, §7.7
end-to-end stage cost and latency, §7.8 aborts, §7.9 artificial workload). The
two integration thresholds affect only Omakase, so 14 and 15 time sequential and
Omakase alone (`ENGINES=omakase`); the older `bench_test`-based sweep scripts
under `pevm/experiments/` (`run_param_sweeps_t8.sh`, `run_thread_sweep.sh`)
produced the submission's figures with the write-after-write-only graph and
are kept for reference only.

Two conventions apply to every statistic: the conflict graph is built as the
paper's Algorithm 1 specifies (RAW edges from the last writer, account-level
reads in the predicted read set; `GRAPH_EDGES=waw TRACK_BASIC_READS=0` selects
the ablated variant), and rounds in which any engine abandoned an optimistic
window and re-executed it sequentially (`*_fallbacks` columns) are excluded by
`analyze.py`.

## Useful knobs

`NUM_THREADS`, `TAU_CV`, `HOT_KEY_THRESHOLD`, `MERGE_CAP`, `BATCH_SIZE`,
`MAX_BATCHES`, `START_BLOCK`, `STATE_DELAY_NS`, `TARGET`, `K`/`M` (artificial
ρ_inter/ρ_intra), `ENGINES=vegeta` (Vegeta and sequential only),
`ENGINES=omakase` (sequential and Omakase only), `PEVM_TRACE_FALLBACK=1` (print
sequential fallbacks). See `EXPERIMENTS.md` for
the full list and the recorded results.
