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
        tau_cv: 0.1,
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
/// transaction, and write the results to rw_time/.
///
/// Uses real prestate + real TxEnvs from blocks/block_XXXXXXX.json so that
/// execution times reflect actual EVM workloads, not TxSimulator loops.
/// txHash is used to match real-tx timing back to the rw_gas entry.
///
/// Output format mirrors rw_gas but with `executionTime` (ns) instead of `gasUsed`.
#[test]
fn test_generate_rw_time() {
    use std::fs;
    use std::path::Path;
    use std::collections::HashMap;
    use pevm::storage::block_loader::{load_block_for_execution, get_spec_id};

    let rw_gas_dir = Path::new(RW_GAS_DIR);
    let blocks_dir = Path::new("/home/ubuntu/eth-block-downloader/test_data/blocks");
    let rw_time_dir = Path::new("/home/ubuntu/eth-block-downloader/test_data/rw_time");
    fs::create_dir_all(rw_time_dir).expect("failed to create rw_time dir");

    let mut rw_gas_files: Vec<_> = fs::read_dir(rw_gas_dir)
        .expect("failed to read rw_gas dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    rw_gas_files.sort();

    let chain = PevmEthereum::mainnet();

    for rw_gas_path in &rw_gas_files {
        // rw_gas_16774645.json → block number 16774645
        let stem = rw_gas_path.file_stem().unwrap().to_str().unwrap();
        let block_num: u64 = stem.trim_start_matches("rw_gas_").parse()
            .unwrap_or_else(|_| panic!("cannot parse block number from {stem}"));

        let block_path = blocks_dir.join(format!("block_{block_num}.json"));
        if !block_path.exists() {
            println!("skipping {block_num}: no block file at {}", block_path.display());
            continue;
        }

        // Load real block: prestate + TxEnvs
        let (block_data, mut storage, txenvs) =
            load_block_for_execution(block_path.to_str().unwrap(), false)
                .unwrap_or_else(|e| panic!("failed to load block {block_num}: {e}"));
        let spec_id = get_spec_id(block_num);
        let block_env = pevm::storage::block_loader::create_block_env(&block_data);

        // Build txHash → measured execution time map by running txns one-by-one.
        let mut hash_to_time_ns: HashMap<String, u64> = HashMap::new();
        for tx in &txenvs {
            // Recover txHash from the original block transactions list using
            // the caller + nonce as a key (TxEnv doesn't carry the hash directly).
            // We time the execution and will match by position after.
            let start = Instant::now();
            let results = execute_revm_sequential(
                &chain, &storage, spec_id, block_env.clone(), vec![tx.clone()],
            ).unwrap_or_else(|e| panic!("block {block_num} tx failed: {e:?}"));
            let elapsed_ns = start.elapsed().as_nanos() as u64;
            update_storage_with_results(&mut storage, results);
            // Use caller+nonce as key (unique per tx within a block)
            let key = format!("{:?}:{}", tx.caller, tx.nonce.unwrap_or(0));
            hash_to_time_ns.insert(key, elapsed_ns);
        }

        // Build the same key from block JSON transactions to look up hash
        let block_txns_owned = block_data.transactions.clone();
        let mut from_nonce_to_hash: HashMap<String, String> = HashMap::new();
        for tx in &block_txns_owned {
            if let (Some(from), Some(nonce), Some(hash)) = (
                tx.get("from").and_then(|v| v.as_str()),
                tx.get("nonce").and_then(|v| v.as_u64()),
                tx.get("hash").and_then(|v| v.as_str()),
            ) {
                let key = format!("{}:{}", from.to_lowercase(), nonce);
                from_nonce_to_hash.insert(key, hash.to_string());
            }
        }

        // Load rw_gas entries and match by txHash
        let content = fs::read_to_string(rw_gas_path).expect("read failed");
        let rw_entries: Vec<serde_json::Value> = serde_json::from_str(&content).expect("parse failed");

        let mut out_entries: Vec<serde_json::Value> = Vec::with_capacity(rw_entries.len());
        let mut matched = 0usize;

        for entry in &rw_entries {
            let tx_hash = entry["txHash"].as_str().unwrap_or("").to_lowercase();

            // Find the from+nonce key for this txHash
            let execution_time_ns = from_nonce_to_hash
                .iter()
                .find(|(_, h)| h.to_lowercase() == tx_hash)
                .and_then(|(key, _)| {
                    // key is "from:nonce", translate to the TxEnv key format
                    // TxEnv caller is Address, printed as 0x... in debug
                    // We stored key as "{:?}:nonce" which is "0xABCD...:N"
                    // Block key is "0xabcd...:N" (lowercase)
                    hash_to_time_ns.iter()
                        .find(|(k, _)| k.to_lowercase() == key.to_lowercase())
                        .map(|(_, &t)| t)
                })
                .unwrap_or_else(|| {
                    // Fallback: match by position if hash lookup fails
                    let idx = out_entries.len();
                    if idx < txenvs.len() {
                        let key = format!("{:?}:{}", txenvs[idx].caller, txenvs[idx].nonce.unwrap_or(0));
                        *hash_to_time_ns.get(&key).unwrap_or(&0)
                    } else { 0 }
                });

            if execution_time_ns > 0 { matched += 1; }

            let mut out = entry.clone();
            let obj = out.as_object_mut().unwrap();
            obj.remove("gasUsed");
            obj.insert("executionTime".to_string(), serde_json::json!(execution_time_ns));
            out_entries.push(out);
        }

        let out_stem = stem.replacen("rw_gas_", "rw_time_", 1);
        let out_path = rw_time_dir.join(format!("{out_stem}.json"));
        fs::write(&out_path, serde_json::to_string_pretty(&out_entries).unwrap())
            .expect("write failed");

        println!("wrote {} ({} txns, {matched} matched)", out_path.display(), out_entries.len());
    }
}

#[test]
fn test_calibrate_sload_ns() {
    use revm::primitives::TransactTo;

    let (state, bytecodes, simulator_address, accounts) =
        tx_simulator::build_storage_n(1);
    let storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;

    // target = 1: exactly one hot SLOAD
    let one_read = [B256::from([0x42u8; 32])];
    let calldata = tx_simulator::contract::TxSimulator::encode_execute(&one_read, &[], 100);
    let tx = TxEnv {
        caller: accounts[0],
        gas_limit: 100 * tx_simulator::GAS_MULTIPLIER,
        gas_price: U256::from(1),
        transact_to: TransactTo::Call(simulator_address),
        data: calldata,
        nonce: Some(0),
        ..TxEnv::default()
    };

    const N: usize = 500;
    let mut times: Vec<u64> = (0..N).map(|_| {
        let start = Instant::now();
        let _ = execute_revm_sequential(
            &chain, &storage, spec_id, BlockEnv::default(), vec![tx.clone()],
        );
        start.elapsed().as_nanos() as u64
    }).collect();

    times.sort();
    let median = times[N / 2];
    let p10   = times[N / 10];
    let p90   = times[9 * N / 10];

    println!("1-SLOAD tx timing over {N} samples:");
    println!("  p10    = {} ns", p10);
    println!("  median = {} ns  ← t_sload_ns", median);
    println!("  p90    = {} ns", p90);
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
    const NUM_BLOCKS: usize = 10;
    const GREEDY_BATCH: usize = 10;
    const RW_TIME_DIR: &str = "/home/ubuntu/eth-block-downloader/test_data/rw_time";

    let (state, bytecodes, _addr, blocks_txs) =
        tx_simulator::load_n_rw_time_blocks(RW_TIME_DIR, NUM_BLOCKS)
            .expect("failed to load rw_time blocks");

    let base_storage = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = NonZeroUsize::new(8).unwrap();
    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

    // ── Pre-computation (not timed) ──────────────────────────────────────────
    let mut prep_storage = base_storage.clone();
    let mut dep_graphs = Vec::new();
    let mut reordered_blocks = Vec::new();

    for (i, txs) in blocks_txs.iter().enumerate() {
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &prep_storage, spec_id, BlockEnv::default(), txs.clone(), i as u64,
        ).unwrap_or_else(|e| panic!("graph build failed on block {i}: {e:?}"));
        update_storage_with_results(&mut prep_storage, results);
        let (reordered, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());
        dep_graphs.push(new_graph);
        reordered_blocks.push(reordered);
    }

    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        num_threads: concurrency.get(),
        ..GreedyIntegratorConfig::default()
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
        "\n=== Throughput comparison V2 (rw_time): {NUM_BLOCKS} blocks, {total_txs} txns, {} threads ===",
        concurrency
    );
    println!("{:<38} {:>10} {:>12} {:>10}", "Mode", "Time (ms)", "Txns/s", "Speedup");
    println!("{}", "-".repeat(73));

    // ── 1. Sequential ─────────────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let t = Instant::now();
    for txs in &blocks_txs {
        let r = execute_revm_sequential(&chain, &s, spec_id, BlockEnv::default(), txs.clone()).unwrap();
        update_storage_with_results(&mut s, r);
    }
    let seq_ms = t.elapsed().as_secs_f64() * 1000.0;
    let seq_tput = total_txs as f64 / (seq_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>10}", "Sequential", seq_ms, seq_tput, "1.00x");

    // ── 2. Pevm parallel ──────────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let t = Instant::now();
    for txs in &blocks_txs {
        let r = Pevm::default().execute_revm_parallel(
            &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency,
        ).unwrap();
        update_storage_with_results(&mut s, r);
    }
    let par_ms = t.elapsed().as_secs_f64() * 1000.0;
    let par_tput = total_txs as f64 / (par_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x", "Pevm parallel", par_ms, par_tput, par_tput / seq_tput);

    // ── 3. GraphPevm parallel ─────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let t = Instant::now();
    for (txs, graph) in reordered_blocks.iter().zip(dep_graphs.iter()) {
        let r = GraphPevm::default().execute_revm_parallel(
            &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
        ).unwrap();
        update_storage_with_results(&mut s, r);
    }
    let graph_ms = t.elapsed().as_secs_f64() * 1000.0;
    let graph_tput = total_txs as f64 / (graph_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x", "GraphPevm parallel", graph_ms, graph_tput, graph_tput / seq_tput);

    // ── 4. Greedy + GraphPevm ─────────────────────────────────────────────────
    let mut s = base_storage.clone();
    let t = Instant::now();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        let r = GraphPevm::default().execute_revm_parallel(
            &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
        ).unwrap();
        update_storage_with_results(&mut s, r);
    }
    let greedy_ms = t.elapsed().as_secs_f64() * 1000.0;
    let greedy_tput = total_txs as f64 / (greedy_ms / 1000.0);
    println!("{:<38} {:>10.1} {:>12.0} {:>9.2}x",
        format!("Greedy(batch={GREEDY_BATCH}) + GraphPevm"), greedy_ms, greedy_tput, greedy_tput / seq_tput);

    println!("{}", "-".repeat(73));
    println!("(greedy: {} groups from {NUM_BLOCKS} blocks)", integrated_txns.len());
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
