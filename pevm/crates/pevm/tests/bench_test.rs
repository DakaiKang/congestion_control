
use std::{num::NonZeroUsize, sync::Arc};
use std::time::Instant;
use alloy_primitives::{Address, B256, U160, U256};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::distributions::{Distribution, WeightedIndex};
use rand::rngs::ThreadRng;
use rand::{thread_rng};
use pevm::{
    api::update_storage_with_results,
    graph_pevm::GraphPevm,
    chain::PevmEthereum, execute_revm_sequential, Bytecodes, ChainState, EvmAccount,
    InMemoryStorage, Pevm, execute_revm_sequential_with_access_sets
};
use pevm::dependency_graph::{
    TransactionGraph, TransactionNode, SimulationResult
};
use pevm::graph_scheduler::GraphScheduler;
use pevm::greedy_integrator::{GreedyIntegrator, GreedyIntegratorConfig};

use std::collections::HashSet;

use revm::primitives::{AccessListItem, BlockEnv, SpecId, TransactTo, TxEnv};
use std::io::Write;

use pevm::storage::block_loader::{load_block_for_execution, create_multi_block_storage, get_spec_id};
use pevm::utils::nonce_tracker::{NonceTracker};

#[path = "../benches/gigagas.rs"]
pub mod gigagas;

/// Concurrency level for parallel executors / graph simulation.
/// Reads NUM_THREADS env var; falls back to available_parallelism().
fn parallel_concurrency() -> NonZeroUsize {
    std::env::var("NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .and_then(NonZeroUsize::new)
        .unwrap_or_else(|| std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN))
}

#[test]
pub fn test_bench_sequential() -> Result<(), Box<dyn std::error::Error>> {
    println!("Running test_bench");
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
    let txn_num = 100;
    let (storage, txs, costs, txs2, costs2) = gigagas::solana_sample_txns_two_batch(txn_num);

    let txs_clone = txs.clone();
    let txs2_clone = txs2.clone();
    let storage2 = storage.clone();
    let storage_cloned = storage.clone();
    let storage2_cloned = storage.clone();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    let (result, access_set) = pevm::execute_revm_sequential_with_access_sets(
        black_box(&chain),
        black_box(&storage),
        black_box(spec_id),
        black_box(block_env.clone()),
        black_box(txs.clone()),
    )?;

    let (result2, access_set2) = pevm::execute_revm_sequential_with_access_sets(
        black_box(&chain),
        black_box(&storage2),
        black_box(spec_id),
        black_box(block_env.clone()),
        black_box(txs2.clone()),
    )?;

    let mut graph = TransactionGraph::new();
    for i in 0..result.len() {
        let txn_node = TransactionNode::new(i as u64, 1, 1, costs[i], access_set[i].read_set.clone(), access_set[i].write_set.clone());
        graph.add_transaction(txn_node);
    }

    let mut graph2 = TransactionGraph::new();
    for i in 0..result2.len() {
        let txn_node = TransactionNode::new(i as u64, 1, 1, costs2[i], access_set2[i].read_set.clone(), access_set2[i].write_set.clone());
        graph2.add_transaction(txn_node);
    }

    graph.simulate_parallel_execution(8);
    graph2.simulate_parallel_execution(8);

    let simulated = graph.simulation_result.unwrap();
    let simulated2 = graph2.simulation_result.unwrap();

    println!("simulated: {:#?}", simulated);

    let mut reordered_txns = Vec::new();
    for i in 0..simulated.execution_order.len() {
        let tx_id = simulated.execution_order[i].1.id; // .1 means the 2nd element in the tuple
        reordered_txns.push(txs_clone[tx_id as usize].clone());
    }

    let mut reordered_txns2 = Vec::new();
    for i in 0..simulated2.execution_order.len() {
        let tx_id = simulated2.execution_order[i].1.id; // .1 means the 2nd element in the tuple
        reordered_txns2.push(txs2_clone[tx_id as usize].clone());
    }

    let start = Instant::now();
    let new_result = Pevm::default().execute_revm_parallel(
        &chain,
        &storage_cloned,
        SpecId::LATEST,
        BlockEnv::default(),
        reordered_txns,
        concurrency_level,
    )?;

    let new_result2 = Pevm::default().execute_revm_parallel(
        &chain,
        &storage2_cloned,
        SpecId::LATEST,
        BlockEnv::default(),
        reordered_txns2,
        concurrency_level,
    )?;
    let duration = start.elapsed();
    // println!("Execution time: {:?}", duration);

    // assert_eq!(result, result2);

    // println!("YES");

    // println!("result2 = {:#?}", result2);
    let output_file = std::fs::File::create("bench_test.txt").unwrap();
    let mut writer = std::io::BufWriter::new(output_file);
    let result_str = format!("{result2:#?}");
    // let execution_str = format!("Execution Time: {duration:#?}");
    // output the results to a file
    writer.write_all(result_str.as_bytes()).unwrap();
    writer.flush().unwrap();

    Ok(())
}



#[test]
pub fn test_bench_combine() -> Result<(), Box<dyn std::error::Error>> {
    println!("Running test_bench");
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
    let txn_num = 100;
    let (storage, txs, costs, txs2, costs2) = gigagas::solana_sample_txns_two_batch(txn_num);

    let txs_clone = txs.clone();
    let txs2_clone = txs2.clone();
    let p_storage = storage.clone();
    let p_storage2 = storage.clone();
    let storage2 = storage.clone();
    let storage_cloned = storage.clone();
    let storage2_cloned = storage.clone();
    let storage_reorder = storage.clone();
    let storage_graph = storage.clone();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    let start = Instant::now();
    let new_result = Pevm::default().execute_revm_parallel(
        &chain,
        &p_storage,
        SpecId::LATEST,
        BlockEnv::default(),
        txs.clone(),
        concurrency_level,
    )?;

    let new_result2 = Pevm::default().execute_revm_parallel(
        &chain,
        &p_storage2,
        SpecId::LATEST,
        BlockEnv::default(),
        txs2.clone(),
        concurrency_level,
    )?;
    let duration_p = start.elapsed();

    let start = Instant::now();
    let (result, access_set) = pevm::execute_revm_sequential_with_access_sets(
        black_box(&chain),
        black_box(&storage),
        black_box(spec_id),
        black_box(block_env.clone()),
        black_box(txs.clone()),
    )?;

    let (result2, access_set2) = pevm::execute_revm_sequential_with_access_sets(
        black_box(&chain),
        black_box(&storage2),
        black_box(spec_id),
        black_box(block_env.clone()),
        black_box(txs2.clone()),
    )?;
    let duration0 = start.elapsed();

    let mut graph = TransactionGraph::new();
    for i in 0..result.len() {
        let txn_node = TransactionNode::new(i as u64, 1, 1, costs[i], access_set[i].read_set.clone(), access_set[i].write_set.clone());
        graph.add_transaction(txn_node);
    }

    let mut graph2 = TransactionGraph::new();
    for i in 0..result2.len() {
        let txn_node = TransactionNode::new(i as u64, 2, 1, costs2[i], access_set2[i].read_set.clone(), access_set2[i].write_set.clone());
        graph2.add_transaction(txn_node);
    }

    let mut g1 = graph.clone();
    let mut g2 = graph2.clone();

    graph.simulate_parallel_execution(8);
    graph2.simulate_parallel_execution(8);
    let mut simulated = graph.simulation_result.unwrap();
    let mut simulated2 = graph2.simulation_result.unwrap();

    let mut reordered_txns = Vec::new();
    for i in 0..simulated.execution_order.len() {
        let tx_id = simulated.execution_order[i].1.id; // .1 means the 2nd element in the tuple
        reordered_txns.push(txs_clone[tx_id as usize].clone());
    }

    let mut reordered_txns2 = Vec::new();
    for i in 0..simulated2.execution_order.len() {
        let tx_id = simulated2.execution_order[i].1.id; // .1 means the 2nd element in the tuple
        reordered_txns2.push(txs2_clone[tx_id as usize].clone());
    }

    let start = Instant::now();
    let new_result = Pevm::default().execute_revm_parallel(
        &chain,
        &storage_cloned,
        SpecId::LATEST,
        BlockEnv::default(),
        reordered_txns,
        concurrency_level,
    )?;

    let new_result2 = Pevm::default().execute_revm_parallel(
        &chain,
        &storage2_cloned,
        SpecId::LATEST,
        BlockEnv::default(),
        reordered_txns2,
        concurrency_level,
    )?;
    let duration1 = start.elapsed();

    g1.integrate_graph(&g2);

    g1.simulate_parallel_execution(8);

    let mut simulated = g1.simulation_result.unwrap();

    // println!("simulated: {:#?}", simulated);

    let mut reordered_txns = Vec::new();
    for i in 0..simulated.execution_order.len() {
        let tx_id = simulated.execution_order[i].1.id; 
        let replica =  simulated.execution_order[i].1.replica; 
        if replica == 1 {
            reordered_txns.push(txs_clone[tx_id as usize].clone());
        } else {
            reordered_txns.push(txs2_clone[tx_id as usize].clone());
        }
    }
    println!("num txn: {}", reordered_txns.len());
    let start = Instant::now();
    let new_result = Pevm::default().execute_revm_parallel(
        &chain,
        &storage_reorder,
        SpecId::LATEST,
        BlockEnv::default(),
        reordered_txns,
        concurrency_level,
    )?;
    let duration = start.elapsed();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    // println!("Execution time 0: {:?}", duration0);
    // println!("Execution time p: {:?}", duration_p);
    // println!("Execution time 1: {:?}", duration1);
    // println!("Execution time 2: {:?}", duration);
    println!("{:?}", duration0);
    println!("{:?}", duration_p);
    println!("{:?}", duration1);
    println!("{:?}", duration);

    let output_file = std::fs::File::create("bench_test_combine.txt").unwrap();
    let mut writer = std::io::BufWriter::new(output_file);
    let execution_str = format!("Execution Time: {duration:#?}");
    // output the results to a file
    writer.write_all(execution_str.as_bytes()).unwrap();
    writer.flush().unwrap();

    Ok(())
}

