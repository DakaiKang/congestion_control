use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::Instant,
};

use std::collections::{HashSet};

use alloy_primitives::{TxNonce, U256};
use alloy_rpc_types_eth::{Block, BlockTransactions};
use hashbrown::HashMap;
use revm::{
    db::CacheDB,
    primitives::{AccountInfo, Bytecode, BlockEnv, InvalidTransaction, SpecId, TxEnv, Address, ResultAndState, ExecutionResult, B256},
    DatabaseCommit,
    Database,
    Evm,
};

use crate::{
    chain::PevmChain,
    compat::get_block_env,
    hash_deterministic,
    mv_memory::MvMemory,
    scheduler::Scheduler,
    storage::StorageWrapper,
    vm::{
        build_evm, ExecutionError, PevmTxExecutionResult, Vm, VmExecutionError, VmExecutionResult,
    },
    EvmAccount, MemoryEntry, MemoryLocation, MemoryValue, Storage, Task, TxIdx, TxVersion,
};

/// Errors when executing a block with pevm.
// TODO: implement traits explicitly due to trait bounds on `C` instead of types of `PevmChain`
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PevmError<C: PevmChain> {
    /// Cannot derive the chain spec from the block header.
    #[error("Cannot derive the chain spec from the block header")]
    BlockSpecError(#[source] C::BlockSpecError),
    /// Transactions lack information for execution.
    #[error("Transactions lack information for execution")]
    MissingTransactionData,
    /// Invalid input transaction.
    #[error("Invalid input transaction")]
    InvalidTransaction(#[source] C::TransactionParsingError),
    /// Nonce too low or too high
    #[error("Nonce mismatch for tx #{tx_idx}. Expected {executed_nonce}, got {tx_nonce}")]
    NonceMismatch {
        /// Transaction index
        tx_idx: TxIdx,
        /// Nonce from tx (from the very input)
        tx_nonce: TxNonce,
        /// Nonce from state and execution
        executed_nonce: TxNonce,
    },
    /// Storage error.
    // TODO: More concrete types than just an arbitrary string.
    #[error("Storage error: {0}")]
    StorageError(String),
    /// EVM execution error.
    #[error("Execution error")]
    ExecutionError(
        #[source]
        #[from]
        ExecutionError,
    ),
    /// Impractical errors that should be unreachable.
    /// The library has bugs if this is yielded.
    #[error("PEVM encountered a bug. Please open an issue in https://github.com/risechain/pevm/issues/new")]
    UnreachableError,
}

/// Execution result of a block
pub type PevmResult<C> = Result<Vec<PevmTxExecutionResult>, PevmError<C>>;

#[derive(Debug)]
pub enum AbortReason {
    FallbackToSequential,
    ExecutionError(ExecutionError),
}

// TODO: Better implementation
#[derive(Debug)]
pub struct AsyncDropper<T> {
    sender: mpsc::Sender<T>,
    _handle: thread::JoinHandle<()>,
}

impl<T: Send + 'static> Default for AsyncDropper<T> {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            _handle: std::thread::spawn(move || receiver.into_iter().for_each(drop)),
        }
    }
}

impl<T> AsyncDropper<T> {
    pub fn drop(&self, t: T) {
        // TODO: Better error handling
        self.sender.send(t).unwrap();
    }
}

// TODO: Port more recyclable resources into here.
#[derive(Debug, Default)]
/// The main pevm struct that executes blocks.
pub struct Pevm {
    execution_results: Vec<Mutex<Option<PevmTxExecutionResult>>>,
    abort_reason: OnceLock<AbortReason>,
    dropper: AsyncDropper<(MvMemory, Scheduler, Vec<TxEnv>)>,
    /// Incarnation-0 access set keys from last parallel execution, for divergence analysis
    pub last_incarnation0_keys: Vec<Option<(std::collections::HashSet<u64>, std::collections::HashSet<u64>)>>,
}

