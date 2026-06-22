/// Integration tests for the TxSimulator workload module.
///
/// These tests verify that:
/// 1. The in-memory storage is correctly initialised with the contract account
///    and 1000 EOA accounts.
/// 2. rw_gas block files are parsed and turned into valid TxEnv batches.
/// 3. The caller-uniqueness constraint is enforced.

#[path = "tx_simulator/mod.rs"]
pub mod tx_simulator;

use pevm::{
    api::update_storage_with_results,
    chain::PevmEthereum,
    execute_revm_sequential,
    execute_revm_sequential_with_access_sets,
    graph_pevm::GraphPevm,
    greedy_integrator::{GreedyIntegrator, GreedyIntegratorConfig},
    InMemoryStorage,
    Pevm,
};
use std::time::Instant;
use revm::primitives::{BlockEnv, SpecId, TransactTo, TxEnv, B256, U256};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Path to the directory containing rw_gas_*.json files.
const RW_GAS_DIR: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_gas";

#[test]
fn test_build_storage() {
    let (state, bytecodes, simulator_address, accounts) = tx_simulator::build_storage();

    println!("=== build_storage ===");
    println!("TxSimulator contract address: {simulator_address}");
    println!("EOA accounts:                 {}", accounts.len());
    println!("state entries:                {} (1000 accounts + 1 contract)", state.len());
    println!("bytecodes entries:            {}", bytecodes.len());

    let sample = state.get(&accounts[0]).unwrap();
    println!("accounts[0] address: {}", accounts[0]);
    println!("accounts[0] balance: {}", sample.balance);
    println!("accounts[0] nonce:   {}", sample.nonce);

    let contract = state.get(&simulator_address).unwrap();
    println!("contract nonce:      {} (non-zero means it is a contract)", contract.nonce);
    println!("contract code_hash:  {:?}", contract.code_hash);

    // 1000 EOA accounts + 1 contract account
    assert_eq!(accounts.len(), tx_simulator::NUM_ACCOUNTS);
    assert_eq!(state.len(), tx_simulator::NUM_ACCOUNTS + 1);
    assert!(state.contains_key(&simulator_address));
    for addr in &accounts {
        let acct = state.get(addr).unwrap();
        assert!(acct.balance > revm::primitives::U256::ZERO);
    }
    assert!(!bytecodes.is_empty());
    let _ = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
}

#[test]
fn test_load_rw_gas_blocks() {
    let (state, bytecodes, blocks_txs) =
        tx_simulator::load_rw_gas_blocks(RW_GAS_DIR)
            .expect("failed to load rw_gas blocks");

    println!("\n=== load_rw_gas_blocks ===");
    println!("blocks loaded: {}", blocks_txs.len());

    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();
    println!("total transactions across all blocks: {total_txs}");

    for (i, block) in blocks_txs.iter().enumerate() {
        println!("  block[{i}]: {} transactions", block.len());
    }

    let tx0 = &blocks_txs[0][0];
    println!("\n--- block[0] first transaction ---");
    println!("  caller:           {}", tx0.caller);
    println!("  gas_limit:        {}", tx0.gas_limit);
    println!("  gas_price:        {}", tx0.gas_price);
    println!("  calldata length:  {} bytes", tx0.data.len());
    println!("  selector (first 4 bytes): {:02x}{:02x}{:02x}{:02x}",
        tx0.data[0], tx0.data[1], tx0.data[2], tx0.data[3]);
    println!("  expected selector = 2a92932d (execute function)");

    let mut all_callers = std::collections::HashSet::new();
    for block in &blocks_txs {
        for tx in block {
            assert!(all_callers.insert(tx.caller), "duplicate caller: {:?}", tx.caller);
        }
    }
    println!("\nunique callers: {} (equals total tx count ✓)", all_callers.len());

    for block in &blocks_txs {
        for tx in block {
            assert!(state.contains_key(&tx.caller));
            assert!(tx.gas_limit > 0);
        }
    }

    let _ = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    println!("InMemoryStorage constructed successfully ✓");
}

#[test]
fn test_execute_block() {
    let (state, bytecodes, blocks_txs) =
        tx_simulator::load_rw_gas_blocks(RW_GAS_DIR).expect("failed to load rw_gas blocks");

    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();
    let concurrency = NonZeroUsize::new(8).unwrap();

    for (block_idx, txs) in blocks_txs.into_iter().enumerate() {
        let num_txs = txs.len();

        // sequential
        let seq_results = execute_revm_sequential(
            &chain, &storage, spec_id, block_env.clone(), txs.clone(),
        )
        .unwrap_or_else(|e| panic!("sequential failed on block {block_idx}: {e:?}"));

        let seq_successes = seq_results.iter().filter(|r| r.receipt.status.coerce_status()).count();
        let seq_gas: u64 = seq_results.iter().map(|r| r.receipt.cumulative_gas_used).sum();

        println!("\n=== block[{block_idx}] sequential ===");
        println!("  transactions:  {num_txs}");
        println!("  succeeded:     {seq_successes}");
        println!("  failed/revert: {}", num_txs - seq_successes);
        println!("  total gas:     {seq_gas}");

        // parallel
        let par_results = Pevm::default()
            .execute_revm_parallel(&chain, &storage, spec_id, block_env.clone(), txs, concurrency)
            .unwrap_or_else(|e| panic!("parallel failed on block {block_idx}: {e:?}"));

        let par_successes = par_results.iter().filter(|r| r.receipt.status.coerce_status()).count();
        let par_gas: u64 = par_results.iter().map(|r| r.receipt.cumulative_gas_used).sum();

        println!("=== block[{block_idx}] parallel ({} threads) ===", concurrency);
        println!("  transactions:  {num_txs}");
        println!("  succeeded:     {par_successes}");
        println!("  failed/revert: {}", num_txs - par_successes);
        println!("  total gas:     {par_gas}");

        assert_eq!(seq_results.len(), par_results.len());
        for (i, (s, p)) in seq_results.iter().zip(par_results.iter()).enumerate() {
            assert_eq!(
                s.receipt.status, p.receipt.status,
                "block[{block_idx}] tx[{i}]: sequential and parallel disagree on status"
            );
        }
        println!("sequential == parallel ✓");
    }
}

#[test]
fn test_graph_pevm_execute_block() {
    let (state, bytecodes, blocks_txs) =
        tx_simulator::load_rw_gas_blocks(RW_GAS_DIR).expect("failed to load rw_gas blocks");

    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();
    let concurrency = NonZeroUsize::new(8).unwrap();

    for (block_idx, txs) in blocks_txs.into_iter().enumerate() {
        let num_txs = txs.len();

        // Step 1: sequential run to build the dependency graph
        let (mut graph, _) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &storage, spec_id, block_env.clone(), txs.clone(), block_idx as u64,
        )
        .unwrap_or_else(|e| panic!("graph construction failed on block {block_idx}: {e:?}"));

        println!("\n=== block[{block_idx}] GraphPevm ===");
        println!("  transactions:   {num_txs}");
        println!("  graph nodes:    {}", graph.nodes.len());

        // Step 2: reorder transactions according to the dependency graph
        let (reordered_txns, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs, &mut graph, concurrency.get());

        println!("  reordered txns: {}", reordered_txns.len());

        // Step 3: parallel execution with the graph
        let par_results = GraphPevm::default()
            .execute_revm_parallel(
                &chain, &storage, spec_id, block_env.clone(),
                reordered_txns, concurrency, new_graph,
            )
            .unwrap_or_else(|e| panic!("GraphPevm parallel failed on block {block_idx}: {e:?}"));

        let successes = par_results.iter().filter(|r| r.receipt.status.coerce_status()).count();
        let total_gas: u64 = par_results.iter().map(|r| r.receipt.cumulative_gas_used).sum();

        println!("  succeeded:      {successes}");
        println!("  failed/revert:  {}", par_results.len() - successes);
        println!("  total gas:      {total_gas}");
        println!("GraphPevm executed successfully ✓");
    }
}