#[test]
pub fn graph_pevm_test() {
    let txn_num = 96;
    // let (storage, txs, costs, txs2, costs2) = gigagas::solana_sample_txns_two_batch(txn_num);
    // let (storage, txs, costs) = gigagas::solana_samples(txn_num, 1); // very dense
    let (storage, txs, costs) = gigagas::solana_samples(txn_num, 2); // 8 sender, not evenly located

    let fist_part_len = txs.len() / 8 * 4;
    let txs_second = txs[fist_part_len..].to_vec();
    let txs_fisrt = txs[..fist_part_len].to_vec();

    let txs_clone1 = txs_fisrt.clone();
    let txs_clone2 = txs_fisrt.clone();
    let txs_clone3 = txs_fisrt.clone();
    let txs2_clone1 = txs_second.clone();
    let txs2_clone2 = txs_second.clone();
    let txs2_clone3 = txs_second.clone();
    let storage_cloned1 = storage.clone();
    let storage_cloned2 = storage.clone();
    let storage_cloned3 = storage.clone();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    // println!("txns: {:#?}", txs);

    let start = Instant::now();
    let result = pevm::execute_revm_sequential(
        &chain,
        &storage_cloned1,
        spec_id,
        block_env.clone(),
        txs_clone1,
    ).unwrap();
    let result = pevm::execute_revm_sequential(
        &chain,
        &storage_cloned1,
        spec_id,
        block_env,
        txs2_clone1,
    ).unwrap();
    let duration1 = start.elapsed();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    let txs1 = txs_fisrt.clone();
    let txs2 = txs_second.clone();
    let storage_3 = storage.clone();
    
    let (mut graph1, r) = GraphPevm::construct_graph_pevm_by_sequential(&chain, &storage, spec_id, block_env.clone(), txs_fisrt, 1).unwrap();
    let (mut graph2, r) = GraphPevm::construct_graph_pevm_by_sequential(&chain, &storage, spec_id, block_env, txs_second, 2).unwrap();

    let mut graph_clone1 = graph1.clone();
    let mut graph_clone2 = graph2.clone();
    //////////////////////////////////
    
    let (reordered_txns, mut new_graph) = GraphPevm::reorder_txs_by_dependency_graph(txs1, &mut graph_clone1, 8);
    let (reordered_txns2, mut new_graph2) = GraphPevm::reorder_txs_by_dependency_graph(txs2, &mut graph_clone2, 8);
    let start = Instant::now();
    let mut pevm = GraphPevm::default();
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();
    
    let result = pevm.execute_revm_parallel(&chain, &storage_3, spec_id, block_env.clone(), reordered_txns, concurrency_level, new_graph).unwrap();
    let result = pevm.execute_revm_parallel(&chain, &storage_3, spec_id, block_env, reordered_txns2, concurrency_level, new_graph2).unwrap();
    let duration3 = start.elapsed();

    ////////////////////////////
    let mut ro_txns = Vec::new();

    let (reordered_txns, mut new_graph) = GraphPevm::reorder_txs_by_dependency_graph(txs_clone2, &mut graph1, 8);
    let (reordered_txns2, mut new_graph2) = GraphPevm::reorder_txs_by_dependency_graph(txs2_clone2, &mut graph2, 8);

    ro_txns.extend(reordered_txns.clone());
    ro_txns.extend(reordered_txns2.clone());

    new_graph.integrate_graph(&new_graph2);

    // new_graph.simulate_parallel_execution(8);

    // let mut simulated = new_graph.simulation_result.clone().unwrap();

    // let mut ro_txns = Vec::new();
    // for i in 0..simulated.execution_order.len() {
    //     let tx_id = simulated.execution_order[i].1.id; 
    //     let replica = simulated.execution_order[i].1.replica; 
    //     if replica == 1 {
    //         ro_txns.push(reordered_txns[tx_id as usize].clone());
    //     } else {
    //         ro_txns.push(reordered_txns2[tx_id as usize].clone());
    //     }
    // }
    
    let (ro_txns, new_graph) = GraphPevm::reorder_txs_by_dependency_graph(ro_txns, &mut new_graph, 8);


    let start = Instant::now();
    let mut pevm = GraphPevm::default();
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();
    
    let result = pevm.execute_revm_parallel(&chain, &storage_cloned2, spec_id, block_env, ro_txns, concurrency_level, new_graph).unwrap();
    let duration4 = start.elapsed();

    // ////////////////////////////
    // let mut pevm = Pevm::default();
    // let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    // let chain = PevmEthereum::mainnet();
    // let spec_id = SpecId::LATEST;
    // let block_env = BlockEnv::default();

    // let start = Instant::now();
    // let result = pevm.execute_revm_parallel(&chain, &storage_cloned3, spec_id, block_env.clone(), txs_clone3, concurrency_level).unwrap();
    // let result = pevm.execute_revm_parallel(&chain, &storage_cloned3, spec_id, block_env, txs2_clone3, concurrency_level).unwrap();
    // let duration2 = start.elapsed();

    // println!("{:?}", duration1);
    // println!("{:?}", duration2);
    // println!("{:?}", duration3);
    // println!("{:?}", duration4);
    
    // println!("result: {:#?}", result);

    ()
}


#[test]
pub fn single_sender_test()  -> Result<(), Box<dyn std::error::Error>>{
    println!("Running single_sender_test");
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
    let txn_num = 100;
    let (storage, txs, costs) = gigagas::solana_single_sender_txns(txn_num);

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    let start = Instant::now();
    let new_result = Pevm::default().execute_revm_parallel(
        &chain,
        &storage,
        SpecId::LATEST,
        BlockEnv::default(),
        txs,
        concurrency_level,
    )?;

    let duration = start.elapsed();
    // println!("Execution time: {:?}", duration);

    // assert_eq!(result, result2);

    // println!("YES");

    // println!("result2 = {:#?}", result2);
    let output_file = std::fs::File::create("bench_test.txt").unwrap();
    let mut writer = std::io::BufWriter::new(output_file);
    let result_str = format!("{new_result:#?}");
    // let execution_str = format!("Execution Time: {duration:#?}");
    // output the results to a file
    writer.write_all(result_str.as_bytes()).unwrap();
    writer.flush().unwrap();

    Ok(())
}


#[test]
pub fn conflict_parameter_sweep() {
    let num_blocks = 100;
    let num_tx_per_block = 500;
    
    // let zipf_thetas = vec![0.01];
    let zipf_thetas = vec![0.99];
    // let zipf_thetas = vec![0.99];
    // let common_access_ratios = vec![0.01, 0.25, 0.5, 0.75, 1.0];
    let common_access_ratios = vec![0.5];
    let unique_hot_ratio = 0.25;
    
    println!("╔═══════════════════════════════════════════════════════════════════╗");
    println!("║           CONFLICT PARAMETER SWEEP EXPERIMENT                     ║");
    println!("╠═══════════════════════════════════════════════════════════════════╣");
    println!("║ Blocks: {}  |  Txs per block: {}                                ║", 
             num_blocks, num_tx_per_block);
    println!("║ Total combinations: {}                                            ║",
             zipf_thetas.len() * common_access_ratios.len());
    println!("╚═══════════════════════════════════════════════════════════════════╝\n");
    
    let mut all_results = Vec::new();
    
    for &theta in &zipf_thetas {
        for &ratio in &common_access_ratios {
            let (seq, par, graph, integrated) = different_conflict_test(
                num_blocks, 
                num_tx_per_block, 
                theta, 
                ratio,
                unique_hot_ratio,
            );
            
            all_results.push((theta, ratio, unique_hot_ratio, seq, par, graph, integrated));
        }
    }
    
    // Summary table
    println!("\n╔═══════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                              FINAL RESULTS SUMMARY                                    ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════════════╣");
    println!("║ Theta │ Ratio │   Sequential │     Parallel │  Graph-Parallel │      Integrated │ Speedup ║");
    println!("╠═══════╪═══════╪══════════════╪══════════════╪═════════════════╪═════════════════╪═════════╣");
    
    for (theta, ratio, seq, par, graph, integrated, speedup) in &all_results {
        let speedup = integrated / seq;
        println!("║ {:>5.2} │ {:>5.2} │ {:>10.2} t/s│ {:>10.2} t/s│ {:>13.2} t/s│ {:>13.2} t/s│ {:>6.2}x ║",
                 theta, ratio, seq, par, graph, integrated, speedup);
    }
    
    println!("╚═══════════════════════════════════════════════════════════════════════════════════════╝");
    
    // Raw data for plotting
    println!("\n=== RAW DATA ===");
    println!("theta ratio unique_hot_ratio seq_tput par_tput graph_tput integrated_tput");
    for (theta, ratio, seq, par, graph, integrated, speedup) in &all_results {
        println!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}", 
                 theta, ratio, unique_hot_ratio, seq, par, graph, integrated);
    }
}

