use std::{
    cmp::min,
    collections::BinaryHeap,
    cmp::Reverse,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use smallvec::SmallVec;

use crate::{FinishExecFlags, IncarnationStatus, Task, TxIdx, TxStatus, TxVersion};
use crate::dependency_graph::TransactionGraph;

/// GraphScheduler coordinates execution and validation tasks among worker threads
/// based on an explicit dependency graph.
pub struct GraphScheduler {
    // ============ Fields inherited from basic Scheduler ============
    block_size: usize,
    transactions_status: Vec<Mutex<TxStatus>>,
    transactions_dependents: Vec<Mutex<SmallVec<[TxIdx; 1]>>>,
    validation_idx: AtomicUsize,
    min_validation_idx: AtomicUsize,
    num_validated: AtomicUsize,
    aborted: AtomicBool,
    
    // ============ New fields for GraphScheduler ============
    dependency_graph: TransactionGraph,
    executable_txs: Mutex<BinaryHeap<Reverse<TxIdx>>>,
    task_available: Condvar,
    remaining_dependencies: Vec<AtomicUsize>,
    /// Whether each tx has already released its children's graph dependencies.
    /// Each tx must decrement children's remaining_deps exactly once (on first
    /// successful execution). This prevents u64 underflow from multiple re-executions.
    children_released: Vec<AtomicBool>,
    /// Total number of re-executions (incarnation > 0) for diagnostics.
    pub re_execution_count: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub blocking_reexecs: AtomicUsize,
}

impl std::fmt::Debug for GraphScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphScheduler")
            .field("block_size", &self.block_size)
            .field("validation_idx", &self.validation_idx.load(Ordering::Relaxed))
            .field("num_validated", &self.num_validated.load(Ordering::Relaxed))
            .field("aborted", &self.aborted.load(Ordering::Relaxed))
            .field("graph_nodes", &self.dependency_graph.nodes.len())
            .finish_non_exhaustive()
    }
}

impl GraphScheduler {
    /// Create a new GraphScheduler with the given dependency graph
    pub fn new(block_size: usize, dependency_graph: TransactionGraph) -> Self {
        // println!("\n=== GraphScheduler Initialization ===");
        
        // Step 1: Initialize remaining_dependencies with parent count for each transaction
        let remaining_dependencies: Vec<AtomicUsize> = dependency_graph.nodes.iter()
            .map(|node| AtomicUsize::new(node.parent_indices.len()))
            .collect();
        
        // Step 2: Find initial executable transactions (root nodes with no parents)
        let mut executable_txs = BinaryHeap::new();
        let mut root_count = 0;
        
        for (tx_idx, node) in dependency_graph.nodes.iter().enumerate() {
            if node.parent_indices.is_empty() {
                executable_txs.push(Reverse(tx_idx));
                root_count += 1;
                // if root_count <= 10 {
                //     println!("  Root node: tx {}", tx_idx);
                // }
            }
        }
        
        // println!("Total transactions: {}", block_size);
        // println!("Root nodes (initial executable): {}", root_count);
        // println!("Total edges in graph: {}", 
        //          dependency_graph.nodes.iter().map(|n| n.children_indices.len()).sum::<usize>());
        // println!("=====================================\n");
        
        Self {
            block_size,
            transactions_status: (0..block_size)
                .map(|_| Mutex::new(TxStatus {
                    incarnation: 0,
                    status: IncarnationStatus::ReadyToExecute,
                }))
                .collect(),
            transactions_dependents: (0..block_size).map(|_| Mutex::default()).collect(),
            validation_idx: AtomicUsize::new(block_size),
            min_validation_idx: AtomicUsize::new(block_size),
            num_validated: AtomicUsize::new(0),
            aborted: AtomicBool::new(false),
            children_released: (0..block_size).map(|_| AtomicBool::new(false)).collect(),
            re_execution_count: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            blocking_reexecs: AtomicUsize::new(0),
            dependency_graph,
            executable_txs: Mutex::new(executable_txs),
            task_available: Condvar::new(),
            remaining_dependencies,
        }
    }
    
