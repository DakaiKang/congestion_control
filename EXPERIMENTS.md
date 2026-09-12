# Experiments Log

Working notes for the `stream` branch, on a 36-core x86-64 EC2 instance. Early
runs used `NUM_THREADS=16`; the thread-count sweep later showed the optimum is
**`t = 8`**, which is the default for all subsequent sweeps. The dataset was
also expanded from 10 000 to 20 000 blocks (see Data).

---

## Data

The dataset was later expanded from 10 000 to **20 000 blocks**, four
contiguous 5 000-block chunks spanning four forks:

| Chunk | Blocks | ~Date | Fork |
|---|---|---|---|
| 1 | `16774645..=16779644` | Mar 2023 | Merge |
| 2 | `18581726..=18586725` | Nov 2023 | Shanghai |
| 3 | `19557289..=19562288` | Feb 2024 | Cancun |
| 4 | `22606458..=22611457` | May 2025 | Prague (Pectra) |

| Dataset | Path | Coverage |
|---|---|---|
| Real ETH blocks | `/home/ubuntu/eth-block-downloader/test_data/blocks_rw/` | 20 000 blocks (4 chunks above) |
| Read/write + measured execution time | `/home/ubuntu/eth-block-downloader/test_data/rw_time/` | Generated from real blocks; adds `executionTime` (trimmed mean over 5 rounds) and `from` (real caller). 19 990 files (8 blocks fail single-block replay). |

The rw_time dataset feeds the V2 synthetic workload — every tx is replayed
against `TxSimulatorV2.execute(reads, writes, target)` where
`target = executionTime_ns / T_SLOAD_NS`.

> **Note on the data path.** The active data now lives under
> `/home/ubuntu/eth-block-downloader/test_data/{blocks_rw,rw_gas,rw_time}/`;
> the older `/home/ubuntu/eth-block-data/...` paths are stale defaults in some
> tests — override with `BLOCKS_DIR` / `RW_TIME_DIR` / `RW_GAS_DIR`.

> **Chunk-4 (Prague) caveat.** The fully-reliable analyses below restrict to
> the **first 150 batches** (chunks 1–3, 15 000 blocks, ~2.4 M txs). Chunk 4
> (Prague) produces a handful of batches whose sequential baseline diverges to
> the gas limit, giving implausibly high speedups; those batches are excluded
> (see EIP-7702 / Prague section).

### Generating the rw_time dataset

`test_generate_rw_time` (in `crates/pevm/tests/tx_simulator_test.rs`) produces
the `rw_time_<N>.json` files by **actually executing** each block's real
transactions sequentially with `execute_revm_sequential_timed` and recording
per-tx wall-clock time. It repeats `ROUNDS` passes in round-first order (round 0
visits every block, then round 1, …, so a block's samples are spread across the
full round to dampen transient noise), drops the top/bottom `TRIM` samples per
tx, and writes the trimmed mean as `executionTime` (ns). Each block's senders
are balance-patched to `u128::MAX` so revm's pre-execution gas check passes on
the ~0.5% of blocks whose single-block prestate is insufficient.

Inputs are two parallel directories keyed by block number:
- `RW_GAS_DIR` — `rw_gas_<N>.json` files; pick which blocks to process **and**
  serve as the output template (`gasUsed` is dropped, `executionTime` + `from`
  added).
- `BLOCKS_DIR` — `block_<N>.json` files; the real txs that get executed/timed.

A block is skipped if its `block_<N>.json` is missing, any round fails, or the
rw_gas/block tx counts disagree.

```bash
RW_GAS_DIR=/home/ubuntu/eth-block-downloader/test_data/rw_gas \
BLOCKS_DIR=/home/ubuntu/eth-block-downloader/test_data/blocks_rw \
RW_TIME_DIR=/home/ubuntu/eth-block-downloader/test_data/rw_time \
SKIP_EXISTING=1 \
cargo test --release --test tx_simulator_test test_generate_rw_time -- --nocapture --exact
```

Other env vars: `ROUNDS` (default 5), `TRIM` (default 1, requires
`ROUNDS >= 2*TRIM+1`), `MAX_FILES` (cap files, for smoke tests),
`PROGRESS_EVERY` (default 500), `SKIP_EXISTING` (skip blocks whose output
already exists — makes the run resumable).

**Runtime:** sequential execution dominated by re-reading block JSON each round.
Measured ~28 s per 500 blocks per round (≈56 ms/block/round) on the 36-core
x86-64 box. For the full 20 000-block set: ~19 min/round → **~1.5–2 h** for
`ROUNDS=5` plus the write phase.

---

## Experiments

### 1. Real-block throughput, 100 batches × 100 blocks

**Test:** `test_eth_block_data_all_batches` (in `crates/pevm/tests/bench_test.rs`)

```bash
NUM_THREADS=16 cargo test --release --test bench_test \
    test_eth_block_data_all_batches -- --nocapture
```

Runs all four strategies (Sequential, Parallel/Block-STM, Graph Parallel,
Integrated) on every 100-block batch in `blocks_rw/`, writes per-batch
timings and throughputs to CSV.

**Output:** `pevm/eth_block_data_batches.csv` (100 rows)

**Aggregate** (defaults: `tau_cv = 0.5`, `hot_key_threshold = 1.5`,
chosen via the sweep below):

| Strategy | Total time | tx/s | Speedup |
|---|---:|---:|---:|
| Sequential | 82.4 s | 18 391 | 1.00× |
| Parallel (Block-STM) | 65.9 s | 23 004 | 1.25× |
| Graph Parallel | 63.9 s | 23 697 | 1.29× |
| Integrated | 44.5 s | 34 012 | **1.85×** |

**Plots** (in `pevm/`): `plot_tput.png`, `plot_speedup.png`, `plot_integ_vs_seq.png`.

