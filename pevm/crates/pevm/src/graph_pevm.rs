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
            txs.clone(),
        )?;

        let mut graph = TransactionGraph::new();
        for i in 0..result.len() {
            let txn_node = TransactionNode::new(i as u64, 1, 1, result[i].receipt.cumulative_gas_used, HashSet::new(), access_set[i].clone());
            graph.add_transaction(txn_node);
        }

        Ok(graph)
    }
}

