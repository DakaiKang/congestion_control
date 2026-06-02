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

use std::{collections::HashMap, fs, path::Path};

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

/// Marginal wall-clock cost of one storage access (SLOAD or SSTORE) inside
/// TxSimulatorV2, measured via execute_revm_sequential_timed which covers
/// only evm.transact() + evm.db_mut().commit().
///
/// Calibrated by least-squares fit of p50 timings at targets [1,10,50,100,500,1000]
/// (test_calibrate_sload_ns):
///   t(target) = T_OVERHEAD_NS + target * T_SLOAD_NS
///   → T_SLOAD_NS ≈ 482 ns
pub const T_SLOAD_NS: u64 = 482;

/// Fixed per-tx overhead within transact() + commit() (ns):
/// bytecode dispatch, ABI decode, account loading, journal setup, etc.
/// Derived from the intercept of the same linear fit: ≈ 3688 ns.
pub const T_OVERHEAD_NS: u64 = 3_688;

/// Raw transaction data deserialized from an rw_time JSON file.
///
/// Both `executionTime` and `gasUsed` are optional — the loader prefers
/// `executionTime / T_SLOAD_NS` (V2 timed mode), falling back to
/// `gasUsed / 100` (V1 gas mode) when only the gas field is present.
#[derive(serde::Deserialize)]
struct TxRwTimeData {
    reads: Vec<revm::primitives::B256>,
    writes: Vec<revm::primitives::B256>,
    #[serde(rename = "executionTime", default)]
    execution_time_ns: Option<u64>,
    #[serde(rename = "gasUsed", default)]
    gas_used: Option<u64>,
}

