/// TxSimulator-based workload generator.
///
/// Reads real Ethereum block data (read/write sets + gas) from the rw_gas
/// JSON files produced by eth-block-downloader and generates PEVM-compatible
/// `TxEnv` batches.  Each TxEnv calls `TxSimulator.execute(reads, writes,
/// gasUsed)`, which replays the original transaction's storage accesses
/// against a shared mapping, reproducing real conflict patterns.
///
/// InMemoryStorage contains:
/// - 1 TxSimulator contract account
/// - 1000 ordinary EOA accounts, each with `u128::MAX` balance
///
/// Callers are assigned uniquely across all TxEnvs from the EOA pool.
/// Returns an error if the total number of transactions across all blocks
/// exceeds the pool size.

use std::{fs, path::Path};

use pevm::{BuildSuffixHasher, Bytecodes, ChainState, EvmAccount};
use revm::primitives::{Address, TransactTo, TxEnv, U256};
use serde::Deserialize;

/// contract module
pub mod contract;
use contract::TxSimulator;

/// v2 contract module (target-based, not gasUsed-based)
pub mod contract_v2;
use contract_v2::TxSimulatorV2;

/// Number of ordinary EOA accounts pre-funded in the storage.
pub const NUM_ACCOUNTS: usize = 1000;

/// Gas multiplier applied to `gasUsed` to set each transaction's gas limit.
/// `gasUsed * 300` is sufficient because repeated hot SLOADs cost only 100
/// gas each (see TxSimulator.sol), so the simulation never runs out of gas.
pub const GAS_MULTIPLIER: u64 = 300;

/// Raw transaction data deserialized from a rw_gas JSON file.
#[derive(Deserialize)]
struct TxRwData {
    #[serde(rename = "gasUsed")]
    gas_used: u64,
    reads: Vec<revm::primitives::B256>,
    writes: Vec<revm::primitives::B256>,
}

/// Generate `NUM_ACCOUNTS` random EOA addresses.
fn generate_accounts(n: usize) -> Vec<Address> {
    (0..n).map(|_| Address::new(rand::random())).collect()
}

/// Build the initial chain state and extract bytecodes with a given account pool size.
pub fn build_storage_n(n: usize) -> (ChainState, Bytecodes, Address, Vec<Address>) {
    let simulator_address = Address::new(rand::random());
    let simulator_account = TxSimulator::build();

    let mut state: ChainState =
        [(simulator_address, simulator_account)].into_iter().collect();
    let accounts = generate_accounts(n);

    for &addr in &accounts {
        state.insert(
            addr,
            EvmAccount {
                balance: U256::from(u128::MAX),
                ..EvmAccount::default()
            },
        );
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    (state, bytecodes, simulator_address, accounts)
}

/// Build the initial chain state with `NUM_ACCOUNTS` EOA accounts.
///
/// Returns:
/// - `state` – `ChainState` containing the TxSimulator contract and all EOA
///   accounts (code has been moved into `bytecodes`)
/// - `bytecodes` – code map required by `InMemoryStorage`
/// - `simulator_address` – address of the deployed TxSimulator
/// - `accounts` – ordered list of the EOA addresses (callers are drawn
///   from this list in order)
pub fn build_storage() -> (ChainState, Bytecodes, Address, Vec<Address>) {
    build_storage_n(NUM_ACCOUNTS)
}

/// Load all `rw_gas_*.json` files from `rw_gas_dir`, sorted lexicographically,
/// and generate one `Vec<TxEnv>` per file (block).
///
/// Each TxEnv:
/// - calls `TxSimulator.execute(reads, writes, gasUsed)`
/// - uses a unique caller drawn sequentially from the 1000-account pool
/// - has `gas_limit = gasUsed * GAS_MULTIPLIER`
/// - has `nonce = 0` (each caller appears only once)
///
/// Returns `Err` if the total number of transactions across all blocks
/// exceeds `NUM_ACCOUNTS`.
pub fn load_rw_gas_blocks(
    rw_gas_dir: &str,
) -> Result<
    (
        ChainState,
        Bytecodes,
        Vec<Vec<TxEnv>>,
    ),
    Box<dyn std::error::Error>,
> {
    let (state, bytecodes, simulator_address, accounts) = build_storage();

    let dir = Path::new(rw_gas_dir);
    let mut json_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|ext| ext == "json").unwrap_or(false)
        })
        .collect();
    json_files.sort();

    if json_files.is_empty() {
        return Err(format!("No JSON files found in {rw_gas_dir}").into());
    }

    let mut all_blocks_txs: Vec<Vec<TxEnv>> = Vec::new();
    let mut account_idx: usize = 0;

    for file_path in &json_files {
        let content = fs::read_to_string(file_path)?;
        let txs_data: Vec<TxRwData> = serde_json::from_str(&content)?;

        let needed = account_idx + txs_data.len();
        if needed > accounts.len() {
            return Err(format!(
                "Not enough accounts: need {needed} total but pool has only {}. \
                 Increase NUM_ACCOUNTS or reduce the number of input transactions.",
                accounts.len()
            )
            .into());
        }

        let block_txs: Vec<TxEnv> = txs_data
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                let caller = accounts[account_idx + i];
                let gas_limit = tx.gas_used.saturating_mul(GAS_MULTIPLIER);
                let calldata =
                    TxSimulator::encode_execute(&tx.reads, &tx.writes, tx.gas_used);

                TxEnv {
                    caller,
                    gas_limit,
                    gas_price: U256::from(1),
                    transact_to: TransactTo::Call(simulator_address),
                    data: calldata,
                    nonce: Some(0),
                    ..TxEnv::default()
                }
            })
            .collect();

        account_idx += txs_data.len();
        all_blocks_txs.push(block_txs);
    }

    Ok((state, bytecodes, all_blocks_txs))
}

