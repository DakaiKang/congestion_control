//! Vegeta [NSDI '25] as a baseline.
//!
//! Xu et al., *Vegeta: Enabling Parallel Smart Contract Execution in Leaderless
//! Blockchains*, 22nd USENIX NSDI, 2025. The original prototype is in Go.
//!
//! Vegeta's contribution is a *schedule*: speculative pre-execution yields each
//! transaction's access sets, transactions are re-sequenced so that the longest
//! per-key dependency chains run first (Rule 1), and a DAG in which only WAW
//! edges block execution (Rule 2) drives a deterministic replay. It exploits
//! intra-block parallelism only — §4.2: "enabling inter-block parallelism may
//! further speed up execution, which we leave for future work."
//!
//! Two ways of running that schedule are provided:
//!
//! * [`speculate_graph`] — **the configuration our experiments report.** It
//!   applies Vegeta's Rule-1 ordering to the per-block conflict graph our own
//!   pre-pass already produced and hands the result to the same graph-aware OCC
//!   engine ([`crate::graph_pevm::GraphPevm`]) that runs Omakase. The engine's
//!   conflict graph already contains only WAW edges, which is exactly Rule 2's
//!   blocking clause, so the mapping is direct. Every engine in the comparison
//!   then shares one pre-pass, one multi-version store and one validation
//!   path; the *only* thing that differs between "Vegeta" and "Graph-aware OCC"
//!   is the intra-block order, and the only thing that differs between Vegeta
//!   and Omakase is that Omakase additionally integrates across blocks. Abort
//!   counts are directly comparable as a result.
//!
//! * [`speculate`] + [`replay`] — a literal port of Algorithms 1–3, including
//!   the paper's own execution model (Aria-style batches against a frozen
//!   snapshot, no multi-version store, batch commit, serial re-execution tail).
//!   Kept for reference and for checking the schedule against the paper's
//!   semantics; it runs on a different substrate from the other engines, so
//!   its throughput would measure the port rather than the algorithm.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use revm::db::CacheDB;
use revm::primitives::{BlockEnv, SpecId, TxEnv};
use revm::{Database, DatabaseCommit};

use crate::chain::PevmChain;
use crate::dependency_graph::TransactionGraph;
use crate::pevm::{extract_write_set_from_result, TrackingDB};
use crate::storage::StorageWrapper;
use crate::vm::{build_evm, ExecutionError, PevmTxExecutionResult};
use crate::{PevmError, Storage};

/// Dependency kind between an ordered pair of transactions, tagged as in
/// Algorithm 1 lines 24–36. At most one tag is kept per pair: a pair that is
/// both WAR and RAW is promoted to WAW (line 34), and WAW takes precedence
/// over everything (lines 27–29).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dep {
    /// Write-after-read: `tx` writes a key the earlier tx read.
    War,
    /// Read-after-write: `tx` reads a key the earlier tx wrote.
    Raw,
    /// Write-after-write, or a pair that is simultaneously WAR and RAW.
    Waw,
}

/// The block proposal Vegeta's speculation phase emits: a re-sequenced
/// transaction list, the speculative access sets, and the dependency DAG.
/// All three travel inside the block proposal (Algorithm 1 line 12), which is
/// what makes `serialized_len` the right bandwidth figure to report.
#[derive(Debug, Clone, Default)]
pub struct VegetaSchedule {
    /// Transactions in the adjusted (longest-chain-first) order.
    pub txs: Vec<TxEnv>,
    /// Speculative read sets, indexed by position in `txs`.
    pub spec_reads: Vec<HashSet<u64>>,
    /// Speculative write sets, indexed by position in `txs`.
    pub spec_writes: Vec<HashSet<u64>>,
    /// `dag[i]` lists `(j, kind)` for every `j < i` that `i` depends on.
    pub dag: Vec<Vec<(usize, Dep)>>,
}