#[test]
fn test_greedy_integration_execute() {
    const NUM_BLOCKS: usize = 10;
    const RW_GAS_DIR_MULTI: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_gas";

    let (state, bytecodes, simulator_address, blocks_txs) =
        tx_simulator::load_n_rw_gas_blocks(RW_GAS_DIR_MULTI, NUM_BLOCKS)
            .expect("failed to load rw_gas blocks");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = NonZeroUsize::new(8).unwrap();

    println!("\n=== greedy integration: {} blocks, {} threads ===", NUM_BLOCKS, concurrency);

    // Step 1: for each block, run sequential to build dependency graph.
    // Update storage after each block so subsequent blocks see accurate state.
    let mut current_storage = base_storage.clone();
    let mut dependency_graphs = Vec::new();
    let mut reordered_blocks_txs = Vec::new();

    for (block_idx, txs) in blocks_txs.iter().enumerate() {
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain,
            &current_storage,
            spec_id,
            BlockEnv::default(),
            txs.clone(),
            block_idx as u64,
        )
        .unwrap_or_else(|e| panic!("graph construction failed on block {block_idx}: {e:?}"));

        update_storage_with_results(&mut current_storage, results);

        let (reordered_txns, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());

        println!(
            "  block[{block_idx}]: {} txns, {} graph nodes",
            txs.len(),
            new_graph.nodes.len()
        );

        dependency_graphs.push(new_graph);
        reordered_blocks_txs.push(reordered_txns);
    }

    // Step 2: greedy integration — merge graphs that can run together.
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        num_threads: concurrency.get(),
        ..GreedyIntegratorConfig::default()
    });
    let (integrated_txns, integrated_graphs) =
        integrator.integrate_pevm_graphs(dependency_graphs, reordered_blocks_txs);

    println!("  integrated into {} execution groups", integrated_txns.len());

    // Step 3: execute each integrated group with GraphPevm.
    let mut exec_storage = base_storage;
    let mut total_txs = 0usize;
    let mut total_successes = 0usize;

    for (group_idx, (txs, graph)) in integrated_txns.iter().zip(integrated_graphs.iter()).enumerate() {
        let results = GraphPevm::default()
            .execute_revm_parallel(
                &chain,
                &exec_storage,
                spec_id,
                BlockEnv::default(),
                txs.clone(),
                concurrency,
                graph.clone(),
            )
            .unwrap_or_else(|e| panic!("GraphPevm failed on group {group_idx}: {e:?}"));

        let successes = results.iter().filter(|r| r.receipt.status.coerce_status()).count();
        let gas: u64 = results.iter().map(|r| r.receipt.cumulative_gas_used).sum();

        println!(
            "  group[{group_idx}]: {} txns, {} succeeded, {} gas",
            txs.len(), successes, gas
        );

        update_storage_with_results(&mut exec_storage, results);
        total_txs += txs.len();
        total_successes += successes;
    }

    println!(
        "total: {total_txs} txns, {total_successes} succeeded, {} failed",
        total_txs - total_successes
    );
    println!("greedy integration executed successfully ✓");
}

#[test]
fn test_throughput_comparison() {
    const NUM_BLOCKS: usize = 100;
    const GREEDY_BATCH: usize = 20;   // tuned on 10 real ETH blocks: best throughput
    const RW_GAS_DIR_MULTI: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_gas";

    let (state, bytecodes, _simulator_address, blocks_txs) =
        tx_simulator::load_n_rw_gas_blocks(RW_GAS_DIR_MULTI, NUM_BLOCKS)
            .expect("failed to load blocks");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = NonZeroUsize::new(8).unwrap();

    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

    // ── Pre-computation (not timed) ─────────────────────────────────────────
    // Build per-block dependency graphs; update storage between blocks.
    let mut prep_storage = base_storage.clone();
    let mut dep_graphs = Vec::new();
    let mut reordered_blocks = Vec::new();

    for (i, txs) in blocks_txs.iter().enumerate() {
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &prep_storage, spec_id, BlockEnv::default(), txs.clone(), i as u64,
        )
        .unwrap_or_else(|e| panic!("graph build failed on block {i}: {e:?}"));
        update_storage_with_results(&mut prep_storage, results);

        let (reordered, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());
        dep_graphs.push(new_graph);
        reordered_blocks.push(reordered);
    }

    // Greedy integration in batches of GREEDY_BATCH blocks.
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        tau_cv: 0.5,
        num_threads: concurrency.get(),
    });
    let mut integrated_txns: Vec<Vec<revm::primitives::TxEnv>> = Vec::new();
    let mut integrated_graphs = Vec::new();
    for chunk_start in (0..NUM_BLOCKS).step_by(GREEDY_BATCH) {
        let chunk_end = (chunk_start + GREEDY_BATCH).min(NUM_BLOCKS);
        let (itxns, igraphs) = integrator.integrate_pevm_graphs(
            dep_graphs[chunk_start..chunk_end].to_vec(),
            reordered_blocks[chunk_start..chunk_end].to_vec(),
        );
        integrated_txns.extend(itxns);
        integrated_graphs.extend(igraphs);
    }

    println!(
        "\n=== Throughput comparison: {NUM_BLOCKS} blocks, {total_txs} txns, {} threads, greedy batch={GREEDY_BATCH} ===",
        concurrency
    );
    println!("{:<38} {:>10} {:>12} {:>10}", "Mode", "Time (ms)", "Txns/s", "Speedup");
    println!("{}", "-".repeat(73));

    // ── 1. Sequential ────────────────────────────────────────────────────────
    let mut seq_storage = base_storage.clone();
    let t = Instant::now();
    for txs in &blocks_txs {
        let results = execute_revm_sequential(
            &chain, &seq_storage, spec_id, BlockEnv::default(), txs.clone(),
        ).unwrap();
        update_storage_with_results(&mut seq_storage, results);
    }
    let seq_ms = t.elapsed().as_secs_f64() * 1000.0;
    let seq_tput = total_txs as f64 / (seq_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>10}", "Sequential", seq_ms, seq_tput, "1.00x");

    // ── 2. Pevm parallel ─────────────────────────────────────────────────────
    let mut par_storage = base_storage.clone();
    let t = Instant::now();
    for txs in &blocks_txs {
        let results = Pevm::default()
            .execute_revm_parallel(
                &chain, &par_storage, spec_id, BlockEnv::default(), txs.clone(), concurrency,
            ).unwrap();
        update_storage_with_results(&mut par_storage, results);
    }
    let par_ms = t.elapsed().as_secs_f64() * 1000.0;
    let par_tput = total_txs as f64 / (par_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x",
             "Pevm parallel", par_ms, par_tput, par_tput / seq_tput);

    // ── 3. GraphPevm parallel ────────────────────────────────────────────────
    let mut graph_storage = base_storage.clone();
    let t = Instant::now();
    for (txs, graph) in reordered_blocks.iter().zip(dep_graphs.iter()) {
        let results = GraphPevm::default()
            .execute_revm_parallel(
                &chain, &graph_storage, spec_id, BlockEnv::default(),
                txs.clone(), concurrency, graph.clone(),
            ).unwrap();
        update_storage_with_results(&mut graph_storage, results);
    }
    let graph_ms = t.elapsed().as_secs_f64() * 1000.0;
    let graph_tput = total_txs as f64 / (graph_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x",
             "GraphPevm parallel", graph_ms, graph_tput, graph_tput / seq_tput);

    // ── 4. Greedy (batch=20) + GraphPevm ─────────────────────────────────────
    let mut greedy_storage = base_storage.clone();
    let t = Instant::now();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        let results = GraphPevm::default()
            .execute_revm_parallel(
                &chain, &greedy_storage, spec_id, BlockEnv::default(),
                txs.clone(), concurrency, graph.clone(),
            ).unwrap();
        update_storage_with_results(&mut greedy_storage, results);
    }
    let greedy_ms = t.elapsed().as_secs_f64() * 1000.0;
    let greedy_tput = total_txs as f64 / (greedy_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x",
             format!("Greedy(batch={GREEDY_BATCH}) + GraphPevm"), greedy_ms, greedy_tput, greedy_tput / seq_tput);

    println!("{}", "-".repeat(73));
    println!("(greedy produced {} execution groups from {} blocks)", integrated_txns.len(), NUM_BLOCKS);
}

