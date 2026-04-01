
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
        let txn_node = TransactionNode::new(i as u64, 1, 1, costs[i], HashSet::new(), access_set[i].clone());
        graph.add_transaction(txn_node);
    }

    let mut graph2 = TransactionGraph::new();
    for i in 0..result2.len() {
        let txn_node = TransactionNode::new(i as u64, 1, 1, costs2[i], HashSet::new(), access_set2[i].clone());
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
        let txn_node = TransactionNode::new(i as u64, 1, 1, costs[i], HashSet::new(), access_set[i].clone());
        graph.add_transaction(txn_node);
    }

    let mut graph2 = TransactionGraph::new();
    for i in 0..result2.len() {
        let txn_node = TransactionNode::new(i as u64, 2, 1, costs2[i], HashSet::new(), access_set2[i].clone());
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

    g1.integrate_graph(g2);

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

    new_graph.integrate_graph(new_graph2);

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
    
    let concurrency_level = std::thread::available_parallelism()
        .unwrap_or(std::num::NonZeroUsize::MIN)
        .get();
    
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

fn execute_sequential_and_update(
    mut storage: InMemoryStorage, 
    spec_id: SpecId,
    txs: Vec<TxEnv>
) -> InMemoryStorage {
    let chain = PevmEthereum::mainnet();
    let mut block_env = BlockEnv::default();
    block_env.gas_limit = U256::MAX; // Set to max to avoid out-of-gas issues in sequential execution
    block_env.basefee = U256::from(1u64); // 1 wei
    
    let result = pevm::execute_revm_sequential(
        &chain,
        &storage,
        spec_id,
        block_env,
        txs.clone(),
    ).unwrap();
    
    update_storage_with_results(&mut storage, result);
    storage
}


fn execute_parallel_and_update(
    mut storage: InMemoryStorage, 
    spec_id: SpecId,
    txs: Vec<TxEnv>
) -> InMemoryStorage {
    let chain = PevmEthereum::mainnet();
    let mut block_env = BlockEnv::default();
    block_env.gas_limit = U256::MAX; // Set to max to avoid out-of-gas issues 
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
    
    let result = Pevm::default().execute_revm_parallel(
        &chain,
        &storage,
        spec_id,
        block_env,
        txs.clone(),
        concurrency_level,
    ).unwrap();
    
    update_storage_with_results(&mut storage, result);
    storage
}


fn execute_parallel_with_graph_and_update(
    mut storage: InMemoryStorage, 
    txs: Vec<TxEnv>,
    graph: pevm::dependency_graph::TransactionGraph,
    spec_id: SpecId,    
    block_env: BlockEnv, 
) -> InMemoryStorage {
    let chain = PevmEthereum::mainnet();
    let concurrency_level = std::thread::available_parallelism()
        .unwrap_or(std::num::NonZeroUsize::MIN);
        
    let mut pevm = GraphPevm::default();
    let spec_id = SpecId::FRONTIER;

    let result = pevm.execute_revm_parallel(
        &chain,
        &storage,
        spec_id,     
        block_env,   
        txs, 
        concurrency_level,
        graph, 
    ).unwrap();
    
    update_storage_with_results(&mut storage, result);
    storage
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
                integrated_graph.integrate_graph(new_graphs[i * batch_size + j].clone());
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
    let storage = match create_multi_block_storage(&block_numbers) {
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
        let filepath = format!("/home/ubuntu/eth-block-downloader/test_data/blocks/block_{}.json", block_num);
        
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
    for txs in blocks_txs.clone() {
        seq_storage = execute_sequential_and_update(seq_storage, spec_id, txs);
    }
    let seq_tput = total_txs as f64 / seq_start.elapsed().as_secs_f64();
    println!("Sequential throughput: {:.2} tx/s", seq_tput);
    
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
    for (idx, (txs, graph)) in reordered_blocks_txs.iter().zip(dependency_graphs.iter()).enumerate() {
        if idx % 10 == 0 {
            println!("  Processing block {}/{}", idx + 1, reordered_blocks_txs.len());
        }
        graph_par_storage = execute_parallel_with_graph_and_update(
            graph_par_storage, 
            txs.clone(),
            graph.clone(),
            spec_id,
            BlockEnv::default(),
        );
    }
    let graph_par_tput = total_txs as f64 / graph_par_start.elapsed().as_secs_f64();
    println!("Parallel with graphs throughput: {:.2} tx/s", graph_par_tput);
    
    // 4. Greedy integration
    println!("\n=== 5. Greedy Integration ===");
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        tau_cv: 0.1,
        num_threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8),
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
        // 16774645,    // start_block,
        // 18581726,   // start_block,
        100,       // num_blocks
    );
    
    println!("Final results:");
    println!("  Sequential:  {:.2} tx/s", seq);
    println!("  Parallel:    {:.2} tx/s", par);
    println!("  Graph:       {:.2} tx/s", graph_par);
    println!("  Integrated:  {:.2} tx/s", integrated);
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