/// Returns (seq_tput, par_tput, graph_par_tput, integrated_tput)
pub fn different_conflict_test(
    num_blocks: usize,
    num_tx_per_block: usize,
    zipf_theta: f64,
    common_access_ratio: f64,
    unique_hot_ratio: f64,
) -> (f64, f64, f64, f64) {
    let (storage, blocks_txs) = gigagas::conflict_workloads(
        num_blocks, 
        num_tx_per_block, 
        zipf_theta, 
        common_access_ratio,
        unique_hot_ratio,
    );
    
    let total_txs = num_blocks * num_tx_per_block;
    
    println!("=== Config: theta={:.2}, ratio={:.2} ===", zipf_theta, common_access_ratio);
    
    let mut seq_tput = 0.0;
    let mut par_tput = 0.0;
    
    let spec_id = SpecId::LATEST;
    // 1. Sequential
    println!("Sequential...");
    let seq_start = Instant::now();
    let mut seq_storage = storage.clone();
    for txs in blocks_txs.clone() {
        seq_storage = execute_sequential_and_update(seq_storage, spec_id, txs);
    }
    let seq_tput = total_txs as f64 / seq_start.elapsed().as_secs_f64();
    
    // 2. Parallel (original)
    println!("Parallel...");
    let par_start = Instant::now();
    let mut par_storage = storage.clone();
    for txs in blocks_txs.clone() {
        par_storage = execute_parallel_and_update(par_storage, spec_id, txs);
    }
    let par_tput = total_txs as f64 / par_start.elapsed().as_secs_f64();
    
    // 3. Generate graphs and parallel with graphs
    println!("Generating graphs...");
    let (_, reordered_blocks_txs, dependency_graphs) = 
        generate_dependency_graphs(storage.clone(), spec_id, blocks_txs);
    
    println!("Parallel with graphs...");
    let graph_par_start = Instant::now();
    let mut graph_par_storage = storage.clone();
    let mut count = 0;
    for (txs, graph) in reordered_blocks_txs.iter().zip(dependency_graphs.iter()) {
        count += 1;
        println!("Processing block {}/{}", count, reordered_blocks_txs.len());
        graph_par_storage = execute_parallel_with_graph_and_update(
            graph_par_storage, 
            txs.clone(),
            graph.clone(),
            SpecId::LATEST,
            BlockEnv::default()
        );
    }
    let graph_par_tput = total_txs as f64 / graph_par_start.elapsed().as_secs_f64();
    let integrated_tput = 0.0;
    // 4. Greedy integration
    println!("Greedy integration...");
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        tau_cv: 0.1,
        num_threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8),
    });
    
    let (integrated_txns, integrated_graphs) = integrator.integrate_pevm_graphs(
        dependency_graphs,
        reordered_blocks_txs,
    );
    
    println!("Parallel with integrated...");
    let integrated_start = Instant::now();
    let mut integrated_storage = storage.clone();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        integrated_storage = execute_parallel_with_graph_and_update(
            integrated_storage,
            txs.clone(),
            graph.clone(),
            SpecId::LATEST,
            BlockEnv::default()
        );
    }
    let integrated_tput = total_txs as f64 / integrated_start.elapsed().as_secs_f64();
    
    println!("Results: {:.2} {:.2} {:.2} {:.2}\n", 
             seq_tput, par_tput, graph_par_tput, integrated_tput);
    
    (seq_tput, par_tput, graph_par_tput, integrated_tput)
}


fn generate_dependency_graphs(
    storage: InMemoryStorage, 
    spec_id: SpecId,
    blocks_txs: Vec<Vec<TxEnv>>
) -> (InMemoryStorage, Vec<Vec<TxEnv>>, Vec<pevm::dependency_graph::TransactionGraph>) {
    
    let mut reordered_txns_list = Vec::new();
    let mut new_graphs = Vec::new();
    let mut current_storage = storage.clone();

    let concurrency_level = parallel_concurrency().get();
    
    for i in 0..blocks_txs.len() {
        // println!("Constructing graph for batch {}", i);
        
        let txs = blocks_txs[i].clone();

        let chain = PevmEthereum::mainnet();
        let block_env = BlockEnv::default();

        // Construct graph using the same storage for all batches
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, 
            &current_storage, 
            spec_id, 
            block_env, 
            txs.clone(), 
            i as u64
        ).unwrap();

        update_storage_with_results(& mut current_storage, results);

        let (reordered_txns, new_graph) = GraphPevm::reorder_txs_by_dependency_graph(
            txs, 
            &mut graph, 
            concurrency_level
        );
        
        reordered_txns_list.push(reordered_txns);
        new_graphs.push(new_graph);
    }
    
    (storage, reordered_txns_list, new_graphs)  // Fixed: return the list, not single item
}

fn make_block_env() -> BlockEnv {
    let mut block_env = BlockEnv::default();
    block_env.gas_limit = U256::from(30_000_000u64);
    block_env.basefee = U256::ZERO;
    block_env
}

fn execute_sequential_and_update(
    mut storage: InMemoryStorage,
    spec_id: SpecId,
    txs: Vec<TxEnv>,
) -> InMemoryStorage {
    let (s, _) = execute_sequential_and_update_with_gas(storage, spec_id, txs);
    s
}

fn execute_sequential_and_update_with_gas(
    mut storage: InMemoryStorage,
    spec_id: SpecId,
    txs: Vec<TxEnv>,
) -> (InMemoryStorage, u64) {
    let chain = PevmEthereum::mainnet();
    let result = pevm::execute_revm_sequential(
        &chain, &storage, spec_id, make_block_env(), txs,
    ).unwrap();
    let gas = result.last().map(|r| r.receipt.cumulative_gas_used).unwrap_or(0);
    update_storage_with_results(&mut storage, result);
    (storage, gas)
}


fn execute_parallel_and_update(
    mut storage: InMemoryStorage,
    spec_id: SpecId,
    txs: Vec<TxEnv>,
) -> InMemoryStorage {
    let chain = PevmEthereum::mainnet();
    let concurrency_level = parallel_concurrency();
    let result = Pevm::default().execute_revm_parallel(
        &chain, &storage, spec_id, make_block_env(), txs, concurrency_level,
    ).unwrap();
    update_storage_with_results(&mut storage, result);
    storage
}


fn execute_parallel_with_graph_and_update(
    mut storage: InMemoryStorage,
    txs: Vec<TxEnv>,
    graph: pevm::dependency_graph::TransactionGraph,
    spec_id: SpecId,
    _block_env: BlockEnv,
) -> InMemoryStorage {
    let (s, _) = execute_parallel_with_graph_and_update_with_gas(storage, txs, graph, spec_id);
    s
}

fn execute_parallel_with_graph_and_update_with_gas(
    mut storage: InMemoryStorage,
    txs: Vec<TxEnv>,
    graph: pevm::dependency_graph::TransactionGraph,
    spec_id: SpecId,
) -> (InMemoryStorage, u64) {
    let chain = PevmEthereum::mainnet();
    let concurrency_level = parallel_concurrency();
    let mut pevm = GraphPevm::default();
    let result = match pevm.execute_revm_parallel(
        &chain, &storage, spec_id, make_block_env(), txs, concurrency_level, graph,
    ) {
        Ok(r) => r,
        Err(e) => {
            println!("  ⚠️  Graph parallel execution failed: {:?}", e);
            return (storage, 0);
        }
    };
    let gas = result.last().map(|r| r.receipt.cumulative_gas_used).unwrap_or(0);
    update_storage_with_results(&mut storage, result);
    (storage, gas)
}


#[test]

pub fn long_test() {
    let txn_num = 400;
    let batch_num = 8;
    let (mut storage, all_batches) = gigagas::solana_samples_vector(txn_num, batch_num, 2);

    let mut storage_history = Vec::new();
    let mut reordered_txns_list = Vec::new();
    let mut new_graphs = Vec::new();
    let mut integrate_graphs = Vec::new();
    let mut integrate_txns = Vec::new();
    let mut integrate_storages = Vec::new();

    for i in 0..batch_num {
        // println!("Constructing graph for batch {}", i);
        storage_history.push(storage.clone());
        let txs = all_batches[i].clone();

        let chain = PevmEthereum::mainnet();
        let spec_id = SpecId::LATEST;
        let block_env = BlockEnv::default();

        let (mut graph, r) = GraphPevm::construct_graph_pevm_by_sequential(&chain, &storage, spec_id, block_env, txs.clone(), i as u64).unwrap();

        update_storage_with_results(&mut storage, r);

        let (reordered_txns, mut new_graph) = GraphPevm::reorder_txs_by_dependency_graph(txs, &mut graph, 8);
        reordered_txns_list.push(reordered_txns);
        new_graphs.push(new_graph);
    }

    let batch_size = 4;

    // Integrate graphs every 'batch_size' graphs
    for i in 0..batch_num/batch_size {
        // let mut id_to_txn = std::collections::HashMap::new();
        let mut reordered_txns = Vec::new();

        for j in 0..batch_size {
            if i * batch_size + j < batch_num {
                reordered_txns.extend(reordered_txns_list[i*batch_size + j].clone());
            }
        }

        let mut integrated_graph = new_graphs[i * batch_size].clone();
        for j in 1..batch_size {
            if i * batch_size + j < batch_num {
                integrated_graph.integrate_graph(&new_graphs[i * batch_size + j]);
            }
        }
        println!("nodes count in integrated graph: {}", integrated_graph.nodes.len());

        let (reordered_txns, integrated_graph) = GraphPevm::reorder_txs_by_dependency_graph(
            reordered_txns,
            &mut integrated_graph,
            8,
        );

        integrate_graphs.push(integrated_graph.clone());
        integrate_txns.push(reordered_txns.clone());
        integrate_storages.push(storage_history[i*batch_size].clone());
        
    }

    let repeat = 5;

    let start = Instant::now();
    for _ in 0..repeat{
        running_in_sequentially(storage_history.clone(), all_batches.clone());
    }
    let duration = start.elapsed();

    let start = Instant::now();
    for _ in 0..repeat{
        running_in_parallel(storage_history.clone(), all_batches.clone());
    }
    let duration1 = start.elapsed();

    let start = Instant::now();
    for _ in 0..repeat{
        running_in_dependency_graph(
            storage_history.clone(),
            reordered_txns_list.clone(),
            new_graphs.clone(),
        );
    }
    let duration2 = start.elapsed();
    

    let start = Instant::now();
    println!("integrate_storages: {}", integrate_storages.len());
    for _ in 0..repeat{
        running_in_dependency_graph(
            integrate_storages.clone(),
            integrate_txns.clone(),
            integrate_graphs.clone(),
        );
    }
    let duration3 = start.elapsed();
    
    println!("Sequential execution time: {:?}", duration);
    println!("Parallel execution time: {:?}", duration1);
    println!("Depedency Graph execution time: {:?}", duration2);
    println!("Integrated Depedency Graph execution time: {:?}", duration3);

}

