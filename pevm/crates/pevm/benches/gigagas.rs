//! Benchmark mocked blocks that exceed 1 Gigagas.

// TODO: More fancy benchmarks & plots.

use std::{num::NonZeroUsize, sync::Arc};

use alloy_primitives::{Address, B256, U160, U256};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::distributions::{Distribution, WeightedIndex};
use rand::rngs::ThreadRng;
use rand::{thread_rng};
use pevm::{
    chain::PevmEthereum, execute_revm_sequential, Bytecodes, ChainState, EvmAccount,
    InMemoryStorage, Pevm,
};
use revm::primitives::{AccessListItem, BlockEnv, SpecId, TransactTo, TxEnv};

// Better project structure

/// common module
#[path = "../tests/common/mod.rs"]
pub mod common;

/// erc20 module
#[path = "../tests/erc20/mod.rs"]
pub mod erc20;

/// uniswap module
#[path = "../tests/uniswap/mod.rs"]
pub mod uniswap;

/// p2p evaluation data.
#[path = "data/p2p.rs"]
pub mod p2p;

#[path = "../tests/chiron/mod.rs"]
pub mod chiron;

use p2p::{TX_FROM, TX_TO};

///  large gas value
const GIGA_GAS: u64 = 1_000_000_000;

#[cfg(feature = "global-alloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Runs a benchmark for executing a set of transactions on a given blockchain state.
pub fn bench(c: &mut Criterion, name: &str, storage: InMemoryStorage, txs: Vec<TxEnv>) {
    for cores in [6,8,10,12,14,16,18,20] {
        let concurrency_level = NonZeroUsize::new(cores).unwrap();
        let chain = PevmEthereum::mainnet();
        let spec_id = SpecId::LATEST;
        let block_env = BlockEnv::default();
        let mut pevm = Pevm::default();
        let mut group = c.benchmark_group(name);
        group.sample_size(10);

        assert_eq!( execute_revm_sequential(
                    black_box(&chain),
                    black_box(&storage),
                    black_box(spec_id),
                    black_box(block_env.clone()),
                    black_box(txs.clone()),
                ),
            pevm.execute_revm_parallel(
                    black_box(&chain),
                    black_box(&storage),
                    black_box(spec_id),
                    black_box(block_env.clone()),
                    black_box(txs.clone()),
                    black_box(concurrency_level)
                ));


        group.bench_function(&format!("Sequential {}", cores), |b| {
            b.iter(|| {
                execute_revm_sequential(
                    black_box(&chain),
                    black_box(&storage),
                    black_box(spec_id),
                    black_box(block_env.clone()),
                    black_box(txs.clone()),
                )
            })
        });
        group.bench_function(&format!("Parallel BlockSTM {}", cores), |b| {
            b.iter(|| {
                pevm.execute_revm_parallel(
                    black_box(&chain),
                    black_box(&storage),
                    black_box(spec_id),
                    black_box(block_env.clone()),
                    black_box(txs.clone()),
                    black_box(concurrency_level)
                )
            })
        });
        group.bench_function(&format!("Parallel Chiron {}", cores), |b| {
            b.iter(|| {
                pevm.execute_revm_parallel(
                    black_box(&chain),
                    black_box(&storage),
                    black_box(spec_id),
                    black_box(block_env.clone()),
                    black_box(txs.clone()),
                    black_box(concurrency_level)
                )
            })
        });
        group.finish();
    }
}

/*
     Running benches/gigagas.rs (target/release/deps/gigagas-970a7d3e6ee19c18)
Benchmarking Contended Uniswap/Sequential: Warming up for 3.0000 s
Warning: Unable to complete 100 samples in 5.0s. You may wish to increase target time to 77.2s, or reduce sample count to 10.
Contended Uniswap/Sequential
                        time:   [726.08 ms 731.20 ms 736.62 ms]
Found 3 outliers among 100 measurements (3.00%)
  3 (3.00%) high mild
Benchmarking Contended Uniswap/Parallel BlockSTM: Warming up for 3.0000 s
Warning: Unable to complete 100 samples in 5.0s. You may wish to increase target time to 38.9s, or reduce sample count to 10.
Contended Uniswap/Parallel BlockSTM
                        time:   [365.47 ms 369.01 ms 372.69 ms]
                        change: [+1.8751% +3.1188% +4.4444%] (p = 0.00 < 0.05)
                        Performance has regressed.
Benchmarking Contended Uniswap/Parallel Chiron: Warming up for 3.0000 s
Warning: Unable to complete 100 samples in 5.0s. You may wish to increase target time to 32.4s, or reduce sample count to 10.
Contended Uniswap/Parallel Chiron
                        time:   [310.23 ms 312.01 ms 313.94 ms]
                        change: [-2.8241% -1.9159% -1.0263%] (p = 0.00 < 0.05)
                        Performance has improved.
Found 6 outliers among 100 measurements (6.00%)
  5 (5.00%) high mild
  1 (1.00%) high severe

 */