#[test]
fn test_greedy_param_sweep() {
    const NUM_BLOCKS: usize = 10;
    const RW_GAS_DIR_MULTI: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_gas";

    let tau_cv_values:   &[f64]   = &[0.05, 0.1, 0.2, 0.5, 1.0];
    let batch_size_values: &[usize] = &[10, 20, 50, 100];

    let (state, bytecodes, _simulator_address, blocks_txs) =
        tx_simulator::load_n_rw_gas_blocks(RW_GAS_DIR_MULTI, NUM_BLOCKS)
            .expect("failed to load blocks");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = NonZeroUsize::new(8).unwrap();
    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

    // ── Pre-build all block graphs (not timed) ───────────────────────────────
    let mut prep_storage = base_storage.clone();
    let mut dep_graphs = Vec::new();
    let mut reordered_blocks = Vec::new();

    for (i, txs) in blocks_txs.iter().enumerate() {
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &prep_storage, spec_id, BlockEnv::default(), txs.clone(), i as u64,
        ).unwrap();
        update_storage_with_results(&mut prep_storage, results);
        let (reordered, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());
        dep_graphs.push(new_graph);
        reordered_blocks.push(reordered);
    }

    // ── Baselines ────────────────────────────────────────────────────────────
    let seq_tput = {
        let mut s = base_storage.clone();
        let t = Instant::now();
        for txs in &blocks_txs {
            let r = execute_revm_sequential(&chain, &s, spec_id, BlockEnv::default(), txs.clone()).unwrap();
            update_storage_with_results(&mut s, r);
        }
        total_txs as f64 / t.elapsed().as_secs_f64()
    };

    let graph_tput = {
        let mut s = base_storage.clone();
        let t = Instant::now();
        for (txs, graph) in reordered_blocks.iter().zip(dep_graphs.iter()) {
            let r = GraphPevm::default().execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
            ).unwrap();
            update_storage_with_results(&mut s, r);
        }
        total_txs as f64 / t.elapsed().as_secs_f64()
    };

    // ── Sweep: collect all results first, then print ─────────────────────────
    // (greedy integrator itself prints verbose logs; collecting first keeps the
    //  final table clean)
    struct SweepResult { tput: f64, num_groups: usize }
    let mut results: Vec<Vec<SweepResult>> = Vec::new();

    let mut best_tput = 0.0_f64;
    let mut best_params = (0.0_f64, 0_usize, 0_usize);

    for &tau_cv in tau_cv_values {
        let mut row = Vec::new();
        for &batch_size in batch_size_values {
            eprintln!("[sweep] tau_cv={tau_cv}  batch_size={batch_size} ...");
            let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
                tau_cv,
                num_threads: concurrency.get(),
            });
            let mut integrated_txns = Vec::new();
            let mut integrated_graphs = Vec::new();
            for chunk_start in (0..NUM_BLOCKS).step_by(batch_size) {
                let chunk_end = (chunk_start + batch_size).min(NUM_BLOCKS);
                let (itxns, igraphs) = integrator.integrate_pevm_graphs(
                    dep_graphs[chunk_start..chunk_end].to_vec(),
                    reordered_blocks[chunk_start..chunk_end].to_vec(),
                );
                integrated_txns.extend(itxns);
                integrated_graphs.extend(igraphs);
            }
            let num_groups = integrated_txns.len();

            let mut exec_storage = base_storage.clone();
            let t = Instant::now();
            for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
                let r = GraphPevm::default().execute_revm_parallel(
                    &chain, &exec_storage, spec_id, BlockEnv::default(),
                    txs.clone(), concurrency, graph.clone(),
                ).unwrap();
                update_storage_with_results(&mut exec_storage, r);
            }
            let tput = total_txs as f64 / t.elapsed().as_secs_f64();

            if tput > best_tput {
                best_tput = tput;
                best_params = (tau_cv, batch_size, num_groups);
            }
            eprintln!("[sweep] tau_cv={tau_cv}  batch_size={batch_size}  → {tput:.0} txns/s ({num_groups} groups)");
            row.push(SweepResult { tput, num_groups });
        }
        results.push(row);
    }

    // ── Print table ──────────────────────────────────────────────────────────
    let col_w = 18_usize;
    let row_label_w = 18_usize;
    let total_w = row_label_w + col_w * batch_size_values.len();

    println!("\n=== Greedy integration parameter sweep ===");
    println!("{NUM_BLOCKS} blocks · {total_txs} txns · {} threads", concurrency);
    println!("Baseline  sequential : {:>6.0} txns/s  (1.00x)", seq_tput);
    println!("Baseline  GraphPevm  : {:>6.0} txns/s  ({:.2}x)", graph_tput, graph_tput / seq_tput);
    println!();
    println!("Cell format: txns/s (Ng groups)");
    println!();

    // header
    print!("{:<row_label_w$}", "tau_cv \\ batch");
    for &b in batch_size_values { print!("{:>col_w$}", format!("batch={b}")); }
    println!();
    println!("{}", "-".repeat(total_w));

    for (i, &tau_cv) in tau_cv_values.iter().enumerate() {
        print!("{:<row_label_w$}", format!("tau_cv={tau_cv}"));
        for j in 0..batch_size_values.len() {
            let r = &results[i][j];
            print!("{:>col_w$}", format!("{:.0}({:>2}g)", r.tput, r.num_groups));
        }
        println!();
    }

    println!("{}", "-".repeat(total_w));
    println!();
    println!(
        "Best: tau_cv={}, batch={}, {} groups → {:.0} txns/s  ({:.2}x seq · {:.2}x GraphPevm)",
        best_params.0, best_params.1, best_params.2,
        best_tput, best_tput / seq_tput, best_tput / graph_tput
    );
}