impl Pevm {
    /// Execute an Alloy block, which is becoming the "standard" format in Rust.
    /// TODO: Better error handling.
    pub fn execute<S, C>(
        &mut self,
        chain: &C,
        storage: &S,
        // We assume the block is still needed afterwards like in most Reth cases
        // so take in a reference and only copy values when needed. We may want
        // to use a [`std::borrow::Cow`] to build [`BlockEnv`] and [`TxEnv`] without
        // (much) copying when ownership can be given. Another challenge with this is
        // the new Alloy [`Transaction`] interface that is mostly `&self`. We'd need
        // to do some dirty destruction to get the owned fields.
        block: &Block<C::Transaction>,
        concurrency_level: NonZeroUsize,
        force_sequential: bool,
    ) -> PevmResult<C>
    where
        C: PevmChain + Send + Sync,
        S: Storage + Send + Sync,
    {
        let spec_id = chain
            .get_block_spec(&block.header)
            .map_err(PevmError::BlockSpecError)?;
        let block_env = get_block_env(&block.header, spec_id);
        let tx_envs = match &block.transactions {
            BlockTransactions::Full(txs) => txs
                .iter()
                .map(|tx| chain.get_tx_env(tx))
                .collect::<Result<Vec<TxEnv>, _>>()
                .map_err(PevmError::InvalidTransaction)?,
            _ => return Err(PevmError::MissingTransactionData),
        };
        // TODO: Continue to fine tune this condition.
        if force_sequential
            || tx_envs.len() < concurrency_level.into()
            || block.header.gas_used < 4_000_000
        {
            execute_revm_sequential(chain, storage, spec_id, block_env, tx_envs)
        } else {
            self.execute_revm_parallel(
                chain,
                storage,
                spec_id,
                block_env,
                tx_envs,
                concurrency_level,
            )
        }
    }

