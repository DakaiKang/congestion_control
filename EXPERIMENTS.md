# Experiments Log

Working notes for the `stream` branch. All runs use `NUM_THREADS=16` on a
36-core x86-64 EC2 instance unless noted.

---

## Data

| Dataset | Path | Coverage |
|---|---|---|
| Real ETH blocks | `/home/ubuntu/eth-block-data/blocks_rw/` | 10 000 blocks, two contiguous chunks (`16774645..=16779644` and `18581726..=18586725`) |
| Read/write + measured execution time | `/home/ubuntu/eth-block-data/rw_time/` | Generated from real blocks; adds `executionTime` (trimmed mean over 5 rounds) and `from` (real caller) |

Total transactions: **1 515 015**.

The rw_time dataset feeds the V2 synthetic workload — every tx is replayed
against `TxSimulatorV2.execute(reads, writes, target)` where
`target = executionTime_ns / T_SLOAD_NS`.

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
