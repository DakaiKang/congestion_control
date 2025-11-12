
use std::{num::NonZeroUsize, sync::Arc};

use alloy_primitives::{Address, B256, U160, U256};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::distributions::{Distribution, WeightedIndex};
use rand::rngs::ThreadRng;
use rand::{thread_rng};
use pevm::{
    chain::PevmEthereum, execute_revm_sequential, Bytecodes, ChainState, EvmAccount,
    InMemoryStorage, Pevm, execute_revm_sequential_with_access_sets
};
use revm::primitives::{AccessListItem, BlockEnv, SpecId, TransactTo, TxEnv};
use std::io::Write;


#[path = "../benches/gigagas.rs"]
pub mod gigagas;

#[test]
pub fn test_bench_sequential() -> Result<(), Box<dyn std::error::Error>> {
    println!("Running test_bench");
    let concurrency_level = std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN);
    let (storage, txs) = gigagas::solana_sample_2_txns();
    let storage2 = storage.clone();
    let txs2 = txs.clone();

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

    let result2 = Pevm::default().execute_revm_parallel(
        &chain,
        &storage2,
        SpecId::LATEST,
        BlockEnv::default(),
        txs2,
        concurrency_level,
    )?;

    assert_eq!(result, result2);

    println!("YES");

    // println!("result2 = {:#?}", result2);
    let output_file = std::fs::File::create("bench_test.txt").unwrap();
    let mut writer = std::io::BufWriter::new(output_file);
    let result_str = format!("{result:#?}");
    // output the results to a file
    writer.write_all(result_str.as_bytes()).unwrap();
    writer.flush().unwrap();

    Ok(())
}