    /// Execute an REVM block.
    // Ideally everyone would go through the [Alloy] interface. This one is currently
    // useful for testing, and for users that are heavily tied to Revm like Reth.
    pub fn execute_revm_parallel<S, C>(
        &mut self,
        chain: &C,
        storage: &S,
        spec_id: SpecId,
        block_env: BlockEnv,
        txs: Vec<TxEnv>,
        concurrency_level: NonZeroUsize,
    ) -> PevmResult<C>
    where
        C: PevmChain + Send + Sync,
        S: Storage + Send + Sync,
    {
        if txs.is_empty() {
            return Ok(Vec::new());
        }

        let block_size = txs.len();
        let scheduler = Scheduler::new(block_size);

        let mv_memory = chain.build_mv_memory(&block_env, &txs);
        let vm = Vm::new(storage, &mv_memory, chain, &block_env, &txs, spec_id);

        let additional = block_size.saturating_sub(self.execution_results.len());
        if additional > 0 {
            self.execution_results.reserve(additional);
            for _ in 0..additional {
                self.execution_results.push(Mutex::new(None));
            }
        }

        // TODO: Better thread handling
        thread::scope(|scope| {
            for _ in 0..concurrency_level.into() {
                scope.spawn(|| {
                    let mut task = scheduler.next_task();
                    while task.is_some() {
                        task = match task.unwrap() {
                            Task::Execution(tx_version) => {
                                self.try_execute(&vm, &scheduler, &mv_memory, tx_version)
                            }
                            Task::Validation(tx_version) => {
                                try_validate(&mv_memory, &scheduler, &tx_version)
                            }
                        };

                        // TODO: Have different functions or an enum for the caller to choose
                        // the handling behaviour when a transaction's EVM execution fails.
                        // Parallel block builders would like to exclude such transaction,
                        // verifiers may want to exit early to save CPU cycles, while testers
                        // may want to collect all execution results. We are exiting early as
                        // the default behaviour for now.
                        if self.abort_reason.get().is_some() {
                            break;
                        }

                        if task.is_none() {
                            task = scheduler.next_task();
                        }
                    }
                });
            }
        });

        if let Some(abort_reason) = self.abort_reason.take() {
            match abort_reason {
                AbortReason::FallbackToSequential => {
                    self.dropper.drop((mv_memory, scheduler, Vec::new()));
                    return execute_revm_sequential(chain, storage, spec_id, block_env, txs);
                }
                AbortReason::ExecutionError(err) => {
                    self.dropper.drop((mv_memory, scheduler, txs));
                    return Err(PevmError::ExecutionError(err));
                }
            }
        }

        let mut fully_evaluated_results = Vec::with_capacity(block_size);
        let mut cumulative_gas_used: u64 = 0;
        for i in 0..block_size {
            let mut execution_result = index_mutex!(self.execution_results, i).take().unwrap();
            cumulative_gas_used =
                cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
            execution_result.receipt.cumulative_gas_used = cumulative_gas_used;
            fully_evaluated_results.push(execution_result);
        }

        // We fully evaluate (the balance and nonce of) the beneficiary account
        // and raw transfer recipients that may have been atomically updated.
        for address in mv_memory.consume_lazy_addresses() {
            let location_hash = hash_deterministic(MemoryLocation::Basic(address));
            if let Some(write_history) = mv_memory.data.get(&location_hash) {
                let mut balance = U256::ZERO;
                let mut nonce = 0;
                // Read from storage if the first multi-version entry is not an absolute value.
                if !matches!(
                    write_history.first_key_value(),
                    Some((_, MemoryEntry::Data(_, MemoryValue::Basic(_))))
                ) {
                    if let Ok(Some(account)) = storage.basic(&address) {
                        balance = account.balance;
                        nonce = account.nonce;
                    }
                }
                // Accounts that take implicit writes like the beneficiary account can be contract!
                let code_hash = match storage.code_hash(&address) {
                    Ok(code_hash) => code_hash,
                    Err(err) => return Err(PevmError::StorageError(err.to_string())),
                };
                let code = if let Some(code_hash) = &code_hash {
                    match storage.code_by_hash(code_hash) {
                        Ok(code) => code,
                        Err(err) => return Err(PevmError::StorageError(err.to_string())),
                    }
                } else {
                    None
                };

                for (tx_idx, memory_entry) in write_history.iter() {
                    let tx = unsafe { txs.get_unchecked(*tx_idx) };
                    match memory_entry {
                        MemoryEntry::Data(_, MemoryValue::Basic(info)) => {
                            // We fall back to sequential execution when reading a self-destructed account,
                            // so an empty account here would be a bug
                            debug_assert!(!(info.balance.is_zero() && info.nonce == 0));
                            balance = info.balance;
                            nonce = info.nonce;
                        }
                        MemoryEntry::Data(_, MemoryValue::LazyRecipient(addition)) => {
                            balance = balance.saturating_add(*addition);
                        }
                        MemoryEntry::Data(_, MemoryValue::LazySender(subtraction)) => {
                            // We must re-do extra sender balance checks as we mock
                            // the max value in [Vm] during execution. Ideally we
                            // can turn off these redundant checks in revm.
                            // Ideally we would share these calculations with revm
                            // (using their utility functions).
                            let mut max_fee = U256::from(tx.gas_limit)
                                .saturating_mul(tx.gas_price)
                                .saturating_add(tx.value);
                            if let Some(blob_fee) = tx.max_fee_per_blob_gas {
                                max_fee = max_fee.saturating_add(
                                    U256::from(tx.get_total_blob_gas())
                                        .saturating_mul(U256::from(blob_fee)),
                                );
                            }
                            if balance < max_fee {
                                Err(ExecutionError::Transaction(
                                    InvalidTransaction::LackOfFundForMaxFee {
                                        balance: Box::new(balance),
                                        fee: Box::new(max_fee),
                                    },
                                ))?
                            }
                            balance = balance.saturating_sub(*subtraction);
                            nonce += 1;
                        }
                        // TODO: Better error handling
                        _ => unreachable!(),
                    }
                    // Assert that evaluated nonce is correct when address is caller.
                    if tx.caller == address {
                        if let Some(tx_nonce) = tx.nonce {
                            let executed_nonce = if nonce == 0 {
                                return Err(PevmError::UnreachableError);
                            } else {
                                nonce - 1
                            };
                            if tx_nonce != executed_nonce {
                                // TODO: Consider falling back to sequential instead
                                return Err(PevmError::NonceMismatch {
                                    tx_idx: *tx_idx,
                                    tx_nonce,
                                    executed_nonce,
                                });
                            }
                        }
                    }
                    // SAFETY: The multi-version data structure should not leak an index over block size.
                    let tx_result = unsafe { fully_evaluated_results.get_unchecked_mut(*tx_idx) };
                    let account = tx_result.state.entry(address).or_default();
                    // TODO: Deduplicate this logic with [PevmTxExecutionResult::from_revm]
                    if chain.is_eip_161_enabled(spec_id)
                        && code_hash.is_none()
                        && nonce == 0
                        && balance == U256::ZERO
                    {
                        *account = None;
                    } else if let Some(account) = account {
                        // Explicit write: only overwrite the account info in case there are storage changes
                        // Code cannot change midblock here as we're falling back to sequential execution
                        // on reading a self-destructed contract.
                        account.balance = balance;
                        account.nonce = nonce;
                    } else {
                        // Implicit write: e.g. gas payments to the beneficiary account,
                        // which doesn't have explicit writes in [tx_result.state]
                        *account = Some(EvmAccount {
                            balance,
                            nonce,
                            code_hash,
                            code: code.clone(),
                            storage: HashMap::default(),
                        });
                    }
                }
            }
        }

        let re_execs = scheduler.total_reexecutions();
        #[cfg(feature = "diagnostics")]
        {
            self.last_incarnation0_keys = mv_memory.get_incarnation0_keys();
            let blocking = scheduler.blocking_reexecs.load(std::sync::atomic::Ordering::Relaxed);
            let blk_est = mv_memory.blocking_estimate.load(std::sync::atomic::Ordering::Relaxed);
            let blk_nonce = mv_memory.blocking_nonce.load(std::sync::atomic::Ordering::Relaxed);
            let blk_retry = mv_memory.blocking_retry.load(std::sync::atomic::Ordering::Relaxed);
            let wnl = mv_memory.wrote_new_location.load(std::sync::atomic::Ordering::Relaxed);
            let total_aborts = mv_memory.total_aborts.load(std::sync::atomic::Ordering::Relaxed);
            let cascade = mv_memory.cascade_aborts.load(std::sync::atomic::Ordering::Relaxed);
            let read_chg = mv_memory.reexec_read_changed.load(std::sync::atomic::Ordering::Relaxed);
            let write_chg = mv_memory.reexec_write_changed.load(std::sync::atomic::Ordering::Relaxed);
            let either_chg = mv_memory.reexec_total.load(std::sync::atomic::Ordering::Relaxed);
            println!(
                "[Pevm] block_size={} re_exec={} | A)blocking={} [est={} nonce={}] B)validation={} C)retry={} | wnl={} cascade={}/{} ({:.0}%) | access_set_changed: read={}/{} write={}/{}",
                block_size, re_execs,
                blocking, blk_est, blk_nonce,
                total_aborts,
                blk_retry,
                wnl,
                cascade, total_aborts, if total_aborts > 0 { cascade as f64 / total_aborts as f64 * 100.0 } else { 0.0 },
                read_chg, either_chg,
                write_chg, either_chg,
            );
        }

        self.dropper.drop((mv_memory, scheduler, txs));

        Ok(fully_evaluated_results)
    }

