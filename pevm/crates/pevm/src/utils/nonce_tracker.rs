use crate::storage::Storage;
use crate::storage::block_loader::{load_block_for_execution, create_block_env, get_spec_id, BlockHotResourcesFromExecution};
use std::collections::{HashSet};
use hashbrown::HashMap;
use anyhow::Result;
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
use revm::primitives::{Address, TxEnv, TransactTo};
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

    /// Set nonce for an address
    pub fn set_nonce(&mut self, address: &Address, nonce: u64) {
        self.nonces.insert(*address, nonce);
    }

    /// Increase nonce for an address by 1
    pub fn increase_nonce(&mut self, address: &Address) {
        let current_nonce = self.nonces.get(address).copied().unwrap_or(0);
        self.nonces.insert(*address, current_nonce + 1);
    }

    /// Update TxEnv list nonces based on tracked nonces
    /// Each transaction from the same sender gets nonce, nonce+1, nonce+2, etc.
    pub fn update_txenv_nonces(&mut self, txenvs: &mut [TxEnv]) {
        for tx in txenvs.iter_mut() {
            let current_nonce = self.get_nonce(&tx.caller);
            tx.nonce = Some(current_nonce);

            // Increase nonce for next transaction from this sender
            self.increase_nonce(&tx.caller);

            // EIP-7702: each authorization in a set-code tx bumps its recovered
            // authority's nonce by 1 during execution. After reordering, a later
            // tx from that authority must observe the bump, otherwise its
            // reassigned nonce is too low ("nonce too high" at validation).
            if let Some(auth_list) = &tx.authorization_list {
                for auth in auth_list.recovered_iter() {
                    if let Some(authority) = auth.authority() {
                        self.increase_nonce(&authority);
                    }
                }
            }
        }
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
fn test_nonce_tracker_incremental_update() -> Result<()> {
    println!("\n=== Test: Incremental Nonce Update ===\n");
    
    let alice = Address::from([1u8; 20]);
    let bob = Address::from([2u8; 20]);
    
    let mut nonce_tracker = NonceTracker::new();
    nonce_tracker.set_nonce(&alice, 5);
    nonce_tracker.set_nonce(&bob, 10);
    
    println!("Initial nonces:");
    println!("  Alice: {}", nonce_tracker.get_nonce(&alice));
    println!("  Bob: {}", nonce_tracker.get_nonce(&bob));
    
    // Create transactions
    let mut txenvs = vec![
        TxEnv {
            caller: alice,
            nonce: None,
            gas_limit: 21000,
            transact_to: TransactTo::Call(bob),
            ..Default::default()
        },
        TxEnv {
            caller: bob,
            nonce: None,
            gas_limit: 21000,
            transact_to: TransactTo::Call(alice),
            ..Default::default()
        },
        TxEnv {
            caller: alice,  // Alice sends another tx
            nonce: None,
            gas_limit: 21000,
            transact_to: TransactTo::Call(bob),
            ..Default::default()
        },
        TxEnv {
            caller: bob,  // Bob sends another tx
            nonce: None,
            gas_limit: 21000,
            transact_to: TransactTo::Call(alice),
            ..Default::default()
        },
    ];
    
    println!("\nBefore update:");
    for (idx, tx) in txenvs.iter().enumerate() {
        println!("  Tx {}: caller={:?}, nonce={:?}", 
                 idx, tx.caller, tx.nonce);
    }
    
    // Update with incremental nonces
    nonce_tracker.update_txenv_nonces(&mut txenvs);
    
    println!("\nAfter update:");
    for (idx, tx) in txenvs.iter().enumerate() {
        println!("  Tx {}: caller={:?}, nonce={:?}", 
                 idx, tx.caller, tx.nonce);
    }
    
    println!("\nFinal tracker state:");
    println!("  Alice: {}", nonce_tracker.get_nonce(&alice));
    println!("  Bob: {}", nonce_tracker.get_nonce(&bob));
    
    // Verify
    assert_eq!(txenvs[0].nonce, Some(5));   // Alice: 5
    assert_eq!(txenvs[1].nonce, Some(10));  // Bob: 10
    assert_eq!(txenvs[2].nonce, Some(6));   // Alice: 6
    assert_eq!(txenvs[3].nonce, Some(11));  // Bob: 11
    
    assert_eq!(nonce_tracker.get_nonce(&alice), 7);   // Alice used 5, 6
    assert_eq!(nonce_tracker.get_nonce(&bob), 12);    // Bob used 10, 11
    
    println!("\n✓ All nonces incremented correctly!");
    
    Ok(())
}