/// Benchmarks the execution time of raw token transfers.
pub fn bench_raw_transfers(c: &mut Criterion) {
    let block_size = 10_000;
    // Skip the built-in precompiled contracts addresses.
    const START_ADDRESS: usize = 1000;

    const MINER_ADDRESS: usize = 0;
    let storage = InMemoryStorage::new(
        std::iter::once(MINER_ADDRESS)
            .chain(START_ADDRESS..100_000)
            .map(common::mock_account)
            .collect(),
        Default::default(),
        Default::default(),
    );

    let p2p_receiver_distribution: WeightedIndex<f64> = WeightedIndex::new(&TX_TO).unwrap();
    let p2p_sender_distribution: WeightedIndex<f64> = WeightedIndex::new(&TX_FROM).unwrap();
    let mut rng: ThreadRng = thread_rng();

    bench(
        c,
        "Independent Raw Transfers",
        storage,
        (0..block_size)
            .map(|_| {
                let sender = Address::from(U160::from(START_ADDRESS + p2p_sender_distribution.sample(&mut rng)));
                let receiver = Address::from(U160::from(START_ADDRESS + p2p_receiver_distribution.sample(&mut rng)));

                TxEnv {
                    caller: sender,
                    transact_to: TransactTo::Call(receiver),
                    value: U256::from(1),
                    gas_limit: common::RAW_TRANSFER_GAS_LIMIT,
                    gas_price: U256::from(1),
                    access_list: vec!(AccessListItem {address: sender, storage_keys: vec!(B256::ZERO)}, AccessListItem {address: receiver, storage_keys: vec!(B256::ZERO)}),
                    ..TxEnv::default()
                }
            })
            .collect::<Vec<_>>()
    );
}

/// Benchmarks the execution time of ERC-20 token transfers.
pub fn bench_erc20(c: &mut Criterion) {
    let block_size = 10_000;
    let (mut state, bytecodes, txs) = erc20::generate_cluster(block_size, 1, 1);
    state.insert(Address::ZERO, EvmAccount::default()); // Beneficiary
    bench(
        c,
        "Independent ERC20",
        InMemoryStorage::new(state, Arc::new(bytecodes), Default::default()),
        txs
    );
}

// /// Benchmark the execution time of erc20 transactions.
// pub fn chiron_bench_erc20(c: &mut Criterion) {
//     let block_size = 10_000;
//     let (mut state, bytecodes, txs) = erc20::generate_chiron_cluster(block_size);
//     state.insert(Address::ZERO, EvmAccount::default()); // Beneficiary
//     bench(
//         c,
//         format!("Chiron ERC20:").as_str(),
//         InMemoryStorage::new(state, Arc::new(bytecodes), Default::default()),
//         txs
//     );
// }

// /// Benchmarks the execution time of Uniswap V3 swap transactions.
// pub fn chiron_bench_uniswap(c: &mut Criterion, bursty: bool) {
//     let block_size = 10_000;
//     let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]); // Beneficiary

//     let mut final_bytecodes = Bytecodes::default();
//     let mut final_txs = Vec::<TxEnv>::new();

//     let (state, bytecodes, txs) = uniswap::generate_trading_history(block_size, bursty);
//     final_state.extend(state);
//     final_bytecodes.extend(bytecodes);
//     final_txs.extend(txs);

//     let load_type;
//     if bursty {
//         load_type = "bursty";
//     } else {
//         load_type = "avg";
//     }

//     bench(
//         c,
//         format!("Chiron Uniswap: {} ", load_type).as_str(),
//         InMemoryStorage::new(final_state, Arc::new(final_bytecodes), Default::default()),
//         final_txs
//     );
// }

/// Benchmarks the execution time of Uniswap V3 swap transactions.
pub fn bench_uniswap(c: &mut Criterion) {
    let block_size = 10_000;
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]); // Beneficiary
    let mut final_bytecodes = Bytecodes::default();
    let mut final_txs = Vec::<TxEnv>::new();
    for _ in 0..block_size {
        let (state, bytecodes, txs) = uniswap::generate_cluster(1, 1);
        final_state.extend(state);
        final_bytecodes.extend(bytecodes);
        final_txs.extend(txs);
    }
    bench(
        c,
        "Independent Uniswap",
        InMemoryStorage::new(final_state, Arc::new(final_bytecodes), Default::default()),
        final_txs
    );
}