/// Measure per-transaction execution time.
///
/// Each block gets a fresh clone of the base storage.  Within each block,
/// transactions are executed one at a time (single-tx sequential execution)
/// and the storage is updated after each tx so every tx sees the correct
/// accumulated state.  Timing covers only the execution of that single tx.
#[test]
fn test_per_tx_execution_time() {
    const NUM_BLOCKS: usize = 10;

    let (state, bytecodes, _addr, blocks_txs) =
        tx_simulator::load_n_rw_gas_blocks(RW_GAS_DIR, NUM_BLOCKS)
            .expect("failed to load blocks");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;

    println!("\n=== Per-tx execution time (µs) ===");
    println!("{:<8} {:>5} {:>8} {:>8} {:>6} {:>8} {:>8} {:>8}",
             "block", "txns", "mean", "std", "CV", "p50", "p90", "max");
    println!("{}", "-".repeat(67));

    for (block_idx, txs) in blocks_txs.iter().enumerate() {
        // Each block starts from the same base state.
        let mut storage = base_storage.clone();
        let mut times_us: Vec<f64> = Vec::with_capacity(txs.len());

        for tx in txs {
            let start = std::time::Instant::now();
            let results = execute_revm_sequential(
                &chain, &storage, spec_id, revm::primitives::BlockEnv::default(),
                vec![tx.clone()],
            )
            .unwrap_or_else(|e| panic!("block[{block_idx}] tx failed: {e:?}"));
            let elapsed_us = start.elapsed().as_secs_f64() * 1e6;
            times_us.push(elapsed_us);
            update_storage_with_results(&mut storage, results);
        }

        let n = times_us.len();
        let mean = times_us.iter().sum::<f64>() / n as f64;
        let std = (times_us.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
        let cv = std / mean;
        let mut sorted = times_us.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = sorted[n / 2];
        let p90 = sorted[(9 * n / 10).min(n - 1)];
        let max = *sorted.last().unwrap();

        println!("{:<8} {:>5} {:>8.1} {:>8.1} {:>6.3} {:>8.1} {:>8.1} {:>8.1}",
                 block_idx, n, mean, std, cv, p50, p90, max);
    }
}

/// Execute every block's real Ethereum transactions sequentially (one tx at a
/// time, with storage updates), record the wall-clock execution time of each
/// transaction over `ROUNDS` repetitions, drop top/bottom `TRIM` outliers per
/// tx, average the rest, and write the results to RW_TIME_DIR.
///
/// Loop ordering is **pure round-first**: round 0 visits every block, then
/// round 1 visits every block, etc. So a single block's N timing samples are
/// spread across the full duration of one round (~minutes), which makes
/// transient system noise affect rounds much less than block-first ordering.
/// We don't cache blocks — only the per-block per-tx timing samples (~60 MB
/// total for 10k blocks × ~150 txs × 5 rounds). Each round re-reads the
/// block JSON, which is the main cost.
///
/// Configurable via env vars:
///   BLOCKS_DIR    (default /home/ubuntu/eth-block-data/blocks_rw)
///   RW_GAS_DIR    (default /home/ubuntu/eth-block-data/rw_gas)
///   RW_TIME_DIR   (default /home/ubuntu/eth-block-data/rw_time)
///   ROUNDS        (default 5)   — number of timed sequential passes per block
///   TRIM          (default 1)   — drop this many highest and lowest samples
///                                 before averaging; ROUNDS - 2*TRIM >= 1
///   MAX_FILES     (optional)    — limit number of rw_gas files (smoke tests)
///   PROGRESS_EVERY (default 500) — print progress every N blocks within a round
///
/// Output format mirrors rw_gas but with `executionTime` (ns, trimmed mean
/// across ROUNDS) and `from` (caller address) instead of `gasUsed`.
#[test]
fn test_generate_rw_time() {
    use std::collections::{BTreeMap, HashSet};
    use std::fs;
    use std::path::Path;
    use std::time::Instant;
    use pevm::execute_revm_sequential_timed;
    use pevm::storage::block_loader::{load_block_for_execution, get_spec_id};

    let rw_gas_dir_str = std::env::var("RW_GAS_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/rw_gas".to_string());
    let blocks_dir_str = std::env::var("BLOCKS_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/blocks_rw".to_string());
    let rw_time_dir_str = std::env::var("RW_TIME_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/rw_time".to_string());
    let rounds: usize = std::env::var("ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let trim: usize = std::env::var("TRIM").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let progress_every: usize = std::env::var("PROGRESS_EVERY")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(500);
    let skip_existing: bool = std::env::var("SKIP_EXISTING")
        .ok().map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
    assert!(rounds >= 2 * trim + 1,
        "ROUNDS ({rounds}) must be >= 2*TRIM ({}) + 1", 2 * trim);

    let rw_gas_dir = Path::new(&rw_gas_dir_str);
    let blocks_dir = Path::new(&blocks_dir_str);
    let rw_time_dir = Path::new(&rw_time_dir_str);
    fs::create_dir_all(rw_time_dir).expect("failed to create rw_time dir");

    let mut rw_gas_files: Vec<_> = fs::read_dir(rw_gas_dir)
        .expect("failed to read rw_gas dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    rw_gas_files.sort();
    if let Some(n) = std::env::var("MAX_FILES").ok().and_then(|v| v.parse::<usize>().ok()) {
        rw_gas_files.truncate(n);
    }

    println!(
        "rw_gas={} blocks={} rw_time={} | rounds={} trim={} | {} files",
        rw_gas_dir.display(), blocks_dir.display(), rw_time_dir.display(),
        rounds, trim, rw_gas_files.len(),
    );

    let chain = PevmEthereum::mainnet();
    let overall = Instant::now();

    // block_num -> [tx_idx][round_idx] timing samples
    let mut samples: BTreeMap<u64, Vec<Vec<u64>>> = BTreeMap::new();
    // Blocks that failed at any round: excluded from subsequent rounds and write phase.
    let mut failed: HashSet<u64> = HashSet::new();
    let mut skipped_no_block: HashSet<u64> = HashSet::new();

    // ── Round-first loop ────────────────────────────────────────────────────
    for round in 0..rounds {
        let round_t = Instant::now();
        let mut visited = 0usize;
        for rw_gas_path in &rw_gas_files {
            let stem = rw_gas_path.file_stem().unwrap().to_str().unwrap();
            let block_num: u64 = match stem.trim_start_matches("rw_gas_").parse() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if failed.contains(&block_num) || skipped_no_block.contains(&block_num) {
                continue;
            }
            if skip_existing
                && rw_time_dir.join(format!("rw_time_{block_num}.json")).exists()
            {
                continue;
            }
            let block_path = blocks_dir.join(format!("block_{block_num}.json"));
            if !block_path.exists() {
                skipped_no_block.insert(block_num);
                continue;
            }
            let (block_data, mut storage, txenvs) =
                match load_block_for_execution(block_path.to_str().unwrap(), false) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("skip block {block_num}: load failed: {e}");
                        failed.insert(block_num);
                        continue;
                    }
                };
            let spec_id = get_spec_id(block_num);
            let block_env = pevm::storage::block_loader::create_block_env(&block_data);
            let ntx = txenvs.len();

            // Patch every sender's balance to u128::MAX so revm's worst-case
            // pre-execution check (gas_limit * max_fee_per_gas) always passes.
            // Single-block prestate is otherwise insufficient for ~0.5% of blocks.
            for tx in &txenvs {
                let acct = storage.accounts.entry(tx.caller).or_default();
                acct.balance = revm::primitives::U256::from(u128::MAX);
            }

            // Initialize sample slot on first round; check consistency on later rounds.
            let entry = samples.entry(block_num).or_insert_with(|| vec![Vec::with_capacity(rounds); ntx]);
            if entry.len() != ntx {
                eprintln!("skip block {block_num}: tx count changed across rounds ({} -> {ntx})", entry.len());
                failed.insert(block_num);
                continue;
            }

            match execute_revm_sequential_timed(
                &chain, &storage, spec_id, block_env, txenvs,
            ) {
                Ok((_results, tx_times_ns)) => {
                    if tx_times_ns.len() != ntx {
                        eprintln!(
                            "skip block {block_num}: round {round} returned {} timings, expected {ntx}",
                            tx_times_ns.len()
                        );
                        failed.insert(block_num);
                        continue;
                    }
                    for (i, &t) in tx_times_ns.iter().enumerate() {
                        entry[i].push(t);
                    }
                }
                Err(e) => {
                    eprintln!("skip block {block_num}: round {round} exec failed: {:?}", e);
                    failed.insert(block_num);
                }
            }

            visited += 1;
            if visited % progress_every == 0 {
                println!(
                    "round {}/{}: {}/{} blocks, cumulative {:.1}s, failed={} no_block={}",
                    round + 1, rounds, visited, rw_gas_files.len(),
                    overall.elapsed().as_secs_f64(),
                    failed.len(), skipped_no_block.len(),
                );
            }
        }
        println!(
            "round {}/{} done in {:.1}s, cumulative {:.1}s, failed={} no_block={}",
            round + 1, rounds, round_t.elapsed().as_secs_f64(),
            overall.elapsed().as_secs_f64(),
            failed.len(), skipped_no_block.len(),
        );
    }

    // ── Aggregate + write phase ────────────────────────────────────────────
    let write_t = Instant::now();
    let mut processed = 0usize;
    let mut write_failed = 0usize;
    let mut count_mismatch = 0usize;

    for rw_gas_path in &rw_gas_files {
        let stem = rw_gas_path.file_stem().unwrap().to_str().unwrap();
        let block_num: u64 = match stem.trim_start_matches("rw_gas_").parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if failed.contains(&block_num) || skipped_no_block.contains(&block_num) {
            continue;
        }
        if skip_existing
            && rw_time_dir.join(format!("rw_time_{block_num}.json")).exists()
        {
            continue;
        }
        let Some(samples_per_tx) = samples.get(&block_num) else { continue; };
        if samples_per_tx.iter().any(|s| s.len() != rounds) {
            eprintln!("skip block {block_num}: incomplete sample count");
            write_failed += 1;
            continue;
        }

        // Re-load just to get caller addresses for the `from` field.
        let block_path = blocks_dir.join(format!("block_{block_num}.json"));
        let (_, _, txenvs) = match load_block_for_execution(block_path.to_str().unwrap(), false) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("skip block {block_num}: write-phase load failed: {e}");
                write_failed += 1;
                continue;
            }
        };

        let content = match fs::read_to_string(rw_gas_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skip block {block_num}: rw_gas read failed: {e}");
                write_failed += 1;
                continue;
            }
        };
        let rw_entries: Vec<serde_json::Value> = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("skip block {block_num}: rw_gas parse failed: {e}");
                write_failed += 1;
                continue;
            }
        };
        if rw_entries.len() != txenvs.len() {
            eprintln!(
                "skip block {block_num}: rw_gas count ({}) != txenv count ({})",
                rw_entries.len(), txenvs.len()
            );
            count_mismatch += 1;
            continue;
        }

        // Trimmed mean per tx.
        let mut trimmed_means: Vec<u64> = Vec::with_capacity(samples_per_tx.len());
        for s_ref in samples_per_tx.iter() {
            let mut s = s_ref.clone();
            s.sort_unstable();
            let kept = &s[trim..s.len() - trim];
            trimmed_means.push(kept.iter().sum::<u64>() / kept.len() as u64);
        }

        let mut out_entries: Vec<serde_json::Value> = Vec::with_capacity(txenvs.len());
        for ((entry, &execution_time_ns), txenv) in
            rw_entries.iter().zip(trimmed_means.iter()).zip(txenvs.iter())
        {
            let mut out = entry.clone();
            let obj = out.as_object_mut().unwrap();
            obj.remove("gasUsed");
            obj.insert("executionTime".to_string(), serde_json::json!(execution_time_ns));
            obj.insert("from".to_string(), serde_json::json!(format!("{:?}", txenv.caller)));
            out_entries.push(out);
        }
        let out_path = rw_time_dir.join(format!("rw_time_{block_num}.json"));
        if let Err(e) = fs::write(&out_path, serde_json::to_string_pretty(&out_entries).unwrap()) {
            eprintln!("skip block {block_num}: write failed: {e}");
            write_failed += 1;
            continue;
        }
        processed += 1;
    }

    println!(
        "\n✓ generated {} rw_time files | write phase {:.1}s | total {:.1}s | skipped: {} failed_during_rounds, {} no_block, {} write_phase_failed, {} count_mismatch",
        processed, write_t.elapsed().as_secs_f64(), overall.elapsed().as_secs_f64(),
        failed.len(), skipped_no_block.len(), write_failed, count_mismatch,
    );
}