fn running_in_sequentially(
    mut storage_history: Vec<InMemoryStorage>,
    all_batches: Vec<Vec<TxEnv>>,
) {
    let batch_num = all_batches.len();

    for i in 0..batch_num {
        let chain = PevmEthereum::mainnet();
        let spec_id = SpecId::LATEST;
        let block_env = BlockEnv::default();

        let result = pevm::execute_revm_sequential(
            &chain,
            &storage_history[i],
            spec_id,
            block_env,
            all_batches[i].clone(),
        ).unwrap();
        println!("Sequential batch {} done", i);
    }
}

fn running_in_parallel(
    mut storage_history: Vec<InMemoryStorage>,
    all_batches: Vec<Vec<TxEnv>>,
) {
    let batch_num = all_batches.len();

    for i in 0..batch_num {
        let chain = PevmEthereum::mainnet();
        let spec_id = SpecId::LATEST;
        let block_env = BlockEnv::default();

        let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
        let result = Pevm::default().execute_revm_parallel(
            &chain,
            &storage_history[i],
            SpecId::LATEST,
            BlockEnv::default(),
            all_batches[i].clone(),
            concurrency_level,
        ).unwrap();
        println!("Parallel batch {} done", i);
    }
}

fn running_in_dependency_graph(
    mut storage_history: Vec<InMemoryStorage>,
    reordered_txns_list: Vec<Vec<TxEnv>>,
    new_graphs: Vec<pevm::dependency_graph::TransactionGraph>,
) {
    let batch_num = reordered_txns_list.len();

    for i in 0..batch_num {
        let chain = PevmEthereum::mainnet();
        let spec_id = SpecId::LATEST;
        let block_env = BlockEnv::default();

        let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
        println!("concurrency_level: {:?}", concurrency_level);
        let mut pevm = GraphPevm::default();
        let start = Instant::now();
        let result = pevm.execute_revm_parallel(
            &chain,
            &storage_history[i],
            spec_id,
            block_env,
            reordered_txns_list[i].clone(),
            concurrency_level,
            new_graphs[i].clone(),
        ).unwrap();
        let duration = start.elapsed();
        println!("Dependency graph batch {} done in {:?}", i, duration);
    }
}


pub fn different_conflict_test_real_blocks(
    start_block: u64,
    num_blocks: usize,
    blocks_dir: &str,
) -> (f64, f64, f64, f64) {
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║  Real Block Execution Test: Blocks {}-{}           ║", 
             start_block, start_block + num_blocks as u64 - 1);
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    let chain = PevmEthereum::mainnet();
    let spec_id = get_spec_id(start_block);
    // Build block numbers array
    let block_numbers: Vec<u64> = (0..num_blocks as u64)
        .map(|i| start_block + i)
        .collect();

    let mut block_env = BlockEnv::default();
    block_env.gas_limit = U256::from(1_000_000_000u64);
    
    // (1) Create merged storage from all blocks
    println!("=== Creating merged storage from {} blocks ===", num_blocks);
    let storage = match create_multi_block_storage(&block_numbers, blocks_dir) {
        Ok(s) => {
            println!("✓ Merged storage created");
            s
        }
        Err(e) => {
            println!("❌ Failed to create merged storage: {:?}", e);
            return (0.0, 0.0, 0.0, 0.0);
        }
    };
    
    // (2) Load transactions from all blocks and create nonce tracker
    println!("\n=== Loading transactions and building nonce tracker ===");
    let mut blocks_txs = Vec::new();
    let mut nonce_tracker = NonceTracker::new();
    
    for (idx, block_num) in block_numbers.iter().enumerate() {
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);
        
        match load_block_for_execution(&filepath, true) {
            Ok((block_data, block_storage, txenvs)) => {
                println!("  Block {}: {} transactions", block_num, txenvs.len());
                nonce_tracker.record_from_prestate(*block_num, &block_storage, &txenvs);
                blocks_txs.push(txenvs);
            }
            Err(e) => {
                println!("  ⚠️  Failed to load block {}: {:?}", block_num, e);
            }
        }
    }
    
    if blocks_txs.is_empty() {
        println!("❌ No blocks loaded successfully!");
        return (0.0, 0.0, 0.0, 0.0);
    }
    
    let total_txs: usize = blocks_txs.iter().map(|txs| txs.len()).sum();
    println!("\n✓ Loaded {} blocks with {} total transactions", blocks_txs.len(), total_txs);
    
    // 1. Sequential
    println!("\n=== 1. Sequential Execution ===");
    let seq_start = Instant::now();
    let mut seq_storage = storage.clone();
    let mut seq_gas_per_block: Vec<u64> = Vec::new();
    for txs in blocks_txs.clone() {
        let (new_s, gas) = execute_sequential_and_update_with_gas(seq_storage, spec_id, txs);
        seq_storage = new_s;
        seq_gas_per_block.push(gas);
    }
    let seq_total_gas: u64 = seq_gas_per_block.iter().sum();
    let seq_tput = total_txs as f64 / seq_start.elapsed().as_secs_f64();
    println!("Sequential throughput: {:.2} tx/s  total_gas={}", seq_tput, seq_total_gas);

    // 2. Parallel (original)
    println!("\n=== 2. Parallel Execution ===");
    let par_start = Instant::now();
    let mut par_storage = storage.clone();
    for txs in blocks_txs.clone() {
        par_storage = execute_parallel_and_update(par_storage, spec_id, txs);
    }
    let par_tput = total_txs as f64 / par_start.elapsed().as_secs_f64();
    println!("Parallel throughput: {:.2} tx/s", par_tput);

    // 3. Generate graphs and parallel with graphs
    println!("\n=== 3. Generating Dependency Graphs ===");
    let (_, reordered_blocks_txs, dependency_graphs) =
        generate_dependency_graphs(storage.clone(), spec_id, blocks_txs);

    println!("=== 4. Parallel with Graphs ===");
    let graph_par_start = Instant::now();
    let mut graph_par_storage = storage.clone();
    let mut graph_gas_per_block: Vec<u64> = Vec::new();
    for (txs, graph) in reordered_blocks_txs.iter().zip(dependency_graphs.iter()) {
        let (new_s, gas) = execute_parallel_with_graph_and_update_with_gas(
            graph_par_storage, txs.clone(), graph.clone(), spec_id,
        );
        graph_par_storage = new_s;
        graph_gas_per_block.push(gas);
    }
    let graph_total_gas: u64 = graph_gas_per_block.iter().sum();
    let graph_par_tput = total_txs as f64 / graph_par_start.elapsed().as_secs_f64();
    println!("Parallel with graphs throughput: {:.2} tx/s  total_gas={}", graph_par_tput, graph_total_gas);

    // Per-block gas comparison
    println!("\n{:>5}  {:>14}  {:>14}  {:>12}  {}", "Block", "Seq Gas", "Graph Gas", "Diff", "Status");
    println!("{}", "-".repeat(60));
    let mut first_mismatch = None;
    for (i, (sg, gg)) in seq_gas_per_block.iter().zip(graph_gas_per_block.iter()).enumerate() {
        let diff = *gg as i64 - *sg as i64;
        let status = if sg == gg { "✓" } else { "✗" };
        println!("{:>5}  {:>14}  {:>14}  {:>+12}  {}",
            block_numbers[i], sg, gg, diff, status);
        if sg != gg && first_mismatch.is_none() {
            first_mismatch = Some(i);
        }
    }
    println!("{}", "-".repeat(60));
    println!("Total  {:>14}  {:>14}  {:>+12}  {}",
        seq_total_gas, graph_total_gas,
        graph_total_gas as i64 - seq_total_gas as i64,
        if seq_total_gas == graph_total_gas { "✓ MATCH" } else { "✗ MISMATCH" });
    if let Some(i) = first_mismatch {
        println!("First mismatch at block index {} (block {})", i, block_numbers[i]);
    }
    
    // 4. Greedy integration
    println!("\n=== 5. Greedy Integration ===");
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        tau_cv: 0.1,
        num_threads: parallel_concurrency().get(),
    });
    
    let (mut integrated_txns, integrated_graphs) = integrator.integrate_pevm_graphs(
        dependency_graphs,
        reordered_blocks_txs,
    );

    for graph in integrated_graphs.iter() {
        if graph.has_cycle() {
            println!("⚠️  Warning: Integrated graph has a cycle!");
        }
        else {
            println!("✓ Integrated graph is acyclic");
        }
    }

    // for txs in integrated_txns.iter_mut() {
    //     for tx in txs.iter() {
    //         println!("Nonce: {:?}, txcaller: {:?}", tx.nonce, tx.caller);
    //     }
    // }
    
    // (3) Update nonces in integrated_txns using nonce_tracker
    println!("  Updating nonces in integrated transactions...");
    for txs in integrated_txns.iter_mut() {
        nonce_tracker.update_txenv_nonces(txs);
    }
    println!("  ✓ Nonces updated");

    // for txs in integrated_txns.iter_mut() {
    //     for (idx, tx)  in txs.iter().enumerate() {
    //         println!("idx: {:?}, Nonce: {:?}, txcaller: {:?}", idx, tx.nonce, tx.caller);
    //     }
    // }
    
    println!("=== 6. Parallel with Integrated Graphs ===");
    let integrated_start = Instant::now();
    let mut integrated_storage = storage.clone();
    
    for (idx, (txs, graph)) in integrated_txns.iter().zip(integrated_graphs.iter()).enumerate() {
        if idx % 1 == 0 {
            println!("  Processing integrated block {}/{}", idx + 1, integrated_txns.len());
        }
        // integrated_storage = execute_parallel_and_update(integrated_storage, txs.clone());
        integrated_storage = execute_parallel_with_graph_and_update(
            integrated_storage,
            txs.clone(),
            graph.clone(),
            spec_id,
            block_env.clone(),
        );
    }
    let integrated_tput = total_txs as f64 / integrated_start.elapsed().as_secs_f64();
    println!("Integrated throughput: {:.2} tx/s", integrated_tput);
    
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║                         Results                                ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║ Sequential:           {:>10.2} tx/s                      ║", seq_tput);
    println!("║ Parallel:             {:>10.2} tx/s                      ║", par_tput);
    println!("║ Graph Parallel:       {:>10.2} tx/s                      ║", graph_par_tput);
    println!("║ Integrated:           {:>10.2} tx/s                      ║", integrated_tput);
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║ Parallel Speedup:     {:>10.2}x                          ║", par_tput / seq_tput);
    println!("║ Graph Speedup:        {:>10.2}x                          ║", graph_par_tput / seq_tput);
    println!("║ Integrated Speedup:   {:>10.2}x                          ║", integrated_tput / seq_tput);
    println!("╚════════════════════════════════════════════════════════════════╝\n");

    println!("spec_id: {:?}", spec_id);
    
    (seq_tput, par_tput, graph_par_tput, integrated_tput)
}