/// Load the first `max_blocks` `rw_gas_*.json` files from `rw_gas_dir` and
/// generate one `Vec<TxEnv>` per file.
///
/// The EOA account pool is sized automatically to fit the exact number of
/// transactions across the selected blocks, so this never errors due to pool
/// exhaustion.  Each TxEnv still gets a unique caller with nonce 0.
pub fn load_n_rw_gas_blocks(
    rw_gas_dir: &str,
    max_blocks: usize,
) -> Result<
    (
        ChainState,
        Bytecodes,
        Address,
        Vec<Vec<TxEnv>>,
    ),
    Box<dyn std::error::Error>,
> {
    let dir = Path::new(rw_gas_dir);
    let mut json_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    json_files.sort();
    json_files.truncate(max_blocks);

    if json_files.is_empty() {
        return Err(format!("No JSON files found in {rw_gas_dir}").into());
    }

    // Parse all files first so we know the total tx count before building storage.
    let mut all_tx_data: Vec<Vec<TxRwData>> = Vec::with_capacity(json_files.len());
    for file_path in &json_files {
        let content = fs::read_to_string(file_path)?;
        all_tx_data.push(serde_json::from_str(&content)?);
    }
    let total_txs: usize = all_tx_data.iter().map(|b| b.len()).sum();

    let (state, bytecodes, simulator_address, accounts) = build_storage_n(total_txs);

    let mut all_blocks_txs: Vec<Vec<TxEnv>> = Vec::with_capacity(json_files.len());
    let mut account_idx: usize = 0;

    for txs_data in &all_tx_data {
        let block_txs: Vec<TxEnv> = txs_data
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                let caller = accounts[account_idx + i];
                let gas_limit = tx.gas_used.saturating_mul(GAS_MULTIPLIER);
                let calldata =
                    TxSimulator::encode_execute(&tx.reads, &tx.writes, tx.gas_used);
                TxEnv {
                    caller,
                    gas_limit,
                    gas_price: U256::from(1),
                    transact_to: TransactTo::Call(simulator_address),
                    data: calldata,
                    nonce: Some(0),
                    ..TxEnv::default()
                }
            })
            .collect();
        account_idx += txs_data.len();
        all_blocks_txs.push(block_txs);
    }

    Ok((state, bytecodes, simulator_address, all_blocks_txs))
}

// ── TxSimulatorV2: target-based loader ───────────────────────────────────────