#[test]
fn test_calibrate_sload_ns() {
    use pevm::{execute_revm_sequential_timed, Bytecodes, ChainState, EvmAccount};
    use revm::primitives::{Address, TransactTo};
    use tx_simulator::contract_v2::TxSimulatorV2;

    // Build a minimal storage with TxSimulatorV2 and one caller per target level.
    const TARGETS: &[u64] = &[1, 10, 50, 100, 500, 1000];
    const SAMPLES: usize = 300;

    let simulator_address = Address::new(rand::random());
    let simulator_account = TxSimulatorV2::build();
    let caller = Address::new(rand::random());

    let mut state: ChainState = [(simulator_address, simulator_account)].into_iter().collect();
    state.insert(caller, EvmAccount { balance: U256::from(u128::MAX), ..Default::default() });

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let read_slot = B256::from([0x42u8; 32]);

    println!("\n=== TxSimulatorV2 calibration (execute_revm_sequential_timed) ===");
    println!("{:<10} {:>10} {:>10} {:>10}  ({SAMPLES} samples each)", "target", "p10 ns", "p50 ns", "p90 ns");
    println!("{}", "-".repeat(50));

    let mut points: Vec<(f64, f64)> = Vec::new(); // (target, median_ns)

    for &target in TARGETS {
        let cold_gas = read_slot.len() as u64 * 2100;
        let gas_limit = 21_000u64 + cold_gas + target * 100 * tx_simulator::GAS_MULTIPLIER;
        let calldata = TxSimulatorV2::encode_execute(&[read_slot], &[], target);
        let tx = TxEnv {
            caller,
            gas_limit,
            gas_price: U256::from(1),
            transact_to: TransactTo::Call(simulator_address),
            data: calldata,
            nonce: Some(0),
            ..TxEnv::default()
        };

        let mut times: Vec<u64> = (0..SAMPLES).map(|_| {
            let (_, ns) = execute_revm_sequential_timed(
                &chain, &storage, spec_id, BlockEnv::default(), vec![tx.clone()],
            ).unwrap();
            ns[0]
        }).collect();
        times.sort_unstable();

        let p10 = times[SAMPLES / 10];
        let p50 = times[SAMPLES / 2];
        let p90 = times[9 * SAMPLES / 10];
        println!("{:<10} {:>10} {:>10} {:>10}", target, p10, p50, p90);
        points.push((target as f64, p50 as f64));
    }

    // Least-squares linear fit: t = T_OVERHEAD + target * T_SLOAD
    let n = points.len() as f64;
    let sum_x: f64 = points.iter().map(|(x, _)| x).sum();
    let sum_y: f64 = points.iter().map(|(_, y)| y).sum();
    let sum_xx: f64 = points.iter().map(|(x, _)| x * x).sum();
    let sum_xy: f64 = points.iter().map(|(x, y)| x * y).sum();
    let t_sload = (n * sum_xy - sum_x * sum_y) / (n * sum_xx - sum_x * sum_x);
    let t_overhead = (sum_y - t_sload * sum_x) / n;

    println!("\nFitted (least-squares):");
    println!("  T_SLOAD_NS   = {:.0} ns", t_sload);
    println!("  T_OVERHEAD_NS= {:.0} ns", t_overhead);
    println!("\nUpdate these constants in tests/tx_simulator/mod.rs");
}

#[test]
fn test_calibrate_sload_vs_sstore_ns() {
    use revm::primitives::TransactTo;

    let (state, bytecodes, simulator_address, accounts) =
        tx_simulator::build_storage_n(2);
    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;

    const N: usize = 500;

    // 1 read, 0 writes, gasUsed=100 → target=1 → 1 SLOAD
    let key = B256::from([0x42u8; 32]);
    let tx_sload = TxEnv {
        caller: accounts[0],
        gas_limit: 100 * tx_simulator::GAS_MULTIPLIER,
        gas_price: U256::from(1),
        transact_to: TransactTo::Call(simulator_address),
        data: tx_simulator::contract::TxSimulator::encode_execute(&[key], &[], 100),
        nonce: Some(0),
        ..TxEnv::default()
    };

    // 0 reads, 1 write, gasUsed=100 → target=1 → 1 SSTORE
    let tx_sstore = TxEnv {
        caller: accounts[1],
        gas_limit: 100 * tx_simulator::GAS_MULTIPLIER,
        gas_price: U256::from(1),
        transact_to: TransactTo::Call(simulator_address),
        data: tx_simulator::contract::TxSimulator::encode_execute(&[], &[key], 100),
        nonce: Some(0),
        ..TxEnv::default()
    };

    let mut times_sload: Vec<u64> = (0..N).map(|_| {
        let t = Instant::now();
        let _ = execute_revm_sequential(&chain, &storage, spec_id, BlockEnv::default(), vec![tx_sload.clone()]);
        t.elapsed().as_nanos() as u64
    }).collect();

    let mut times_sstore: Vec<u64> = (0..N).map(|_| {
        let t = Instant::now();
        let _ = execute_revm_sequential(&chain, &storage, spec_id, BlockEnv::default(), vec![tx_sstore.clone()]);
        t.elapsed().as_nanos() as u64
    }).collect();

    times_sload.sort();
    times_sstore.sort();

    println!("1-op tx timing over {N} samples (ns):");
    println!("{:<10} {:>10} {:>10} {:>10}", "op", "p10", "median", "p90");
    println!("{}", "-".repeat(42));
    println!("{:<10} {:>10} {:>10} {:>10}", "SLOAD",
        times_sload[N/10], times_sload[N/2], times_sload[9*N/10]);
    println!("{:<10} {:>10} {:>10} {:>10}", "SSTORE",
        times_sstore[N/10], times_sstore[N/2], times_sstore[9*N/10]);
}