### 2. V2 synthetic throughput, same 100 batches

**Test:** `test_v2_all_batches` (in `crates/pevm/tests/tx_simulator_test.rs`)

```bash
NUM_THREADS=16 cargo test --release --test tx_simulator_test \
    test_v2_all_batches -- --nocapture
```

**Output:** `pevm/v2_batches.csv` (100 rows)

**Aggregate** (same defaults: `tau_cv = 0.5`, `hot_key_threshold = 1.5`):

| Strategy | Total time | tx/s | Speedup |
|---|---:|---:|---:|
| Sequential | 116.4 s | 13 013 | 1.00× |
| Parallel | 70.1 s | 21 602 | 1.66× |
| Graph Parallel | 57.6 s | 26 293 | 2.02× |
| Integrated | 50.3 s | 30 095 | **2.31×** |

**Plots:** `plot_v2_tput.png`, `plot_v2_speedup.png`, `plot_v2_integ_vs_seq.png`.

V2 Sequential is *slower* than real-block Sequential despite the synthetic
contract being simpler — V2 still pays ABI decode + contract dispatch +
SLOAD/SSTORE loop overhead per tx, while many real txs are simple ETH
transfers. V2's higher Graph/Integrated speedups reflect that its conflict
structure is exactly the `reads/writes` arrays with no implicit
dependencies (no beneficiary, no CALL chains).

V2 Integrated dropped from 2.45× (at `tau_cv = 0.1`) to 2.31× under the new
default `tau_cv = 0.5`. This is the expected V2-specific cost we identified
in the parameter sweep — V2's compressed per-tx `gas_cost` distribution makes
`decent_graph` early-exit fire more readily than on real ETH, fragmenting
groups. The trade is intentional: the new default optimises for real
production workloads, accepting a 6 % regression on the synthetic benchmark.

### 3. Integration cost vs integrated-execution cost

**Test:** `test_integration_vs_execution_smoke` (in `bench_test.rs`)

```bash
NUM_THREADS=16 [TAU_CV=<x>] cargo test --release --test bench_test \
    test_integration_vs_execution_smoke -- --nocapture
```

Isolates 100 real blocks into two timed sections —
`integrator.integrate_pevm_graphs(...)` alone, then the integrated
parallel execution. Per-block dependency-graph construction (which uses
`execute_revm_sequential_with_access_sets`) is run once but explicitly
**excluded** from the integration timer.

**Headline numbers** (post-optimization, τ_CV = 1.0):

| Phase | Time |
|---|---:|
| Graph construction (excluded from integration) | ~0.80 s |
| Integration (greedy merge) | **0.19 s** |
| Integrated parallel execution | **0.36 s** |
| Execution / Integration ratio | **1.94×** |

**Important caveat about the ratio:** these numbers are measured against
`InMemoryStorage`. A production EVM client pays substantial state-trie
disk I/O per block; realistic execution is 10–100× slower while
integration is unchanged. Production integration share is < 5 %, not the
40 % implied here.

---

## Tunable parameters that affect execution speed

Parameters are listed roughly in the order they take effect, from
workload preparation to the final parallel execution.

### Workload-shape (data + batching)

| Parameter | Where | Default | Effect |
|---|---|---|---|
| `BATCH_SIZE` | env var on `test_eth_block_data_all_batches` / `test_v2_all_batches` | 100 blocks | Larger batch → more candidates for greedy to fuse, but more dependency-graph nodes per integrated execution. |
| `GREEDY_BATCH` | env var on V2 tests | 50 blocks | Chunk size inside greedy. Greedy runs once per chunk; merges cannot cross chunk boundaries. Tested 50 vs 100 → almost no difference on this dataset. |
| `MAX_BATCHES` | env var on the all-batches tests | unset (all) | Number of 100-block batches to run; mainly for smoke tests. |
| `START_BLOCK`, `NUM_BLOCKS` | env vars on real-block tests | 16774645, 100 | Selects the slice of blocks fed to the test. |

### Graph construction

| Parameter | Where | Default | Effect |
|---|---|---|---|
| `hot_key_threshold` | `TransactionGraph::set_hot_key_threshold` (default 1.5 in `TransactionGraph::new`) | 1.5 | A key is "hot" when its lifespan during simulated execution exceeds `hot_key_threshold × avg_thread_completion`. Lower → more hot keys → greedy's `has_common_hot_keys` filter rejects more candidate merges → smaller groups. |

### Greedy integration

| Parameter | Where | Default | Effect |
|---|---|---|---|
| `tau_cv` | `GreedyIntegratorConfig.tau_cv` (default in `Default` impl) | 1.0 | CV threshold for "decent". When G's CV ≤ τ_CV, the inner candidate loop breaks early. On real ETH, raising 0.1 → 1.0 saved ~15 % of integration time without changing group composition. On V2, anything > 0.5 fragments groups badly because V2's `gas_cost` distribution is compressed. |
| `num_threads` (inside config) | `GreedyIntegratorConfig.num_threads` | `available_parallelism()` | Concurrency assumed by `simulate_parallel_execution` when computing CV. Higher → CV computed against a wider pool → harder to satisfy → fewer early-exits. Usually matched to the actual execution thread count. |
| Per-group merge cap | hard-coded in `greedy_decent_integration_with_indices`: `if source_indices.len() > 10 { break; }` | 10 absorbed → 11 total per group | The dominant termination condition on real ETH. Lower cap → more, smaller groups → less integration benefit. Raising it could let greedy fuse harder but blow up integrate_graph cost (O(group_size) per merge). |

### Block-STM parallel execution

