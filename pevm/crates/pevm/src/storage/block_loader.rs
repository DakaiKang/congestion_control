// src/block_loader.rs

use serde::{Deserialize, Serialize};
use serde_json::Value;
// use std::collections::HashMap;
use hashbrown::HashMap;
use std::fs;
use std::error::Error;
use anyhow::Result;

// Adjust these imports based on your actual project
use revm::primitives::{Address, U256, B256, Bytes, AccountInfo, Bytecode, BlockEnv, SpecId, BlobExcessGasAndPrice};
use alloy_consensus::TxEnvelope;
use revm::primitives::TxEnv;
use crate::{Bytecodes, ChainState, EvmAccount, InMemoryStorage, chain::PevmEthereum, EvmCode, BlockHashes, BuildSuffixHasher};
use crate::{PevmError, execute_revm_sequential, PevmResult};
use rustc_hash::FxBuildHasher;

/// Block data structure from JSON file
#[derive(Debug, Deserialize)]
pub struct BlockData {
    pub number: u64,
    pub timestamp: u64,
    pub hash: String,
    pub transactions: Vec<Value>,
    pub prestate: Value, 

    // Block environment fields
    #[serde(default)]
    pub coinbase: Option<String>,
    #[serde(rename = "gasLimit", default)]
    pub gas_limit: Option<String>,
    #[serde(rename = "baseFeePerGas", default)]
    pub base_fee_per_gas: Option<String>,
    #[serde(default)]
    pub difficulty: Option<String>,
    #[serde(rename = "parentHash", default)] 
    pub parent_hash: Option<String>,
    #[serde(rename = "mixHash", default)]
    pub mix_hash: Option<String>, 
    #[serde(rename = "excessBlobGas", default)]
    pub excess_blob_gas: Option<String>,
}

/// Account data in prestate
#[derive(Debug, Deserialize)]
pub struct PrestateAccount {
    #[serde(default)]
    pub balance: Option<String>,
    #[serde(default)]
    pub nonce: Option<u64>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub storage: Option<HashMap<String, String>>,
}

/// Load block data from file
pub fn load_block_from_file(filepath: &str) -> Result<BlockData> {
    let content = fs::read_to_string(filepath)?;
    println!("Loaded block file: {}", filepath);
    let block_data: BlockData = serde_json::from_str(&content)?;
    Ok(block_data)
}

/// Parse hex string to U256
fn parse_hex_u256(hex: &str) -> U256 {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    if hex.is_empty() || hex == "0" {
        return U256::ZERO;
    }
    U256::from_str_radix(hex, 16).unwrap_or(U256::ZERO)
}

/// Parse hex string to Address
fn parse_hex_address(hex: &str) -> Address {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    Address::from_slice(&hex::decode(hex).unwrap_or_default())
}

/// Parse hex string to Bytes
fn parse_hex_bytes(hex: &str) -> Bytes {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    Bytes::from(hex::decode(hex).unwrap_or_default())
}

/// Parse hex string to B256
fn parse_hex_b256(hex: &str) -> B256 {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes = hex::decode(hex).unwrap_or_else(|_| vec![0u8; 32]);
    B256::from_slice(&bytes)
}

/// Create BlockEnv from block data
pub fn create_block_env(block_data: &BlockData) -> BlockEnv {
    BlockEnv {
        number: U256::from(block_data.number),
        
        coinbase: block_data.coinbase
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default(),
        
        timestamp: U256::from(block_data.timestamp),
        
        gas_limit: block_data.gas_limit
            .as_ref()
            .map(|s| parse_hex_u256(s))
            .unwrap_or(U256::from(u64::MAX)),
        
        basefee: block_data.base_fee_per_gas
            .as_ref()
            .map(|s| parse_hex_u256(s))
            .unwrap_or(U256::ZERO),
        
        difficulty: block_data.difficulty
            .as_ref()
            .map(|s| parse_hex_u256(s))
            .unwrap_or(U256::ZERO),
        
        prevrandao: block_data.mix_hash
            .as_ref()
            .map(|s| parse_hex_b256(s)),

        blob_excess_gas_and_price: block_data.excess_blob_gas
            .as_ref()
            .map(|gas_str| {
                let excess = parse_hex_u64(gas_str);
                BlobExcessGasAndPrice::new(excess, false)  
            }),
    }
}

// Parse hex string to u64
fn parse_hex_u64(hex: &str) -> u64 {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    u64::from_str_radix(hex, 16).unwrap_or(0)
}