    pub fn abort(&self) {
        self.aborted.store(true, Ordering::Relaxed);
    }

    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Relaxed)
    }
    
    /// Get the next task for a worker thread to execute
    pub fn next_task(&self) -> Option<Task> {
        let start_time = Instant::now();
        let timeout = Duration::from_secs(1);

        while !self.aborted.load(Ordering::Relaxed) {
            // Hard timeout guard against scheduling deadlocks.
            if start_time.elapsed() > timeout {
                println!("\n⚠️  TIMEOUT in next_task");
                self.print_deadlock_info();
                self.abort();
                return None;
            }

            // Step 1: Check if all tasks are done
            if self.check_all_done() {
                return None;
            }

            // Step 2: Try to get a validation task
            if let Some(task) = self.try_validation_task() {
                return Some(task);
            }

            // Step 3: Try to get an execution task
            if let Some(task) = self.try_execution_task() {
                return Some(task);
            }

            // Step 4: No task ready — park on the condvar instead of
            // busy-spinning on `thread::yield_now()`. Idle threads otherwise
            // hammer the `executable_txs` mutex once per iteration, starving
            // worker threads trying to push new ready txs.
            //
            // The 1 ms timeout bounds stragglers: validation_idx is bumped
            // via an AtomicUsize that doesn't fire the condvar, so we must
            // re-poll periodically. 1 ms is short enough that worst-case
            // validation-task latency stays low, long enough to eliminate
            // the raw CPU burn from the old busy-spin.
            {
                let guard = self.executable_txs.lock().unwrap();
                if guard.is_empty() {
                    let _ = self
                        .task_available
                        .wait_timeout(guard, Duration::from_millis(1));
                    // guard dropped after wait_timeout returns
                }
                // else: a task appeared between our pop attempt and lock
                // acquisition; fall through and re-poll immediately.
            }
        }
        None
    }
    
    fn check_all_done(&self) -> bool {
        let has_executable = {
            let heap = self.executable_txs.lock().unwrap();
            !heap.is_empty()
        };
        
        let validation_idx = self.validation_idx.load(Ordering::Relaxed);
        let num_validated = self.num_validated.load(Ordering::Relaxed);
        let min_validation_idx = self.min_validation_idx.load(Ordering::Relaxed);
        
        let all_done = !has_executable 
            && validation_idx >= self.block_size
            && num_validated >= self.block_size - min_validation_idx;
        
        if all_done {
            self.task_available.notify_all();
        }
        
        all_done
    }
    
    fn try_validation_task(&self) -> Option<Task> {
        let validation_idx = self.validation_idx.load(Ordering::Relaxed);
        
        let exec_idx = {
            let heap = self.executable_txs.lock().unwrap();
            heap.peek().map(|Reverse(idx)| *idx).unwrap_or(self.block_size)
        };
        
        if validation_idx < exec_idx {
            let tx_idx = self.validation_idx.fetch_add(1, Ordering::Relaxed);
            
            if tx_idx < self.block_size {
                // Try to steal execution job
                let steal_attempted = {
                    let tx = index_mutex!(self.transactions_status, tx_idx);
                    let is_ready = tx.status == IncarnationStatus::ReadyToExecute;
                    drop(tx);
                    is_ready
                };
                
                if steal_attempted && self.all_dependencies_satisfied(tx_idx) {
                    let mut tx = index_mutex!(self.transactions_status, tx_idx);
                    if tx.status == IncarnationStatus::ReadyToExecute {
                        tx.status = IncarnationStatus::Executing;
                        return Some(Task::Execution(TxVersion {
                            tx_idx,
                            tx_incarnation: tx.incarnation,
                        }));
                    }
                }
                
                // Check for validation task
                let tx = index_mutex!(self.transactions_status, tx_idx);
                
                if matches!(
                    tx.status,
                    IncarnationStatus::Executed | IncarnationStatus::Validated
                ) {
                    return Some(Task::Validation(TxVersion {
                        tx_idx,
                        tx_incarnation: tx.incarnation,
                    }));
                }
                
                if tx.status == IncarnationStatus::Aborting {
                    return None;
                }
            }
        }
        
        None
    }
    
    fn try_execution_task(&self) -> Option<Task> {
        let tx_idx = {
            let mut heap = self.executable_txs.lock().unwrap();
            let result = heap.pop().map(|Reverse(idx)| idx);
            if let Some(idx) = result {
                // println!("DEBUG [TryExecTask] Popped tx {} from heap", idx);
            }
            result
        }?;
        
        let task = self.try_execute(tx_idx).map(Task::Execution);
        if task.is_none() {
            // println!("DEBUG [TryExecTask] try_execute failed for tx {}", tx_idx);
        }
        task
    }
    
    fn try_execute(&self, tx_idx: TxIdx) -> Option<TxVersion> {
        if tx_idx >= self.block_size {
            return None;
        }

        if !self.all_dependencies_satisfied(tx_idx) {
            return None;
        }

        let mut tx = self.transactions_status[tx_idx].lock().unwrap();

        match tx.status {
            IncarnationStatus::ReadyToExecute => {
                if tx.incarnation > 0 {
                    self.re_execution_count.fetch_add(1, Ordering::Relaxed);
                }
                tx.status = IncarnationStatus::Executing;
                Some(TxVersion { tx_idx, tx_incarnation: tx.incarnation })
            }
            IncarnationStatus::Aborting => {
                tx.incarnation += 1;
                self.re_execution_count.fetch_add(1, Ordering::Relaxed);
                tx.status = IncarnationStatus::Executing;
                Some(TxVersion { tx_idx, tx_incarnation: tx.incarnation })
            }
            _ => None,
        }
    }
    
    fn all_dependencies_satisfied(&self, tx_idx: TxIdx) -> bool {
        self.remaining_dependencies[tx_idx].load(Ordering::Relaxed) == 0
    }
    
    pub fn add_dependency(&self, tx_idx: TxIdx, blocking_tx_idx: TxIdx) -> bool {
        let blocking_tx = index_mutex!(self.transactions_status, blocking_tx_idx);
        if matches!(
            blocking_tx.status,
            IncarnationStatus::Executed | IncarnationStatus::Validated
        ) {
            return false;
        }

        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        debug_assert_eq!(tx.status, IncarnationStatus::Executing);
        tx.status = IncarnationStatus::Aborting;

        let mut blocking_dependents = index_mutex!(self.transactions_dependents, blocking_tx_idx);
        blocking_dependents.push(tx_idx);

        #[cfg(feature = "diagnostics")]
        self.blocking_reexecs.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn set_ready_status(&self, tx_idx: TxIdx) {
        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        debug_assert_eq!(tx.status, IncarnationStatus::Aborting);
        tx.status = IncarnationStatus::ReadyToExecute;
        tx.incarnation += 1;
    }
    
    pub fn finish_execution(
        &self,
        tx_version: TxVersion,
        flags: FinishExecFlags,
    ) -> Option<Task> {
        let mut tx = index_mutex!(self.transactions_status, tx_version.tx_idx);
        debug_assert_eq!(tx.status, IncarnationStatus::Executing);
        debug_assert_eq!(tx.incarnation, tx_version.tx_incarnation);

        // Release dependency graph children — exactly once per tx (on first successful
        // execution, regardless of incarnation number). Graph ordering is purely for initial
        // scheduling efficiency; correctness is handled by OCC validation.
        // Re-executions must NOT decrement children's remaining_deps again, as that causes
        // u64 underflow and leaves children permanently stuck in Aborting state.
        let already_released = self.children_released[tx_version.tx_idx]
            .swap(true, Ordering::Relaxed);
        if !already_released {
            let children_indices = self.dependency_graph.nodes[tx_version.tx_idx]
                .children_indices.clone();

            for &child_idx in &children_indices {
                let prev_count = self.remaining_dependencies[child_idx]
                    .fetch_sub(1, Ordering::Relaxed);

                if prev_count == 1 {
                    // Child's graph deps are now all satisfied. Add to heap regardless of
                    // current status — try_execute handles ReadyToExecute and Aborting both.
                    let mut heap = self.executable_txs.lock().unwrap();
                    heap.push(Reverse(child_idx));
                    drop(heap);
                    self.task_available.notify_one();
                }
            }
        }
        
        // Resume OCC dependents
        let mut dependents = self.transactions_dependents[tx_version.tx_idx].lock().unwrap();
        for tx_idx in dependents.drain(..) {
            let remaining = self.remaining_dependencies[tx_idx].load(Ordering::Relaxed);
            let deps_satisfied = self.all_dependencies_satisfied(tx_idx);
    
            // println!("DEBUG [FinishExec]   Tx {} after resume: remaining_deps={}, satisfied={}", 
                    //  tx_idx, remaining, deps_satisfied);
    
            if deps_satisfied {
                self.set_ready_status(tx_idx);
                let mut heap = self.executable_txs.lock().unwrap();
                heap.push(Reverse(tx_idx));
                drop(heap);
                self.task_available.notify_one();
        
                // println!("DEBUG [FinishExec]   Resumed OCC dependent tx {} (added to heap)", tx_idx);
            } else {
                // println!("DEBUG [FinishExec]   Tx {} NOT added to heap (deps not satisfied)", tx_idx);
            }
        }
        // Handle validation scheduling
        let min_validation_idx = if flags.contains(FinishExecFlags::NeedValidation) {
            min(
                self.min_validation_idx
                    .fetch_min(tx_version.tx_idx, Ordering::Relaxed),
                tx_version.tx_idx,
            )
        } else {
            self.min_validation_idx.load(Ordering::Relaxed)
        };
        
        if min_validation_idx < self.block_size {
            if tx_version.tx_idx < min_validation_idx {
                if flags.contains(FinishExecFlags::WroteNewLocation) {
                    self.validation_idx
                        .fetch_min(min_validation_idx, Ordering::Relaxed);
                }
            } else if tx_version.tx_idx < self.validation_idx.load(Ordering::Relaxed) {
                if flags.contains(FinishExecFlags::WroteNewLocation) {
                    self.validation_idx
                        .fetch_min(tx_version.tx_idx + 1, Ordering::Relaxed);
                }
                if flags.contains(FinishExecFlags::NeedValidation) {
                    tx.status = IncarnationStatus::Executed;
                    return Some(Task::Validation(tx_version));
                }
                tx.status = IncarnationStatus::Validated;
                self.num_validated.fetch_add(1, Ordering::Relaxed);
            }
        }

        if flags.contains(FinishExecFlags::NeedValidation) {
            tx.status = IncarnationStatus::Executed;
        } else {
            tx.status = IncarnationStatus::Validated;
            self.num_validated.fetch_add(1, Ordering::Relaxed);
        }
        None
    }
    
    pub fn try_validation_abort(&self, tx_version: &TxVersion) -> bool {
        let mut tx = index_mutex!(self.transactions_status, tx_version.tx_idx);
        if tx.status == IncarnationStatus::Validated {
            self.num_validated.fetch_sub(1, Ordering::Relaxed);
        }

        let aborting = matches!(
            tx.status,
            IncarnationStatus::Executed | IncarnationStatus::Validated
        );
        if aborting {
            tx.status = IncarnationStatus::Aborting;
        }
        aborting
    }

    pub fn finish_validation(&self, tx_version: &TxVersion, aborted: bool) -> Option<Task> {
        if aborted {
            // println!("DEBUG [FinishVal] Tx {} validation failed, aborting", tx_version.tx_idx);
            
            self.set_ready_status(tx_version.tx_idx);
            self.validation_idx
                .fetch_min(tx_version.tx_idx + 1, Ordering::Relaxed);
            
            // Do NOT re-increment children's remaining_deps here. Graph ordering is only
            // for initial scheduling; OCC validation ensures correctness on re-executions.
            // Re-incrementing causes the u64 underflow bug (extra decrements from
            // re-executions accumulate without matching increments).
            if self.all_dependencies_satisfied(tx_version.tx_idx) {
                return self.try_execute(tx_version.tx_idx).map(Task::Execution);
            }
        } else {
            let mut tx = index_mutex!(self.transactions_status, tx_version.tx_idx);
            if tx.status == IncarnationStatus::Executed {
                tx.status = IncarnationStatus::Validated;
                self.num_validated.fetch_add(1, Ordering::Relaxed);
            }
        }
        None
    }
    
    fn print_deadlock_info(&self) {
        println!("\n=== DEADLOCK DIAGNOSTICS ===");
        
        let heap = self.executable_txs.lock().unwrap();
        println!("Executable heap size: {}", heap.len());
        if !heap.is_empty() {
            let heap_vec: Vec<_> = heap.iter().map(|Reverse(idx)| *idx).collect();
            println!("Executable txs: {:?}", heap_vec);
        }
        drop(heap);
        
        println!("Validation idx: {}", self.validation_idx.load(Ordering::Relaxed));
        println!("Min validation idx: {}", self.min_validation_idx.load(Ordering::Relaxed));
        println!("Num validated: {}/{}", 
                 self.num_validated.load(Ordering::Relaxed), 
                 self.block_size);
        
        // Show which txs are waiting as OCC dependents of each tx
        println!("\nOCC dependents (Aborting txs waiting for which tx):");
        for idx in 0..self.block_size {
            let deps = self.transactions_dependents[idx].lock().unwrap();
            if !deps.is_empty() {
                let tx = index_mutex!(self.transactions_status, idx);
                println!("  Tx {} (status={:?}) has OCC dependents: {:?}", idx, tx.status, *deps);
            }
        }

        println!("\nStuck transactions:");
        let mut stuck_count = 0;

        for idx in 0..self.block_size {
            let tx = index_mutex!(self.transactions_status, idx);
            let remaining = self.remaining_dependencies[idx].load(Ordering::Relaxed);

            if !matches!(tx.status, IncarnationStatus::Validated) || remaining > 0 {
                stuck_count += 1;
                if stuck_count <= 20 {
                    println!("  Tx {}: status={:?}, remaining_deps={}",
                             idx, tx.status, remaining);

                    if remaining > 0 {
                        let parents = &self.dependency_graph.nodes[idx].parent_indices;
                        println!("    Parents: {:?}", parents);

                        for &parent_idx in parents.iter().take(5) {
                            {
                                let parent_tx = self.transactions_status[parent_idx].lock().unwrap();
                                println!("      Parent {}: status={:?}", parent_idx, parent_tx.status);
                            }
                        }
                    }
                }
            }
        }

        println!("\nTotal stuck: {}/{}", stuck_count, self.block_size);
        println!("=== END DIAGNOSTICS ===\n");

    }
}