| Parameter | Where | Default | Effect |
|---|---|---|---|
| `NUM_THREADS` | env var routed through `parallel_concurrency()` (in `bench_test.rs`) and via the V2 test's `concurrency` binding | 16 | Worker threads spawned by `Pevm::execute_revm_parallel` and `GraphPevm::execute_revm_parallel`. Linear-ish speedup up to physical core count; beyond that, abort/validation contention dominates. We saw `1 → 0.96×`, `2 → 1.79×`, `4 → 3.28×`, `8 → 4.93×`, `16 → 5.97×` for V2 pure Parallel. |
| `block_env.gas_limit` | hard-coded in `make_block_env()` (`bench_test.rs`) | 30 000 000 (real default); `1_000_000_000` for multi-block integrated runs | Caps the per-block gas budget revm enforces. Setting this too low rejects integrated batches whose summed gas exceeds the cap. Set to 1 G in multi-block integrated execution to make room for absorbed blocks. |
| `hot_key_threshold` (also affects scheduling indirectly) | as above | 1.5 | Carries over from graph construction — affects which keys the scheduler considers "hot" and therefore the structure of dependency edges that drive Block-STM aborts. |

### Workload calibration (V2 only)

| Parameter | Where | Default | Effect |
|---|---|---|---|
| `T_SLOAD_NS` | `tests/tx_simulator/mod.rs` constant | 482 ns | Inverse-mapping from real wall-clock time to TxSimulatorV2's `target` loop count. Re-calibrate per machine via `test_calibrate_sload_ns`. Wrong value → V2 sequential time deviates from the real per-tx time it's supposed to mirror. |
| `GAS_MULTIPLIER` | `tests/tx_simulator/mod.rs` constant | (used in `cold_gas` and `hot_gas` formula) | Scales the gas-limit applied to each V2 tx; protects against revm refusing the call. |
| Target clamp `[2, 5000]` | hard-coded in V2 loader | — | Caps per-tx loop count. Clipping compresses the per-tx-cost distribution, contributing to V2's lower CV relative to real blocks. |

### Aggregation / output

| Parameter | Where | Default | Effect |
|---|---|---|---|
| `OUTPUT` | env var on batch tests | per-test default | CSV destination. |
| `PROGRESS_EVERY` | env var on rw_time generator | 500 blocks | Log cadence; cosmetic only. |
| `ROUNDS`, `TRIM` | env vars on rw_time generator | 5, 1 | Per-tx timing samples and how many extremes to drop before averaging. Affects only the produced `rw_time` files, not the executor. |

---

## Parameter sweeps — finding the defaults

