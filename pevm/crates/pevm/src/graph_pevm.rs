use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{mpsc, Mutex, OnceLock},
    thread,
};

use std::collections::{HashSet};

use alloy_primitives::{TxNonce, U256};
use alloy_rpc_types_eth::{Block, BlockTransactions};
use hashbrown::HashMap;
use revm::{
    db::CacheDB,
    primitives::{BlockEnv, InvalidTransaction, SpecId, TxEnv, Address, ResultAndState},
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
    dependency_graph::{TransactionId, TransactionNode, TransactionGraph},
    pevm::{PevmError, execute_revm_sequential_with_access_sets},
    vm::{
        build_evm, ExecutionError, PevmTxExecutionResult, Vm, VmExecutionError, VmExecutionResult,
    },
    EvmAccount, MemoryEntry, MemoryLocation, MemoryValue, Storage, Task, TxIdx, TxVersion,
};

#[derive(Debug, Default)]

pub struct GraphPevm {

}

impl GraphPevm {
    pub fn construct_graph_pevm_by_sequential<S: Storage, C: PevmChain>(
        chain: &C,
        storage: &S,
        spec_id: SpecId,
        block_env: BlockEnv,
        txs: Vec<TxEnv>
    ) -> Result<TransactionGraph, PevmError<C>> {
        // Execute the txs in sequential to fetch the read/write set
        let (result, access_set) = execute_revm_sequential_with_access_sets(
            chain,
            storage,
            spec_id,
            block_env.clone(),
            txs,
        )?;

        let mut graph = TransactionGraph::new();
        for i in 0..result.len() {
            let txn_node = TransactionNode::new(i as u64, 1, 1, result[i].receipt.cumulative_gas_used, HashSet::new(), access_set[i].clone());
            graph.add_transaction(txn_node);
        }

        Ok(graph)
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
        (reordered_txs, new_graph)
    }
}

