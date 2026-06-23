# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

**RISE Parallel EVM (pevm)** — a high-performance parallel block executor for the EVM, implemented in Rust. It targets 30+ Gigagas/s throughput using Block-STM optimistic concurrency with an explicit transaction dependency graph.

Working directory structure:
- `pevm/` — the Cargo workspace; **run all `cargo` commands from here.** Members are `crates/pevm` (the library + tests + benches) and `bins/fetch` (the `pevm-fetch` block downloader).
- `pevm/crates/pevm/src/` — library source. All `*.rs` files named in this doc (e.g. `pevm.rs`, `graph_scheduler.rs`) live here unless stated otherwise; tests live in `pevm/crates/pevm/tests/`.
- `EXPERIMENTS.md` (repo root) — the active research log for the `stream` branch: datasets, exact test invocations, tunable parameters, and parameter-sweep results. Read it before touching the dependency-graph / greedy-integrator path.
- `mysticeti/` — consensus layer (separate project).

## Commands

### Build
```bash
# Install cmake first — required to build the snmalloc allocator.
cd pevm                              # workspace root; all cargo commands run from here
cargo build --release
cargo build --features full          # includes Optimism + RPC storage
cargo build --features global-alloc  # includes custom allocators (for benchmarks)
```

### Test
```bash
# Run all tests sequentially to avoid resource contention (each test is already internally parallel)
cargo test --workspace --release -- --test-threads=1

# Initialize ethereum/tests submodule (required for ethereum state tests)
git submodule update --init

# Run a single test (substring match on test name)
cargo test --release --test tx_simulator_test test_throughput_comparison_v2 -- --nocapture

# Many simulator/bench tests are parameterized by env vars. Common ones:
#   NUM_THREADS, NUM_BLOCKS, START_BLOCK, GREEDY_BATCH, TAU_CV, HOT_KEY_THRESHOLD,
#   BLOCKS_DIR, RW_TIME_DIR, MAX_BATCHES, OUTPUT
# (see std::env::var calls in tests/bench_test.rs for the full set)
# Example:
NUM_THREADS=16 NUM_BLOCKS=20 START_BLOCK=16774645 cargo test --release --test bench_test test_real_blocks_performance_100 -- --nocapture
```

Real-block tests in `tests/bench_test.rs` read from external data dirs (overridable via the `*_DIR` env vars), defaulting to `/home/ubuntu/eth-block-downloader/test_data/blocks/batch_1` and `/home/ubuntu/eth-block-data/blocks_rw/` — those tests are skipped/fail if the paths are absent.

### Lint & Format
```bash
cargo fmt
cargo clippy
```

### Benchmarks
```bash
cargo bench --features global-alloc --bench mainnet
cargo bench --features global-alloc --bench gigagas

# Pin to specific cores for accurate results
taskset -c -a 0-15 cargo bench --features global-alloc --bench mainnet

# Flamegraph profiling
cargo flamegraph --profile profiling --bench mainnet -- --bench
```

### Fetch Real Blocks
```bash
cargo run -p pevm-fetch <RPC_URL> <BLOCK_ID>
```

## Architecture

### Core Algorithm: Block-STM

Transactions execute optimistically in parallel. Conflicts are detected via multi-version memory tracking and resolved by re-executing the conflicting transaction. A static dependency graph (built before execution) guides scheduling to minimize conflicts.

### Execution Flow

1. `graph_pevm.rs` / `pevm.rs` — top-level executor that launches worker threads
2. `graph_scheduler.rs` — assigns `Task::Execute` or `Task::Validate` to workers based on dependency graph
3. `vm.rs` — executes a single transaction via `revm`, recording `ReadSet`/`WriteSet`
4. `mv_memory.rs` — multi-version data structure; each tx version writes to its own slot; readers check prior slots for the latest committed value or an `ESTIMATE` marker
5. Conflict detected → tx is aborted (marked `ESTIMATE`), re-queued for execution

### Key Types (`lib.rs`)
- `MemoryLocation` — address, code hash, or storage slot
- `MemoryValue` — includes `LazyRecipient`/`LazySender` for deferred balance updates
- `TxVersion` — `(tx_idx, incarnation)` pair; incarnation increments on re-execution
- `Task` — `Execute(TxVersion)` or `Validate(TxVersion)`
- `IncarnationStatus` — state machine: `ReadyToExecute → Executing → Executed → Validated | Aborting`

### Lazy Updates Optimization

To avoid false dependencies, some memory writes are deferred:
- **Beneficiary balance** — all gas fees go to the block beneficiary; accumulated lazily and resolved at block end or on explicit read
- **Raw ETH transfers** — sender/recipient pairs use `LazySender`/`LazyRecipient` values

### GraphScheduler vs BasicScheduler

`GraphScheduler` (`graph_scheduler.rs`) uses a pre-computed dependency graph (`dependency_graph.rs`) to avoid scheduling a transaction until all its dependencies have been validated. `BasicScheduler` (`scheduler.rs`) is a simpler fallback without dependency awareness.

### Nonce Tracking

`utils/nonce_tracker.rs` validates and sequences nonces across blocks for real Ethereum transactions, which is required because parallel execution may observe nonces out of order.

### Dependency Graph + Greedy Integrator (active research path)

- `dependency_graph.rs` — `TransactionGraph` captures read/write conflicts between txs and can simulate parallel execution to derive metrics like CV (coefficient of variation across worker timelines).
- `greedy_integrator.rs` — packs multiple small blocks' graphs into a larger "integrated" graph for batched parallel execution. Tuned via `GreedyIntegratorConfig.tau_cv` (default 0.2). Many `tx_simulator_test.rs` and `bench_test.rs` tests compare Sequential / Parallel / Graph / Integrated throughput.
- `graph_pevm.rs` is the executor that consumes a `TransactionGraph`; plain `pevm.rs` is the graph-free variant.

### TxSimulator (synthetic workloads)

`tests/tx_simulator/` (V1) and `TxSimulatorV2` (timed execution via `tx_simulator/contract_v2.rs`) generate synthetic blocks that approximate real-tx work. The calibration constants live in `tests/tx_simulator/mod.rs`:

- `T_SLOAD_NS` (~482 ns) — cost per simulated SLOAD/SSTORE op
- `T_OVERHEAD_NS` (~3688 ns) — per-tx fixed cost

`target = (executionTime - T_OVERHEAD_NS) / T_SLOAD_NS` converts a desired real wall-clock duration into a loop count. Recalibrate via `test_calibrate_sload_ns` / `test_calibrate_sload_vs_sstore_ns` if the host machine changes.

### Mysticeti Integration

`api.rs` (`PevmAPI`) is the external entry point used by the `mysticeti/` consensus crate. It wraps `Pevm` with workload loading helpers (`load_in_memory_storage`, ERC20/Uniswap setups) and a streaming/back-pressure interface — not consumed by the main `cargo test` path.

## Feature Flags

| Flag | Purpose |
|------|---------|
| `full` | Enables `optimism` + `rpc-storage` |
| `optimism` | OP Stack–specific execution |
| `rpc-storage` | RPC-backed storage backend |
| `global-alloc` | Custom allocators: rpmalloc (x86), snmalloc (ARM), jemalloc |
| `diagnostics` | Compiles in re-execution / write-set / MV-memory counters used to debug scheduler behavior. Off by default — has runtime overhead. |

## Release Profiles

The release profile uses `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = true`. The `profiling` profile inherits from release but adds `debug = "full"`.
