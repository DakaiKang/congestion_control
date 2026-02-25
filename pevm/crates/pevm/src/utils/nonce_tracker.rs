use crate::storage::Storage;
use crate::storage::block_loader::{load_block_for_execution, create_block_env, get_spec_id, BlockHotResourcesFromExecution};
use std::collections::{HashSet};
use hashbrown::HashMap;
// use serde::{Deserialize, Serialize};
// use serde_json::Value;
// // use std::collections::HashMap;
// use hashbrown::HashMap;
// use std::{
//     collections::{VecDeque, HashSet},
//     num::NonZeroUsize,
//     thread,
//     sync::{Arc, Mutex as StdMutex},
//     fs,
//     error::Error,
// };
// use anyhow::Result;

// // Adjust these imports based on your actual project
// use revm::primitives::{Address, TxEnv, U256, B256, Bytes, AccountInfo, Bytecode, BlockEnv, SpecId, BlobExcessGasAndPrice};
use revm::primitives::{Address, TxEnv};
// use alloy_consensus::TxEnvelope;
// use alloy_consensus::Eip658Value::Eip658;
// use crate::{Bytecodes, ChainState, EvmAccount, InMemoryStorage, , EvmCode, BlockHashes, BuildSuffixHasher, Pevm};
use crate::{InMemoryStorage, execute_revm_sequential_with_access_sets, chain::PevmEthereum};
// use crate::{PevmError, execute_revm_sequential, , PevmResult, graph_pevm::GraphPevm};
// use rustc_hash::FxBuildHasher;

/// Track nonces across blocks
pub struct NonceTracker {
    // address -> nonce
    pub nonces: HashMap<Address, u64>,
}

impl NonceTracker {
    pub fn new() -> Self {
        Self {
            nonces: HashMap::new(),
        }
    }
    
    /// Record nonces from a block's prestate
    pub fn record_from_prestate(
        &mut self,
        block_num: u64,
        storage: &InMemoryStorage,
        txenvs: &[TxEnv],
    ) {
        let addresses: HashSet<Address> = txenvs.iter()
            .map(|tx| tx.caller)
            .collect();
        
        for addr in addresses {
            if !self.nonces.contains_key(&addr) {
                if let Ok(Some(account)) = storage.basic(&addr) {
                    self.nonces.insert(addr, account.nonce);
                }
            }
        }
    }
    
    /// Get nonce for an address (0 if never seen)
    pub fn get_nonce(&self, address: &Address) -> u64 {
        self.nonces.get(address).copied().unwrap_or(0)
    }

    /// Increase nonce for an address by 1
    pub fn increase_nonce(&mut self, address: &Address) {
        let current_nonce = self.nonces.get(address).copied().unwrap_or(0);
        self.nonces.insert(*address, current_nonce + 1);
    }
    
    /// Print statistics
    pub fn print_stats(&self) {
        println!("\n=== Nonce Tracker ===");
        println!("Total accounts tracked: {}", self.nonces.len());
        
        // Group by nonce
        let mut nonce_groups: HashMap<u64, usize> = HashMap::new();
        for (addr, nonce) in &self.nonces {
            *nonce_groups.entry(*nonce).or_insert(0) += 1;
            println!("  Address: {:?}, Nonce: {}", addr, nonce);
        }
        
        println!("Accounts by nonce:");
        let mut sorted: Vec<_> = nonce_groups.iter().collect();
        sorted.sort_by_key(|(nonce, _)| *nonce);
        
        for (nonce, count) in sorted.iter().take(10) {
            println!("  nonce {}: {} accounts", nonce, count);
        }
        
        if sorted.len() > 10 {
            println!("  ... ({} more nonce values)", sorted.len() - 10);
        }
    }
}


#[test]
fn analyze_with_nonce_tracking() -> Result<(), Box<dyn std::error::Error>> {  
    let block_numbers = vec![10646426];
    let hot_threshold = 0.1; // Top 10% as hot resources

    let chain = PevmEthereum::mainnet();
    let mut all_blocks_hot = Vec::new();
    let mut nonce_tracker = NonceTracker::new();
    
    for (idx, block_num) in block_numbers.iter().enumerate() {
        println!("\n=== Block {} ({}/{}) ===", block_num, idx + 1, block_numbers.len());
        
        let filepath = format!("/home/ubuntu/eth-block-downloader/test_data/blocks/block_{}.json", block_num);
        let (block_data, storage, txenvs) = load_block_for_execution(&filepath)?;  
        let spec_id = get_spec_id(*block_num);
        let block_env = create_block_env(&block_data);
        
        // Record nonces from this block's prestate
        nonce_tracker.record_from_prestate(*block_num, &storage, &txenvs); 
        
        let total_transactions = txenvs.len();
        println!("  Transactions: {}", total_transactions);
        
        // Execute
        let (results, access_sets) = execute_revm_sequential_with_access_sets(
            &chain,
            &storage,
            spec_id,
            block_env,
            txenvs,
        )?;
        
        // Count access frequency
        let mut resource_count: HashMap<u64, usize> = HashMap::new();
        for access_set in &access_sets {
            for &key in access_set {
                *resource_count.entry(key).or_insert(0) += 1;
            }
        }
        
        println!("  Unique resources: {}", resource_count.len());
        
        // Sort and get hot resources
        let mut sorted_resources: Vec<_> = resource_count.iter().collect();
        sorted_resources.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        
        let hot_count = (sorted_resources.len() as f64 * hot_threshold).ceil() as usize;
        let hot_resources: Vec<_> = sorted_resources.iter()
            .take(hot_count)
            .map(|(key, count)| (**key, **count))
            .collect();
        
        println!("  Hot resources: {}", hot_resources.len());
        
        let block_hot = BlockHotResourcesFromExecution {
            block_num: *block_num,
            total_transactions,
            resource_access_count: resource_count,
            hot_resources,
        };
        
        all_blocks_hot.push(block_hot);
    }
    
    nonce_tracker.print_stats();
    
    Ok(()) 
}