/// Integration tests for the TxSimulator workload module.
///
/// These tests verify that:
/// 1. The in-memory storage is correctly initialised with the contract account
///    and 1000 EOA accounts.
/// 2. rw_gas block files are parsed and turned into valid TxEnv batches.
/// 3. The caller-uniqueness constraint is enforced.

#[path = "tx_simulator/mod.rs"]
pub mod tx_simulator;

use pevm::InMemoryStorage;
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