#[test]
fn test_real_blocks_performance() {
    let (seq, par, graph_par, integrated) = different_conflict_test_real_blocks(
        16774645,  // start_block
        2000,      // num_blocks
        "/home/ubuntu/eth-block-downloader/test_data/blocks/16774645_2000",
    );

    println!("Final results:");
    println!("  Sequential:  {:.2} tx/s", seq);
    println!("  Parallel:    {:.2} tx/s", par);
    println!("  Graph:       {:.2} tx/s", graph_par);
    println!("  Integrated:  {:.2} tx/s", integrated);
}

#[test]
fn test_real_blocks_performance_100() {
    let num_blocks: usize = std::env::var("NUM_BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let start_block: u64 = std::env::var("START_BLOCK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16774645);

    let (seq, par, graph_par, integrated) = different_conflict_test_real_blocks(
        start_block,
        num_blocks,
        "/home/ubuntu/eth-block-data/blocks_rw",
    );

    println!("Final results ({} blocks from {}):", num_blocks, start_block);
    println!("  Sequential:  {:.2} tx/s", seq);
    println!("  Parallel:    {:.2} tx/s", par);
    println!("  Graph:       {:.2} tx/s", graph_par);
    println!("  Integrated:  {:.2} tx/s", integrated);
}


/// Compare incarnation-0 access sets (first parallel execution) vs sequential access sets.
/// Tests whether real EVM txs diverge from their sequential access patterns on first parallel run.
#[test]
fn test_incarnation0_divergence() {
    let num_blocks: usize = std::env::var("NUM_BLOCKS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let blocks_dir = "/home/ubuntu/eth-block-downloader/test_data/blocks/batch_1";
    let start_block = 16774645u64;
    let block_numbers: Vec<u64> = (0..num_blocks as u64).map(|i| start_block + i).collect();
    let chain = PevmEthereum::mainnet();
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    let storage = create_multi_block_storage(&block_numbers, blocks_dir).unwrap();

    // Aggregate counters
    let (mut total_txs, mut read_diverged, mut write_diverged, mut either_diverged) = (0usize, 0usize, 0usize, 0usize);

    let mut seq_storage = storage.clone();
    let mut par_storage = storage.clone();

    for block_num in &block_numbers {
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);
        let (_bd, _bs, txs) = load_block_for_execution(&filepath, true).unwrap();
        let spec_id = get_spec_id(*block_num);
        let n = txs.len();

        // Sequential: capture access sets
        let (seq_results, seq_access) = pevm::execute_revm_sequential_with_access_sets(
            &chain, &seq_storage, spec_id, make_block_env(), txs.clone(),
        ).unwrap();
        update_storage_with_results(&mut seq_storage, seq_results);

        // Parallel: capture incarnation-0 sets via Pevm
        let mut pevm_instance = Pevm::default();
        let par_results = pevm_instance.execute_revm_parallel(
            &chain, &par_storage, spec_id, make_block_env(), txs, concurrency_level,
        ).unwrap();
        update_storage_with_results(&mut par_storage, par_results);

        let inc0 = &pevm_instance.last_incarnation0_keys;

        let mut blk_read_div = 0usize;
        let mut blk_write_div = 0usize;
        let mut blk_either = 0usize;

        for (tx_idx, (seq_a, par_opt)) in seq_access.iter().zip(inc0.iter()).enumerate() {
            let Some((par_read, par_write)) = par_opt else { continue };
            let seq_read: std::collections::HashSet<u64> = seq_a.read_set.iter().copied().collect();
            let seq_write: std::collections::HashSet<u64> = seq_a.write_set.iter().copied().collect();

            let rd = seq_read != *par_read;
            let wd = seq_write != *par_write;
            if rd { blk_read_div += 1; }
            if wd { blk_write_div += 1; }
            if rd || wd { blk_either += 1; }
        }

        println!(
            "Block {}: {} txs  read_diverged={}/{} ({:.0}%)  write_diverged={}/{} ({:.0}%)  either={}/{} ({:.0}%)",
            block_num, n,
            blk_read_div, n, blk_read_div as f64 / n as f64 * 100.0,
            blk_write_div, n, blk_write_div as f64 / n as f64 * 100.0,
            blk_either, n, blk_either as f64 / n as f64 * 100.0,
        );

        total_txs += n;
        read_diverged += blk_read_div;
        write_diverged += blk_write_div;
        either_diverged += blk_either;
    }

    println!("\n=== Incarnation-0 Divergence Summary ({} blocks, {} txs) ===", num_blocks, total_txs);
    println!("  Read set diverged:  {}/{} = {:.1}%", read_diverged, total_txs, read_diverged as f64 / total_txs as f64 * 100.0);
    println!("  Write set diverged: {}/{} = {:.1}%", write_diverged, total_txs, write_diverged as f64 / total_txs as f64 * 100.0);
    println!("  Either diverged:    {}/{} = {:.1}%", either_diverged, total_txs, either_diverged as f64 / total_txs as f64 * 100.0);
}

#[test]
fn test_first_block_critical_path() {
    let blocks_dir = "/home/ubuntu/eth-block-downloader/test_data/blocks/batch_1";
    let block_num = 16774645u64;
    let filepath = format!("{}/block_{}.json", blocks_dir, block_num);

    let chain = PevmEthereum::mainnet();
    let spec_id = get_spec_id(block_num);
    let (_block_data, _single_storage, txs) = load_block_for_execution(&filepath, true).unwrap();
    // Use merged storage (like different_conflict_test_real_blocks) to ensure enough funds
    let storage = create_multi_block_storage(&[block_num], blocks_dir).unwrap();
    let block_env = BlockEnv::default();
    let num_txs = txs.len();

    // Also capture access sets for conflict analysis
    let (results, access_sets) = pevm::execute_revm_sequential_with_access_sets(
        &chain, &storage, spec_id, block_env.clone(), txs.clone(),
    ).unwrap();
    let (mut graph, _results2) = GraphPevm::construct_graph_pevm_by_sequential(
        &chain, &storage, spec_id, block_env, txs, 1,
    ).unwrap();

    // 统计图的基本属性
    let num_edges: usize = graph.nodes.iter().map(|n| n.children_indices.len()).sum();
    let num_roots = graph.nodes.iter().filter(|n| n.parent_indices.is_empty()).count();
    let max_parents = graph.nodes.iter().map(|n| n.parent_indices.len()).max().unwrap_or(0);
    let max_children = graph.nodes.iter().map(|n| n.children_indices.len()).max().unwrap_or(0);

    println!("=== Block {} Dependency Graph Analysis ===", block_num);
    println!("Transactions:  {}", num_txs);
    println!("Edges (WW):    {}", num_edges);
    println!("Root nodes:    {} ({:.1}% of txs)", num_roots, 100.0 * num_roots as f64 / num_txs as f64);
    println!("Max in-degree: {}", max_parents);
    println!("Max out-degree:{}", max_children);

    // 模拟并行执行，获取 critical path (makespan)
    for threads in [1, 2, 4, 8, 16] {
        let mut g = graph.clone();
        g.simulate_parallel_execution(threads);
        if let Some(ref sim) = g.simulation_result {
            let total_exec_time: u64 = g.nodes.iter().map(|n| n.execution_time).sum();
            let critical_path_pct = 100.0 * sim.total_time as f64 / total_exec_time as f64;
            println!(
                "  {:2} threads → makespan={} ns, critical_path/total={:.1}%, speedup={:.2}x",
                threads,
                sim.total_time,
                critical_path_pct,
                total_exec_time as f64 / sim.total_time as f64,
            );
        }
    }

    // 找出关键路径上的 tx 链（longest_suffix 最大的根节点往下追踪）
    graph.simulate_parallel_execution(8);
    graph.update_longest_suffix_postorder();
    let total_exec_time: u64 = graph.nodes.iter().map(|n| n.execution_time).sum();
    let cp_root = graph.nodes.iter().enumerate()
        .filter(|(_, n)| n.parent_indices.is_empty())
        .max_by_key(|(_, n)| n.longest_suffix)
        .map(|(i, _)| i);

    if let Some(mut idx) = cp_root {
        println!("\nCritical path (8 threads, WW conflict source):");
        let mut depth = 0;
        let mut prev_idx: Option<usize> = None;
        loop {
            let node = &graph.nodes[idx];
            // Print conflict with previous tx in the chain
            if let Some(prev) = prev_idx {
                let overlap: Vec<u64> = access_sets[prev].write_set
                    .intersection(&access_sets[idx].write_set)
                    .copied().collect();
                println!("    WW conflict on {} location(s): {:?}", overlap.len(), &overlap[..overlap.len().min(3)]);
            }
            println!("  [{}] tx_idx={} write_set_size={} exec_time={}ns",
                depth, idx, access_sets[idx].write_set.len(), node.execution_time);
            depth += 1;
            prev_idx = Some(idx);
            let next = node.children_indices.iter()
                .max_by_key(|&&c| graph.nodes[c].longest_suffix);
            match next {
                Some(&c) => idx = c,
                None => break,
            }
        }
        println!("\n  Critical path depth: {} txs", depth);
        println!("  Total exec time: {} ns", total_exec_time);
    }
    let _ = results;
}