/// Throughput comparison using TxSimulatorV2 loaded from rw_time files.
/// The target loop count is derived from real measured execution times
/// (executionTime_ns / T_SLOAD_NS) rather than gasUsed / 100.
#[test]
fn test_throughput_comparison_v2() {
    let num_blocks: usize = std::env::var("NUM_BLOCKS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let greedy_batch: usize = std::env::var("GREEDY_BATCH")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let rw_time_dir = std::env::var("RW_TIME_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/rw_gas".to_string());

    let (state, bytecodes, _addr, blocks_txs) =
        tx_simulator::load_n_rw_time_blocks_with_callers(&rw_time_dir, num_blocks)
            .expect("failed to load rw_time blocks with callers");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = NonZeroUsize::new(
        std::env::var("NUM_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(8),
    )
    .expect("NUM_THREADS must be > 0");
    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

    // ── Pre-computation (not timed) ──────────────────────────────────────────
    let mut prep_storage = base_storage.clone();
    let mut dep_graphs = Vec::new();
    let mut reordered_blocks = Vec::new();

    for (i, txs) in blocks_txs.iter().enumerate() {
        let t = Instant::now();
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &prep_storage, spec_id, BlockEnv::default(), txs.clone(), i as u64,
        ).unwrap_or_else(|e| panic!("graph build failed on block {i}: {e:?}"));
        let exec_ms = t.elapsed().as_millis();
        update_storage_with_results(&mut prep_storage, results);
        let (reordered, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());
        let total_ms = t.elapsed().as_millis();
        println!("block {i:3}: {} txns, exec={exec_ms}ms, total={total_ms}ms", txs.len());
        dep_graphs.push(new_graph);
        reordered_blocks.push(reordered);
    }

    let tau_cv: f64 = std::env::var("TAU_CV")
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| GreedyIntegratorConfig::default().tau_cv);
    println!("V2 tau_cv = {}", tau_cv);
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        num_threads: concurrency.get(),
        tau_cv,
        ..GreedyIntegratorConfig::default()
    });
    let mut integrated_txns: Vec<Vec<revm::primitives::TxEnv>> = Vec::new();
    let mut integrated_graphs = Vec::new();
    for chunk_start in (0..num_blocks).step_by(greedy_batch) {
        let chunk_end = (chunk_start + greedy_batch).min(num_blocks);
        let (itxns, igraphs) = integrator.integrate_pevm_graphs(
            dep_graphs[chunk_start..chunk_end].to_vec(),
            reordered_blocks[chunk_start..chunk_end].to_vec(),
        );
        integrated_txns.extend(itxns);
        integrated_graphs.extend(igraphs);
    }
    // Greedy integration reorders txs across block boundaries, so sequential
    // per-block nonces are no longer valid.  Re-assign nonces in merged-block
    // order so same-sender dependencies are preserved correctly.
    {
        let mut greedy_nonce_tracker = pevm::utils::nonce_tracker::NonceTracker::new();
        for txs in &mut integrated_txns {
            greedy_nonce_tracker.update_txenv_nonces(txs);
        }
    }

    println!(
        "\n=== Throughput comparison V2 (rw_time, caller-aware): {num_blocks} blocks, {total_txs} txns, {} threads ===",
        concurrency
    );
    println!("{:<38} {:>10} {:>12} {:>10}", "Mode", "Time (ms)", "Txns/s", "Speedup");
    println!("{}", "-".repeat(73));

    let gas_sum = |results: &[pevm::PevmTxExecutionResult]| -> u64 {
        results.last().map(|r| r.receipt.cumulative_gas_used).unwrap_or(0)
    };

    // ── 1. Sequential ─────────────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let mut seq_gas: u64 = 0;
    let t = Instant::now();
    for txs in &blocks_txs {
        let r = execute_revm_sequential(&chain, &s, spec_id, BlockEnv::default(), txs.clone()).unwrap();
        seq_gas += gas_sum(&r);
        update_storage_with_results(&mut s, r);
    }
    let seq_ms = t.elapsed().as_secs_f64() * 1000.0;
    let seq_tput = total_txs as f64 / (seq_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>10}  gas={}", "Sequential", seq_ms, seq_tput, "1.00x", seq_gas);

    // ── 2. Pevm parallel ──────────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let mut par_gas: u64 = 0;
    let t = Instant::now();
    for txs in &blocks_txs {
        let r = Pevm::default().execute_revm_parallel(
            &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency,
        ).unwrap();
        par_gas += gas_sum(&r);
        update_storage_with_results(&mut s, r);
    }
    let par_ms = t.elapsed().as_secs_f64() * 1000.0;
    let par_tput = total_txs as f64 / (par_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x  gas={} {}",
        "Pevm parallel", par_ms, par_tput, par_tput / seq_tput,
        par_gas, if par_gas == seq_gas { "✓" } else { "✗ MISMATCH" });

    // ── 3. GraphPevm parallel ─────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let mut graph_gas: u64 = 0;
    let t = Instant::now();
    for (txs, graph) in reordered_blocks.iter().zip(dep_graphs.iter()) {
        let r = GraphPevm::default().execute_revm_parallel(
            &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
        ).unwrap();
        graph_gas += gas_sum(&r);
        update_storage_with_results(&mut s, r);
    }
    let graph_ms = t.elapsed().as_secs_f64() * 1000.0;
    let graph_tput = total_txs as f64 / (graph_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x  gas={} {}",
        "GraphPevm parallel", graph_ms, graph_tput, graph_tput / seq_tput,
        graph_gas, if graph_gas == seq_gas { "✓" } else { "✗ MISMATCH" });

    // ── 4. Greedy + GraphPevm ─────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let mut greedy_gas: u64 = 0;
    let t = Instant::now();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        let r = GraphPevm::default().execute_revm_parallel(
            &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
        ).unwrap();
        greedy_gas += gas_sum(&r);
        update_storage_with_results(&mut s, r);
    }
    let greedy_ms = t.elapsed().as_secs_f64() * 1000.0;
    let greedy_tput = total_txs as f64 / (greedy_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x  gas={} {}",
        format!("Greedy(batch={greedy_batch}) + GraphPevm"), greedy_ms, greedy_tput, greedy_tput / seq_tput,
        greedy_gas, if greedy_gas == seq_gas { "✓" } else { "✗ MISMATCH" });

    println!("{}", "-".repeat(73));
    println!("(greedy: {} groups from {num_blocks} blocks)", integrated_txns.len());
}

