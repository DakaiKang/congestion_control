
use std::{num::NonZeroUsize, sync::Arc};
use std::time::Instant;
use alloy_primitives::{Address, B256, U160, U256};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::distributions::{Distribution, WeightedIndex};
use rand::rngs::ThreadRng;
use rand::{thread_rng};
use pevm::{
    graph_pevm::GraphPevm,
    chain::PevmEthereum, execute_revm_sequential, Bytecodes, ChainState, EvmAccount,
    InMemoryStorage, Pevm, execute_revm_sequential_with_access_sets
};
use pevm::dependency_graph::{
    TransactionGraph, TransactionNode, SimulationResult
};
use pevm::graph_scheduler::GraphScheduler;

use std::collections::HashSet;

use revm::primitives::{AccessListItem, BlockEnv, SpecId, TransactTo, TxEnv};
use std::io::Write;


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
    let txn_num = 5;
    let (storage, txs, costs, txs2, costs2) = gigagas::solana_sample_txns_two_batch(txn_num);

    let txs_clone = txs.clone();
    let txs2_clone = txs2.clone();
    let storage2 = storage.clone();
    let storage_cloned = storage.clone();
    let storage2_cloned = storage.clone();

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    println!("txns: {:#?}", txs);

    let mut graph = GraphPevm::construct_graph_pevm_by_sequential(&chain, &storage, spec_id, block_env, txs_clone).unwrap();

    let (reordered_txns, new_graph) = GraphPevm::reorder_txs_by_dependency_graph(txs, &mut graph, 8);

    println!("reorder_txns: {:#?}", reordered_txns);

    // let graph_scheduler = GraphScheduler::new(txn_num, new_graph);
    // println!("graph_scheduler: {:#?}", graph_scheduler);

    // for i in 0..txn_num {
    //     let task = graph_scheduler.next_task();
    //     println!("next_task: {:#?}", task);

    //     match task {
    //         Some(t) => {
    //             match t {
    //                 pevm::Task::Execution(tx_version) => {
    //                     graph_scheduler.finish_execution(tx_version, pevm::FinishExecFlags::NeedValidation);
    //                 },
    //                 pevm::Task::Validation(tx_version) => {
    //                     println!("Validating tx_version: {:#?}", tx_version);
    //                 },
    //             }
    //         }
    //         None => {
    //             println!("No task available");
    //         }
    //     }
    //     println!("---");
    // }

    let mut pevm = GraphPevm::default();
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    let result = pevm.execute_revm_parallel(&chain, &storage, spec_id, block_env, reordered_txns, concurrency_level, new_graph).unwrap();

    println!("result: {:#?}", result);

    // let output_file = std::fs::File::create("constructed.txt").unwrap();
    // let mut writer = std::io::BufWriter::new(output_file);
    // let execution_str = format!("{graph:#?}");
    // // output the results to a file
    // writer.write_all(execution_str.as_bytes()).unwrap();
    // writer.flush().unwrap();

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