    fn try_execute<S: Storage, C: PevmChain>(
        &self,
        vm: &Vm<'_, S, C>,
        scheduler: &Scheduler,
        mv_memory: &MvMemory,
        tx_version: TxVersion,
    ) -> Option<Task> {
        loop {
            return match vm.execute(&tx_version) {
                Err(VmExecutionError::Retry) => {
                    if self.abort_reason.get().is_none() {
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::FallbackToSequential) => {
                    scheduler.abort();
                    self.abort_reason
                        .get_or_init(|| AbortReason::FallbackToSequential);
                    None
                }
                Err(VmExecutionError::Blocking(blocking_tx_idx, cause)) => {
                    if scheduler.add_dependency(tx_version.tx_idx, blocking_tx_idx) {
                        // Successfully blocked: increment cause-specific counter now that
                        // we know this will result in an actual re-execution (incarnation++).
                        #[cfg(feature = "diagnostics")]
                        match cause {
                            crate::vm::BlockingCause::Estimate =>
                                mv_memory.blocking_estimate.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                            crate::vm::BlockingCause::Nonce =>
                                mv_memory.blocking_nonce.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                        };
                        None
                    } else if self.abort_reason.get().is_none() {
                        // Retry the execution immediately if the blocking transaction was
                        // re-executed by the time we can add it as a dependency.
                        #[cfg(feature = "diagnostics")]
                        mv_memory.blocking_retry.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        continue;
                    } else {
                        None
                    }
                }
                Err(VmExecutionError::ExecutionError(err)) => {
                    scheduler.abort();
                    self.abort_reason
                        .get_or_init(|| AbortReason::ExecutionError(err));
                    None
                }
                Ok(VmExecutionResult {
                    execution_result,
                    flags,
                }) => {
                    *index_mutex!(self.execution_results, tx_version.tx_idx) =
                        Some(execution_result);
                    scheduler.finish_execution(tx_version, flags)
                }
            };
        }
    }
}

fn try_validate(
    mv_memory: &MvMemory,
    scheduler: &Scheduler,
    tx_version: &TxVersion,
) -> Option<Task> {
    let read_set_valid = mv_memory.validate_read_locations(tx_version.tx_idx);
    let aborted = !read_set_valid && scheduler.try_validation_abort(tx_version);
    if aborted {
        mv_memory.convert_writes_to_estimates(tx_version.tx_idx);
    }
    scheduler.finish_validation(tx_version, aborted)
}

/// Execute REVM transactions sequentially.
// Useful for falling back for (small) blocks with many dependencies.
// TODO: Use this for a long chain of sequential transactions even in parallel mode.
pub fn execute_revm_sequential<S: Storage, C: PevmChain>(
    chain: &C,
    storage: &S,
    spec_id: SpecId,
    block_env: BlockEnv,
    txs: Vec<TxEnv>,
) -> PevmResult<C> {
    let mut db = CacheDB::new(StorageWrapper(storage));
    let mut evm = build_evm(&mut db, chain, spec_id, block_env, None, true);
    let mut results = Vec::with_capacity(txs.len());
    let mut cumulative_gas_used: u64 = 0;
    let mut x = 0;
    let mut count_failed = 0;
    for (tx_idx, tx) in txs.into_iter().enumerate() {
        *evm.tx_mut() = tx;
        
        // TODO: More concrete type for `EVMError<StorageWrapperError<S>>`
        let result_and_state = evm
            .transact()
            .map_err(|err| ExecutionError::Custom(err.to_string()))?;

        match &result_and_state.result {
            ExecutionResult::Success { .. } => {
                // Success - no print
                // println!("✅ Transaction {} executed successfully", tx_idx);
            }
            ExecutionResult::Revert { .. } => {
                count_failed += 1;
            }
            ExecutionResult::Halt { .. } => {
                count_failed += 1;
            }
        }

        evm.db_mut().commit(result_and_state.state.clone());

        let mut execution_result =
            PevmTxExecutionResult::from_revm(chain, spec_id, result_and_state);

        cumulative_gas_used =
            cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
        execution_result.receipt.cumulative_gas_used = cumulative_gas_used;

        results.push(execution_result);
    }
    println!("✅ Executed {} transactions, {} failed.", x + results.len(), count_failed);
    Ok(results)
}

/// Like [`execute_revm_sequential`] but also returns per-transaction timing.
///
/// For each transaction the timer starts immediately before `evm.transact()`
/// and stops immediately after `evm.db_mut().commit(...)`, capturing only the
/// core execute-and-commit cost and excluding EVM/CacheDB construction and
/// result conversion overhead.
///
/// Returns `(results, tx_times_ns)` where `tx_times_ns[i]` is the
/// nanoseconds spent on transaction `i`.
pub fn execute_revm_sequential_timed<S: Storage, C: PevmChain>(
    chain: &C,
    storage: &S,
    spec_id: SpecId,
    block_env: BlockEnv,
    txs: Vec<TxEnv>,
) -> Result<(Vec<PevmTxExecutionResult>, Vec<u64>), PevmError<C>> {
    let mut db = CacheDB::new(StorageWrapper(storage));
    let mut evm = build_evm(&mut db, chain, spec_id, block_env, None, true);
    let mut results = Vec::with_capacity(txs.len());
    let mut tx_times_ns = Vec::with_capacity(txs.len());
    let mut cumulative_gas_used: u64 = 0;
    for tx in txs.into_iter() {
        *evm.tx_mut() = tx;

        let t = Instant::now();
        let result_and_state = evm
            .transact()
            .map_err(|err| ExecutionError::Custom(err.to_string()))?;
        evm.db_mut().commit(result_and_state.state.clone());
        tx_times_ns.push(t.elapsed().as_nanos() as u64);

        let mut execution_result =
            PevmTxExecutionResult::from_revm(chain, spec_id, result_and_state);
        cumulative_gas_used =
            cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
        execution_result.receipt.cumulative_gas_used = cumulative_gas_used;
        results.push(execution_result);
    }
    Ok((results, tx_times_ns))
}

#[derive(Debug, Clone, Default)]
pub struct TxAccessSets {
    pub read_set: HashSet<u64>,
    pub write_set: HashSet<u64>,
}

/// A transparent DB wrapper that records every `basic()` and `storage()` call
/// made by the EVM. Placed between the EVM and CacheDB so that ALL reads —
/// both cold (cache-miss) and warm (cache-hit) — are captured.
///
/// Architecture: EVM → TrackingDB → CacheDB → StorageWrapper → S
///
/// Mirrors pevm's parallel path exclusions:
/// - `coinbase` is never recorded (pevm uses `with_reward_beneficiary=false` +
///   `LazyRecipient` delta writes, so coinbase never appears in read/write sets).
/// - For lazy txs (pure ETH transfers: recipient is an EOA), `caller` and
///   `recipient` are excluded from the read set (pevm returns a mock account and
///   writes only a `LazySender`/`LazyRecipient` delta, not a full account entry).
struct TrackingDB<DB> {
    pub inner: DB,
    pub reads: HashSet<u64>,
    /// Block-level: coinbase address, excluded from all sets.
    coinbase: Address,
    /// Tx-level: caller + recipient for lazy (pure-ETH-transfer) txs.
    /// Cleared and repopulated before each transaction.
    lazy_addresses: HashSet<Address>,
}

impl<DB: Database> TrackingDB<DB> {
    fn should_skip(&self, address: &Address) -> bool {
        *address == Address::ZERO
            || *address == self.coinbase
            || self.lazy_addresses.contains(address)
    }
}

impl<DB: Database> Database for TrackingDB<DB> {
    type Error = DB::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // MemoryLocation::Basic excluded: only storage slots are compared for divergence analysis
        self.inner.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.inner.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        if !self.should_skip(&address) {
            self.reads.insert(hash_deterministic(MemoryLocation::Storage(address, index)));
        }
        self.inner.storage(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.inner.block_hash(number)
    }
}

impl<DB: DatabaseCommit> DatabaseCommit for TrackingDB<DB> {
    fn commit(&mut self, changes: std::collections::HashMap<Address, revm::primitives::Account, alloy_primitives::map::foldhash::fast::RandomState>) {
        self.inner.commit(changes)
    }
}

pub fn execute_revm_sequential_with_access_sets<S: Storage, C: PevmChain>(
        chain: &C,
        storage: &S,
        spec_id: SpecId,
        block_env: BlockEnv,
        txs: Vec<TxEnv>,
    ) -> Result<(Vec<PevmTxExecutionResult>, Vec<TxAccessSets>), PevmError<C>> {
    let coinbase = block_env.coinbase;
    let cache_db = CacheDB::new(StorageWrapper(storage));
    let tracking_db = TrackingDB {
        inner: cache_db,
        reads: HashSet::new(),
        coinbase,
        lazy_addresses: HashSet::new(),
    };
    let mut evm = build_evm(tracking_db, chain, spec_id, block_env, None, true);
    let mut results = Vec::with_capacity(txs.len());
    let mut access_sets = Vec::with_capacity(txs.len());
    let mut cumulative_gas_used: u64 = 0;

    for tx in txs {
        // Determine if this is a lazy tx (pure ETH transfer: recipient is an EOA).
        // Mirrors pevm's `is_lazy` check in VmDb: if `to` has no code, pevm skips
        // recording caller/recipient in the read set and uses delta writes instead.
        let caller = tx.caller;
        let recipient = tx.transact_to.to().copied();
        let is_lazy = if let Some(to) = recipient {
            evm.db_mut()
                .inner
                .basic(to)
                .map(|a| a.map_or(true, |acc| acc.is_empty_code_hash()))
                .unwrap_or(false)
        } else {
            false
        };

        {
            let db = evm.db_mut();
            db.lazy_addresses.clear();
            if is_lazy {
                db.lazy_addresses.insert(caller);
                if let Some(to) = recipient {
                    db.lazy_addresses.insert(to);
                }
            }
        }

        *evm.tx_mut() = tx;
        let result_and_state = evm
            .transact()
            .map_err(|err| ExecutionError::Custom(err.to_string()))?;

        // Collect storage-only read set from TrackingDB (Basic excluded for divergence analysis).
        let db = evm.db_mut();
        let read_set = std::mem::take(&mut db.reads);

        // Derive write set from result state: touched accounts + changed storage slots,
        // excluding only coinbase (handled via LazyRecipient in pevm).
        // lazy_addresses (pure-ETH-transfer sender/recipient) are intentionally kept in
        // write_set so that downstream txs that read their balance get a proper RAW edge.
        let write_set = extract_write_set_from_result(&result_and_state, coinbase);

        db.commit(result_and_state.state.clone());

        let mut execution_result =
            PevmTxExecutionResult::from_revm(chain, spec_id, result_and_state);

        cumulative_gas_used =
            cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
        execution_result.receipt.cumulative_gas_used = cumulative_gas_used;

        results.push(execution_result);
        access_sets.push(TxAccessSets { read_set, write_set });
    }

    Ok((results, access_sets))
}

/// Extract write set from execution result:
/// - Basic(addr): account's ETH balance/nonce/code actually changed.
///   We detect this by checking whether the account has no storage changes:
///   - EOA sending/receiving ETH: always has storage.is_empty() → Basic added ✓
///   - Contract with only storage changes (DeFi swap reserves, balanceOf): storage non-empty
///     → Basic NOT added, avoiding spurious WW edges between txs sharing the same contract
///   - Contract that both receives ETH AND modifies storage: Basic missed, but it already
///     has WW conflict on storage slots with any tx touching the same contract, so the
///     dependency graph edge still exists.
///   - Newly created contract: status Created → Basic added ✓
/// - Storage(addr, slot): slot value changed
///
/// Excludes `coinbase` (pevm applies gas rewards via LazyRecipient, never as a full write).
fn extract_write_set_from_result(
    result_and_state: &ResultAndState,
    coinbase: Address,
) -> HashSet<u64> {
    let mut write_set = HashSet::new();

    for (address, account) in &result_and_state.state {
        if *address == Address::ZERO || *address == coinbase {
            continue;
        }

        let changed_storage_slots: Vec<_> = account.storage.iter()
            .filter(|(_, s)| s.is_changed())
            .collect();

        // Add Basic(address) only when balance/nonce/code changed, not merely storage.
        // Heuristic: if no storage slots changed, the account must have had a Basic-level
        // change (ETH transfer, nonce increment, contract creation/destruction).
        // Newly created contracts also get Basic regardless.
        let info_changed = changed_storage_slots.is_empty() && account.is_touched()
            || account.is_created();
        if info_changed {
            write_set.insert(hash_deterministic(MemoryLocation::Basic(*address)));
        }

        for (slot, _) in &changed_storage_slots {
            write_set.insert(hash_deterministic(MemoryLocation::Storage(*address, **slot)));
        }
    }

    write_set
}