/// Pure per-SLOAD cost (ns), excluding fixed per-tx EVM overhead.
/// Derived from two-point calibration:
///   t_tx(n) = T_OVERHEAD_NS + n * T_SLOAD_NS
///   1-loop tx  ≈ 11 500 ns  (test_calibrate_sload_ns)
///   500-loop tx ≈ 500 000 ns (test_per_tx_execution_time median)
///   → T_SLOAD_NS ≈ (500 000 - 11 500) / 499 ≈ 979 ns
pub const T_SLOAD_NS: u64 = 1000;

/// Fixed per-tx EVM overhead (ns): ABI decoding, account loading, etc.
pub const T_OVERHEAD_NS: u64 = 10_500;

/// Raw transaction data deserialized from an rw_time JSON file.
#[derive(serde::Deserialize)]
struct TxRwTimeData {
    reads: Vec<revm::primitives::B256>,
    writes: Vec<revm::primitives::B256>,
    #[serde(rename = "executionTime")]
    execution_time_ns: u64,
}

/// Load the first `max_blocks` `rw_time_*.json` files from `rw_time_dir` and
/// generate one `Vec<TxEnv>` per file using `TxSimulatorV2`.
///
/// Each TxEnv calls `TxSimulatorV2.execute(reads, writes, target)` where
/// `target = execution_time_ns / T_SLOAD_NS`, so the synthetic workload's
/// execution-time distribution mirrors the real one.
///
/// The EOA pool is auto-sized to the total number of transactions.
pub fn load_n_rw_time_blocks(
    rw_time_dir: &str,
    max_blocks: usize,
) -> Result<
    (
        ChainState,
        Bytecodes,
        Address,
        Vec<Vec<TxEnv>>,
    ),
    Box<dyn std::error::Error>,
> {
    let dir = Path::new(rw_time_dir);
    let mut json_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    json_files.sort();
    json_files.truncate(max_blocks);

    if json_files.is_empty() {
        return Err(format!("No JSON files found in {rw_time_dir}").into());
    }

    let mut all_tx_data: Vec<Vec<TxRwTimeData>> = Vec::with_capacity(json_files.len());
    for file_path in &json_files {
        let content = fs::read_to_string(file_path)?;
        all_tx_data.push(serde_json::from_str(&content)?);
    }
    let total_txs: usize = all_tx_data.iter().map(|b| b.len()).sum();

    // Build storage with TxSimulatorV2 instead of TxSimulator.
    let simulator_address = Address::new(rand::random());
    let simulator_account = TxSimulatorV2::build();

    let mut state: ChainState = [(simulator_address, simulator_account)].into_iter().collect();
    let accounts: Vec<Address> = (0..total_txs).map(|_| Address::new(rand::random())).collect();
    for &addr in &accounts {
        state.insert(
            addr,
            EvmAccount {
                balance: U256::from(u128::MAX),
                ..EvmAccount::default()
            },
        );
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    let mut all_blocks_txs: Vec<Vec<TxEnv>> = Vec::with_capacity(json_files.len());
    let mut account_idx: usize = 0;

    for txs_data in &all_tx_data {
        let block_txs: Vec<TxEnv> = txs_data
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                let target = (tx.execution_time_ns.saturating_sub(T_OVERHEAD_NS) / T_SLOAD_NS).max(1);
                // Gas budget: base tx cost + cold SLOAD/SSTORE on first pass + hot remainder.
                // Cold SLOAD = 2100 gas, cold SSTORE (0→1) = 22100 gas, hot = 100 gas each.
                let cold_gas = tx.reads.len() as u64 * 2100 + tx.writes.len() as u64 * 22100;
                let hot_gas = target.saturating_mul(100).saturating_mul(GAS_MULTIPLIER);
                let gas_limit = 21_000u64.saturating_add(cold_gas).saturating_add(hot_gas);
                let calldata =
                    TxSimulatorV2::encode_execute(&tx.reads, &tx.writes, target);
                TxEnv {
                    caller: accounts[account_idx + i],
                    gas_limit,
                    gas_price: U256::from(1),
                    transact_to: revm::primitives::TransactTo::Call(simulator_address),
                    data: calldata,
                    nonce: Some(0),
                    ..TxEnv::default()
                }
            })
            .collect();
        account_idx += txs_data.len();
        all_blocks_txs.push(block_txs);
    }

    Ok((state, bytecodes, simulator_address, all_blocks_txs))
}