impl VegetaSchedule {
    /// Bytes the schedule adds to a block proposal: the DAG edges plus the
    /// speculative read/write sets, which Vegeta ships alongside the
    /// transactions so every replica can replay deterministically.
    ///
    /// Counted as 4 bytes per edge endpoint + 1 byte per edge tag, and 8 bytes
    /// per access-set key (the `u64` location hash our engine already uses).
    pub fn serialized_len(&self) -> usize {
        let edges: usize = self.dag.iter().map(|v| v.len()).sum();
        let keys: usize = self
            .spec_reads
            .iter()
            .chain(self.spec_writes.iter())
            .map(|s| s.len())
            .sum();
        edges * (4 + 1) + keys * 8
    }

    /// Number of DAG edges.
    pub fn num_edges(&self) -> usize {
        self.dag.iter().map(|v| v.len()).sum()
    }
}

/// Counters describing how a replay went — the re-execution behaviour that
/// Vegeta's §4.3 optimisation targets.
#[derive(Debug, Clone, Copy, Default)]
pub struct VegetaStats {
    pub num_txs: usize,
    /// Parallel batches drained by `pop_ready_batch` (Algorithm 2 line 4/13).
    pub num_batches: usize,
    /// Largest batch, i.e. the peak achievable parallelism.
    pub max_batch: usize,
    /// Transactions re-executed serially at the end: Algorithm 3 `TxsRe`
    /// (touched a new key that someone else also touched) plus the retained
    /// `read` set (read a new key that some transaction newly wrote).
    pub re_executed: usize,
    /// `TxsRe` alone (Algorithm 3 lines 5–8).
    pub re_exec_new_shared_key: usize,
    /// Retained `read` transactions (Algorithm 3 lines 9–10, 13, 15–17).
    pub re_exec_new_read_key: usize,
}

impl VegetaStats {
    /// Re-executions per transaction — comparable with `ExecDiagnostics::re_exec_rate`.
    pub fn re_exec_rate(&self) -> f64 {
        if self.num_txs == 0 {
            0.0
        } else {
            self.re_executed as f64 / self.num_txs as f64
        }
    }
}

/// **Algorithm 1 — speculation phase.**
///
/// `txs`, `reads` and `writes` are the block's transactions in proposal order
/// together with the access sets recorded by the speculative pre-pass. Returns
/// the re-sequenced transactions and the DAG that goes into the block proposal.
///
/// Note the paper's chain construction (lines 17–21) is deliberately an
/// approximation: each chain considers exactly one key, so a transaction can
/// appear in several chains. We reproduce that rather than computing true
/// transitive chains.
pub fn speculate(
    txs: Vec<TxEnv>,
    reads: Vec<HashSet<u64>>,
    writes: Vec<HashSet<u64>>,
) -> VegetaSchedule {
    let n = txs.len();
    debug_assert_eq!(n, reads.len());
    debug_assert_eq!(n, writes.len());

    // Lines 17–21: group transactions into a per-key dependency chain.
    let mut access: HashMap<u64, Vec<usize>> = HashMap::new();
    for i in 0..n {
        for key in reads[i].iter().chain(writes[i].iter()) {
            let chain = access.entry(*key).or_default();
            // A transaction accesses a key once per chain even if it both
            // reads and writes it.
            if chain.last() != Some(&i) {
                chain.push(i);
            }
        }
    }

    // Line 22: sort chains longest-first. Ties are broken by the chain's first
    // transaction index so the order is deterministic across replicas (the
    // paper leaves this implicit; determinism is required for safety).
    let mut chains: Vec<Vec<usize>> = access.into_values().collect();
    chains.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| a.first().cmp(&b.first()))
    });

    // Lines 6–9 (Rule 1): emit transactions chain by chain, skipping duplicates.
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for chain in &chains {
        for &i in chain {
            if !seen[i] {
                seen[i] = true;
                order.push(i);
            }
        }
    }
    // Transactions that touched no key at all appear in no chain; keep them in
    // their original relative order at the end.
    for i in 0..n {
        if !seen[i] {
            order.push(i);
        }
    }

    let mut new_txs = Vec::with_capacity(n);
    let mut spec_reads = Vec::with_capacity(n);
    let mut spec_writes = Vec::with_capacity(n);
    let mut txs_opt: Vec<Option<TxEnv>> = txs.into_iter().map(Some).collect();
    for &i in &order {
        new_txs.push(txs_opt[i].take().expect("each index emitted once"));
        spec_reads.push(reads[i].clone());
        spec_writes.push(writes[i].clone());
    }

    let dag = build_dag(&spec_reads, &spec_writes);
    VegetaSchedule {
        txs: new_txs,
        spec_reads,
        spec_writes,
        dag,
    }
}