/// Convert prestate to InMemoryStorage
pub fn prestate_to_storage(block_data: &BlockData) -> InMemoryStorage {
    use std::sync::Arc;
    
    println!("\n=== DEBUG: prestate_to_storage ===");
    println!("Block number: {}", block_data.number);
    println!("Prestate is_object: {}", block_data.prestate.is_object());
    println!("Prestate is_array: {}", block_data.prestate.is_array());
    
    // Print raw prestate
    if block_data.prestate.is_array() {
        if let Some(arr) = block_data.prestate.as_array() {
            println!("Prestate array length: {}", arr.len());
            // Print first transaction's prestate
            if let Some(first) = arr.first() {
                println!("First tx prestate:");
                println!("{}", serde_json::to_string_pretty(first).unwrap());
            }
        }
    }
    
    let mut chain_state = ChainState::default();
    let mut bytecodes = Bytecodes::default();
    
    // Handle prestate format
    let prestate_map: HashMap<String, PrestateAccount> = if block_data.prestate.is_object() {
        serde_json::from_value(block_data.prestate.clone()).unwrap_or_default()
    } else if block_data.prestate.is_array() {
        let merged = merge_transaction_prestates(&block_data.prestate);
        println!("\n=== After merge ===");
        println!("Merged accounts: {}", merged.len());
        // for (addr, account) in merged.iter() {
        //     println!("Address: {}", addr);
        //     println!("  Balance: {:?}", account.balance);
        //     println!("  Nonce: {:?}", account.nonce);
        //     println!("  Code: {:?}", account.code.as_ref().map(|s| &s[..20.min(s.len())]));
        //     println!("  Storage: {} slots", account.storage.as_ref().map(|s| s.len()).unwrap_or(0));
        // }
        merged
    } else {
        HashMap::new()
    };
    
    // Process each account
    println!("\n=== Converting to EvmAccount ===");
    for (addr_str, account) in &prestate_map {
        let address = parse_hex_address(addr_str);
        
        // Parse balance
        let balance = account.balance.as_ref()
            .map(|b| {
                let val = parse_hex_u256(b);
                println!("Address {}: balance {} -> {:?}", &addr_str[..10], b, val);
                val
            })
            .unwrap_or(U256::ZERO);
        
        // Parse nonce
        let nonce = account.nonce.unwrap_or_else(|| {
            println!("Address {}: nonce is None, defaulting to 0", &addr_str[..10]);
            0
        });
        
        println!("Address {}: final nonce = {}", &addr_str[..10], nonce);
        
        // Parse code
        let code_hash = if let Some(code_hex) = &account.code {
            if code_hex == "0x" || code_hex.is_empty() {
                None
            } else {
                let code_bytes = parse_hex_bytes(code_hex);
                let hash = revm::primitives::keccak256(&code_bytes);
                let bytecode = Bytecode::new_raw(code_bytes);
                bytecodes.insert(hash, bytecode.into());
                println!("Address {}: has code, hash = {:?}", &addr_str[..10], hash);
                Some(hash)
            }
        } else {
            None
        };
        
        // Parse storage slots
        let storage = if let Some(slots) = &account.storage {
            let mut s: HashMap<U256, U256, FxBuildHasher> = 
                HashMap::with_hasher(FxBuildHasher::default());
            for (slot_str, value_str) in slots {
                s.insert(parse_hex_u256(slot_str), parse_hex_u256(value_str));
            }
            println!("Address {}: {} storage slots", &addr_str[..10], s.len());
            s
        } else {
            HashMap::with_hasher(FxBuildHasher::default())
        };
        
        // Create EvmAccount
        let evm_account = EvmAccount {
            balance,
            nonce,
            code_hash,
            code: None,
            storage,
        };
        
        chain_state.insert(address, evm_account);
    }
    
    let mut block_hashes = BlockHashes::default();
    if block_data.number > 0 {
        if let Some(parent_hash_str) = &block_data.parent_hash {
            let parent_hash = parse_hex_b256(parent_hash_str);
            block_hashes.insert(block_data.number - 1, parent_hash);
        }
    }
    
    println!("\n=== Final InMemoryStorage ===");
    println!("Total accounts: {}", chain_state.len());
    println!("Total bytecodes: {}", bytecodes.len());
    
    InMemoryStorage::new(
        chain_state,
        Arc::new(bytecodes),
        Arc::new(block_hashes),
    )
}


