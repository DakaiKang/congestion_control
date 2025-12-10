
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
    let (reordered_txns, mut new_graph) = GraphPevm::reorder_txs_by_dependency_graph(txs_clone2, &mut graph1, 8);
    let (reordered_txns2, mut new_graph2) = GraphPevm::reorder_txs_by_dependency_graph(txs2_clone2, &mut graph2, 8);

    new_graph.integrate_graph(new_graph2);

    new_graph.simulate_parallel_execution(8);

    let mut simulated = new_graph.simulation_result.clone().unwrap();

    let mut ro_txns = Vec::new();
    for i in 0..simulated.execution_order.len() {
        let tx_id = simulated.execution_order[i].1.id; 
        let replica = simulated.execution_order[i].1.replica; 
        if replica == 1 {
            ro_txns.push(reordered_txns[tx_id as usize].clone());
        } else {
            ro_txns.push(reordered_txns2[tx_id as usize].clone());
        }
    }
    
    let start = Instant::now();
    let mut pevm = GraphPevm::default();
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();
    
    let result = pevm.execute_revm_parallel(&chain, &storage_cloned2, spec_id, block_env, ro_txns, concurrency_level, new_graph).unwrap();
    let duration4 = start.elapsed();

    ////////////////////////////
    let mut pevm = Pevm::default();
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);

    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let block_env = BlockEnv::default();

    let start = Instant::now();
    let result = pevm.execute_revm_parallel(&chain, &storage_cloned3, spec_id, block_env.clone(), txs_clone3, concurrency_level).unwrap();
    let result = pevm.execute_revm_parallel(&chain, &storage_cloned3, spec_id, block_env, txs2_clone3, concurrency_level).unwrap();
    let duration2 = start.elapsed();

    // println!("Execution time sequential: {:?}", duration1);
    // println!("Execution time parallel: {:?}", duration2);
    // println!("Execution time graph pevm: {:?}", duration3);

    println!("{:?}", duration1);
    println!("{:?}", duration2);
    println!("{:?}", duration3);
    println!("{:?}", duration4);
    
    // println!("result: {:#?}", result);

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

#[test]

pub fn long_test() {
    let txn_num = 10;
    let batch_num = 4;
    let (mut storage, all_batches) = gigagas::solana_samples_vector(txn_num, batch_num, 2);

    let mut storage_history = Vec::new();
    let mut reordered_txns_list = Vec::new();
    let mut new_graphs = Vec::new();
    let mut integrate_graphs = Vec::new();
    let mut integrate_txns = Vec::new();
    let mut integrate_storages = Vec::new();

    for i in 0..batch_num {
        println!("Constructing graph for batch {}", i);
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

    // Integrate graphs every 4 graphs
    for i in 0..batch_num/4 {
        let mut id_to_txn = std::collections::HashMap::new();

        for j in 0..4 {
            if i * 4 + j < batch_num {
                let graph = &new_graphs[i * 4 + j];
                // println!("nodes: {:#?}", graph.nodes);
                let mut x = 0;
                for txn_node in &graph.nodes {
                    // println!("Mapping txn id: {}, replica: {} to txn", txn_node.id, i*4+j);
                    id_to_txn.insert((txn_node.id, i*4+j), reordered_txns_list[i*4+j][x].clone());
                    x += 1;
                }
            }
        }

        let mut integrated_graph = new_graphs[i * 4].clone();
        for j in 1..4 {
            if i * 4 + j < batch_num {
                integrated_graph.integrate_graph(new_graphs[i * 4 + j].clone());
            }
        }
        println!("nodes count in integrated graph: {}", integrated_graph.nodes.len());
        // println!("node: {:#?}", integrated_graph.nodes);
        integrated_graph.simulate_parallel_execution(8);
        let simulated = integrated_graph.simulation_result.clone().unwrap();
        let mut reordered_txns = Vec::new();
        println!("The length of execution order: {}", simulated.execution_order.len());
        for k in 0..simulated.execution_order.len() {
            // println!("Reordering txn id: {}, replica: {}", simulated.execution_order[k].1.id, simulated.execution_order[k].1.replica);
            reordered_txns.push(id_to_txn.get(&(simulated.execution_order[k].1.id, simulated.execution_order[k].1.replica as usize)).unwrap().clone());
        }
        integrate_graphs.push(integrated_graph);
        integrate_txns.push(reordered_txns);
        integrate_storages.push(storage_history[i*4].clone());
    }

    let repeat = 3;

    let start = Instant::now();
    for _ in 0..repeat{
        running_in_sequentially(storage_history.clone(), all_batches.clone());
    }
    let duration = start.elapsed();
    println!("Sequential execution time: {:?}", duration);

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
    println!("Parallel execution time: {:?}", duration1);
    println!("Depedency Graph execution time: {:?}", duration2);

    let start = Instant::now();
    for _ in 0..repeat{
        running_in_dependency_graph(
            integrate_storages.clone(),
            integrate_txns.clone(),
            integrate_graphs.clone(),
        );
    }
    let duration3 = start.elapsed();
    
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
        // println!("Dependency graph batch {} done in {:?}", i, duration);
    }
}