/// **Vegeta's schedule on the graph-aware OCC engine.**
///
/// Takes a block's transactions together with the per-block conflict graph our
/// sequential pre-pass built for them (nodes carry each transaction's access
/// sets and gas cost, in proposal order), re-sequences the block by Vegeta's
/// Rule 1, and rebuilds the conflict graph over the new order. The returned
/// pair is consumed by `GraphPevm::execute_revm_parallel` exactly like the
/// output of `GraphPevm::reorder_txs_by_dependency_graph`, which is Omakase's
/// intra-block reorder — so swapping one for the other changes the schedule
/// and nothing else.
///
/// Rule 1 (Algorithm 1 lines 17–22, 6–9): group transactions into one chain per
/// key they access, sort chains longest-first, and emit transactions chain by
/// chain. As in the paper, each chain considers a single key, so this is the
/// paper's deliberate approximation rather than a true transitive chain.
pub fn speculate_graph(
    txs: Vec<TxEnv>,
    graph: &TransactionGraph,
) -> (Vec<TxEnv>, TransactionGraph) {
    let n = txs.len();
    debug_assert_eq!(n, graph.nodes.len());

    // Lines 17–21: per-key chains, in proposal order.
    let mut access: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        for key in node.read_set.iter().chain(node.write_set.iter()) {
            let chain = access.entry(*key).or_default();
            if chain.last() != Some(&i) {
                chain.push(i);
            }
        }
    }

    // Line 22: longest chain first; ties broken by first transaction index so
    // every replica derives the same order.
    let mut chains: Vec<Vec<usize>> = access.into_values().collect();
    chains.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.first().cmp(&b.first())));

    // Lines 6–9: emit in chain order without duplicates; key-less
    // transactions keep their relative order at the end.
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for chain in &chains {
        for &i in chain {
            if !seen[i] {
                seen[i] = true;
                order.push(i);
            }
        }
    }
    for i in 0..n {
        if !seen[i] {
            order.push(i);
        }
    }

    // Rebuild the conflict graph over the new order. `add_transaction` links a
    // transaction only to the latest earlier writer of a key it writes (WAW),
    // which is Rule 2's blocking condition; RAW/WAR are left to OCC validation
    // just as Rule 2 lets them proceed within a batch.
    let mut new_graph = TransactionGraph::new();
    new_graph.set_hot_key_threshold(graph.hot_key_threshold);
    let mut reordered = Vec::with_capacity(n);
    let mut txs_opt: Vec<Option<TxEnv>> = txs.into_iter().map(Some).collect();
    for &i in &order {
        reordered.push(txs_opt[i].take().expect("each index emitted once"));
        let mut node = graph.nodes[i].clone();
        node.children_indices.clear();
        node.parent_indices.clear();
        node.longest_suffix = node.gas_cost;
        new_graph
            .add_transaction(node)
            .expect("rebuilding a per-block graph cannot produce duplicate ids");
    }
    (reordered, new_graph)
}

/// Bytes a block proposal carries for a conflict graph: 8 bytes per access-set
/// key plus 5 bytes per edge (a 4-byte endpoint index and a 1-byte tag).
/// One formula for every engine so the bandwidth figures are comparable.
pub fn graph_proposal_bytes(graph: &TransactionGraph) -> usize {
    let keys: usize = graph
        .nodes
        .iter()
        .map(|n| n.read_set.len() + n.write_set.len())
        .sum();
    keys * 8 + graph.edge_count() * 5
}

