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

/// Build the initial chain state and extract bytecodes.
///
/// Returns:
/// - `state` – `ChainState` containing the TxSimulator contract and all EOA
///   accounts (code has been moved into `bytecodes`)
/// - `bytecodes` – code map required by `InMemoryStorage`
/// - `simulator_address` – address of the deployed TxSimulator
/// - `accounts` – ordered list of the 1000 EOA addresses (callers are drawn
///   from this list in order)
pub fn build_storage() -> (
    ChainState,
    Bytecodes,
    Address,
    Vec<Address>,
) {
    let simulator_address = Address::new(rand::random());
    let simulator_account = TxSimulator::build();

    let mut state: ChainState =
        [(simulator_address, simulator_account)].into_iter().collect();
    let accounts = generate_accounts(NUM_ACCOUNTS);

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