#[test]
fn test_50_blocks_speedup_analysis() {
    let blocks_dir = "/home/ubuntu/eth-block-downloader/test_data/blocks/batch_1";
    let start_block = 16774645u64;
    let num_blocks = 50;

    struct BlockStats {
        block_num: u64,
        num_txs: usize,
        num_edges: usize,
        root_pct: f64,
        critical_path_depth: usize,
        max_speedup_8t: f64,
    }

    let mut stats: Vec<BlockStats> = Vec::new();

    println!("\n{:-<80}", "");
    println!("{:^80}", "50-Block Critical Path / Max Speedup Analysis (8 threads)");
    println!("{:-<80}", "");
    println!("{:<12} {:>7} {:>7} {:>7} {:>8} {:>10}",
        "Block", "#Txs", "#Edges", "Root%", "CPDepth", "MaxSpdup");
    println!("{:-<80}", "");

    for i in 0..num_blocks {
        let block_num = start_block + i as u64;
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);

        // Load txs
        let (_, _, txs) = match load_block_for_execution(&filepath, true) {
            Ok(r) => r,
            Err(e) => {
                println!("  block {} SKIP (load error: {:?})", block_num, e);
                continue;
            }
        };
        if txs.is_empty() {
            println!("  block {} SKIP (0 txs)", block_num);
            continue;
        }

        let chain = PevmEthereum::mainnet();
        let spec_id = get_spec_id(block_num);
        let storage = match create_multi_block_storage(&[block_num], blocks_dir) {
            Ok(s) => s,
            Err(e) => {
                println!("  block {} SKIP (storage error: {:?})", block_num, e);
                continue;
            }
        };
        let block_env = BlockEnv::default();
        let num_txs = txs.len();

        // Build dependency graph
        let (mut graph, _) = match GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &storage, spec_id, block_env, txs, block_num,
        ) {
            Ok(r) => r,
            Err(e) => {
                println!("  block {} SKIP (graph error: {:?})", block_num, e);
                continue;
            }
        };

        let num_edges: usize = graph.nodes.iter().map(|n| n.children_indices.len()).sum();
        let num_roots = graph.nodes.iter().filter(|n| n.parent_indices.is_empty()).count();
        let root_pct = 100.0 * num_roots as f64 / num_txs as f64;
        let total_exec_time: u64 = graph.nodes.iter().map(|n| n.execution_time).sum();

        // Simulate at 8 threads
        graph.simulate_parallel_execution(8);
        let max_speedup_8t = if let Some(ref sim) = graph.simulation_result {
            if sim.total_time > 0 { total_exec_time as f64 / sim.total_time as f64 } else { 1.0 }
        } else { 1.0 };

        // Critical path depth: trace from the root with longest_suffix
        graph.update_longest_suffix_postorder();
        let cp_root = graph.nodes.iter().enumerate()
            .filter(|(_, n)| n.parent_indices.is_empty())
            .max_by_key(|(_, n)| n.longest_suffix)
            .map(|(i, _)| i);

        let critical_path_depth = if let Some(mut idx) = cp_root {
            let mut depth = 0usize;
            loop {
                depth += 1;
                let next = graph.nodes[idx].children_indices.iter()
                    .max_by_key(|&&c| graph.nodes[c].longest_suffix)
                    .copied();
                match next {
                    Some(c) => idx = c,
                    None => break,
                }
            }
            depth
        } else { 1 };

        println!("{:<12} {:>7} {:>7} {:>6.1}% {:>8} {:>10.2}x",
            block_num, num_txs, num_edges, root_pct, critical_path_depth, max_speedup_8t);

        stats.push(BlockStats {
            block_num, num_txs, num_edges, root_pct, critical_path_depth, max_speedup_8t,
        });
    }

    if stats.is_empty() {
        println!("No blocks processed.");
        return;
    }

    // Aggregate statistics
    let mut speedups: Vec<f64> = stats.iter().map(|s| s.max_speedup_8t).collect();
    speedups.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let n = speedups.len() as f64;
    let mean = speedups.iter().sum::<f64>() / n;
    let median = if speedups.len() % 2 == 0 {
        (speedups[speedups.len()/2 - 1] + speedups[speedups.len()/2]) / 2.0
    } else {
        speedups[speedups.len()/2]
    };
    let p25 = speedups[(speedups.len() as f64 * 0.25) as usize];
    let p75 = speedups[(speedups.len() as f64 * 0.75).min(speedups.len() as f64 - 1.0) as usize];

    let total_txs: usize = stats.iter().map(|s| s.num_txs).sum();
    let avg_edges: f64 = stats.iter().map(|s| s.num_edges as f64).sum::<f64>() / n;
    let avg_root_pct: f64 = stats.iter().map(|s| s.root_pct).sum::<f64>() / n;
    let avg_cp_depth: f64 = stats.iter().map(|s| s.critical_path_depth as f64).sum::<f64>() / n;

    println!("{:-<80}", "");
    println!("\n=== Aggregate Statistics ({} blocks) ===", stats.len());
    println!("  Total txs:          {}", total_txs);
    println!("  Avg txs/block:      {:.1}", total_txs as f64 / n);
    println!("  Avg WW edges:       {:.1}", avg_edges);
    println!("  Avg root%:          {:.1}%", avg_root_pct);
    println!("  Avg critical depth: {:.1}", avg_cp_depth);
    println!("\n  Max Speedup @ 8 threads (theoretical):");
    println!("    Min:    {:.2}x  (block {})", speedups[0],
        stats.iter().min_by(|a,b| a.max_speedup_8t.partial_cmp(&b.max_speedup_8t).unwrap()).unwrap().block_num);
    println!("    P25:    {:.2}x", p25);
    println!("    Median: {:.2}x", median);
    println!("    Mean:   {:.2}x", mean);
    println!("    P75:    {:.2}x", p75);
    println!("    Max:    {:.2}x  (block {})", speedups[speedups.len()-1],
        stats.iter().max_by(|a,b| a.max_speedup_8t.partial_cmp(&b.max_speedup_8t).unwrap()).unwrap().block_num);

    // Distribution buckets
    println!("\n  Speedup distribution:");
    let buckets = [(1.0f64, 1.5), (1.5, 2.0), (2.0, 3.0), (3.0, 4.0), (4.0, 6.0), (6.0, f64::MAX)];
    for (lo, hi) in buckets {
        let count = speedups.iter().filter(|&&x| x >= lo && x < hi).count();
        let label = if hi == f64::MAX { format!("{:.1}x+", lo) } else { format!("{:.1}x–{:.1}x", lo, hi) };
        let bar: String = "#".repeat(count * 2);
        println!("    {:12} {:3} blocks  {}", label, count, bar);
    }
}