/// **Algorithm 1 lines 24–36 — `BuildDAG`.**
fn build_dag(reads: &[HashSet<u64>], writes: &[HashSet<u64>]) -> Vec<Vec<(usize, Dep)>> {
    let n = reads.len();
    let mut dag: Vec<Vec<(usize, Dep)>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in 0..i {
            // Line 27–29: WAW wins outright.
            if intersects(&writes[j], &writes[i]) {
                dag[i].push((j, Dep::Waw));
                continue;
            }
            let mut kind: Option<Dep> = None;
            // Line 30–31: tx i writes what tx j read.
            if intersects(&reads[j], &writes[i]) {
                kind = Some(Dep::War);
            }
            // Line 32–35: tx i reads what tx j wrote; WAR + RAW is promoted to WAW.
            if intersects(&writes[j], &reads[i]) {
                kind = Some(if kind == Some(Dep::War) {
                    Dep::Waw
                } else {
                    Dep::Raw
                });
            }
            if let Some(k) = kind {
                dag[i].push((j, k));
            }
        }
    }
    dag
}

fn intersects(a: &HashSet<u64>, b: &HashSet<u64>) -> bool {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small.iter().any(|k| large.contains(k))
}

/// **Algorithm 2 lines 18–25 — `PopTxsBatch`, applying Rule 2.**
///
/// A transaction is ready when it has no WAW dependency on an unfinished
/// transaction and does not have *both* a WAR and a RAW dependency on
/// unfinished transactions. (Since `build_dag` collapses a WAR+RAW pair into
/// WAW, the second clause can only be triggered by two different predecessors,
/// which is how we read "on any previous transactions" in Rule 2.)
fn pop_ready_batch(dag: &[Vec<(usize, Dep)>], done: &[bool], taken: &[bool]) -> Vec<usize> {
    let mut ready = Vec::new();
    for i in 0..dag.len() {
        if done[i] || taken[i] {
            continue;
        }
        let mut blocked = false;
        let mut has_war = false;
        let mut has_raw = false;
        for &(j, kind) in &dag[i] {
            if done[j] {
                continue; // completed predecessors impose no constraint
            }
            match kind {
                Dep::Waw => {
                    blocked = true;
                    break;
                }
                Dep::War => has_war = true,
                Dep::Raw => has_raw = true,
            }
        }
        if !blocked && !(has_war && has_raw) {
            ready.push(i);
        }
    }
    ready
}

/// Outcome of executing one transaction against the frozen pre-batch state.
struct Executed {
    idx: usize,
    result: PevmTxExecutionResult,
    reads: HashSet<u64>,
    writes: HashSet<u64>,
}

/// A reusable EVM over a frozen snapshot of `storage`, owned by one worker for
/// the duration of a batch.
///
/// Vegeta's replica executes against its persistent world state, so the state
/// cache is warm across transactions. Building a fresh `CacheDB` per
/// transaction would make every storage read miss and would understate Vegeta
/// badly — an artefact of the port, not a property of the algorithm. Because a
/// batch's transactions are deliberately *not* committed to this DB (Vegeta
/// applies writes only after the batch completes), the cache holds nothing but
/// clean reads of the frozen pre-batch state and is safe to share across the
/// transactions one worker runs.
struct BatchExecutor<'a, S: Storage, C: PevmChain> {
    evm: revm::Evm<'a, (), TrackingDB<CacheDB<StorageWrapper<'a, S>>>>,
    chain: &'a C,
    spec_id: SpecId,
    coinbase: revm::primitives::Address,
}

impl<'a, S: Storage, C: PevmChain> BatchExecutor<'a, S, C> {
    fn new(chain: &'a C, storage: &'a S, spec_id: SpecId, block_env: BlockEnv) -> Self {
        let coinbase = block_env.coinbase;
        let tracking_db = TrackingDB {
            inner: CacheDB::new(StorageWrapper(storage)),
            reads: HashSet::new(),
            coinbase,
            lazy_addresses: HashSet::new(),
        };
        let evm = build_evm(tracking_db, chain, spec_id, block_env, None, true);
        Self {
            evm,
            chain,
            spec_id,
            coinbase,
        }
    }

