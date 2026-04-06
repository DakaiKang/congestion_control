use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{Mutex, OnceLock},
    thread,
};

use std::collections::{HashSet};

use alloy_primitives::{U256};
use hashbrown::HashMap;
use revm::{
    primitives::{BlockEnv, InvalidTransaction, SpecId, TxEnv},
    Database,
};

use crate::{
    chain::PevmChain,
    hash_deterministic,
    mv_memory::MvMemory,
    graph_scheduler::GraphScheduler,
    dependency_graph::{TransactionNode, TransactionGraph},
    pevm::{PevmError, execute_revm_sequential_with_access_sets, execute_revm_sequential, AbortReason, AsyncDropper, PevmResult},
    vm::{
        ExecutionError, PevmTxExecutionResult, Vm, VmExecutionError, VmExecutionResult,
    },
    EvmAccount, MemoryEntry, MemoryLocation, MemoryValue, Storage, Task, TxVersion,
};

#[derive(Debug, Default)]
/// The main pevm struct that executes blocks.
pub struct GraphPevm {
    execution_results: Vec<Mutex<Option<PevmTxExecutionResult>>>,
    abort_reason: OnceLock<AbortReason>,
    dropper: AsyncDropper<(MvMemory, GraphScheduler, Vec<TxEnv>)>,
}

impl GraphPevm {
    pub fn construct_graph_pevm_by_sequential<S: Storage, C: PevmChain>(
        chain: &C,
        storage: &S,
        spec_id: SpecId,
        block_env: BlockEnv,
        txs: Vec<TxEnv>,
        replica: u64,
    ) -> Result<(TransactionGraph, Vec<PevmTxExecutionResult>), PevmError<C>> {
        // Execute the txs in sequential to fetch the read/write set
        let (result, access_set) = execute_revm_sequential_with_access_sets(
            chain,
            storage,
            spec_id,
            block_env.clone(),
            txs,
        )?;
    
        let mut graph = TransactionGraph::new();
        let mut previous_cumulative_gas = 0u64;  
        
        for i in 0..result.len() {
            let tx_gas_used = result[i].receipt.cumulative_gas_used - previous_cumulative_gas;
            previous_cumulative_gas = result[i].receipt.cumulative_gas_used;
            
            let txn_node = TransactionNode::new(
                i as u64,
                replica,
                1,
                tx_gas_used,
                access_set[i].read_set.clone(),
                access_set[i].write_set.clone(),
            );
            graph.add_transaction(txn_node);
        }
    
        Ok((graph, result))
    }

