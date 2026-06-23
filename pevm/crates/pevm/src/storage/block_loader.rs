// src/block_loader.rs

use serde::{Deserialize, Serialize};
use serde_json::Value;
// use std::collections::HashMap;
use hashbrown::HashMap;
use std::{
    collections::{VecDeque, HashSet},
    num::NonZeroUsize,
    thread,
    sync::{Arc, Mutex as StdMutex},
    fs,
    error::Error,
};
use anyhow::Result;

// Adjust these imports based on your actual project
use revm::primitives::{Address, TxEnv, U256, B256, Bytes, AccountInfo, Bytecode, BlockEnv, SpecId, BlobExcessGasAndPrice};
use revm::primitives::{Authorization, AuthorizationList, SignedAuthorization};
use alloy_consensus::TxEnvelope;
use alloy_consensus::Eip658Value::Eip658;
use crate::{Bytecodes, ChainState, EvmAccount, InMemoryStorage, chain::PevmEthereum, EvmCode, BlockHashes, BuildSuffixHasher, Pevm};
use crate::{PevmError, execute_revm_sequential, execute_revm_sequential_with_access_sets, PevmResult, graph_pevm::GraphPevm};
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
    println!("Prestate is_array: {}", block_data.prestate.is_array());
    
    let mut chain_state = ChainState::default();
    let mut bytecodes = Bytecodes::default();
    
    // Handle prestate format
    let prestate_map: HashMap<String, PrestateAccount> = if block_data.prestate.is_object() {
        serde_json::from_value(block_data.prestate.clone()).unwrap_or_default()
    } else if block_data.prestate.is_array() {
        let merged = merge_transaction_prestates(&block_data.prestate);
        println!("\n=== After merge ===");
        println!("Merged accounts: {}", merged.len());
        for (addr, account) in merged.iter() {
            // println!("Address: {}", addr);
            // println!("  Balance: {:?}", account.balance);
            // println!("  Nonce: {:?}", account.nonce);
            // println!("  Code: {:?}", account.code.as_ref().map(|s| &s[..20.min(s.len())]));
            // println!("  Storage: {} slots", account.storage.as_ref().map(|s| s.len()).unwrap_or(0));
        }
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
                // println!("Address {}: balance {} -> {:?}", &addr_str[..10], b, val);
                val
            })
            .unwrap_or(U256::ZERO);
        
        // Parse nonce
        let nonce = account.nonce.unwrap_or_else(|| {
            // println!("Address {}: nonce is None, defaulting to 0", &addr_str[..10]);
            0
        });
        
        // println!("Address {}: final nonce = {}", &addr_str[..10], nonce);
        
        // Parse code
        let code_hash = if let Some(code_hex) = &account.code {
            if code_hex == "0x" || code_hex.is_empty() {
                None
            } else {
                let code_bytes = parse_hex_bytes(code_hex);
                let hash = revm::primitives::keccak256(&code_bytes);
                let bytecode = Bytecode::new_raw(code_bytes);
                bytecodes.insert(hash, bytecode.into());
                // println!("Address {}: has code, hash = {:?}", &addr_str[..10], hash);
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
            // println!("Address {}: {} storage slots", &addr_str[..10], s.len());
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
        
        // println!("Transaction {}: hash={}", idx, tx_hash);
        // println!("  From: {:?}", tx_env.caller);
        // println!("  To: {:?}", tx_env.transact_to);
        
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

    // EIP-7702 set-code transaction (type 0x04) - PRAGUE. Each authorization
    // bumps its (recovered) authority's nonce by 1 during execution, so the
    // list must be carried or later txs from those authorities see a stale
    // nonce ("nonce too high").
    if tx_type == 0x04 {
        if let Some(auths) = tx.get("authorizationList").and_then(|v| v.as_array()) {
            let signed: Vec<SignedAuthorization> = auths
                .iter()
                .map(|a| {
                    let inner = Authorization {
                        chain_id: a.get("chainId").and_then(|v| v.as_str())
                            .map(parse_hex_u256).unwrap_or(U256::ZERO),
                        address: parse_hex_address(
                            a.get("address").and_then(|v| v.as_str()).unwrap_or("0x0"),
                        ),
                        nonce: a.get("nonce").and_then(|v| v.as_str())
                            .map(|s| parse_hex_u256(s).to::<u64>()).unwrap_or(0),
                    };
                    let y_parity = a.get("yParity").or_else(|| a.get("v"))
                        .and_then(|v| v.as_str())
                        .map(|s| parse_hex_u256(s).to::<u64>() as u8)
                        .unwrap_or(0);
                    let r = a.get("r").and_then(|v| v.as_str())
                        .map(parse_hex_u256).unwrap_or(U256::ZERO);
                    let s = a.get("s").and_then(|v| v.as_str())
                        .map(parse_hex_u256).unwrap_or(U256::ZERO);
                    SignedAuthorization::new_unchecked(inner, y_parity, r, s)
                })
                .collect();
            tx_env.authorization_list = Some(AuthorizationList::Signed(signed));
        }
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
    large_gas_limit: bool
) -> Result<(BlockData, InMemoryStorage, Vec<TxEnv>)> {
    let block_data = load_block_from_file(filepath)?;
    let storage = prestate_to_storage(&block_data);
    let mut txenvs = transactions_to_txenvs(&block_data)?;
    if large_gas_limit {
        for txenv in &mut txenvs {    
            if txenv.gas_limit < 30_000_000 {
                txenv.gas_limit = 30_000_000; 
            }
        }
    }
    
    Ok((block_data, storage, txenvs))
}

/// Merge prestate from multiple transactions into a single map
fn merge_transaction_prestates(prestate_array: &Value) -> HashMap<String, PrestateAccount> {
    let mut merged = HashMap::new();
    
    if let Some(array) = prestate_array.as_array() {
        if let Some(first_tx_prestate) = array.first() {
            if let Some(result) = first_tx_prestate.get("result") {
                if let Some(accounts) = result.as_object() {
                    for (addr, account_data) in accounts {
                        let account = PrestateAccount {
                            balance: account_data.get("balance")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            
                            nonce: account_data.get("nonce")
                                .and_then(|v| v.as_u64()).or_else(|| Some(0)),
                            
                            code: account_data.get("code")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            
                            storage: account_data.get("storage")
                                .and_then(|v| v.as_object())
                                .map(|obj| {
                                    obj.iter()
                                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("0x0").to_string()))
                                        .collect()
                                }),
                        };
                        
                        merged.insert(addr.clone(), account);
                    }
                }
            }
        }
        
        for tx_prestate in array.iter().skip(1) {
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
                        
                        if entry.balance.is_none() {
                            entry.balance = account_data.get("balance")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                        
                        if entry.nonce.is_none() {
                            entry.nonce = account_data.get("nonce")
                                .and_then(|v| v.as_u64()).or_else(|| Some(0));
                        }
                        
                        if entry.code.is_none() {
                            entry.code = account_data.get("code")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                        
                        if let Some(new_storage) = account_data.get("storage")
                            .and_then(|v| v.as_object()) 
                        {
                            let storage = entry.storage.get_or_insert_with(HashMap::new);
                            for (k, v) in new_storage {
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

/// Merge prestates from multiple blocks by block numbers
pub fn merge_multiple_blocks_prestates(
    block_numbers: &[u64],
    blocks_dir: &str,
) -> Result<HashMap<String, PrestateAccount>> {
    let mut merged = HashMap::new();

    for block_num in block_numbers {
        let filepath = format!("{}/block_{}.json", blocks_dir, block_num);
        println!("Loading prestate from block {}: {}", block_num, filepath);
        
        // Load block data
        let block_data = load_block_from_file(&filepath)?;
        
        // Merge this block's prestate
        let block_prestate = merge_transaction_prestates(&block_data.prestate);
        
        // Merge into the global map
        for (addr, account) in block_prestate {
            merge_account_into(&mut merged, addr, account);
        }
    }
    
    println!("Total unique addresses across all blocks: {}", merged.len());
    
    Ok(merged)
}

/// Merge a single account into the existing map
/// Strategy: Keep the latest (most recent) non-None values
fn merge_account_into(
    merged: &mut HashMap<String, PrestateAccount>,
    addr: String,
    new_account: PrestateAccount,
) {
    let entry = merged.entry(addr).or_insert_with(|| {
        PrestateAccount {
            balance: None,
            nonce: None,
            code: None,
            storage: None,
        }
    });
    
    if entry.balance.is_none() && new_account.balance.is_some() {
        entry.balance = new_account.balance;
    }
    
    if entry.nonce.is_none() && new_account.nonce.is_some() {
        entry.nonce = new_account.nonce;
    }
    
    if entry.code.is_none() && new_account.code.is_some() {
        entry.code = new_account.code;
    }
    
    if let Some(new_storage) = new_account.storage {
        let storage = entry.storage.get_or_insert_with(HashMap::new);
        for (k, v) in new_storage {
            storage.entry(k).or_insert(v); 
        }
    }
}

/// Create a combined InMemoryStorage from multiple blocks
pub fn create_multi_block_storage(
    block_numbers: &[u64],
    blocks_dir: &str,
) -> Result<InMemoryStorage> {
    // Merge all prestates
    let merged_prestate = merge_multiple_blocks_prestates(block_numbers, blocks_dir)?;

    // Get the first block's number and parent_hash for block_hashes
    let first_filepath = format!("{}/block_{}.json", blocks_dir, block_numbers[0]);
    let first_block = load_block_from_file(&first_filepath)?;
    
    // Convert to InMemoryStorage
    let mut chain_state = ChainState::default();
    let mut bytecodes = Bytecodes::default();

    // Inflate Balance to avoid Lack of Funds errors during integrated execution with transaction reordering across blocks, which is not well handled by the current implementation of Vm. 

    let huge_balance = U256::from(1_000_000u128) * U256::from(10u128.pow(18)); // 1M ETH
    
    for (addr_str, account) in &merged_prestate {
        let address = parse_hex_address(addr_str);
        
        let original_balance = account.balance.as_ref()
            .map(|b| parse_hex_u256(b))
            .unwrap_or(U256::ZERO);
        
        let balance = if original_balance < huge_balance {
            huge_balance
        } else {
            original_balance
        };
        
        let nonce = account.nonce.unwrap_or(0);
        
        let code_hash = if let Some(code_hex) = &account.code {
            if code_hex == "0x" || code_hex.is_empty() {
                None
            } else {
                let code_bytes = parse_hex_bytes(code_hex);
                let hash = revm::primitives::keccak256(&code_bytes);
                let bytecode = Bytecode::new_raw(code_bytes);
                bytecodes.insert(hash, bytecode.into());
                Some(hash)
            }
        } else {
            None
        };
        
        let storage = if let Some(slots) = &account.storage {
            let mut s: HashMap<U256, U256, FxBuildHasher> = 
                HashMap::with_hasher(FxBuildHasher::default());
            for (slot_str, value_str) in slots {
                s.insert(parse_hex_u256(slot_str), parse_hex_u256(value_str));
            }
            s
        } else {
            HashMap::with_hasher(FxBuildHasher::default())
        };
        
        let evm_account = EvmAccount {
            balance,
            nonce,
            code_hash,
            code: None,
            storage,
        };
        
        chain_state.insert(address, evm_account);
    }

    // Create block hashes
    let mut block_hashes = BlockHashes::default();
    if first_block.number > 0 {
        if let Some(parent_hash_str) = &first_block.parent_hash {
            let parent_hash = parse_hex_b256(parent_hash_str);
            block_hashes.insert(first_block.number - 1, parent_hash);
        }
    }
    
    
    Ok(InMemoryStorage::new(
        chain_state,
        Arc::new(bytecodes),
        Arc::new(block_hashes),
    ))
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
        19_426_590..=22_431_083 => SpecId::CANCUN,
        // Prague (Pectra) activated on mainnet at block 22_431_084
        // (timestamp 1746612311, 2025-05-07). EIP-7702 changes nonce
        // semantics, so pre-Prague specs reject these blocks' txs.
        22_431_084.. => SpecId::PRAGUE,
    }
}


// Simple test for loading functionality
#[test]
fn test_load_block() {
    // let blocknum = 4_370_000;
    // let blocknum = 12_965_000;
    // let blocknum = 15_537_393;
    // let blocknum = 18_581_726;
    // let blocknum = 19_426_587;
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
            let concurrency_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
            // Sequential
            // let exec_result = execute_revm_sequential(&chain, &storage, spec_id, block_env, txs);

            // OCC
            // let exec_result = Pevm::default().execute_revm_parallel(&chain, &storage, spec_id, block_env, txs, concurrency_level);
            
            // OCC + Dependency Graph
            let (mut graph1, r) = GraphPevm::construct_graph_pevm_by_sequential(&chain, &storage, spec_id, block_env.clone(), txs.clone(), 1).unwrap();
            let (txs, mut new_graph) = GraphPevm::reorder_txs_by_dependency_graph(txs, &mut graph1, 8);
            let mut pevm = GraphPevm::default();
            let exec_result = pevm.execute_revm_parallel(&chain, &storage, spec_id, block_env, txs, concurrency_level, new_graph);

            // output execution result to file "result.txt"
            match exec_result {
                Ok(result) => {
                    let total = result.len();
                    let successful = result.iter()
                        .filter(|r| matches!(r.receipt.status, Eip658(true)))
                        .count();
                    let failed = total - successful;
                    println!("✓ Successfully executed block");
                    println!("✅ Executed {} transactions, {} failed.", total, failed);
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

/// Hot resources statistics based on actual execution access sets
#[derive(Debug)]
pub struct BlockHotResourcesFromExecution {
    pub block_num: u64,
    pub total_transactions: usize,
    pub resource_access_count: HashMap<u64, usize>,  // key -> access_count
    pub hot_resources: Vec<(u64, usize)>,            // (key, access_count) sorted
}

impl BlockHotResourcesFromExecution {
    pub fn print_hot_details(&self) {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║ Block {:<10}                                              ║", self.block_num);
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ Total Transactions:    {:<8}                               ║", self.total_transactions);
        println!("║ Total Unique Resources: {:<8}                               ║", self.resource_access_count.len());
        println!("║ Hot Resources (top 20%): {:<8}                              ║", self.hot_resources.len());
        println!("╚════════════════════════════════════════════════════════════════╝");
        
        println!("\nTop Hot Resources:");
        println!("  {:<20} {:>10} {:>12}", "Resource Key", "Accesses", "Ratio");
        println!("  {}", "-".repeat(45));
        
        for (key, count) in self.hot_resources.iter().take(20) {
            let ratio = *count as f64 / self.total_transactions as f64;
            println!("  {:<20} {:>10} {:>11.2}%", 
                     format!("0x{:016x}", key), 
                     count, 
                     ratio * 100.0);
        }
    }
}

/// Analyze hot resources from execution access sets
pub fn analyze_hot_resources_from_execution(
    block_numbers: &[u64],
    hot_threshold: f64,  // e.g., 0.2 for top 20%
) -> Result<Vec<BlockHotResourcesFromExecution>> {
    let chain = PevmEthereum::mainnet();
    let mut all_blocks_hot = Vec::new();
    
    for block_num in block_numbers {
        println!("\n=== Analyzing Block {} ===", block_num);
        
        // Load block
        let filepath = format!("/home/ubuntu/eth-block-downloader/test_data/blocks/block_{}.json", block_num);
        let (block_data, storage, txenvs) = load_block_for_execution(&filepath, false)?;
        let spec_id = get_spec_id(*block_num);
        let block_env = create_block_env(&block_data);

        let total_transactions = txenvs.len();
        println!("Total transactions: {}", total_transactions);
        
        // Execute and get access sets
        let (_results, access_sets) = execute_revm_sequential_with_access_sets(
            &chain,
            &storage,
            spec_id,
            block_env,
            txenvs,
        )?;
        
        // Count access frequency for each resource
        let mut resource_count: HashMap<u64, usize> = HashMap::new();
        
        for access_set in access_sets.iter() {
            for &key in access_set.read_set.iter().chain(access_set.write_set.iter()) {
                *resource_count.entry(key).or_insert(0) += 1;
            }
        }
        
        println!("Unique resources accessed: {}", resource_count.len());
        
        // Sort by access count
        let mut sorted_resources: Vec<_> = resource_count.iter().collect();
        sorted_resources.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        
        // Get hot resources (top X%)
        let hot_count = (sorted_resources.len() as f64 * hot_threshold).ceil() as usize;
        let hot_resources: Vec<_> = sorted_resources.iter()
            .take(hot_count)
            .map(|(key, count)| (**key, **count))
            .collect();
        
        println!("Hot resources (top {:.0}%): {}", hot_threshold * 100.0, hot_resources.len());
        
        let block_hot = BlockHotResourcesFromExecution {
            block_num: *block_num,
            total_transactions,
            resource_access_count: resource_count,
            hot_resources,
        };
        
        block_hot.print_hot_details();
        
        all_blocks_hot.push(block_hot);
    }
    
    Ok(all_blocks_hot)
}

/// Analyze hot resource overlap across blocks
pub fn analyze_hot_resource_overlap_from_execution(
    block_numbers: &[u64],
    hot_threshold: f64,
) -> Result<()> {
    let blocks_hot = analyze_hot_resources_from_execution(block_numbers, hot_threshold)?;
    
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║          Hot Resource Overlap Analysis                        ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    // Build overlap map: resource_key -> list of (block_num, access_count)
    let mut resource_blocks: HashMap<u64, Vec<(u64, usize)>> = HashMap::new();
    
    for block_hot in &blocks_hot {
        for (key, count) in &block_hot.hot_resources {
            resource_blocks.entry(*key)
                .or_insert_with(Vec::new)
                .push((block_hot.block_num, *count));
        }
    }
    
    // Find resources appearing in multiple blocks
    let mut multi_block_resources: Vec<_> = resource_blocks.iter()
        .filter(|(_, blocks)| blocks.len() > 1)
        .collect();
    
    multi_block_resources.sort_by_key(|(_, blocks)| std::cmp::Reverse(blocks.len()));
    
    println!("Resources appearing in multiple blocks: {}", multi_block_resources.len());
    
    if !multi_block_resources.is_empty() {
        println!("\nTop Common Hot Resources:");
        println!("  {:<20} {:>10} {:>12}", "Resource Key", "# Blocks", "Avg Ratio");
        println!("  {}", "-".repeat(45));
        
        for (key, blocks) in multi_block_resources.iter().take(20) {
            // Calculate average access ratio
            let mut total_ratio = 0.0;
            for (block_num, count) in *blocks {
                if let Some(block_hot) = blocks_hot.iter().find(|b| b.block_num == *block_num) {
                    let ratio = *count as f64 / block_hot.total_transactions as f64;
                    total_ratio += ratio;
                }
            }
            let avg_ratio = total_ratio / blocks.len() as f64;
            
            println!("  {:<20} {:>10} {:>11.2}%", 
                     format!("0x{:016x}", key),
                     blocks.len(),
                     avg_ratio * 100.0);
        }
        
        // Detailed per-block breakdown for top common resources
        println!("\n\nDetailed Per-Block Access for Top Common Resources:");
        for (key, blocks) in multi_block_resources.iter().take(5) {
            println!("\n  Resource: 0x{:016x}", key);
            println!("    {:<12} {:>10} {:>12}", "Block", "Accesses", "Ratio");
            println!("    {}", "-".repeat(37));
            
            for (block_num, count) in *blocks {
                if let Some(block_hot) = blocks_hot.iter().find(|b| b.block_num == *block_num) {
                    let ratio = *count as f64 / block_hot.total_transactions as f64;
                    println!("    {:<12} {:>10} {:>11.2}%", 
                             block_num, count, ratio * 100.0);
                }
            }
        }
    }
    
    // Calculate overlap statistics
    let total_hot_instances: usize = blocks_hot.iter()
        .map(|b| b.hot_resources.len())
        .sum();
    let unique_hot_resources = resource_blocks.len();
    let overlap_ratio = if total_hot_instances > 0 {
        1.0 - (unique_hot_resources as f64 / total_hot_instances as f64)
    } else {
        0.0
    };
    
    println!("\n=== Overlap Metrics ===");
    println!("  Total hot resource instances: {}", total_hot_instances);
    println!("  Unique hot resources: {}", unique_hot_resources);
    println!("  Overlap ratio: {:.2}%", overlap_ratio * 100.0);
    
    Ok(())
}

#[test]
fn test_hot_resources_from_execution() -> Result<()> {
    let block_nums:Vec<u64> = (10646423..10646472).collect();
    let block_nums:Vec<u64> = (9646423..9646472).collect();
    
    // Analyze hot resources based on actual execution
    analyze_hot_resource_overlap_from_execution(&block_nums, 0.05)?;
    
    Ok(())
}