#[test]
fn test_50_blocks_speedup_real_time() {
    let rw_dir = "/home/ubuntu/eth-block-downloader/test_data/rw_time";
    let start_block = 16774645u64;
    let num_blocks = 50;

    // Parse one tx entry from rw_time JSON
    struct RwEntry {
        writes: Vec<u64>,  // storage key → u64 (last 8 bytes of 32-byte hex)
        reads:  Vec<u64>,
        exec_ns: u64,
    }

    fn hex_to_u64(s: &str) -> u64 {
        let s = s.trim_start_matches("0x");
        // Take last 16 hex chars = last 8 bytes
        let start = s.len().saturating_sub(16);
        u64::from_str_radix(&s[start..], 16).unwrap_or(0)
    }

    fn load_rw_file(path: &str) -> Vec<RwEntry> {
        let content = std::fs::read_to_string(path).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&content).unwrap();
        arr.as_array().unwrap().iter().map(|v| {
            let writes = v["writes"].as_array().unwrap_or(&vec![])
                .iter().map(|s| hex_to_u64(s.as_str().unwrap())).collect();
            let reads = v["reads"].as_array().unwrap_or(&vec![])
                .iter().map(|s| hex_to_u64(s.as_str().unwrap())).collect();
            let exec_ns = v["executionTime"].as_u64().unwrap_or(0);
            RwEntry { writes, reads, exec_ns }
        }).collect()
    }

    struct BlockStats {
        block_num: u64,
        num_txs: usize,
        num_edges: usize,
        root_pct: f64,
        cp_depth: usize,
        speedup_gas: f64,   // old: gas-based
        speedup_real: f64,  // new: real execution time
        total_ns: u64,
        makespan_ns: u64,
    }

    let mut stats: Vec<BlockStats> = Vec::new();

    println!("\n{:-<90}", "");
    println!("{:^90}", "50-Block Theoretical Speedup: Gas-proxy vs Real ExecutionTime (8 threads)");
    println!("{:-<90}", "");
    println!("{:<12} {:>6} {:>6} {:>7} {:>8} {:>10} {:>10}",
        "Block", "#Txs", "#Edges", "Root%", "CPDepth", "SpeedGas", "SpeedReal");
    println!("{:-<90}", "");

    for i in 0..num_blocks {
        let block_num = start_block + i as u64;
        let rw_path = format!("{}/rw_time_{}.json", rw_dir, block_num);

        if !std::path::Path::new(&rw_path).exists() {
            println!("{:<12} SKIP (no rw_time file)", block_num);
            continue;
        }

        let entries = load_rw_file(&rw_path);
        if entries.is_empty() {
            continue;
        }
        let num_txs = entries.len();

        // Build TransactionGraph with real exec time
        let mut graph_real = TransactionGraph::new();
        for (i, e) in entries.iter().enumerate() {
            let write_set: std::collections::HashSet<u64> = e.writes.iter().copied().collect();
            let read_set:  std::collections::HashSet<u64> = e.reads.iter().copied().collect();
            let node = pevm::dependency_graph::TransactionNode::new(
                i as u64, 1, 1, e.exec_ns, read_set, write_set,
            );
            graph_real.add_transaction(node);
        }

        // Also build with gas proxy (using exec_ns as a stand-in — but we need the gas
        // values; load from block file)
        let blocks_dir = "/home/ubuntu/eth-block-downloader/test_data/blocks/batch_1";
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);
        let chain = PevmEthereum::mainnet();
        let spec_id = get_spec_id(block_num);

        let speedup_gas = if let Ok((_, _, txs)) = load_block_for_execution(&filepath, true) {
            if let Ok(storage) = create_multi_block_storage(&[block_num], blocks_dir) {
                if let Ok((mut g, _)) = GraphPevm::construct_graph_pevm_by_sequential(
                    &chain, &storage, spec_id, BlockEnv::default(), txs, block_num,
                ) {
                    let total: u64 = g.nodes.iter().map(|n| n.execution_time).sum();
                    g.simulate_parallel_execution(8);
                    if let Some(ref sim) = g.simulation_result {
                        if sim.total_time > 0 { total as f64 / sim.total_time as f64 } else { 1.0 }
                    } else { 1.0 }
                } else { f64::NAN }
            } else { f64::NAN }
        } else { f64::NAN };

        // Graph stats (from real-time graph)
        let num_edges: usize = graph_real.nodes.iter().map(|n| n.children_indices.len()).sum();
        let num_roots = graph_real.nodes.iter().filter(|n| n.parent_indices.is_empty()).count();
        let root_pct = 100.0 * num_roots as f64 / num_txs as f64;
        let total_ns: u64 = graph_real.nodes.iter().map(|n| n.execution_time).sum();

        graph_real.simulate_parallel_execution(8);
        let makespan_ns = graph_real.simulation_result.as_ref()
            .map(|s| s.total_time).unwrap_or(total_ns);
        let speedup_real = if makespan_ns > 0 { total_ns as f64 / makespan_ns as f64 } else { 1.0 };

        // Critical path depth
        graph_real.update_longest_suffix_postorder();
        let cp_depth = if let Some(mut idx) = graph_real.nodes.iter().enumerate()
            .filter(|(_, n)| n.parent_indices.is_empty())
            .max_by_key(|(_, n)| n.longest_suffix)
            .map(|(i, _)| i)
        {
            let mut depth = 0usize;
            loop {
                depth += 1;
                let next = graph_real.nodes[idx].children_indices.iter()
                    .max_by_key(|&&c| graph_real.nodes[c].longest_suffix)
                    .copied();
                match next { Some(c) => idx = c, None => break }
            }
            depth
        } else { 1 };

        let gas_str = if speedup_gas.is_nan() { "  N/A  ".to_string() }
                      else { format!("{:>9.2}x", speedup_gas) };
        println!("{:<12} {:>6} {:>6} {:>6.1}% {:>8} {} {:>9.2}x",
            block_num, num_txs, num_edges, root_pct, cp_depth, gas_str, speedup_real);

        stats.push(BlockStats {
            block_num, num_txs, num_edges, root_pct, cp_depth,
            speedup_gas, speedup_real, total_ns, makespan_ns,
        });
    }

    if stats.is_empty() {
        println!("No blocks processed.");
        return;
    }

    // Aggregate
    let n = stats.len() as f64;
    let mut real_speedups: Vec<f64> = stats.iter().map(|s| s.speedup_real).collect();
    real_speedups.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_real = real_speedups.iter().sum::<f64>() / n;
    let median_real = real_speedups[real_speedups.len() / 2];

    let gas_valid: Vec<f64> = stats.iter().filter(|s| !s.speedup_gas.is_nan()).map(|s| s.speedup_gas).collect();
    let mean_gas = gas_valid.iter().sum::<f64>() / gas_valid.len() as f64;
    let mut gas_sorted = gas_valid.clone(); gas_sorted.sort_by(|a,b| a.partial_cmp(b).unwrap());
    let median_gas = gas_sorted[gas_sorted.len() / 2];

    let total_work_ns: u64 = stats.iter().map(|s| s.total_ns).sum();
    let total_makespan_ns: u64 = stats.iter().map(|s| s.makespan_ns).sum();

    println!("{:-<90}", "");
    println!("\n=== Aggregate Statistics ({} blocks, 8 threads) ===", stats.len());
    println!("\n  Metric              Gas-proxy    Real-time");
    println!("  ─────────────────────────────────────────");
    println!("  Median speedup     {:>8.2}x    {:>8.2}x", median_gas, median_real);
    println!("  Mean speedup       {:>8.2}x    {:>8.2}x", mean_gas, mean_real);
    println!("  Min speedup        {:>8.2}x    {:>8.2}x",
        gas_sorted[0], real_speedups[0]);
    println!("  Max speedup        {:>8.2}x    {:>8.2}x",
        gas_sorted[gas_sorted.len()-1], real_speedups[real_speedups.len()-1]);
    println!("\n  Aggregate (all blocks as one):");
    println!("  Total work (ns):   {:>15}", total_work_ns);
    println!("  Total makespan(ns):{:>15}", total_makespan_ns);
    println!("  Agg. real speedup: {:>8.2}x", total_work_ns as f64 / total_makespan_ns as f64);

    // Distribution
    println!("\n  Real-time speedup distribution:");
    let buckets: &[(f64, f64)] = &[(1.0, 2.0),(2.0,3.0),(3.0,5.0),(5.0,8.0),(8.0,f64::MAX)];
    for &(lo, hi) in buckets {
        let count = real_speedups.iter().filter(|&&x| x >= lo && x < hi).count();
        let label = if hi == f64::MAX { format!("{:.0}x+", lo) } else { format!("{:.0}x–{:.0}x", lo, hi) };
        println!("    {:10} {:3} blocks  {}", label, count, "#".repeat(count));
    }
}

fn has_cycle(graph: &TransactionGraph) -> bool {
    let n = graph.node_count();
    let mut visited = vec![false; n];
    let mut rec_stack = vec![false; n];
    
    fn dfs(
        node: usize,
        graph: &TransactionGraph,
        visited: &mut Vec<bool>,
        rec_stack: &mut Vec<bool>,
    ) -> bool {
        visited[node] = true;
        rec_stack[node] = true;
        
        for dep in graph.dependencies(node) {
            if !visited[dep] {
                if dfs(dep, graph, visited, rec_stack) {
                    return true;
                }
            } else if rec_stack[dep] {
                println!("      Cycle: {} -> {}", node, dep);
                return true;
            }
        }
        
        rec_stack[node] = false;
        false
    }
    
    for i in 0..n {
        if !visited[i] && dfs(i, graph, &mut visited, &mut rec_stack) {
            return true;
        }
    }

    false
}

// ============================================================================
// Batch benchmark: run all blocks in eth-block-data, 100 at a time, log CSV.
// ============================================================================

#[derive(Debug, Clone)]
struct BatchMetrics {
    batch_idx: usize,
    start_block: u64,
    end_block: u64,
    num_blocks: usize,
    num_txs: usize,
    seq_time_s: f64,
    seq_tput: f64,
    par_time_s: f64,
    par_tput: f64,
    graph_time_s: f64,
    graph_tput: f64,
    integrated_time_s: f64,
    integrated_tput: f64,
    num_integrated_groups: usize,
}

fn run_one_batch(
    batch_idx: usize,
    block_numbers: &[u64],
    blocks_dir: &str,
) -> Option<BatchMetrics> {
    let spec_id = get_spec_id(block_numbers[0]);

    let storage = create_multi_block_storage(block_numbers, blocks_dir).ok()?;

    let mut blocks_txs = Vec::new();
    let mut nonce_tracker = NonceTracker::new();
    for &block_num in block_numbers {
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);
        match load_block_for_execution(&filepath, true) {
            Ok((_, block_storage, txenvs)) => {
                nonce_tracker.record_from_prestate(block_num, &block_storage, &txenvs);
                blocks_txs.push(txenvs);
            }
            Err(e) => {
                eprintln!("  ⚠️  load failed for block {}: {:?}", block_num, e);
                return None;
            }
        }
    }
    if blocks_txs.is_empty() {
        return None;
    }
    let total_txs: usize = blocks_txs.iter().map(|txs| txs.len()).sum();

    // 1. Sequential
    let seq_start = Instant::now();
    let mut seq_storage = storage.clone();
    for txs in blocks_txs.clone() {
        seq_storage = execute_sequential_and_update(seq_storage, spec_id, txs);
    }
    let seq_time_s = seq_start.elapsed().as_secs_f64();
    let seq_tput = total_txs as f64 / seq_time_s;

    // 2. Parallel (Block-STM, no graph)
    let par_start = Instant::now();
    let mut par_storage = storage.clone();
    for txs in blocks_txs.clone() {
        par_storage = execute_parallel_and_update(par_storage, spec_id, txs);
    }
    let par_time_s = par_start.elapsed().as_secs_f64();
    let par_tput = total_txs as f64 / par_time_s;

    // 3. Graph parallel (per-block dependency graph)
    let (_, reordered_blocks_txs, dependency_graphs) =
        generate_dependency_graphs(storage.clone(), spec_id, blocks_txs);
    let graph_start = Instant::now();
    let mut graph_storage = storage.clone();
    for (txs, graph) in reordered_blocks_txs.iter().zip(dependency_graphs.iter()) {
        let (new_s, _) = execute_parallel_with_graph_and_update_with_gas(
            graph_storage, txs.clone(), graph.clone(), spec_id,
        );
        graph_storage = new_s;
    }
    let graph_time_s = graph_start.elapsed().as_secs_f64();
    let graph_tput = total_txs as f64 / graph_time_s;

    // 4. Integrated (greedy multi-block merge)
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        tau_cv: 0.1,
        num_threads: parallel_concurrency().get(),
    });
    let (mut integrated_txns, integrated_graphs) =
        integrator.integrate_pevm_graphs(dependency_graphs, reordered_blocks_txs);
    for txs in integrated_txns.iter_mut() {
        nonce_tracker.update_txenv_nonces(txs);
    }
    let num_integrated_groups = integrated_graphs.len();

    let block_env = make_block_env();
    let integrated_start = Instant::now();
    let mut integrated_storage = storage.clone();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        integrated_storage = execute_parallel_with_graph_and_update(
            integrated_storage,
            txs.clone(),
            graph.clone(),
            spec_id,
            block_env.clone(),
        );
    }
    let integrated_time_s = integrated_start.elapsed().as_secs_f64();
    let integrated_tput = total_txs as f64 / integrated_time_s;

    Some(BatchMetrics {
        batch_idx,
        start_block: *block_numbers.first().unwrap(),
        end_block: *block_numbers.last().unwrap(),
        num_blocks: block_numbers.len(),
        num_txs: total_txs,
        seq_time_s,
        seq_tput,
        par_time_s,
        par_tput,
        graph_time_s,
        graph_tput,
        integrated_time_s,
        integrated_tput,
        num_integrated_groups,
    })
}