/// Benchmarks the execution time of Solana/Mixed tx
pub fn bench_solana(c: &mut Criterion) {
    let block_size = 100;
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]); // Beneficiary
    let (state, bytecodes, txs) = chiron::generate_loop_exchange(block_size);
    final_state.extend(state);

    bench(
        c,
        "Solana",
        InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()),
        txs
    );
    println!("The bench test is done");

}


pub fn solana_sample_2_txns() -> (InMemoryStorage, Vec<TxEnv>) {
    let block_size = 1;
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);
    let (state, bytecodes, txs) = chiron::generate_loop_exchange(block_size);
    final_state.extend(state);

    (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), txs)
}

pub fn solana_sample_txns(block_size: usize) -> (InMemoryStorage, Vec<TxEnv>, Vec<u64>) {
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);
    let (state, bytecodes, txs, costs) = chiron::generate_loop_exchange_with_cost(block_size);
    final_state.extend(state);

    (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), txs, costs)
}

pub fn solana_samples(block_size: usize, workload_type: u64) -> (InMemoryStorage, Vec<TxEnv>, Vec<u64>) {
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);
    if workload_type == 1 {
        let (state, bytecodes, txs, costs) = chiron::generate_single_sender_loop_exchange(block_size);
        final_state.extend(state);
        return (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), txs, costs)
    }
    else if workload_type == 2 {
        let (state, bytecodes, txs, costs) = chiron::generate_dense_multiple_sender_loop_exchange(block_size, 8);
        final_state.extend(state);
        return (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), txs, costs)
    }
    else {
        return solana_sample_txns(block_size);
    }
}

pub fn solana_samples_vector(block_size: usize, batch_num: usize, workload_type: u64) 
    -> (InMemoryStorage, Vec<Vec<TxEnv>>) {
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);
    let (state, bytecodes, all_batches) = chiron::generate_loop_exchange_batches(block_size, batch_num);
    final_state.extend(state);
    return (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), all_batches)
}


pub fn solana_sample_txns_two_batch(block_size: usize) -> (InMemoryStorage, Vec<TxEnv>, Vec<u64>, Vec<TxEnv>, Vec<u64>) {
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);
    let (state, bytecodes, txs, costs, txs2, costs2) = chiron::generate_loop_exchange_with_cost_two_batch(block_size);
    final_state.extend(state);

    (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), txs, costs, txs2, costs2)
}

pub fn solana_single_sender_txns(block_size: usize) -> (InMemoryStorage, Vec<TxEnv>, Vec<u64>) {
    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);
    let (state, bytecodes, txs, costs) = chiron::generate_single_sender_loop_exchange(block_size);
    final_state.extend(state);

    (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), txs, costs)
}

pub fn conflict_workloads(num_blocks: usize, num_tx_per_block: usize) -> (InMemoryStorage, Vec<Vec<TxEnv>>) {
    let config = chiron::BlockGenerationConfig {
        address_space_per_block: 100,
        common_access_ratio: 0.01,
        zipf_theta: 1.5,
        avg_accesses_per_tx: 2.0,
    };

    let mut final_state = ChainState::from_iter([(Address::ZERO, EvmAccount::default())]);

    // Generate 50 blocks, each with 100 transactions
    let (state, bytecodes, blocks_txs) = chiron::generate_n_blocks_with_controlled_conflicts(
        num_blocks,   // num_blocks
        num_tx_per_block,  // num_tx_per_block
        config,
    );

    final_state.extend(state);

    (InMemoryStorage::new(final_state, Arc::new(bytecodes), Default::default()), blocks_txs)
}


/// Runs a series of benchmarks to evaluate the performance of different transaction types.
pub fn benchmark_gigagas(c: &mut Criterion) {

    //bench_raw_transfers(c);
    //bench_erc20(c);
    //bench_uniswap(c);

    // The erc bench has around 1600/10k re-executions. Not that much, not that little.
    // They all seem to come from executiom, not from validation though, which is weird.
    //chiron_bench_erc20(c);

    //chiron_bench_erc20(c);
    //chiron_bench_uniswap(c, true);
    //chiron_bench_uniswap(c, false);

    bench_solana(c);
}

// HACK: we can't document public items inside of the macro
#[allow(missing_docs)]
mod benches {
    use super::*;
    criterion_group!(benches, benchmark_gigagas);
}

criterion_main!(benches::benches);