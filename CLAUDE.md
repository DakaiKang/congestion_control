# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

**RISE Parallel EVM (pevm)** — a high-performance parallel block executor for the EVM, implemented in Rust. It targets 30+ Gigagas/s throughput using Block-STM optimistic concurrency with an explicit transaction dependency graph.

Working directory structure:
- `pevm/` — main workspace (Rust library + CLI + benchmarks)
- `mysticeti/` — consensus layer (separate project)

## Commands

### Build
```bash
cargo build --release
cargo build --features full          # includes Optimism + RPC storage
cargo build --features global-alloc  # includes custom allocators (for benchmarks)
```

### Test
```bash
# Run all tests sequentially to avoid resource contention
cargo test --workspace --release -- --test-threads=1

# Initialize ethereum/tests submodule (required for state tests)
git submodule update --init
```

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

## Feature Flags

| Flag | Purpose |
|------|---------|
| `full` | Enables `optimism` + `rpc-storage` |
| `optimism` | OP Stack–specific execution |
| `rpc-storage` | RPC-backed storage backend |
| `global-alloc` | Custom allocators: rpmalloc (x86), snmalloc (ARM), jemalloc |

## Release Profiles

The release profile uses `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = true`. The `profiling` profile inherits from release but adds `debug = "full"`.