/// Compare incarnation-0 access sets vs sequential for synthetic (rw_time) workload.
#[test]
fn test_synthetic_incarnation0_divergence() {
    let num_blocks: usize = std::env::var("NUM_BLOCKS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    const RW_TIME_DIR: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_time";

    let (state, bytecodes, _addr, blocks_txs) =
        tx_simulator::load_n_rw_time_blocks(RW_TIME_DIR, num_blocks).expect("load failed");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = NonZeroUsize::new(8).unwrap();

    let (mut total_txs, mut read_div, mut write_div, mut either_div) = (0usize, 0usize, 0usize, 0usize);
    let mut storage = base_storage.clone();

    for (blk_i, txs) in blocks_txs.iter().enumerate() {
        let n = txs.len();

        let (seq_results, seq_access) = pevm::execute_revm_sequential_with_access_sets(
            &chain, &storage, spec_id, BlockEnv::default(), txs.clone(),
        ).unwrap();

        let mut pevm_inst = pevm::Pevm::default();
        let par_results = pevm_inst.execute_revm_parallel(
            &chain, &storage, spec_id, BlockEnv::default(), txs.clone(), concurrency,
        ).unwrap();

        // Use sequential results to advance storage
        pevm::api::update_storage_with_results(&mut storage, seq_results);

        let inc0 = &pevm_inst.last_incarnation0_keys;
        let (mut rd, mut wd, mut ei) = (0, 0, 0);
        for (seq_a, par_opt) in seq_access.iter().zip(inc0.iter()) {
            let Some((par_read, par_write)) = par_opt else { continue };
            let r = seq_a.read_set != *par_read;
            let w = seq_a.write_set != *par_write;
            if r { rd += 1; }
            if w { wd += 1; }
            if r || w { ei += 1; }
        }
        println!("Block {:3}: {} txs  read_div={}/{} ({:.0}%)  write_div={}/{} ({:.0}%)  either={}/{} ({:.0}%)",
            blk_i, n, rd, n, rd as f64/n as f64*100.0, wd, n, wd as f64/n as f64*100.0, ei, n, ei as f64/n as f64*100.0);
        total_txs += n; read_div += rd; write_div += wd; either_div += ei;
        let _ = par_results;
    }

    println!("\n=== Synthetic Incarnation-0 Divergence ({} blocks, {} txs) ===", num_blocks, total_txs);
    println!("  Read set diverged:  {}/{} = {:.1}%", read_div, total_txs, read_div as f64/total_txs as f64*100.0);
    println!("  Write set diverged: {}/{} = {:.1}%", write_div, total_txs, write_div as f64/total_txs as f64*100.0);
    println!("  Either diverged:    {}/{} = {:.1}%", either_div, total_txs, either_div as f64/total_txs as f64*100.0);
}

#[test]
fn test_v2_single_block() {
    const RW_TIME_DIR: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_time";
    let (state, bytecodes, _addr, blocks_txs) =
        tx_simulator::load_n_rw_time_blocks(RW_TIME_DIR, 1).expect("load failed");

    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();

    let txs = &blocks_txs[0];
    println!("block[0]: {} txns, gas_limits: min={} max={}",
        txs.len(),
        txs.iter().map(|t| t.gas_limit).min().unwrap(),
        txs.iter().map(|t| t.gas_limit).max().unwrap(),
    );

    let results = execute_revm_sequential(
        &chain, &storage, SpecId::LATEST, BlockEnv::default(), txs.clone(),
    ).unwrap();

    let ok  = results.iter().filter(|r| r.receipt.status.coerce_status()).count();
    let fail = results.len() - ok;
    let gas: u64 = results.iter().map(|r| r.receipt.cumulative_gas_used).sum();
    println!("success={ok}  fail={fail}  total_gas={gas}");
    println!("first tx gas_used={}", results[0].receipt.cumulative_gas_used);
}

#[test]
fn test_v2_per_tx_timing() {
    const RW_TIME_DIR: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_time";
    let (state, bytecodes, _addr, blocks_txs) =
        tx_simulator::load_n_rw_time_blocks(RW_TIME_DIR, 1).expect("load failed");

    let mut storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();

    let txs = &blocks_txs[0];
    let mut times_us: Vec<f64> = Vec::new();
    let mut gas_used_vec: Vec<u64> = Vec::new();

    for tx in txs {
        let start = Instant::now();
        let results = execute_revm_sequential(
            &chain, &storage, SpecId::LATEST, BlockEnv::default(), vec![tx.clone()],
        ).unwrap();
        let us = start.elapsed().as_secs_f64() * 1e6;
        times_us.push(us);
        gas_used_vec.push(results[0].receipt.cumulative_gas_used);
        update_storage_with_results(&mut storage, results);
    }

    let n = times_us.len();
    let mut sorted = times_us.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    println!("V2 per-tx timing ({n} txns):");
    println!("  p10={:.0}µs  p50={:.0}µs  p90={:.0}µs  max={:.0}µs",
        sorted[n/10], sorted[n/2], sorted[9*n/10], sorted[n-1]);
    println!("First 5 txns: time(µs) | gas_used | gas_limit");
    for i in 0..5.min(n) {
        println!("  tx[{i}]: {:.0}µs | {} gas | {} limit",
            times_us[i], gas_used_vec[i], txs[i].gas_limit);
    }
}

#[test]
fn test_v1_first_tx_gas() {
    // Compare V1 gas usage for the same tx as V2
    let (state, bytecodes, blocks_txs) =
        tx_simulator::load_rw_gas_blocks(RW_GAS_DIR).expect("load failed");
    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let txs = &blocks_txs[0];

    let results = execute_revm_sequential(
        &chain, &storage, SpecId::LATEST, BlockEnv::default(), txs[..1].to_vec(),
    ).unwrap();
    println!("V1 tx[0]: gas_used={} gas_limit={}",
        results[0].receipt.cumulative_gas_used, txs[0].gas_limit);
}

/// Verify that `execute_revm_sequential_with_access_sets` correctly builds read/write sets.
///
/// Note: TxSimulatorV2's `reads` array is dead code (Solidity optimizer eliminates the SLOAD
/// because the result is unused). Only `writes` produce real storage operations.
///
/// Two transactions:
///   tx_a: writes=[k1, k2]  → SSTOREs k1 and k2 (each SSTORE first SLOADs the original value)
///   tx_b: writes=[k2, k3]  → SSTOREs k2 and k3
///
/// Verified invariants:
///   1. write_set ⊆ read_set  (SSTORE always reads before writing)
///   2. Each write_set has ≥2 entries (the 2 storage slots + Basic account locations)
///   3. tx_a.write_set ∩ tx_b.write_set is non-empty  (WW conflict on k2)
#[test]
fn test_access_sets_read_write_distinction() {
    use tx_simulator::contract_v2::TxSimulatorV2;
    use std::sync::Arc;

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;

    let simulator_address = revm::primitives::Address::new(rand::random());
    let caller_a = revm::primitives::Address::new(rand::random());
    let caller_b = revm::primitives::Address::new(rand::random());

    let simulator_account = TxSimulatorV2::build();
    let eoa = pevm::EvmAccount {
        balance: U256::from(u128::MAX),
        nonce: 0,
        ..pevm::EvmAccount::default()
    };

    let state: pevm::ChainState = [
        (simulator_address, simulator_account),
        (caller_a, eoa.clone()),
        (caller_b, eoa.clone()),
    ]
    .into_iter()
    .collect();

    let mut bytecodes = pevm::Bytecodes::default();
    let mut state_no_code = state.clone();
    for account in state_no_code.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }
    let storage = InMemoryStorage::new(state_no_code, Arc::new(bytecodes), Default::default());

    let k1 = B256::from([1u8; 32]);
    let k2 = B256::from([2u8; 32]);
    let k3 = B256::from([3u8; 32]);

    // 2 cold SSTOREs = 2 * 22100 gas + base tx + margin
    let gas = 21_000 + 2 * 22_100 + 50_000;

    let tx_a = TxEnv {
        caller: caller_a,
        gas_limit: gas,
        gas_price: U256::from(1),
        transact_to: TransactTo::Call(simulator_address),
        data: TxSimulatorV2::encode_execute(&[], &[k1, k2], 2),
        nonce: Some(0),
        ..TxEnv::default()
    };
    let tx_b = TxEnv {
        caller: caller_b,
        gas_limit: gas,
        gas_price: U256::from(1),
        transact_to: TransactTo::Call(simulator_address),
        data: TxSimulatorV2::encode_execute(&[], &[k2, k3], 2),
        nonce: Some(0),
        ..TxEnv::default()
    };

    let (results, access_sets) = execute_revm_sequential_with_access_sets(
        &chain,
        &storage,
        spec_id,
        BlockEnv::default(),
        vec![tx_a, tx_b],
    )
    .expect("execution failed");

    for (i, r) in results.iter().enumerate() {
        println!("tx[{}]: gas_used={} status={:?}", i, r.receipt.cumulative_gas_used, r.receipt.status);
    }

    let asets_a = &access_sets[0];
    let asets_b = &access_sets[1];
    println!("tx_a: read_set={}, write_set={}", asets_a.read_set.len(), asets_a.write_set.len());
    println!("tx_b: read_set={}, write_set={}", asets_b.read_set.len(), asets_b.write_set.len());

    // Invariant 1: write_set ⊆ read_set for both txs (SSTORE first reads original value).
    assert!(asets_a.write_set.is_subset(&asets_a.read_set),
        "tx_a: write_set must be ⊆ read_set");
    assert!(asets_b.write_set.is_subset(&asets_b.read_set),
        "tx_b: write_set must be ⊆ read_set");

    // Invariant 2: each tx has at least 2 write_set entries (2 storage slots written).
    assert!(asets_a.write_set.len() >= 2,
        "tx_a: expected ≥2 write_set entries, got {}", asets_a.write_set.len());
    assert!(asets_b.write_set.len() >= 2,
        "tx_b: expected ≥2 write_set entries, got {}", asets_b.write_set.len());

    // Invariant 3: the two txs share at least one write_set entry (k2 → WW conflict).
    let shared: usize = asets_a.write_set.iter()
        .filter(|k| asets_b.write_set.contains(*k))
        .count();
    println!("shared write_set entries (WW conflicts): {}", shared);
    assert!(shared >= 1,
        "tx_a and tx_b should share ≥1 write_set entry (k2), got {}", shared);

    println!("✓ write_set ⊆ read_set");
    println!("✓ WW conflict on k2 detected in write_sets");
}

// ============================================================================
// V2 batched benchmark: parallel to bench_test::test_eth_block_data_all_batches
// but using TxSimulatorV2 with caller-aware rw_time loading. Writes one CSV row
// per 100-block batch with timings and throughputs for each of 4 strategies.
// ============================================================================