#[test]
fn test_eth_block_data_all_batches() {
    use std::io::Write;

    let blocks_dir = std::env::var("BLOCKS_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/blocks_rw".to_string());
    let output_path = std::env::var("OUTPUT").unwrap_or_else(|_| {
        "/home/ubuntu/congestion_control/pevm/eth_block_data_batches.csv".to_string()
    });
    let batch_size: usize = std::env::var("BATCH_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let max_batches: Option<usize> =
        std::env::var("MAX_BATCHES").ok().and_then(|v| v.parse().ok());

    // Discover all block numbers in the data dir
    let mut all_blocks: Vec<u64> = std::fs::read_dir(&blocks_dir)
        .expect("blocks_dir not readable")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_prefix("block_")
                .and_then(|s| s.strip_suffix(".json"))
                .and_then(|s| s.parse::<u64>().ok())
        })
        .collect();
    all_blocks.sort();
    assert!(!all_blocks.is_empty(), "no block_*.json files in {}", blocks_dir);

    let total_batches = (all_blocks.len() + batch_size - 1) / batch_size;
    println!(
        "Found {} blocks (range {}..={}). batch_size={}, total_batches={}. NUM_THREADS={}",
        all_blocks.len(),
        all_blocks.first().unwrap(),
        all_blocks.last().unwrap(),
        batch_size,
        total_batches,
        parallel_concurrency().get(),
    );
    println!("Writing CSV to: {}", output_path);

    let mut writer = std::io::BufWriter::new(
        std::fs::File::create(&output_path).expect("cannot create output file"),
    );
    writeln!(
        writer,
        "batch_idx,start_block,end_block,num_blocks,num_txs,\
         seq_time_s,seq_tput,par_time_s,par_tput,\
         graph_time_s,graph_tput,integrated_time_s,integrated_tput,\
         num_integrated_groups"
    )
    .unwrap();
    writer.flush().unwrap();

    let overall_start = Instant::now();
    let mut completed = 0usize;
    for (i, chunk) in all_blocks.chunks(batch_size).enumerate() {
        if let Some(lim) = max_batches {
            if i >= lim {
                break;
            }
        }
        let t = Instant::now();
        println!(
            "\n=== Batch {}/{}  blocks {}..={} ({} blocks) ===",
            i + 1,
            total_batches,
            chunk[0],
            chunk[chunk.len() - 1],
            chunk.len()
        );
        match run_one_batch(i, chunk, &blocks_dir) {
            Some(m) => {
                println!(
                    "  done in {:.2}s | txs={} | seq={:.0}t/s par={:.0} graph={:.0} integ={:.0} groups={}",
                    t.elapsed().as_secs_f64(),
                    m.num_txs,
                    m.seq_tput,
                    m.par_tput,
                    m.graph_tput,
                    m.integrated_tput,
                    m.num_integrated_groups
                );
                writeln!(
                    writer,
                    "{},{},{},{},{},{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},{}",
                    m.batch_idx,
                    m.start_block,
                    m.end_block,
                    m.num_blocks,
                    m.num_txs,
                    m.seq_time_s,
                    m.seq_tput,
                    m.par_time_s,
                    m.par_tput,
                    m.graph_time_s,
                    m.graph_tput,
                    m.integrated_time_s,
                    m.integrated_tput,
                    m.num_integrated_groups
                )
                .unwrap();
                writer.flush().unwrap();
                completed += 1;
            }
            None => {
                eprintln!("  batch {} failed", i);
                writeln!(
                    writer,
                    "{},{},{},{},,,,,,,,,,",
                    i,
                    chunk[0],
                    chunk[chunk.len() - 1],
                    chunk.len()
                )
                .unwrap();
                writer.flush().unwrap();
            }
        }
    }
    println!(
        "\n✓ {} / {} batches done in {:.1}s. CSV → {}",
        completed,
        total_batches,
        overall_start.elapsed().as_secs_f64(),
        output_path
    );
}

// ============================================================================
// Smoke: integration time vs integrated-execution time on REAL blocks.
//
// The greedy integrator merges per-block dependency graphs into larger
// "integrated" graphs. We want to confirm that the merging step itself is
// fast relative to the parallel execution it enables.
//
// Excluded from "integration time":
//   - Per-block dependency-graph construction
//     (`GraphPevm::construct_graph_pevm_by_sequential` ->
//      `execute_revm_sequential_with_access_sets`)
//   - Per-block tx reordering (`reorder_txs_by_dependency_graph`)
//   - Nonce re-assignment after integration
//
// Included:
//   - Just `integrator.integrate_pevm_graphs(...)` — the greedy merge itself.
// ============================================================================

#[test]
fn test_integration_vs_execution_smoke() {
    let num_blocks: usize = std::env::var("NUM_BLOCKS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let start_block: u64 = std::env::var("START_BLOCK")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(16774645);
    let blocks_dir: String = std::env::var("BLOCKS_DIR")
        .unwrap_or_else(|_| "/home/ubuntu/eth-block-data/blocks_rw".to_string());

    let chain = PevmEthereum::mainnet();
    let spec_id = get_spec_id(start_block);

    let block_numbers: Vec<u64> = (0..num_blocks as u64).map(|i| start_block + i).collect();

    println!("\n=== Setup: load {} blocks from {} ===", num_blocks, blocks_dir);
    let storage = create_multi_block_storage(&block_numbers, &blocks_dir)
        .expect("create_multi_block_storage failed");

    let mut blocks_txs = Vec::new();
    let mut nonce_tracker = NonceTracker::new();
    for &block_num in &block_numbers {
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);
        let (_, block_storage, txenvs) =
            load_block_for_execution(&filepath, true).expect("load_block_for_execution failed");
        nonce_tracker.record_from_prestate(block_num, &block_storage, &txenvs);
        blocks_txs.push(txenvs);
    }
    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();
    println!("loaded {} blocks, {} txs total", blocks_txs.len(), total_txs);

    // Build per-block dependency graphs (NOT TIMED — this uses
    // execute_revm_sequential_with_access_sets internally and is excluded
    // from the integration cost on purpose).
    println!("\n=== Untimed: per-block graph construction ===");
    let prep_t = Instant::now();
    let (_, reordered_blocks_txs, dependency_graphs) =
        generate_dependency_graphs(storage.clone(), spec_id, blocks_txs);
    let prep_s = prep_t.elapsed().as_secs_f64();
    println!("graph construction (excluded from integration): {:.3}s for {} blocks",
        prep_s, dependency_graphs.len());

    // ── Timed: greedy integration ──────────────────────────────────────────
    println!("\n=== Timed: greedy integration (only integrate_pevm_graphs) ===");
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        tau_cv: 0.1,
        num_threads: parallel_concurrency().get(),
    });
    let integ_t = Instant::now();
    let (mut integrated_txns, integrated_graphs) =
        integrator.integrate_pevm_graphs(dependency_graphs, reordered_blocks_txs);
    let integration_s = integ_t.elapsed().as_secs_f64();
    let num_groups = integrated_graphs.len();
    println!(
        "integration: {:.4}s → {} groups from {} blocks",
        integration_s, num_groups, num_blocks
    );

    // Nonce re-assignment (excluded from integration; this is a cheap
    // post-processing step needed only because integration shuffles tx order).
    for txs in integrated_txns.iter_mut() {
        nonce_tracker.update_txenv_nonces(txs);
    }

    // ── Timed: integrated parallel execution ───────────────────────────────
    println!("\n=== Timed: integrated parallel execution ===");
    let exec_t = Instant::now();
    let mut s = storage.clone();
    let block_env = make_block_env();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        s = execute_parallel_with_graph_and_update(
            s, txs.clone(), graph.clone(), spec_id, block_env.clone(),
        );
    }
    let execution_s = exec_t.elapsed().as_secs_f64();
    println!("integrated execution: {:.4}s for {} groups ({} txs)",
        execution_s, num_groups, total_txs);

    // ── Comparison ────────────────────────────────────────────────────────
    let ratio = execution_s / integration_s.max(1e-9);
    println!("\n╔═══════════════════════════════════════════════════════════════╗");
    println!("║  Integration vs Integrated-Execution (real blocks, smoke)     ║");
    println!("╠═══════════════════════════════════════════════════════════════╣");
    println!("║ Integration time          : {:>9.4} s                       ║", integration_s);
    println!("║ Integrated execution time : {:>9.4} s                       ║", execution_s);
    println!("║ Execution / Integration   : {:>9.2}x                        ║", ratio);
    println!("║ Integration fraction      : {:>9.2}% of total              ║",
        100.0 * integration_s / (integration_s + execution_s));
    println!("╠═══════════════════════════════════════════════════════════════╣");
    println!("║ (excluded from integration: {:.3}s graph construction)        ║", prep_s);
    println!("╚═══════════════════════════════════════════════════════════════╝");
}