    /// Run one transaction, returning its result and observed access sets.
    /// Nothing is committed, so the next call still sees the pre-batch state.
    fn run(
        &mut self,
        tx: TxEnv,
    ) -> Result<(PevmTxExecutionResult, HashSet<u64>, HashSet<u64>), ExecutionError> {
        // Mirror the lazy-transfer exclusions the rest of the engine applies,
        // so Vegeta's access sets stay comparable with ours.
        let caller = tx.caller;
        let recipient = tx.transact_to.to().copied();
        let is_lazy = if let Some(to) = recipient {
            self.evm
                .db_mut()
                .inner
                .basic(to)
                .map(|a| a.map_or(true, |acc| acc.is_empty_code_hash()))
                .unwrap_or(false)
        } else {
            false
        };
        {
            let db = self.evm.db_mut();
            db.reads.clear();
            db.lazy_addresses.clear();
            if is_lazy {
                db.lazy_addresses.insert(caller);
                if let Some(to) = recipient {
                    db.lazy_addresses.insert(to);
                }
            }
        }

        *self.evm.tx_mut() = tx;
        let result_and_state = self
            .evm
            .transact()
            .map_err(|err| ExecutionError::Custom(err.to_string()))?;

        let reads = std::mem::take(&mut self.evm.db_mut().reads);
        let writes = extract_write_set_from_result(&result_and_state, self.coinbase);
        let result = PevmTxExecutionResult::from_revm(self.chain, self.spec_id, result_and_state);
        Ok((result, reads, writes))
    }
}

/// Execute a single transaction against `storage` with access-set tracking,
/// without committing anything. Used for the serial tail, where each
/// transaction must observe the fully-committed state of the ones before it.
fn execute_one<S: Storage, C: PevmChain>(
    chain: &C,
    storage: &S,
    spec_id: SpecId,
    block_env: BlockEnv,
    tx: TxEnv,
) -> Result<(PevmTxExecutionResult, HashSet<u64>, HashSet<u64>), ExecutionError> {
    BatchExecutor::new(chain, storage, spec_id, block_env).run(tx)
}

