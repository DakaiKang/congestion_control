//! Each cluster has one ERC20 contract and X families.
//! Each family has Y people.
//! Each person performs Z transfers to random people within the family.

#[path = "../common/mod.rs"]
pub mod common;

#[path = "./mod.rs"]
pub mod erc20;

#[path = "./workload_generation.rs"]
pub mod workload_generation;

use common::test_execute_revm;
use erc20::generate_cluster;
use pevm::chain::PevmEthereum;
use pevm::api;
use pevm::serialization::deserializer;
use pevm::{Bytecodes, ChainState, EvmAccount, InMemoryStorage};
use revm::primitives::{Address, TxEnv};
use std::sync::Arc;

pub use ethers::types::Address as EthAddress;

#[test]
fn erc20_independent() {
    const N: usize = 37123;
    let (mut state, bytecodes, txs) = generate_cluster(N, 1, 1);
    state.insert(Address::ZERO, EvmAccount::default()); // Beneficiary
    test_execute_revm(
        &PevmEthereum::mainnet(),
        InMemoryStorage::new(state, Arc::new(bytecodes), Default::default()),
        txs,
    );
}

#[test]
fn erc20_clusters() {
    const NUM_CLUSTERS: usize = 10;
    const NUM_FAMILIES_PER_CLUSTER: usize = 15;
    const NUM_PEOPLE_PER_FAMILY: usize = 15;
    const NUM_TRANSFERS_PER_PERSON: usize = 15;

    let mut final_state = ChainState::default();
    final_state.insert(Address::ZERO, EvmAccount::default()); // Beneficiary
    let mut final_bytecodes = Bytecodes::default();
    let mut final_txs = Vec::<TxEnv>::new();
    for _ in 0..NUM_CLUSTERS {
        let (state, bytecodes, txs) = generate_cluster(
            NUM_FAMILIES_PER_CLUSTER,
            NUM_PEOPLE_PER_FAMILY,
            NUM_TRANSFERS_PER_PERSON,
        );
        final_state.extend(state);
        final_bytecodes.extend(bytecodes);
        final_txs.extend(txs);
    }
    common::test_execute_revm(
        &PevmEthereum::mainnet(),
        InMemoryStorage::new(final_state, Arc::new(final_bytecodes), Default::default()),
        final_txs,
    )
}


#[tokio::test]
async fn test_workload_generation() -> Result<(), Box<dyn std::error::Error>> {
    workload_generation::generate_workload().await;
    Ok(())
}

#[test]
fn split_test() -> std::io::Result<()> {
    workload_generation::split_file_round_robin("erc_20_workload", 4);
    Ok(())
}

#[test]
fn read_output_test() {
    let code_callers = api::PevmAPI::read_workload_from_file("workload_0.txt").unwrap();
    let first_ten: Vec<(String, EthAddress)> = code_callers.iter().take(1).cloned().collect();
    println!("First ten entries: {:?}", first_ten);
    let result = deserializer::decode_batch_hex(first_ten);
    println!("result: {:#?}", result);
}