/// Parse trace array to prestate format
fn parse_trace_to_prestate(traces: &[Value]) -> HashMap<String, PrestateAccount> {
    let mut prestate = HashMap::new();
    
    for trace in traces {
        if let Some(trace_obj) = trace.as_object() {
            // Extract from stateDiff or other trace formats
            if let Some(state_diff) = trace_obj.get("stateDiff") {
                // Handle stateDiff format
                if let Some(diff_obj) = state_diff.as_object() {
                    for (addr, state) in diff_obj {
                        let account = PrestateAccount {
                            balance: state.get("balance")
                                .and_then(|v| v.get("*"))
                                .and_then(|v| v.get("to"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            nonce: state.get("nonce")
                                .and_then(|v| v.get("*"))
                                .and_then(|v| v.get("to"))
                                .and_then(|v| v.as_u64()),
                            code: state.get("code")
                                .and_then(|v| v.get("*"))
                                .and_then(|v| v.get("to"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            storage: None, // TODO: parse storage diff
                        };
                        prestate.insert(addr.clone(), account);
                    }
                }
            }
        }
    }
    
    prestate
}

/// Convert JSON transactions to TxEnv list
pub fn transactions_to_txenvs(block_data: &BlockData) -> Result<Vec<TxEnv>> {
    let mut txenvs = Vec::new();
    
    println!("\n=== Loading Transactions ===");
    
    for (idx, tx_value) in block_data.transactions.iter().enumerate() {
        let tx_env = parse_transaction(tx_value)?;
        
        let tx_hash = tx_value.get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        
        println!("Transaction {}: hash={}", idx, tx_hash);
        println!("  From: {:?}", tx_env.caller);
        println!("  To: {:?}", tx_env.transact_to);
        
        txenvs.push(tx_env);
    }
    
    println!();
    
    Ok(txenvs)
}

/// Parse a single transaction
fn parse_transaction(tx: &Value) -> Result<TxEnv> {
    let tx_type = tx.get("type")
        .and_then(|v| v.as_str())
        .and_then(|s| u8::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16).ok())
        .unwrap_or(0);

    // Extract fields from JSON
    let from = tx["from"].as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'from' field"))?;
    let from = parse_hex_address(from);
    
    let to = tx["to"].as_str().map(|s| parse_hex_address(s));
    
    let value = tx["value"].as_str()
        .map(|s| parse_hex_u256(s))
        .unwrap_or(U256::ZERO);
    
    let data = tx["input"].as_str()
        .or_else(|| tx["data"].as_str())
        .map(|s| parse_hex_bytes(s))
        .unwrap_or_default();
    
    let gas_limit = tx["gas"].as_str()
        .map(|s| parse_hex_u256(s).to::<u64>())
        .unwrap_or(21000);
    
    let gas_price = tx["gasPrice"].as_str()
        .map(|s| parse_hex_u256(s))
        .or_else(|| {
            // EIP-1559 transactions
            tx["maxFeePerGas"].as_str().map(|s| parse_hex_u256(s))
        })
        .unwrap_or(U256::ZERO);
    
    let nonce = tx["nonce"].as_str()
        .map(|s| parse_hex_u256(s).to::<u64>())
        .unwrap_or(0);
    
    // Create TxEnv
    let mut tx_env = TxEnv {
        caller: from,
        gas_limit,
        gas_price,
        transact_to: if let Some(to_addr) = to {
            revm::primitives::TransactTo::Call(to_addr)
        } else {
            revm::primitives::TransactTo::Create
        },
        value,
        data,
        nonce: Some(nonce),
        chain_id: Some(1), // Mainnet
        access_list: Vec::new(),
        gas_priority_fee: None,
        blob_hashes: Vec::new(),
        max_fee_per_blob_gas: None,
        #[cfg(feature = "optimism")]
        optimism: Default::default(),
        ..Default::default()
    };

    // Blob transaction (type 0x03) - CANCUN
    if tx_type == 0x03 {
        tx_env.blob_hashes = tx.get("blobVersionedHashes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())  // ← filter_map 只用在这里
                    .map(|s| parse_hex_b256(s))  // ← 用 map（不是 filter_map）
                    .collect()
            })
            .unwrap_or_default();
        
            tx_env.max_fee_per_blob_gas = tx.get("maxFeePerBlobGas")
            .and_then(|v| v.as_str())
            .map(|s| parse_hex_u256(s));
    }

    Ok(tx_env)
}

// Parse hex string to u128
fn parse_hex_u128(hex: &str) -> u128 {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    u128::from_str_radix(hex, 16).unwrap_or(0)
}

/// Convenience function: load block and convert in one step
pub fn load_block_for_execution(
    filepath: &str,
) -> Result<(InMemoryStorage, Vec<TxEnv>)> {
    let block_data = load_block_from_file(filepath)?;
    let storage = prestate_to_storage(&block_data);
    let txenvs = transactions_to_txenvs(&block_data)?;
    
    Ok((storage, txenvs))
}

/// Merge prestate from multiple transactions into a single map
fn merge_transaction_prestates(prestate_array: &Value) -> HashMap<String, PrestateAccount> {
    let mut merged = HashMap::new();
    
    if let Some(array) = prestate_array.as_array() {
        for tx_prestate in array {
            if let Some(result) = tx_prestate.get("result") {
                if let Some(accounts) = result.as_object() {
                    for (addr, account_data) in accounts {
                        let entry = merged.entry(addr.clone()).or_insert_with(|| {
                            PrestateAccount {
                                balance: None,
                                nonce: None,
                                code: None,
                                storage: None,
                            }
                        });
                        
                        // 更新 balance/nonce/code (保留第一次出现的)
                        if entry.balance.is_none() {
                            entry.balance = account_data.get("balance")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                        
                        if entry.nonce.is_none() {
                            entry.nonce = account_data.get("nonce")
                                .and_then(|v| v.as_u64());
                        }
                        
                        if entry.code.is_none() {
                            entry.code = account_data.get("code")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                        
                        // 合并 storage (收集所有出现过的 slots)
                        if let Some(new_storage) = account_data.get("storage")
                            .and_then(|v| v.as_object()) 
                        {
                            let storage = entry.storage.get_or_insert_with(HashMap::new);
                            for (k, v) in new_storage {
                                // 只在 slot 第一次出现时插入
                                storage.entry(k.clone()).or_insert_with(|| {
                                    v.as_str().unwrap_or("0x0").to_string()
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    
    merged
}

pub fn get_spec_id(block_num: u64) -> SpecId {
    match block_num {
        0..=1_149_999 => SpecId::FRONTIER,
        1_150_000..=1_919_999 => SpecId::HOMESTEAD,
        1_920_000..=2_462_999 => SpecId::TANGERINE,
        2_463_000..=2_674_999 => SpecId::SPURIOUS_DRAGON,
        2_675_000..=4_369_999 => SpecId::BYZANTIUM,
        4_370_000..=7_279_999 => SpecId::CONSTANTINOPLE,
        7_280_000..=9_068_999 => SpecId::PETERSBURG,
        9_069_000..=9_199_999 => SpecId::ISTANBUL,
        9_200_000..=12_243_999 => SpecId::MUIR_GLACIER,
        12_244_000..=12_964_999 => SpecId::BERLIN,
        12_965_000..=15_537_393 => SpecId::LONDON,
        15_537_394..=17_034_869 => SpecId::MERGE,
        17_034_870..=19_426_589 => SpecId::SHANGHAI,     
        19_426_590.. => SpecId::CANCUN,              
    }
}


// Simple test for loading functionality
#[test]
fn test_load_block() {
    let blocknum = 4_370_000;
    let blocknum = 12_965_000;
    let blocknum = 15_537_393;
    let blocknum = 18_581_726;
    let blocknum = 19_426_587;
    let blocknum = 20_000_000;
    let filepath = "/home/ubuntu/eth-block-downloader/test_data/blocks/block_{}.json".replace("{}", &blocknum.to_string());
    
    println!("Attempting to load: {}", filepath);
    
    // Check if file exists
    if !std::path::Path::new(&filepath).exists() {
        println!("File does not exist at: {}", filepath);
        println!("Current directory: {:?}", std::env::current_dir());
        panic!("File not found");
    }
    
    let result = load_block_from_file(&filepath);
    
    match result {
        Ok(block) => {
            println!("✓ Successfully loaded block");
            println!("  Block number: {}", block.number);
            println!("  Transactions: {}", block.transactions.len());
            
            let txs = transactions_to_txenvs(&block).expect("Failed to convert transactions");            

            // Handle prestate based on its type
            if let Some(obj) = block.prestate.as_object() {
                println!("  Prestate accounts: {}", obj.len());
            } else if let Some(arr) = block.prestate.as_array() {
                println!("  Prestate traces: {}", arr.len());
            } else {
                println!("  Prestate: unknown format");
            }
            let storage = prestate_to_storage(&block);
            println!("First account info: {}", storage.accounts.iter().next().map(|(k, v)| format!("{}: {:?}", k, v)).unwrap_or("None".to_string()));
            let chain = PevmEthereum::mainnet();
            let spec_id = get_spec_id(blocknum);
            let block_env = create_block_env(&block);
            let exec_result = execute_revm_sequential(&chain, &storage, spec_id, block_env, txs);
            // output execution result to file "result.txt"
            match exec_result {
                Ok(result) => {
                    println!("✓ Successfully executed block");
                    std::fs::write("result.txt", format!("{:#?}", result)).expect("Failed to write execution result to file");
                }
                Err(e) => {
                    println!("✗ Error executing block: {}", e);
                    println!("  Cause: {:?}", e.source());
                    panic!("Failed to execute block: {}", e);
                }
            }
        }
        Err(e) => {
            println!("✗ Error loading block: {}", e);
            println!("  Cause: {:?}", e.source());
            panic!("Failed to load block: {}", e);
        }


    }

    
}