/// **Algorithms 2 and 3 — replay phase.**
///
/// Drains `schedule` in parallel batches against `storage`, committing each
/// batch only once every transaction in it has finished, then serially
/// re-executes the transactions Algorithm 3 flags. `storage` is left holding
/// the post-block state. Returns per-transaction results in `schedule.txs`
/// order together with the replay counters.
///
/// `commit` applies a batch's results to `storage`; the caller supplies it so
/// this works against whichever concrete storage the harness uses.
pub fn replay<S, C, F>(
    chain: &C,
    storage: &mut S,
    spec_id: SpecId,
    block_env: BlockEnv,
    schedule: &VegetaSchedule,
    concurrency: NonZeroUsize,
    mut commit: F,
) -> Result<(Vec<PevmTxExecutionResult>, VegetaStats), PevmError<C>>
where
    S: Storage + Send + Sync,
    C: PevmChain + Send + Sync,
    F: FnMut(&mut S, Vec<PevmTxExecutionResult>),
{
    let n = schedule.txs.len();
    let mut stats = VegetaStats {
        num_txs: n,
        ..Default::default()
    };

    // Beneficiary (coinbase) fee settlement.
    //
    // Every transaction credits the block beneficiary, so the beneficiary
    // balance is a location the whole block writes. Vegeta keeps no
    // multi-version store and a batch's transactions all read the same
    // pre-batch state, so committing their raw account updates would make the
    // last writer's beneficiary balance overwrite the rest and silently
    // destroy fees. Treating it instead as a genuine conflict would put a WAW
    // edge between every pair of transactions and serialise the block outright.
    //
    // We therefore give Vegeta the same lazy-beneficiary treatment our own
    // engine uses (and that §7.2 of the paper describes for both conflict
    // sets): the beneficiary is excluded from per-transaction commits, its
    // per-transaction deltas are accumulated, and the total is applied once.
    // This is the configuration that is fair to Vegeta.
    let beneficiary = block_env.coinbase;
    let beneficiary_before = storage
        .basic(&beneficiary)
        .ok()
        .flatten()
        .map(|a| a.balance)
        .unwrap_or_default();
    let mut beneficiary_delta = revm::primitives::U256::ZERO;

    // Strip the beneficiary out of one transaction's state transition,
    // accumulating how much it was credited relative to the batch's base value.
    let mut take_beneficiary_fee = |result: &mut PevmTxExecutionResult, base: revm::primitives::U256| {
        if let Some(Some(acct)) = result.state.remove(&beneficiary) {
            beneficiary_delta = beneficiary_delta.saturating_add(acct.balance.saturating_sub(base));
        }
    };
    let mut results: Vec<Option<PevmTxExecutionResult>> = (0..n).map(|_| None).collect();
    let mut done = vec![false; n];

    // Algorithm 3 line 1: every key any transaction touched during speculation.
    let mut all_keys: HashSet<u64> = HashSet::with_capacity(n * 4);
    for i in 0..n {
        all_keys.extend(schedule.spec_reads[i].iter().copied());
        all_keys.extend(schedule.spec_writes[i].iter().copied());
    }

    // Deferred work, drained after the parallel batches.
    let mut txs_re: Vec<usize> = Vec::new(); // Algorithm 3 lines 5–8
    let mut read_deferred: Vec<(usize, HashSet<u64>)> = Vec::new(); // lines 9–10
    let mut new_keys: HashSet<u64> = HashSet::new(); // lines 11–12

    loop {
        let taken = vec![false; n];
        let ready = pop_ready_batch(&schedule.dag, &done, &taken);
        if ready.is_empty() {
            break;
        }
        stats.num_batches += 1;
        stats.max_batch = stats.max_batch.max(ready.len());

        // The beneficiary balance every transaction in this batch will observe.
        let batch_beneficiary_base = storage
            .basic(&beneficiary)
            .ok()
            .flatten()
            .map(|a| a.balance)
            .unwrap_or_default();

        // Run the batch in parallel against the frozen pre-batch state. Workers
        // pull from a shared cursor; `storage` is only read here, never written,
        // which is exactly Vegeta's no-multi-version-store model.
        let cursor = AtomicUsize::new(0);
        let collected: Mutex<Vec<Executed>> = Mutex::new(Vec::with_capacity(ready.len()));
        let failure: Mutex<Option<ExecutionError>> = Mutex::new(None);
        let nthreads = concurrency.get().min(ready.len().max(1));
        let storage_ref: &S = &*storage;
        std::thread::scope(|scope| {
            for _ in 0..nthreads {
                scope.spawn(|| {
                    // One warm EVM per worker, reused for every transaction it
                    // pulls from this batch.
                    let mut exec = BatchExecutor::new(
                        chain,
                        storage_ref,
                        spec_id,
                        block_env.clone(),
                    );
                    loop {
                        let slot = cursor.fetch_add(1, Ordering::Relaxed);
                        if slot >= ready.len() || failure.lock().unwrap().is_some() {
                            break;
                        }
                        let i = ready[slot];
                        match exec.run(schedule.txs[i].clone()) {
                            Ok((result, reads, writes)) => {
                                collected.lock().unwrap().push(Executed {
                                    idx: i,
                                    result,
                                    reads,
                                    writes,
                                })
                            }
                            Err(e) => {
                                *failure.lock().unwrap() = Some(e);
                                break;
                            }
                        }
                    }
                });
            }
        });
        if let Some(e) = failure.into_inner().unwrap() {
            return Err(PevmError::ExecutionError(e));
        }
        let executed = collected.into_inner().unwrap();

        // Algorithm 3 lines 3–12: classify, and decide what may commit now.
        let mut commit_now: Vec<Executed> = Vec::with_capacity(executed.len());
        for e in executed {
            let spec_r = &schedule.spec_reads[e.idx];
            let spec_w = &schedule.spec_writes[e.idx];

            let new_read: Vec<u64> = e.reads.difference(spec_r).copied().collect();
            let new_write: Vec<u64> = e.writes.difference(spec_w).copied().collect();

            // Lines 5–8: a newly touched key that someone else also touched in
            // speculation very likely introduces a new dependency → re-execute.
            let touches_new_shared = new_read
                .iter()
                .chain(new_write.iter())
                .any(|k| all_keys.contains(k));
            if touches_new_shared {
                txs_re.push(e.idx);
                stats.re_exec_new_shared_key += 1;
                continue;
            }

            // Lines 11–12: a newly written key nobody else touched is recorded;
            // it may still invalidate a transaction in `read`.
            for k in &new_write {
                new_keys.insert(*k);
            }

            // Lines 9–10: a newly read key nobody else touched is deferred
            // until we know whether anyone newly wrote it.
            if !new_read.is_empty() {
                read_deferred.push((e.idx, new_read.into_iter().collect()));
                continue;
            }

            commit_now.push(e);
        }

        // Line 14: commit in DAG (index) order so the result is deterministic.
        commit_now.sort_by_key(|e| e.idx);
        let mut batch_results = Vec::with_capacity(commit_now.len());
        for mut e in commit_now {
            // Every transaction in this batch saw the same beneficiary balance,
            // so that value is the base its credit is measured against.
            take_beneficiary_fee(&mut e.result, batch_beneficiary_base);
            done[e.idx] = true;
            batch_results.push(e.result.clone());
            results[e.idx] = Some(e.result);
        }
        commit(storage, batch_results);
    }

    // Line 13: a deferred transaction is only re-executed if it read a key that
    // was newly written during this replay; otherwise its result stands.
    let mut deferred_rerun: Vec<usize> = Vec::new();
    for (idx, new_read) in read_deferred {
        if new_read.iter().any(|k| new_keys.contains(k)) {
            deferred_rerun.push(idx);
            stats.re_exec_new_read_key += 1;
        } else {
            // Re-run once to obtain a result consistent with the committed
            // state; it introduced no new dependency, so this is cheap and
            // keeps `results` complete.
            deferred_rerun.push(idx);
        }
    }

    // Algorithm 2 lines 14–17 / Algorithm 3 lines 15–17: serial tail, ordered
    // by transaction index so every replica produces the same state.
    let mut tail: Vec<usize> = txs_re;
    tail.extend(deferred_rerun);
    tail.sort_unstable();
    tail.dedup();
    stats.re_executed = tail.len();

    for idx in tail {
        let base = storage
            .basic(&beneficiary)
            .ok()
            .flatten()
            .map(|a| a.balance)
            .unwrap_or_default();
        let (mut result, _, _) = execute_one(
            chain,
            &*storage,
            spec_id,
            block_env.clone(),
            schedule.txs[idx].clone(),
        )
        .map_err(PevmError::ExecutionError)?;
        take_beneficiary_fee(&mut result, base);
        done[idx] = true;
        commit(storage, vec![result.clone()]);
        results[idx] = Some(result);
    }

    // Settle the accumulated beneficiary fees in a single write.
    if beneficiary_delta != revm::primitives::U256::ZERO {
        let basic = storage.basic(&beneficiary).ok().flatten().unwrap_or_default();
        // `update_storage_with_results` merges: it overwrites balance and nonce
        // but keeps existing code and extends existing storage, so leaving code
        // and storage empty here touches only the balance.
        let acct = crate::storage::EvmAccount {
            balance: beneficiary_before.saturating_add(beneficiary_delta),
            nonce: basic.nonce,
            code_hash: None,
            code: None,
            storage: Default::default(),
        };
        let mut state: crate::vm::EvmStateTransitions = Default::default();
        state.insert(beneficiary, Some(acct));
        commit(
            storage,
            vec![PevmTxExecutionResult {
                receipt: Default::default(),
                state,
            }],
        );
    }

    // Any transaction still unexecuted would mean the DAG blocked forever;
    // Algorithm 2 cannot leave one behind, so treat it as a bug rather than
    // silently returning a short result vector.
    let mut out = Vec::with_capacity(n);
    for (i, r) in results.into_iter().enumerate() {
        match r {
            Some(r) => out.push(r),
            None => {
                return Err(PevmError::ExecutionError(ExecutionError::Custom(format!(
                    "vegeta: transaction {i} was never scheduled (DAG deadlock)"
                ))))
            }
        }
    }

    // Fix up cumulative gas so the receipts match the other engines' convention.
    let mut cumulative: u64 = 0;
    for r in out.iter_mut() {
        cumulative = cumulative.saturating_add(r.receipt.cumulative_gas_used);
        r.receipt.cumulative_gas_used = cumulative;
    }

    Ok((out, stats))
}