#[test]
fn test_v2_all_batches() {
    use std::io::Write;
    use std::time::Instant;
    use pevm::api::update_storage_with_results;
    use pevm::graph_pevm::GraphPevm;
    use pevm::greedy_integrator::{GreedyIntegrator, GreedyIntegratorConfig};

    let rw_time_dir = std::env::var("RW_TIME_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/rw_time".to_string());
    let output_path = std::env::var("OUTPUT").unwrap_or_else(|_| {
        "/home/ubuntu/congestion_control/pevm/experiments/v2_batches.csv".to_string()
    });
    let batch_size: usize = std::env::var("BATCH_SIZE")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let max_batches: Option<usize> =
        std::env::var("MAX_BATCHES").ok().and_then(|v| v.parse().ok());
    let greedy_batch: usize = std::env::var("GREEDY_BATCH")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let concurrency = NonZeroUsize::new(
        std::env::var("NUM_THREADS")
            .ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(16),
    ).expect("NUM_THREADS must be > 0");
    // Honor TAU_CV / HOT_KEY_THRESHOLD so the v2 sweep matches the real sweep.
    let tau_cv: f64 = std::env::var("TAU_CV")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(0.5);
    let hot_key_threshold: f64 = std::env::var("HOT_KEY_THRESHOLD")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(1.5);

    // Discover all rw_time files.
    let mut all_files: Vec<_> = std::fs::read_dir(&rw_time_dir)
        .expect("rw_time_dir not readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    all_files.sort();
    assert!(!all_files.is_empty(), "no rw_time_*.json in {}", rw_time_dir);

    let total_files = all_files.len();
    let total_batches = (total_files + batch_size - 1) / batch_size;
    println!(
        "rw_time_dir={} | files={} batch_size={} batches={} concurrency={} greedy_batch={}",
        rw_time_dir, total_files, batch_size, total_batches,
        concurrency.get(), greedy_batch,
    );
    println!("Writing CSV → {}", output_path);

    let mut writer = std::io::BufWriter::new(
        std::fs::File::create(&output_path).expect("cannot create output"),
    );
    writeln!(writer,
        "batch_idx,start_block,end_block,num_blocks,num_txs,\
         seq_time_s,seq_tput,par_time_s,par_tput,\
         graph_time_s,graph_tput,integrated_time_s,integrated_tput,\
         num_integrated_groups").unwrap();
    writer.flush().unwrap();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let overall = Instant::now();

    // Helper to extract block number from rw_time_<N>.json or rw_gas_<N>.json.
    let block_num_of = |p: &std::path::Path| -> u64 {
        let stem = p.file_stem().unwrap().to_str().unwrap();
        stem.trim_start_matches("rw_time_")
            .trim_start_matches("rw_gas_")
            .parse::<u64>().unwrap()
    };

    for (batch_idx, chunk) in all_files.chunks(batch_size).enumerate() {
        if let Some(lim) = max_batches { if batch_idx >= lim { break; } }
        let start_block = block_num_of(&chunk[0]);
        let end_block = block_num_of(&chunk[chunk.len() - 1]);
        let bt = Instant::now();
        println!("\n=== Batch {}/{}: blocks {}..={} ({} files) ===",
            batch_idx + 1, total_batches, start_block, end_block, chunk.len());

        // Use START_INDEX env var so the loader picks exactly this slice.
        let start_idx_str = (batch_idx * batch_size).to_string();
        std::env::set_var("START_INDEX", &start_idx_str);

        let (state, bytecodes, _addr, blocks_txs) =
            match tx_simulator::load_n_rw_time_blocks_with_callers(&rw_time_dir, chunk.len()) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("  batch {} load failed: {e}", batch_idx);
                    writeln!(writer, "{},{},{},{},,,,,,,,,,",
                        batch_idx, start_block, end_block, chunk.len()).unwrap();
                    writer.flush().unwrap();
                    continue;
                }
            };
        let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
        let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

        // ── Pre-compute dependency graphs (untimed; same as test_throughput_comparison_v2) ──
        let mut prep_storage = base_storage.clone();
        let mut dep_graphs = Vec::with_capacity(blocks_txs.len());
        let mut reordered_blocks = Vec::with_capacity(blocks_txs.len());
        for (i, txs) in blocks_txs.iter().enumerate() {
            let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
                &chain, &prep_storage, spec_id, BlockEnv::default(), txs.clone(), i as u64,
            ).unwrap_or_else(|e| panic!("batch {batch_idx} block {i} graph build failed: {e:?}"));
            update_storage_with_results(&mut prep_storage, results);
            let (reordered, new_graph) =
                GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());
            dep_graphs.push(new_graph);
            reordered_blocks.push(reordered);
        }
        // Apply hot_key_threshold before greedy simulates each graph (mirrors
        // the real all-batches test); default 1.5 = TransactionGraph::new default.
        for g in &mut dep_graphs {
            g.set_hot_key_threshold(hot_key_threshold);
        }
        let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
            num_threads: concurrency.get(),
            tau_cv,
        });
        let mut integrated_txns: Vec<Vec<revm::primitives::TxEnv>> = Vec::new();
        let mut integrated_graphs = Vec::new();
        for chunk_start in (0..blocks_txs.len()).step_by(greedy_batch) {
            let chunk_end = (chunk_start + greedy_batch).min(blocks_txs.len());
            let (itxns, igraphs) = integrator.integrate_pevm_graphs(
                dep_graphs[chunk_start..chunk_end].to_vec(),
                reordered_blocks[chunk_start..chunk_end].to_vec(),
            );
            integrated_txns.extend(itxns);
            integrated_graphs.extend(igraphs);
        }
        let num_integrated_groups = integrated_txns.len();
        // Greedy reorders across block boundaries → re-assign nonces.
        let mut greedy_nonce_tracker = pevm::utils::nonce_tracker::NonceTracker::new();
        for txs in &mut integrated_txns {
            greedy_nonce_tracker.update_txenv_nonces(txs);
        }

        // ── 1. Sequential ───────────────────────────────────────────────────
        let mut s = base_storage.clone();
        let t = Instant::now();
        for txs in &blocks_txs {
            let r = execute_revm_sequential(&chain, &s, spec_id, BlockEnv::default(), txs.clone()).unwrap();
            update_storage_with_results(&mut s, r);
        }
        let seq_time_s = t.elapsed().as_secs_f64();
        let seq_tput = total_txs as f64 / seq_time_s;

        // ── 2. Pevm parallel (Block-STM) ────────────────────────────────────
        let mut s = base_storage.clone();
        let t = Instant::now();
        for txs in &blocks_txs {
            let r = Pevm::default().execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency,
            ).unwrap();
            update_storage_with_results(&mut s, r);
        }
        let par_time_s = t.elapsed().as_secs_f64();
        let par_tput = total_txs as f64 / par_time_s;

        // ── 3. GraphPevm parallel ───────────────────────────────────────────
        let mut s = base_storage.clone();
        let t = Instant::now();
        for (txs, graph) in reordered_blocks.iter().zip(dep_graphs.iter()) {
            let r = GraphPevm::default().execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
            ).unwrap();
            update_storage_with_results(&mut s, r);
        }
        let graph_time_s = t.elapsed().as_secs_f64();
        let graph_tput = total_txs as f64 / graph_time_s;

        // ── 4. Greedy + GraphPevm ───────────────────────────────────────────
        let mut s = base_storage.clone();
        let t = Instant::now();
        for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
            let r = GraphPevm::default().execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
            ).unwrap();
            update_storage_with_results(&mut s, r);
        }
        let integrated_time_s = t.elapsed().as_secs_f64();
        let integrated_tput = total_txs as f64 / integrated_time_s;

        println!(
            "  done in {:.2}s | txs={} | seq={:.0}t/s par={:.0} graph={:.0} integ={:.0} groups={} | cumulative {:.1}s",
            bt.elapsed().as_secs_f64(), total_txs,
            seq_tput, par_tput, graph_tput, integrated_tput,
            num_integrated_groups,
            overall.elapsed().as_secs_f64(),
        );
        writeln!(writer,
            "{},{},{},{},{},{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},{}",
            batch_idx, start_block, end_block, chunk.len(), total_txs,
            seq_time_s, seq_tput, par_time_s, par_tput,
            graph_time_s, graph_tput, integrated_time_s, integrated_tput,
            num_integrated_groups,
        ).unwrap();
        writer.flush().unwrap();
    }
    // Unset for cleanliness.
    std::env::remove_var("START_INDEX");

    println!("\n✓ done in {:.1}s | CSV → {}",
        overall.elapsed().as_secs_f64(), output_path);
}