impl TxRwTimeData {
    /// TxSimulatorV2 loop count, clamped to `[2, 50000]`.
    fn target(&self) -> u64 {
        let raw = self
            .execution_time_ns
            .map(|t| t / T_SLOAD_NS)
            .or_else(|| self.gas_used.map(|g| g / 100))
            .expect("rw_time entry missing both `executionTime` and `gasUsed`");
        raw.max(2).min(50000)
    }
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
                // target = (executionTime - T_OVERHEAD_NS) / T_SLOAD_NS, capped at 5000
                // (p99.5 of the distribution) to bound pre-computation time.
                // Falls back to gasUsed/100 when executionTime is absent.
                let target = tx.target();
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

// ── Caller-aware loader: maps real ETH callers to simulated addresses ─────────

/// rw_time JSON entry (with optional `from` field added by `test_generate_rw_time`).
///
/// When `executionTime` is absent the loader falls back to `gasUsed / 100`.
/// When `from` is absent the loader will look up the real caller from the
/// corresponding `block_<N>.json` (matched by `txHash`) if a `blocks_dir`
/// was provided; otherwise it errors out (since dropping caller-identity
/// would artificially destroy same-sender dependencies and inflate parallelism).
#[derive(Deserialize)]
struct TxRwTimeDataFull {
    #[serde(default)]
    from: Option<String>,
    #[serde(rename = "txHash", default)]
    tx_hash: Option<String>,
    reads: Vec<revm::primitives::B256>,
    writes: Vec<revm::primitives::B256>,
    #[serde(rename = "executionTime", default)]
    execution_time_ns: Option<u64>,
    #[serde(rename = "gasUsed", default)]
    gas_used: Option<u64>,
}

impl TxRwTimeDataFull {
    fn target(&self) -> u64 {
        let raw = self
            .execution_time_ns
            .map(|t| t / T_SLOAD_NS)
            .or_else(|| self.gas_used.map(|g| g / 1000))
            .expect("rw_time entry missing both `executionTime` and `gasUsed`");
        raw.max(2).min(50000)
    }
}

/// Load the first `max_blocks` rw_time files and generate caller-aware TxEnvs.
///
/// `blocks_dir` (optional) points at a directory of `block_<N>.json` files —
/// used to backfill the `from` (real caller) of each tx when the rw_time
/// entry doesn't carry one (e.g. rw_gas-format files). When `blocks_dir`
/// is `None`, every entry must already contain `from`.
///
/// Every unique real caller is mapped to a freshly-generated simulated
/// address, so txs from the same real caller share the same simulated
/// caller. Sequential nonces are assigned per simulated caller across all
/// blocks in block order, preserving same-sender dependencies.
///
/// For greedy integration the caller must re-assign nonces with a fresh
/// `NonceTracker` after `integrate_pevm_graphs` reorders transactions across
/// block boundaries.
pub fn load_n_rw_time_blocks_with_callers(
    rw_time_dir: &str,
    max_blocks: usize,
) -> Result<
    (ChainState, Bytecodes, Address, Vec<Vec<TxEnv>>),
    Box<dyn std::error::Error>,
> {
    let blocks_dir = std::env::var("BLOCKS_DIR").ok();
    load_n_rw_time_blocks_with_callers_inner(rw_time_dir, blocks_dir.as_deref(), max_blocks)
}

pub fn load_n_rw_time_blocks_with_callers_inner(
    rw_time_dir: &str,
    blocks_dir: Option<&str>,
    max_blocks: usize,
) -> Result<
    (ChainState, Bytecodes, Address, Vec<Vec<TxEnv>>),
    Box<dyn std::error::Error>,
> {
    // ── 1. Collect and sort rw_time files ────────────────────────────────────
    let dir = Path::new(rw_time_dir);
    let mut json_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "json").unwrap_or(false))
        .collect();
    json_files.sort();
    // Optional batch offset: skip the first START_INDEX files (per env var).
    let start_idx: usize = std::env::var("START_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if start_idx < json_files.len() {
        json_files.drain(..start_idx);
    } else {
        json_files.clear();
    }
    json_files.truncate(max_blocks);

    if json_files.is_empty() {
        return Err(format!("No JSON files found in {rw_time_dir} (start_idx={start_idx})").into());
    }

    // ── 2. Parse all rw_time files ───────────────────────────────────────────
    let mut all_tx_data: Vec<Vec<TxRwTimeDataFull>> =
        Vec::with_capacity(json_files.len());
    for file_path in &json_files {
        all_tx_data.push(serde_json::from_str(&fs::read_to_string(file_path)?)?);
    }

    // ── 2b. Backfill missing `from` by looking up txHash in block_<N>.json ───
    if let Some(blocks_dir) = blocks_dir {
        for (file_path, block) in json_files.iter().zip(all_tx_data.iter_mut()) {
            let needs_lookup = block.iter().any(|tx| tx.from.is_none());
            if !needs_lookup {
                continue;
            }
            // Derive block number from rw_time/rw_gas filename: strip leading
            // "rw_time_" or "rw_gas_" prefix.
            let stem = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("bad rw_time filename")?;
            let block_num: u64 = stem
                .trim_start_matches("rw_time_")
                .trim_start_matches("rw_gas_")
                .parse()
                .map_err(|_| format!("cannot parse block number from {stem}"))?;
            let block_path = format!("{}/block_{}.json", blocks_dir, block_num);
            let block_json: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&block_path).map_err(|e| {
                    format!("backfill: cannot read {block_path}: {e}")
                })?)?;
            // Build hash → from map from the block's tx list.
            let mut hash_to_from: HashMap<String, String> = HashMap::new();
            if let Some(txs) = block_json.get("transactions").and_then(|v| v.as_array()) {
                for (i, t) in txs.iter().enumerate() {
                    // Some snapshots store the hash on a sibling "hashes" array
                    // or in the tx envelope — fall back to nothing if absent.
                    let hash = t
                        .get("hash")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            block_json
                                .get("hashes")
                                .and_then(|h| h.as_array())
                                .and_then(|h| h.get(i))
                                .and_then(|h| h.as_str())
                        })
                        .map(str::to_owned);
                    let from = t.get("from").and_then(|v| v.as_str()).map(str::to_owned);
                    if let (Some(h), Some(f)) = (hash, from) {
                        hash_to_from.insert(h.to_lowercase(), f);
                    }
                }
            }
            // Inject `from` into rw_time entries by txHash match.
            for tx in block.iter_mut() {
                if tx.from.is_some() {
                    continue;
                }
                let hash_lc = tx
                    .tx_hash
                    .as_ref()
                    .map(|h| h.to_lowercase())
                    .ok_or("rw_time entry without `from` and without `txHash`")?;
                let from = hash_to_from
                    .get(&hash_lc)
                    .ok_or_else(|| format!("backfill: no from for txHash {hash_lc} in block {block_num}"))?;
                tx.from = Some(from.clone());
            }
        }
    }

    // Final sanity: every tx must have `from` now.
    for (file_path, block) in json_files.iter().zip(all_tx_data.iter()) {
        if block.iter().any(|tx| tx.from.is_none()) {
            return Err(format!(
                "rw_time entries missing `from` in {} — pass BLOCKS_DIR to backfill from block JSON",
                file_path.display()
            )
            .into());
        }
    }

    // ── 3. Assign a unique simulated address per unique real caller ──────────
    // All txs from the same real caller share one simulated address.
    let mut real_to_simulated: HashMap<Address, Address> = HashMap::new();
    for block in &all_tx_data {
        for tx in block {
            let from_str = tx.from.as_ref().unwrap();
            let real_from: Address = from_str
                .parse()
                .map_err(|_| format!("invalid address: {}", from_str))?;
            real_to_simulated
                .entry(real_from)
                .or_insert_with(|| Address::new(rand::random()));
        }
    }

    // ── 4. Build storage (simulator contract + one account per unique caller) ─
    let simulator_address = Address::new(rand::random());
    let simulator_account = TxSimulatorV2::build();

    let mut state: ChainState =
        [(simulator_address, simulator_account)].into_iter().collect();
    for &sim_addr in real_to_simulated.values() {
        state.insert(
            sim_addr,
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

    // ── 5. Build TxEnvs with sequential nonces per simulated caller ───────────
    // nonce_state tracks the next nonce for each simulated caller across all blocks.
    let mut nonce_state: HashMap<Address, u64> = HashMap::new();
    let mut all_blocks_txs: Vec<Vec<TxEnv>> = Vec::with_capacity(all_tx_data.len());

    for block in &all_tx_data {
        let mut block_txs: Vec<TxEnv> = Vec::with_capacity(block.len());

        for tx in block {
            let real_from: Address = tx.from.as_ref().unwrap().parse().unwrap();
            let sim_caller = *real_to_simulated.get(&real_from).unwrap();

            let tx_nonce = *nonce_state.entry(sim_caller).or_insert(0);
            *nonce_state.get_mut(&sim_caller).unwrap() += 1;

            let target = tx.target();
            let cold_gas =
                tx.reads.len() as u64 * 2100 + tx.writes.len() as u64 * 22100;
            let hot_gas =
                target.saturating_mul(100).saturating_mul(GAS_MULTIPLIER);
            let gas_limit = 21_000u64
                .saturating_add(cold_gas)
                .saturating_add(hot_gas);
            let calldata =
                TxSimulatorV2::encode_execute(&tx.reads, &tx.writes, target);

            block_txs.push(TxEnv {
                caller: sim_caller,
                gas_limit,
                gas_price: U256::from(1),
                transact_to: TransactTo::Call(simulator_address),
                data: calldata,
                nonce: Some(tx_nonce),
                ..TxEnv::default()
            });
        }

        all_blocks_txs.push(block_txs);
    }

    Ok((state, bytecodes, simulator_address, all_blocks_txs))
}