> **Superseded by the t=8 re-sweep over 20 000 blocks** (see "Parameter
> re-sweep at t = 8" below). Kept for history; the chosen defaults
> (`hot_kt = 1.5`, `tau_cv = 0.5`) are unchanged.

Two full 100-batch × 100-block sweeps on real ETH mainnet blocks
(`test_eth_block_data_all_batches`, NUM_THREADS=16), each varying one
parameter while holding the other at the prior chosen value.

### Sweep 1 — `hot_key_threshold` (fixed tau_cv = 0.5)

| hot_kt | mean speedup | median | min | max |
|---:|---:|---:|---:|---:|
| 0.5 | 1.75× | 1.71× | 1.31 | 4.35 |
| 1.0 | 1.84× | 1.80× | 1.37 | 4.56 |
| **1.5** | **1.88×** | 1.85× | 1.40 | 4.88 |
| 3.0 | 1.89× | 1.88× | 1.44 | 3.21 |
| 5.0 | 1.89× | 1.86× | 1.43 | 3.25 |

Plot: `pevm/plot_hot_sweep.png`. Per-batch CSVs: `pevm/hot_sweep/`.

**Findings:**
- `hot_kt = 0.5` is clearly the worst — over-rejection fragments groups into 88+ small ones.
- `hot_kt ∈ [1.5, 5.0]` are statistically indistinguishable (mean 1.88-1.89×).
- Tail max speedup peaks at `hot_kt = 1.5` (4.88×); higher thresholds clip the tail
  to 3.2-3.3× because the resulting big groups suffer from Block-STM aborts
  on hot-key conflicts they failed to filter.

### Sweep 2 — `tau_cv` (fixed hot_key_threshold = 1.5)

| tau_cv | mean speedup | median | min | max |
|---:|---:|---:|---:|---:|
| 0.1 | 1.88× | 1.86× | 1.34 | 4.83 |
| 0.3 | 1.89× | 1.87× | 1.37 | 4.91 |
| **0.5** | **1.89×** | 1.85× | 1.28 | 4.83 |
| 1.0 | 1.86× | 1.83× | 1.35 | 4.85 |
| 2.0 | 1.79× | 1.77× | 1.36 | 3.56 |

Plot: `pevm/plot_tau_sweep.png`. Per-batch CSVs: `pevm/tau_sweep/`.

**Findings:**
- `tau_cv ∈ [0.1, 1.0]` form a wide plateau of ~1.86-1.89×.
- `tau_cv = 2.0` is the only setting that hurts noticeably (-5 %) — at that point
  the `decent_graph` early-exit fires after just one or two merges, fragmenting
  groups into ~50-60 pieces.
- Tail max speedup is unchanged in [0.1, 1.0]; only `tau_cv = 2.0` clips the tail
  to 3.56× for the same reason.

### Chosen defaults

`GreedyIntegratorConfig::default()` and the default `TransactionGraph` setting:

| Parameter | Default | Rationale |
|---|---:|---|
| `tau_cv` | **0.5** | Mid-plateau on the sweep; lets `decent_graph` early-exit fire just a bit sooner than 0.3 and save a few `simulate` calls without measurable quality loss. |
| `hot_key_threshold` | **1.5** | Top of the plateau on the sweep, and best tail behaviour (max speedup 4.88×). Bumping higher gives no mean improvement and shortens the tail. |

### Which parameter matters more?

- `hot_key_threshold` swept range (1.75 → 1.89): **+8 % mean speedup**.
- `tau_cv` swept range (1.79 → 1.89): **+5.6 % mean speedup**.
- Both have similar shape: a wide plateau plus one bad tail.

`hot_key_threshold` is the **group-size knob**: it decides how aggressively
candidate merges are vetoed for hot-key overlap, which directly drives the
final group count. `tau_cv` is the **inner-loop stop knob**: at the chosen
values it only affects when an anchor's inner scan terminates, not which
candidates land in which group — so its impact is bounded.

**Asymmetric failure modes:** "over-rejecting" (low `hot_kt`) is more harmful
than "over-permitting" (high `tau_cv`). Both produce ~50-90 small groups,
but low `hot_kt` also slashes the tail max because the resulting groups
have no Block-STM-friendly batching of complementary work.

---

## EIP-7702 / Prague support (chunk-4 enablement)

Extending the dataset to chunk 4 (block ≥ 22 431 084, the Prague/Pectra
activation) broke the real all-batches run. Root causes and fixes:

- **`get_spec_id` capped at Cancun.** Added the Prague boundary
  (`22_431_084.. => SpecId::PRAGUE`) in `storage/block_loader.rs`; otherwise
  revm rejects type-4 (set-code) transactions outright.
- **EIP-7702 `authorizationList` dropped on load.** `parse_transaction` now
  parses the authorization list for type-0x04 txs into
  `tx_env.authorization_list` (as `Signed`), so revm recovers each authority
  and bumps its nonce during execution.
- **Nonce reassignment ignored authorities.** `NonceTracker::update_txenv_nonces`
  now, after reordering, also increments each recovered authority's nonce —
  otherwise a later tx from that authority sees a stale nonce ("nonce too high").
- **Block gas-limit cap.** `make_block_env` raised 30M → **36M** (real
  post-Pectra block gas limit). 1G was tried first but let runaway/diverging
  txs spin to 1G; 36M accepts every real tx while bounding spin.
- **Resilience + resume.** `run_one_batch` now skips a batch (empty CSV row)
  when its seq/par baseline errors; `test_eth_block_data_all_batches` gained
  `START_BATCH` (resume + append) and `SKIP_BATCHES` (skip batches that *hang*
  or *panic* rather than error). Driver `run_real_resume_autoskip.sh` auto-detects
  stalls (tx loops to gas limit) and crashes, adds the offending batch to the
  skip set, and resumes.

Net: the real sweep covers **195 / 200 batches**; excluded chunk-4 batches
159/164/168/171/182 (gas-loop hangs, a storage bytecode-missing panic, one exec
error). Analyses that need clean data restrict to the first 150 batches.

---

## Thread-count sweep (20 000 blocks)

**Test:** `test_eth_block_data_all_batches` (real) and `test_v2_all_batches`
(V2). **Driver:** `experiments/run_thread_sweep.sh`. `NUM_THREADS` is the single
knob — it unifies both the actual parallel execution and the dependency-graph /
greedy CV simulation. Swept `t ∈ {2,4,8,12,16,20,24}`, `tau_cv=0.5`,
`hot_kt=1.5`, over all 200 batches (real: 195 usable; V2: 200).

Per-batch mean speedup vs sequential (chunks 1–3, t shown for the peak region):

| t | real par | real graph | real integ | v2 integ |
|---:|---:|---:|---:|---:|
| 2 | 1.38 | 1.40 | 1.95 | 1.64 |
| 4 | 1.74 | 1.74 | 2.64 | 2.16 |
| **8** | **1.85** | **1.77** | **2.64** | **2.35** |
| 12 | 1.52 | 1.68 | 2.45 | 2.15 |
| 16 | 1.28 | 1.64 | 2.35 | 2.05 |
| 24 | 1.19 | 1.56 | 2.18 | 1.89 |

**All three strategies and both workloads peak at `t = 8` and regress beyond
it** (plain Block-STM degrades hardest — abort/validation contention past the
physical core count). Integrated is consistently best. We adopt **`t = 8`** as
the default for the parameter sweeps below.

---

## Parameter re-sweep at t = 8 (20 000 blocks)

Re-ran the `hot_key_threshold` and `tau_cv` sweeps at the new throughput
optimum (`t = 8`) over the full dataset. **Driver:**
`experiments/run_param_sweeps_t8.sh` (uses `SWEEP_INTEG_ONLY=1` — both params
only affect the integrated path, so Parallel/Graph are skipped). Reported as the
mean integrated speedup over the first 150 batches.

| `hot_kt` (tau_cv=0.5) | 0.5 | 1.0 | **1.5** | 3.0 | 5.0 |
|---|---:|---:|---:|---:|---:|
| mean integ speedup | 2.44 | 2.62 | **2.63** | 2.63 | 2.65 |

| `tau_cv` (hot_kt=1.5) | 0.1 | 0.3 | **0.5** | 1.0 | 2.0 |
|---|---:|---:|---:|---:|---:|
| mean integ speedup | 2.66 | 2.63 | **2.64** | 2.57 | 2.33 |

`hot_kt = 0.5` is clearly worst (over-rejects merges); `1.5–5.0` are a flat
plateau. `tau_cv` is flat over `[0.1, 1.0]` and drops at `2.0`. The current
defaults **`hot_kt = 1.5`, `tau_cv = 0.5`** sit at/near the optimum and are
retained. Plots: `experiments/plot_{hot,tau}_sweep_t8.png`,
`plot_param_sweep_means_t8.png`.

---

## Fully-artificial workload — controllable inter/intra-block conflict

The real and V2 workloads inherit whatever conflict structure mainnet
happens to have. To study how the integrator behaves as a *function* of
conflict, we add a third, fully-synthetic workload whose inter-block and
intra-block conflict are dialed independently by two knobs, `K` and `M`.
Transactions still call `TxSimulatorV2.execute(reads, writes, target)`; only
the read/write sets are generated, not loaded.

**Conflict keys** live in two disjoint namespaces so they never collide:
*cold* keys are the integers `[0, 10000)` and *hot* keys are a fixed set of
ten resources `{h_1, …, h_10}`.

**Inter-block conflict — `K`.** Each block is assigned exactly one hot
resource. Hot resources are assigned in repeating groups of `9 + K`
consecutive blocks: the first `K` blocks of every group are assigned `h_1`,
and the remaining 9 blocks are assigned `h_2 … h_10` respectively. Thus `h_1`
is shared by `K` of every `9 + K` blocks while each other hot resource is
used once per group — larger `K` concentrates more blocks on `h_1` and raises
cross-block conflict.

**Intra-block conflict — `M`.** Every transaction's write set is three
values drawn uniformly at random from `[0, 10000)` (seeded by
`(batch_idx, block_idx, txn_idx)` for reproducibility). The first `M`% of a
block's transactions additionally write that block's hot resource, so larger
`M` makes more transactions inside a block contend on the same hot key. The
block's transactions are then shuffled with seed `(batch_idx, block_idx)` so
the hot-touching transactions are not clustered at the front. Read sets are
empty; each `state[w] += 1` is itself a read-modify-write, so the writes
alone drive Block-STM conflicts and cascading aborts.

**Fixed structure.** 100 batches × 50 blocks × 100 transactions. Per-tx work
is constant (`target = 100` loop iterations) so the only variable across the
sweep is the conflict structure. Each transaction uses a unique caller with
nonce 0 (the integrator reassigns nonces after reordering), so nonce chains
add no spurious conflict. Integration uses the same defaults as the other
workloads: `tau_cv = 0.5`, `hot_key_threshold = 1.5`, `NUM_THREADS = 8`.

**Sweep.** `K ∈ {1, 2, 3, 4}` × `M ∈ {10, 20, 30, 40}` (16 configurations).

**Test:** `test_artificial_all_batches` (in `tx_simulator_test.rs`);
generator `tx_simulator::build_artificial_blocks` (in `tests/tx_simulator/mod.rs`).

```bash
# one (K, M) point
K=4 M=40 NUM_THREADS=8 TAU_CV=0.5 HOT_KEY_THRESHOLD=1.5 \
OUTPUT=experiments/artificial_sweep/k4_m40.csv \
cargo test --release --test tx_simulator_test test_artificial_all_batches -- --nocapture --exact

# full 16-config sweep
bash experiments/run_artificial_sweep.sh
```

Env vars: `NUM_BATCHES` (100), `BLOCKS_PER_BATCH` (50), `TXNS_PER_BLOCK` (100),
`K`, `M`, `TARGET` (100), `WRITES_PER_TX` (3), `GREEDY_BATCH` (50),
`NUM_THREADS`, `TAU_CV`, `HOT_KEY_THRESHOLD`, `SWEEP_INTEG_ONLY`, `OUTPUT`.
Output CSV columns: `batch_idx,k,m,num_blocks,num_txs,seq_time_s,seq_tput,
par_time_s,par_tput,graph_time_s,graph_tput,integrated_time_s,integrated_tput,
num_integrated_groups`.

---

## SIGMOD rebuttal experiments (Sep 2026)

Added in response to the review round. Harness: `crates/pevm/tests/rebuttal_test.rs`
(three tests, one CSV schema); drivers and analysis in `pevm/experiments/rebuttal/`;
generated tables in `pevm/experiments/rebuttal/REPORT.md` (`python3 experiments/rebuttal/analyze.py`).

| Reviewer ask | What was added | Where |
|---|---|---|
| R2-O3, R4-O1: absolute times / throughput / latency percentiles, not just speedups | every engine reports seconds and tx/s per batch; per-block latency p50/p90/p99 derived | all CSVs, `analyze.py::throughput_table` |
| R2-O1, R4-O1: cost of pre-execution and the preparatory stages | timed phases: pre-execute, graph build + intra-block reorder, greedy integration, execution | `phase_*` columns |
| R4-O1: bandwidth of the shipped graph | bytes of access-set hashes + WAW edges per proposal, vs calldata bytes | `omakase_graph_bytes`, `vegeta_sched_bytes`, `calldata_bytes` |
| R4-O2: concatenate a round's blocks into one block and run Block-STM | "Concat" engine: unmodified `Pevm` on the fused block (gas limit scaled by block count) | `concat_*` columns |
| R2-O2, R4-O3: comparison with a prior system | Vegeta (NSDI '25) — see below | `src/vegeta.rs`, `vegeta_*` columns |
| R2-O3, R4-O4: aborts / cascading aborts measured | `ExecDiagnostics` snapshot exposed by `Pevm` and `GraphPevm` (`last_diagnostics`); re-executions always counted, validation/cascade/new-location breakdown under `--features diagnostics` | `*_re_exec`, `*_validation_aborts`, `*_cascade_aborts`, `*_wrote_new_loc` |

### Tests

```bash
# artificial (no dataset): same K x M grid as Figure 7
K=4 M=40 NUM_THREADS=8 OUTPUT=experiments/rebuttal/artificial/k4_m40.csv \
  cargo test --release --test rebuttal_test test_rebuttal_artificial -- --nocapture --exact
# real mainnet blocks, one 5000-block chunk = 50 batches
START_BLOCK=16774645 MAX_BATCHES=50 NUM_THREADS=8 OUTPUT=experiments/rebuttal/real_16774645.csv \
  cargo test --release --test rebuttal_test test_rebuttal_real -- --nocapture --exact
# synthetic V2 from rw_time
MAX_BATCHES=150 NUM_THREADS=8 OUTPUT=experiments/rebuttal/v2.csv \
  cargo test --release --test rebuttal_test test_rebuttal_v2 -- --nocapture --exact
# add --features diagnostics for the abort breakdown (separate run: the counters cost time)
```

Full campaign: `experiments/rebuttal/run_artificial.sh` (runs immediately) and
`experiments/rebuttal/run_real_v2.sh` (waits for the dataset, then real x3 chunks + V2, timing and
diagnostics builds). Defaults everywhere: t=8, tau_cv=0.5, hot_kt=1.5, 100-block batches.

### Correctness guard

On both synthetic workloads every write is `state[k] += 1`, which commutes, so the harness
asserts (`CHECK_STATE=1`, default): Block-STM and Concat reproduce the sequential state
byte-for-byte; Omakase and Vegeta reproduce every storage slot and nonce and conserve total
balance. Per-caller balances legitimately differ under reordering (which tx pays SSTORE_SET
vs SSTORE_RESET moves), so they are excluded from the reordering engines' digest. On real
blocks digests are recorded, not asserted (reordered real EVM execution diverges by design —
the reason the synthetic workload exists — and even the order-preserving parallel engines
differ from sequential only in lazy beneficiary settlement).

### Vegeta baseline — design decision

Vegeta's contribution is a schedule: longest per-key dependency chain first (Rule 1) and a
DAG in which only WAW edges block (Rule 2), intra-block only (§4.2 of their paper defers
inter-block parallelism to future work). We run that schedule on **our graph-aware OCC
engine**: `vegeta::speculate_graph` re-sequences the per-block `TransactionGraph` our own
pre-pass built (same access sets, same gas costs) and rebuilds it in Vegeta's order;
`GraphPevm` executes it. `TransactionGraph::add_transaction` already emits WAW-only edges,
which is exactly Rule 2's blocking clause. Consequences: every engine shares one pre-pass, one
MV store and one validation path — "Vegeta" vs "Graph-aware OCC" differ *only* in intra-block
order, and Vegeta vs Omakase differ *only* in Omakase's inter-block integration — and abort
counts are directly comparable.

A literal port of Algorithms 1–3 including Vegeta's own execution model (Aria-style batches
on a frozen snapshot, no MV store, serial re-execution tail) is kept in `vegeta.rs`
(`speculate` + `replay`) as a documented reference. It is correct (passes the exact-state check)
but ~100x slower on this stack, i.e. it measures the port, not the algorithm, so it is not used
for reported numbers.

### Dataset notes from this round

- The paper's dataset is chunks 1–3 (15 000 blocks). `download.sh` also fetched chunk 4 and a
  `test_data/blocks/` tree that no experiment reads (~170 GB); both skipped this time.
- `download_rw` had no retry and reported Alchemy 429s as "need Alchemy Growth", silently
  dropping the block. Fixed: exponential backoff with jitter (`RPC_MAX_ATTEMPTS`,
  `RPC_BASE_DELAY_MS`) and `SKIP_EXISTING=1` for resumable runs. Running several ranges in
  parallel still exceeds the compute-unit cap; `finish_download.sh` waits, then fills gaps
  with serial passes until all 15 000 blocks are present.
- `pevm/generate_synthetic.sh` produces `rw_time` incrementally (`SKIP_EXISTING=1`, repeated
  passes) so it overlaps with the download; the box is 36-core and the download is
  network-bound, so the single-threaded timing replay is unaffected.

### Where does the concatenated baseline stop winning? (added after first results)

First rebuttal numbers showed the concatenated-block Block-STM baseline (R4-O2) *ahead* of
Omakase at 100-block rounds on all three workloads (e.g. artificial K=2 M=40: concat 2.70 ms/block,
Omakase 3.02, per-block Block-STM 4.98 — Omakase's groups averaged only 2.3 blocks). Three
reasons, each now testable:

1. **Fixed-cost amortisation** — per-execution thread spawn / MvMemory / beneficiary settlement
   is large relative to a 100-tx synthetic block; one 5 000-tx window pays it once.
2. **The synthetic workloads cannot exhibit Block-STM's cascade collapse** — access sets are
   fixed by calldata, so a re-execution never writes a new location (`wrote_new_location`
   stays 0). Only real Ethereum shows cascades; there Block-STM is already at 0.64 re-exec/tx
   vs Omakase 0.37.
3. **The integrator is conservative** — hot-key veto + τ_CV keep groups small, and the merge cap
   was hard-coded at 10 blocks. Concat is Omakase with one group, no graph, no filtering.

Additions in `rebuttal_test.rs` (all CSVs now also carry `threads`, `merge_cap`):

- **Concat + graph** (`cgraph_*`): graph-aware OCC over the whole round as a single group,
  built by `round_graph()` from the per-block graphs (no reorder, no hot-key veto). Isolates
  what the graph is worth at full window size. Early smoke: helps on artificial (+10 % over
  concat), *hurts* on real blocks (mispredicted edges over-serialise — consistent with §7.4).
- **Concat diagnostics** (`concat_*` abort columns): cascades of the reviewer's baseline are
  now measured, not inferred.
- **`MERGE_CAP`** env → `GreedyIntegratorConfig.max_group_blocks` (default 10, the historical
  hard-coded value; other harnesses pass 10 explicitly so their results are unchanged).

Sweeps (`experiments/rebuttal/run_sweeps.sh`, outputs in `experiments/rebuttal/sweeps/`):
threads ∈ {4,8,16,24,32}; round size ∈ {5,10,20,50,100} blocks (total txs held roughly
constant); merge cap ∈ {10,25,50,∞}. Artificial points at the high-contention corner
(K=4, M=40 %), real points on 20 batches of the Cancun chunk.

`run_campaign.sh` serialises everything (artificial grid → real + V2 → sweeps) so no two
timing runs share the machine. Test binaries run with cwd `crates/pevm/`, so
`crates/pevm/experiments` is a symlink to `../../experiments` and relative `OUTPUT` paths
land in `pevm/experiments/`.

### Rebuttal campaign — headline results (all runs complete, 0 panics; full tables in `pevm/experiments/rebuttal/REPORT.md`)

Real Ethereum, 150 batches × 100 blocks, t=8: Sequential 17.3k tx/s; Block-STM 32.5k (1.87×);
**Concat Block-STM 37.0k (2.14×)**; Concat + graph 34.0k (1.96×); Graph-aware OCC 30.5k (1.76×);
Vegeta 28.6k (1.65×); **Omakase 46.3k (2.67×)**, p99 5.9 ms/block vs 13.8 for Concat.
Re-executions/tx: Block-STM 0.55, Concat 0.83, Concat+graph 0.65, Omakase 0.42.
Preparatory phases (cost as a fraction of sequential execution time): pre-execute 1.01×, graph build
0.10×, integrate 0.19×, execute 0.37×. **Validator-side (integrate + execute) speedup is 1.78×, below
Block-STM's 1.87× in this in-memory setting**: integration (1.73 ms/block) costs more than the
execution it saves (1.46 ms/block). All stages on one node: 0.60× (i.e. slower than sequential);
with P proposers sharing pre-execution, 1.19× (P=4) / 1.48× (P=10). Integration cost is independent
of per-tx work while the saving scales with it: on the artificial TARGET sweep the validator-side
speedup is 2.82× / 3.68× / 3.88× (TARGET 100 / 500 / 2000) vs Block-STM 2.08× / 2.35× / 2.41×, with
integration fixed at ~0.5 ms/block against 1.8 / 8.0 / 30.6 ms saved. **Pipelining** (`test_rebuttal_pipeline_real`:
integrate round r+1 on a separate thread while round r executes; 10 rounds x 100 Cancun blocks, 170,746 txs):
sequential 9.87 s; Block-STM 4.96 s (1.99x); Omakase serialised 6.24 s (1.58x, integration 2.04 s);
Omakase pipelined 4.45 s (2.22x) — 88 % of integration hidden. Proposal metadata 12 % of calldata.

Synthetic V2 (exact hints): Concat + graph 2.99× > Concat 2.73× > Omakase 2.34× > Graph OCC 2.10×
≈ Vegeta 2.08× > Block-STM 1.96×; every graph-driven engine has 0 re-executions.

Sweeps (real = 20 Cancun batches; artificial = K=4, M=40 %):
- **threads**: Omakase leads on real at every t (2.44→2.68→2.07 for t=4→8→32) while Concat collapses
  2.37→1.59; on artificial Omakase overtakes Concat from t=16 (3.11 vs 2.52).
- **round size** (real): Concat ahead at ≤20 blocks/round (2.36–2.54 vs 2.18–2.38), Omakase ahead at
  ≥50 (2.59, 2.68).
- **merge cap** {10,25,50,∞}: no effect — the cap is never binding; τ_CV and the hot-key veto set group size.
- **aggressive integration** (τ_CV=0.01): artificial — groups 25→4.8, Omakase 3.24→3.49, now above
  Concat (3.40); real — cap 10 gives 2.73 (best observed), cap 50 drops to 2.36, cap ∞ collapses to 1.68
  (below Block-STM): with imperfect hints, large groups over-serialise on mispredicted edges even
  though re-executions fall to 0.25/tx. Bounding the window is what makes Omakase beat Concat on real data.
- **per-tx cost** (artificial, TARGET 100→2000): Concat 3.41→4.25, Omakase 3.26→3.92 — the
  "expensive transactions favour Omakase" hypothesis is *not* supported on this workload.

Reading: Concat is the upper bound when access-set hints are exact (V2, artificial); Omakase is the
better point when hints are state-dependent and imperfect (real Ethereum), because it limits the
optimistic window and isolates hot keys. Vegeta's longest-chain-first order is neutral with exact hints
and harmful with imperfect ones.

### Live multi-proposer deployment (Mysticeti prototype)

Addresses R2-O1's "full end-to-end measurement under a live multi-proposer deployment" (paper TODO P1).
`mysticeti/` runs a 4-validator committee on one machine (`dry-run` mode, fixed ports, so one committee
at a time); each validator executes every committed round with `PevmExecutor`, now in one of four modes
selected by `PEVM_EXECUTION_MODE`: `sequential`, `parallel` (Block-STM per block), `concatenated`
(Block-STM over the fused round), `integrated` (Omakase: pre-execute -> per-block graph + reorder ->
greedy integration -> graph-aware OCC per group). `Core::handle_committed_subdag_with_pevm` hands all
blocks of a commit to the executor as one round and logs `ROUND blocks= txs= exec_ms= ... Throughput =`
at error level.

Workload: the prototype's built-in ERC20 generator (`PEVM_WORKLOAD=8,4,8` = 8 token contracts x 4
families x 8 accounts = 256 EOAs; snapshot via `tests/erc20_snapshot.rs`). Offered load per validator
`PEVM_LOAD` tx/s (one block every 10 ms); the default 200 tx/s yields 1-2 blocks of 20 txs per round and a
consensus-bound ~7.8k tx/s in every mode, so runs use 150k and 300k, where rounds are 3-5 blocks x
100-150 txs and the executor is busy most of the time. Driver: `experiments/rebuttal/run_live_mysticeti.sh`
(90 s per mode, first 20 s discarded); results in `experiments/rebuttal/live/summary.csv` and REPORT.md.

Caveats to state: synthetic ERC20 transfers with exact hints (no state-dependent access sets), 4 proposers
so at most ~4 blocks per round, all validators on one 36-thread machine (8 executor workers each).

**Live results** (`experiments/rebuttal/live/summary_minround{1,1000}.csv`; PEVM_LOAD 150k/300k, 90 s, first 20 s
discarded). Every mode: ~40.0k committed tx/s per validator (generator-bound; sequential executor 39-42 % busy).
Executor time per committed tx with ~1000-tx rounds (9.8 blocks): sequential 9.9 us, Block-STM 11.1 us,
concatenated 8.1 us, Omakase all stages 19.9 us (busy 80 %). With 1-2-block rounds Omakase is 84 % busy at the
same throughput. Conclusion: functional end-to-end under multi-proposer consensus; on this cheap, exact-hint
workload no parallel executor beats sequential and the preparatory stages double CPU per tx — the cost side of
the ledger, consistent with the offline all-stages accounting. `PEVM_MIN_ROUND_TXS` accumulates commits into
larger execution rounds.

### Emulated state-access latency (`test_rebuttal_state_latency_real`, `run_state_latency.sh`)

`LatencyStorage` adds a spin-wait of δ to every account/slot read for every engine; graph build and
integration never touch state. 20 Cancun batches, t=8 (speedup over sequential at the same δ):

| δ µs | seq ms/blk | Block-STM | Concat | Omakase exec | Omakase exec+integ | integ share |
|---:|---:|---:|---:|---:|---:|---:|
| 0 | 10.6 | 1.95 | 2.37 | 2.74 | 1.86 | 32 % |
| 2 | 13.8 | 2.10 | 2.58 | 2.78 | 2.03 | 27 % |
| 5 | 18.5 | 2.25 | 2.76 | 2.83 | 2.20 | 22 % |
| 10 | 26.0 | 2.35 | 2.86 | 2.82 | 2.35 | 17 % |
| 20 | 41.1 | 2.43 | 2.93 | 2.81 | 2.49 | 11 % |
| 50 | 85.5 | 2.43 | 2.96 | 2.74 | 2.59 | 6 % |

Reading: integration's share of validator time falls 32 % → 6 %; Omakase including integration
overtakes per-block Block-STM at δ ≈ 10 µs and leads by 0.16× at 50 µs. **Concat stays ahead at every δ**
(its larger window hides read latency better; Omakase's groups average ~5 blocks and the graph serialises
part of them). The in-memory argument therefore closes the gap to Block-STM, not to concatenation.

### Correction: the "re-exec wrote a new location" metric (2026-09-12)

`MvMemory::record` is only called by a *successful* incarnation. A transaction whose incarnation 0
was blocked (ESTIMATE / nonce) therefore never recorded, and its first successful execution was compared
against an empty previous write set and counted as "wrote a new location". The column was measuring
post-blocking first executions, not control-flow divergence. Fixed: the counter now requires a previously
recorded incarnation (`recorded_once`). Re-measured on 2 Cancun batches (~18k txs each): Block-STM 33 / 60,
Graph OCC 0 / 1, Omakase 6 / 1 (previously ~3000 / ~1200 / ~900). **Re-executions on real Ethereum almost
never change the write set**: cascades propagate through *values* (dependents re-read and re-execute), not
through access-set changes. All `*_diag.csv` were regenerated with the corrected build (old files in
`old_newloc_metric/`); Table 4 of the paper and the "cascade trigger 3x" claim in the feedback are withdrawn.

Ablation (MERGE_CAP=0 → groups of ≤2 blocks): Omakase's excess validation aborts over Graph-aware OCC
disappears (2972 vs 2647; 2668 vs 2625; 3000 vs 2953), confirming that the excess comes from merged blocks
(cross-block RAW/WAR are not graph edges; the block barrier used to resolve them for free; per-batch
correlation with blocks merged r = 0.54). Its lower cascade-abort count persists (134-172 vs 209-233) and is
therefore not a merging effect — likely the integrator's hot-key-aware re-simulation; unverified.

### Vegeta, faithful execution model (re-measured 2026-09-12)

Reported Vegeta numbers now come from `speculate` + `replay` (Algorithms 1-3: Rule-1 re-sequencing,
Aria-style parallel batches against a frozen pre-batch state, mispredicted transactions deferred to a
serial tail; no multi-version store, no concurrent re-execution). The earlier "schedule on our OCC engine"
variant (`speculate_graph`) stays in the code but is not reported. `replay` had a bug that made it look
~100x slow on real blocks: deferred transactions were never removed from batch selection and were
re-executed and re-deferred forever. Fixed (deferred transactions leave batch selection and no longer
constrain dependents, as in the paper's Fig. 6; nonce failures caused by a deferred predecessor defer the
successor; tail transactions take the sender's current nonce).

| workload | Vegeta speedup | tx/s | serial tail (% txs) |
|---|---:|---:|---:|
| real (150 batches) | 1.31x | 22.8k | 1.0 % |
| V2 (150) | 1.36x | 16.6k | 0.0 % |
| artificial M=10 / 20 / 30 / 40 % | 3.29 / 2.23 / 1.70 / 1.38x | | 0 % |

The tail is small; the cost is the batch model itself: each dependency-chain step becomes a batch barrier
(a 318-tx Cancun block ran as 23 batches), so intra-block chains leave workers idle. Vegeta's schedule
adds 15 % of calldata (its DAG carries WAR/RAW edges too) vs 12 % for Omakase's WAW-only graph.

### Aggressive integration on all 150 real batches (tau_cv=0.01, cap 10)

Merge 2.48x vs default 2.46x; Shanghai 2.53x vs 2.55x; Cancun 2.98x vs 2.93x (all 150 batches).
Within +-2 %: merging more (22 -> 14 groups) buys nothing on real Ethereum; the default already sits on a
flat optimum, and the gap to the concatenated round at low contention is structural, not a tuning choice.
The "adopt as default" sentence is withdrawn.