    // Reorder the transactions based on the simluation results of the depedency graph
    // And regenerate the dependency graph with new transactions indices
    // The current implementation requires to simulate again, which could be optimized later.
    pub fn reorder_txs_by_dependency_graph(
        txs: Vec<TxEnv>, 
        graph: &mut TransactionGraph, 
        concurrency_level: usize
    ) -> (Vec<TxEnv>, TransactionGraph) {
        graph.simulate_parallel_execution(concurrency_level);
        let mut new_graph = TransactionGraph::new();

        let mut reordered_txs = Vec::new();
        // Access simulation_result directly from graph
        if let Some(ref simulation_result) = graph.simulation_result {
            for (thread_id, tx_id, start, end) in &simulation_result.execution_order {
                if let Some(&index) = graph.id_to_index.get(tx_id) {
                    reordered_txs.push(txs[index].clone());
                    let mut txn_node = graph.nodes[index].clone();
                    txn_node.children_indices.clear();
                    txn_node.parent_indices.clear();
                    txn_node.longest_suffix = txn_node.execution_time;
                    new_graph.add_transaction(txn_node);
                }
            }
        }

        new_graph.simulate_parallel_execution(concurrency_level);

        // println!("new_graph: {:#?}", new_graph.nodes);
        (reordered_txs, new_graph)
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
        dependency_graph: TransactionGraph,
    ) -> PevmResult<C>
    where
        C: PevmChain + Send + Sync,
        S: Storage + Send + Sync,
    {
        if txs.is_empty() {
            return Ok(Vec::new());
        }

        let block_size = txs.len();
        let scheduler = GraphScheduler::new(block_size, dependency_graph);

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
                    // println!("Spawned thread {:?}", thread::current().id());
                    let mut task = scheduler.next_task();
                    while task.is_some() {
                        task = match task.unwrap() {
                            Task::Execution(tx_version) => {
                                // println!("GraphPevm: Working on execution task of {:#?} by thread {:?}", tx_version, thread::current().id());
                                self.try_execute(&vm, &scheduler, tx_version)
                            }
                            Task::Validation(tx_version) => {
                                // println!("GraphPevm: Working on validation task of {:#?} by thread {:?}", tx_version.tx_idx, thread::current().id());
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
                    // println!("GraphPevm: Out of Loop by thread {:?}", thread::current().id());
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

        // Scheduler aborted due to timeout/deadlock (no abort_reason set).
        // Fall back to sequential execution to guarantee correctness.
        if scheduler.is_aborted() {
            self.dropper.drop((mv_memory, scheduler, Vec::new()));
            return execute_revm_sequential(chain, storage, spec_id, block_env, txs);
        }

        let re_execs = scheduler.re_execution_count.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!("[GraphPevm] block_size={} re_executions={} rate={:.1}%",
            block_size, re_execs, 100.0 * re_execs as f64 / block_size as f64);

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
                        // Non-balance entries (e.g. CodeHash for a contract deployed to
                        // a previously-lazy address) are unexpected but safe to skip here:
                        // the storage path already handled the account info correctly.
                        _ => {}
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

        self.dropper.drop((mv_memory, scheduler, txs));

        Ok(fully_evaluated_results)
    }

    fn try_execute<S: Storage, C: PevmChain>(
        &self,
        vm: &Vm<'_, S, C>,
        scheduler: &GraphScheduler,
        tx_version: TxVersion,
    ) -> Option<Task> {
        // Track the last blocking_tx_idx that caused add_dependency to return false.
        // If the same tx blocks us twice in a row, it's the LackOfFundForMaxFee heuristic
        // failing (not a real MvMemory ESTIMATE case), so we fall back to sequential
        // instead of spinning forever.
        let mut last_failed_blocking: Option<usize> = None;
        loop {
            return match vm.execute(&tx_version) {
                Err(VmExecutionError::Retry) => {
                    // println!("GraphPevm: Retry execution for {:#?} by thread {:?}", tx_version, thread::current().id());
                    if self.abort_reason.get().is_none() && !scheduler.is_aborted() {
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::FallbackToSequential) => {
                    // println!("GraphPevm: Fallback to sequential execution for {:#?} by thread {:?}", tx_version, thread::current().id());
                    scheduler.abort();
                    self.abort_reason
                        .get_or_init(|| AbortReason::FallbackToSequential);
                    None
                }
                Err(VmExecutionError::Blocking(blocking_tx_idx)) => {
                    // println!("GraphPevm: Blocking on transaction index {} for {:#?} by thread {:?}", blocking_tx_idx, tx_version, thread::current().id());

                    if !scheduler.add_dependency(tx_version.tx_idx, blocking_tx_idx) {
                        if self.abort_reason.get().is_some() || scheduler.is_aborted() {
                            return None;
                        }
                        if last_failed_blocking == Some(blocking_tx_idx) {
                            // Same tx blocked us twice: LackOfFundForMaxFee heuristic failed.
                            // The blocking tx is already done but this tx still can't execute.
                            // Fall back to sequential rather than spinning forever.
                            scheduler.abort();
                            self.abort_reason
                                .get_or_init(|| AbortReason::FallbackToSequential);
                            return None;
                        }
                        // First failure: blocking tx just finished, retry once so MvMemory
                        // ESTIMATE cases can read the now-committed data.
                        last_failed_blocking = Some(blocking_tx_idx);
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::ExecutionError(err)) => {
                    // println!("GraphPevm: Execution error for {:#?} by thread {:?}: {:?}", tx_version, thread::current().id(), err);
                    scheduler.abort();
                    self.abort_reason
                        .get_or_init(|| AbortReason::ExecutionError(err));
                    None
                }
                Ok(VmExecutionResult {
                    execution_result,
                    flags,
                }) => {
                    // println!("GraphPevm: Finished execution for {:#?} by thread {:?}, scheduling validation", tx_version, thread::current().id());
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
    scheduler: &GraphScheduler,
    tx_version: &TxVersion,
) -> Option<Task> {
    // println!("start validation for tx {}", tx_version.tx_idx);
    let read_set_valid = mv_memory.validate_read_locations(tx_version.tx_idx);
    let aborted = !read_set_valid && scheduler.try_validation_abort(tx_version);
    if aborted {
        println!("Validation aborted for tx {}", tx_version.tx_idx);
        mv_memory.convert_writes_to_estimates(tx_version.tx_idx);
    }
    scheduler.finish_validation(tx_version, aborted)
}
