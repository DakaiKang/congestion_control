//! Experiments added in response to the SIGMOD review round.
//!
//! Each test here targets a specific reviewer request:
//!
//! * **Absolute cost, not just speedup** (R2-O3, R4-O1) — every run reports
//!   wall-clock seconds and tx/s for every engine, plus a per-phase breakdown
//!   (pre-execute / graph build / integrate / execute).
//! * **Pre-execution and integration overhead** (R2-O1, R4-O1) — the
//!   preparatory phases our paper excluded from the timers are measured here
//!   explicitly and reported next to execution.
//! * **Bandwidth of the shipped metadata** (R4-O1) — bytes of conflict graph
//!   and access sets a proposal carries, for both Omakase and Vegeta.
//! * **The concatenated-block baseline** (R4-O2) — concatenate a round's
//!   blocks into one large block and run Block-STM on it.
//! * **A prior-system comparison** (R2-O2, R4-O3) — Vegeta [NSDI '25],
//!   reimplemented in `pevm::vegeta`.
//! * **Cascading aborts, measured** (R2-O3, R4-O4) — re-execution and abort
//!   counts per engine. These require `--features diagnostics`; without it the
//!   abort columns are reported as 0 and only `re_executions` is meaningful.
//!
//! Correctness guard: on the artificial workload every transaction's effect is
//! `state[k] += 1`, which commutes, so *all* engines — including the ones that
//! reorder — must end at byte-identical state. The tests assert that, which is
//! what makes the Vegeta and concatenated-block numbers trustworthy.

#[path = "tx_simulator/mod.rs"]
pub mod tx_simulator;

use pevm::{
    api::update_storage_with_results,
    chain::PevmEthereum,
    execute_revm_sequential, execute_revm_sequential_with_access_sets,
    graph_pevm::GraphPevm,
    greedy_integrator::{GreedyIntegrator, GreedyIntegratorConfig},
    vegeta, ExecDiagnostics, InMemoryStorage, Pevm,
};
use revm::primitives::{BlockEnv, SpecId, TxEnv};
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn env_usize(k: &str, d: usize) -> usize {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn env_f64(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

/// Exact digest of the post-execution state: balances, nonces and storage.
/// Two engines that agree on this produced byte-identical world state.
fn state_digest(storage: &InMemoryStorage) -> u64 {
    fold_state(storage, true)
}

/// Digest of only the *order-independent* part of the state: storage slots and
/// nonces, with balances excluded.
///
/// Balances cannot be compared across engines that reorder. Every transaction
/// here does `state[k] += 1`, so the first writer of a slot pays SSTORE_SET
/// (20 000 gas) and later writers pay SSTORE_RESET (2 900). Reordering moves
/// which transaction pays the expensive write, so per-caller balances differ
/// by construction even though the aggregate gas and the final slot values do
/// not. Slot values and nonces are the invariants a reordering engine must
/// still preserve exactly.
fn logical_digest(storage: &InMemoryStorage) -> u64 {
    fold_state(storage, false)
}

fn fold_state(storage: &InMemoryStorage, include_balance: bool) -> u64 {
    let mut acc: u64 = 0;
    for (addr, account) in storage.accounts.iter() {
        // Fold each account independently and XOR, so the map's iteration
        // order cannot change the result.
        let mut h: u64 = 1469598103934665603;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(1099511628211);
            }
        };
        feed(addr.as_slice());
        if include_balance {
            feed(&account.balance.to_be_bytes::<32>());
        }
        feed(&account.nonce.to_be_bytes());
        // Storage slots also folded order-independently.
        let mut slot_acc: u64 = 0;
        for (slot, value) in account.storage.iter() {
            let mut sh: u64 = 14695981039346656037;
            for b in slot.to_be_bytes::<32>().iter().chain(value.to_be_bytes::<32>().iter()) {
                sh ^= *b as u64;
                sh = sh.wrapping_mul(1099511628211);
            }
            slot_acc ^= sh;
        }
        feed(&slot_acc.to_be_bytes());
        acc ^= h;
    }
    acc
}

/// Total balance across all accounts — conserved regardless of ordering, so a
/// reordering engine that loses or invents ether is caught here even though
/// per-account balances legitimately move.
fn total_balance(storage: &InMemoryStorage) -> revm::primitives::U256 {
    storage
        .accounts
        .values()
        .fold(revm::primitives::U256::ZERO, |a, acc| a.saturating_add(acc.balance))
}

/// Per-phase wall-clock cost of the preparatory pipeline the paper's timers
/// excluded, plus the execution it feeds.
#[derive(Debug, Clone, Copy, Default)]
struct Phases {
    /// Sequential pre-execution that yields each transaction's access sets.
    /// Run once per block by the proposing replica.
    pre_execute: Duration,
    /// Building the per-block conflict graph and reordering within the block.
    graph_build: Duration,
    /// Greedy inter-block integration (includes the CV simulations).
    integrate: Duration,
    /// The parallel execution itself — the only phase the paper timed.
    execute: Duration,
}

impl Phases {
    fn total(&self) -> Duration {
        self.pre_execute + self.graph_build + self.integrate + self.execute
    }
}

/// Everything one batch produces, for one CSV row.
#[derive(Debug, Clone, Default)]
struct Row {
    batch_idx: usize,
    num_blocks: usize,
    num_txs: usize,

    threads: usize,
    merge_cap: usize,

    seq_s: f64,
    par_s: f64,
    concat_s: f64,
    /// Graph-aware OCC over the whole concatenated round: one group, no
    /// hot-key filtering — the "concat + graph" point of the design space.
    cgraph_s: f64,
    graph_s: f64,
    integ_s: f64,
    vegeta_s: f64,

    phases: Phases,
    vegeta_spec_s: f64,

    num_integrated_groups: usize,

    // Bandwidth: bytes of metadata a proposal ships.
    omakase_graph_bytes: usize,
    vegeta_sched_bytes: usize,
    calldata_bytes: usize,

    // Abort accounting.
    par_diag: ExecDiagnostics,
    concat_diag: ExecDiagnostics,
    cgraph_diag: ExecDiagnostics,
    graph_diag: ExecDiagnostics,
    integ_diag: ExecDiagnostics,
    vegeta_diag: ExecDiagnostics,

    // Correctness.
    digest_seq: u64,
    digest_par: u64,
    digest_concat: u64,
    // Reordering engines are compared on the order-independent digest.
    logical_seq: u64,
    logical_integ: u64,
    logical_vegeta: u64,
}

const CSV_HEADER: &str = "batch_idx,num_blocks,num_txs,threads,merge_cap,\
seq_time_s,seq_tput,par_time_s,par_tput,concat_time_s,concat_tput,cgraph_time_s,cgraph_tput,\
graph_time_s,graph_tput,integrated_time_s,integrated_tput,vegeta_time_s,vegeta_tput,\
phase_pre_execute_s,phase_graph_build_s,phase_integrate_s,phase_execute_s,phase_total_s,\
vegeta_speculate_s,num_integrated_groups,\
omakase_graph_bytes,vegeta_sched_bytes,calldata_bytes,\
par_re_exec,par_validation_aborts,par_cascade_aborts,par_wrote_new_loc,\
concat_re_exec,concat_validation_aborts,concat_cascade_aborts,concat_wrote_new_loc,\
cgraph_re_exec,cgraph_validation_aborts,cgraph_cascade_aborts,cgraph_wrote_new_loc,\
graph_re_exec,graph_validation_aborts,graph_cascade_aborts,graph_wrote_new_loc,\
integ_re_exec,integ_validation_aborts,integ_cascade_aborts,integ_wrote_new_loc,\
vegeta_re_exec,vegeta_validation_aborts,vegeta_cascade_aborts,vegeta_wrote_new_loc,\
digest_seq,digest_par,digest_concat,logical_seq,logical_integ,logical_vegeta";

fn write_row(w: &mut impl std::io::Write, r: &Row) {
    let tp = |s: f64| if s > 0.0 { r.num_txs as f64 / s } else { 0.0 };
    writeln!(
        w,
        "{},{},{},{},{},\
{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},\
{:.6},{:.2},{:.6},{:.2},{:.6},{:.2},\
{:.6},{:.6},{:.6},{:.6},{:.6},\
{:.6},{},\
{},{},{},\
{},{},{},{},\
{},{},{},{},\
{},{},{},{},\
{},{},{},{},\
{},{},{},{},\
{},{},{},{},\
{},{},{},{},{},{}",
        r.batch_idx, r.num_blocks, r.num_txs, r.threads, r.merge_cap,
        r.seq_s, tp(r.seq_s), r.par_s, tp(r.par_s), r.concat_s, tp(r.concat_s), r.cgraph_s, tp(r.cgraph_s),
        r.graph_s, tp(r.graph_s), r.integ_s, tp(r.integ_s), r.vegeta_s, tp(r.vegeta_s),
        r.phases.pre_execute.as_secs_f64(), r.phases.graph_build.as_secs_f64(),
        r.phases.integrate.as_secs_f64(), r.phases.execute.as_secs_f64(),
        r.phases.total().as_secs_f64(),
        r.vegeta_spec_s, r.num_integrated_groups,
        r.omakase_graph_bytes, r.vegeta_sched_bytes, r.calldata_bytes,
        r.par_diag.re_executions, r.par_diag.validation_aborts,
        r.par_diag.cascade_aborts, r.par_diag.wrote_new_location,
        r.concat_diag.re_executions, r.concat_diag.validation_aborts,
        r.concat_diag.cascade_aborts, r.concat_diag.wrote_new_location,
        r.cgraph_diag.re_executions, r.cgraph_diag.validation_aborts,
        r.cgraph_diag.cascade_aborts, r.cgraph_diag.wrote_new_location,
        r.graph_diag.re_executions, r.graph_diag.validation_aborts,
        r.graph_diag.cascade_aborts, r.graph_diag.wrote_new_location,
        r.integ_diag.re_executions, r.integ_diag.validation_aborts,
        r.integ_diag.cascade_aborts, r.integ_diag.wrote_new_location,
        r.vegeta_diag.re_executions, r.vegeta_diag.validation_aborts,
        r.vegeta_diag.cascade_aborts, r.vegeta_diag.wrote_new_location,
        r.digest_seq, r.digest_par, r.digest_concat, r.logical_seq, r.logical_integ, r.logical_vegeta,
    )
    .unwrap();
}

/// Bytes of transaction calldata in a batch — the denominator for the
/// bandwidth-overhead figure (metadata as a share of what the block already
/// carries).
fn calldata_bytes(blocks: &[Vec<TxEnv>]) -> usize {
    blocks
        .iter()
        .flat_map(|b| b.iter())
        .map(|tx| tx.data.len() + 32 * 3)
        .sum()
}

/// One conflict graph over a whole round, in proposal order, built from the
/// per-block graphs the pre-pass produced. Feeding it to `GraphPevm` with the
/// concatenated transaction list is "concat + graph": the largest possible
/// group, no hot-key veto, no reordering — i.e. Omakase with an unbounded merge
/// cap and its filters switched off. Comparing it with plain concatenated
/// Block-STM isolates what the graph is worth at full window size.
fn round_graph(
    base_graphs: &[pevm::dependency_graph::TransactionGraph],
    hot_key_threshold: f64,
) -> pevm::dependency_graph::TransactionGraph {
    let mut g = pevm::dependency_graph::TransactionGraph::new();
    g.set_hot_key_threshold(hot_key_threshold);
    for bg in base_graphs {
        for node in &bg.nodes {
            let mut n = node.clone();
            n.children_indices.clear();
            n.parent_indices.clear();
            n.longest_suffix = n.gas_cost;
            // TransactionId = (index within block, block index) is unique
            // across the round, so nodes from different blocks never collide.
            g.add_transaction(n).expect("unique (id, replica) per round");
        }
    }
    g
}

/// Shared knobs for the two synthetic workloads.
#[derive(Debug, Clone)]
struct SynthCfg {
    concurrency: NonZeroUsize,
    tau_cv: f64,
    hot_key_threshold: f64,
    greedy_batch: usize,
    /// `GreedyIntegratorConfig::max_group_blocks` (paper default 10).
    merge_cap: usize,
    /// Assert exact slot/nonce equivalence across every engine. Valid for
    /// both synthetic workloads because every write is `state[k] += 1`.
    check_state: bool,
}

impl SynthCfg {
    fn from_env() -> Self {
        Self {
            concurrency: NonZeroUsize::new(env_usize("NUM_THREADS", 8)).expect("NUM_THREADS>0"),
            tau_cv: env_f64("TAU_CV", 0.5),
            hot_key_threshold: env_f64("HOT_KEY_THRESHOLD", 1.5),
            greedy_batch: env_usize("GREEDY_BATCH", 50),
            merge_cap: env_usize("MERGE_CAP", 10),
            check_state: env_usize("CHECK_STATE", 1) == 1,
        }
    }
}

/// Run every engine on one synthetic batch (TxSimulator contract, in-memory
/// state) and return the CSV row. Used by both the artificial workload and the
/// V2 synthetic workload; only how the blocks are built differs.
fn run_synthetic_batch(
    batch_idx: usize,
    base: &InMemoryStorage,
    blocks_txs: &[Vec<TxEnv>],
    cfg: &SynthCfg,
) -> Row {
    let chain = PevmEthereum::mainnet();
    let spec_id = SpecId::LATEST;
    let concurrency = cfg.concurrency;
    let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

    let mut row = Row {
        batch_idx,
        num_blocks: blocks_txs.len(),
        num_txs: total_txs,
        threads: concurrency.get(),
        merge_cap: cfg.merge_cap,
        calldata_bytes: calldata_bytes(blocks_txs),
        ..Default::default()
    };

    // ── Phase 1: sequential pre-execution — the proposer-side pre-pass that
    //    yields each transaction's access sets. Omakase and Vegeta both
    //    consume its output (via the per-block graph built in phase 2), so
    //    neither gets better hints, and this is the "pre-execution
    //    overhead" R2-O1 asks us to quantify.
    let mut prep = base.clone();
    let t = Instant::now();
    for txs in blocks_txs.iter() {
        let (results, _sets) = execute_revm_sequential_with_access_sets(
            &chain, &prep, spec_id, BlockEnv::default(), txs.clone(),
        )
        .expect("pre-execution failed");
        update_storage_with_results(&mut prep, results);
    }
    row.phases.pre_execute = t.elapsed();

    // ── Phase 2: per-block conflict graph + intra-block reorder (Omakase).
    let mut prep = base.clone();
    let mut dep_graphs = Vec::with_capacity(blocks_txs.len());
    let mut reordered = Vec::with_capacity(blocks_txs.len());
    // Proposal-order graphs, before Omakase's intra-block reorder: the
    // shared input Vegeta's schedule is derived from.
    let mut base_graphs = Vec::with_capacity(blocks_txs.len());
    let t = Instant::now();
    for (i, txs) in blocks_txs.iter().enumerate() {
        let (mut graph, results) = GraphPevm::construct_graph_pevm_by_sequential(
            &chain, &prep, spec_id, BlockEnv::default(), txs.clone(), i as u64,
        )
        .expect("graph build failed");
        update_storage_with_results(&mut prep, results);
        base_graphs.push(graph.clone());
        let (rtxs, new_graph) =
            GraphPevm::reorder_txs_by_dependency_graph(txs.clone(), &mut graph, concurrency.get());
        dep_graphs.push(new_graph);
        reordered.push(rtxs);
    }
    // `construct_graph_pevm_by_sequential` re-runs the sequential pre-pass
    // internally, so subtract it to leave only graph construction itself.
    row.phases.graph_build = t.elapsed().saturating_sub(row.phases.pre_execute);
    for g in &mut dep_graphs {
        g.set_hot_key_threshold(cfg.hot_key_threshold);
    }
    row.omakase_graph_bytes = dep_graphs.iter().map(vegeta::graph_proposal_bytes).sum();

    // ── Phase 3: greedy inter-block integration (Omakase only).
    let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
        num_threads: concurrency.get(),
        tau_cv: cfg.tau_cv,
        max_group_blocks: cfg.merge_cap,
    });
    let mut integrated_txns: Vec<Vec<TxEnv>> = Vec::new();
    let mut integrated_graphs = Vec::new();
    let t = Instant::now();
    for chunk_start in (0..blocks_txs.len()).step_by(cfg.greedy_batch) {
        let chunk_end = (chunk_start + cfg.greedy_batch).min(blocks_txs.len());
        let (itxns, igraphs) = integrator.integrate_pevm_graphs(
            dep_graphs[chunk_start..chunk_end].to_vec(),
            reordered[chunk_start..chunk_end].to_vec(),
        );
        integrated_txns.extend(itxns);
        integrated_graphs.extend(igraphs);
    }
    row.phases.integrate = t.elapsed();
    row.num_integrated_groups = integrated_txns.len();
    // Reordering across blocks breaks nonce chains; reassign (same as the
    // existing harnesses). Each reordering engine gets its own tracker.
    let mut nt = pevm::utils::nonce_tracker::NonceTracker::new();
    for txs in &mut integrated_txns {
        nt.update_txenv_nonces(txs);
    }

    // ── Vegeta's schedule (Rule 1 order, WAW-only DAG) from the same
    //    per-block graphs, per block. See `pevm::vegeta`.
    let t = Instant::now();
    let mut vegeta_blocks: Vec<(Vec<TxEnv>, pevm::dependency_graph::TransactionGraph)> = blocks_txs
        .iter()
        .zip(base_graphs.iter())
        .map(|(txs, g)| vegeta::speculate_graph(txs.clone(), g))
        .collect();
    let mut vnt = pevm::utils::nonce_tracker::NonceTracker::new();
    for (txs, _) in vegeta_blocks.iter_mut() {
        vnt.update_txenv_nonces(txs);
    }
    row.vegeta_spec_s = t.elapsed().as_secs_f64();
    row.vegeta_sched_bytes = vegeta_blocks.iter().map(|(_, g)| vegeta::graph_proposal_bytes(g)).sum();

    // ── 1. Sequential (baseline).
    let mut s = base.clone();
    let t = Instant::now();
    for txs in blocks_txs {
        let r = execute_revm_sequential(&chain, &s, spec_id, BlockEnv::default(), txs.clone())
            .expect("sequential failed");
        update_storage_with_results(&mut s, r);
    }
    row.seq_s = t.elapsed().as_secs_f64();
    row.digest_seq = state_digest(&s);
    row.logical_seq = logical_digest(&s);
    let seq_total_balance = total_balance(&s);

    // ── 2. Block-STM, per block.
    let mut s = base.clone();
    let mut pevm_engine = Pevm::default();
    let t = Instant::now();
    for txs in blocks_txs {
        let r = pevm_engine
            .execute_revm_parallel(&chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency)
            .expect("block-stm failed");
        row.par_diag.add(&pevm_engine.last_diagnostics);
        update_storage_with_results(&mut s, r);
    }
    row.par_s = t.elapsed().as_secs_f64();
    row.digest_par = state_digest(&s);

    // ── 3. Concatenated-block Block-STM (R4-O2): one round's blocks fused
    //      into a single large block, executed by unmodified Block-STM.
    let concat: Vec<TxEnv> = blocks_txs.iter().flat_map(|b| b.iter().cloned()).collect();
    let mut s = base.clone();
    let mut pevm_engine = Pevm::default();
    let t = Instant::now();
    let r = pevm_engine
        .execute_revm_parallel(&chain, &s, spec_id, BlockEnv::default(), concat.clone(), concurrency)
        .expect("concat block-stm failed");
    row.concat_diag.add(&pevm_engine.last_diagnostics);
    update_storage_with_results(&mut s, r);
    row.concat_s = t.elapsed().as_secs_f64();
    row.digest_concat = state_digest(&s);

    // ── 3b. Concat + graph: graph-aware OCC over the whole round as one group.
    let rg = round_graph(&base_graphs, cfg.hot_key_threshold);
    let mut s = base.clone();
    let mut engine = GraphPevm::default();
    let t = Instant::now();
    let r = engine
        .execute_revm_parallel(&chain, &s, spec_id, BlockEnv::default(), concat, concurrency, rg)
        .expect("concat+graph failed");
    row.cgraph_diag.add(&engine.last_diagnostics);
    update_storage_with_results(&mut s, r);
    row.cgraph_s = t.elapsed().as_secs_f64();
    let digest_cgraph = state_digest(&s);

    // ── 4. Graph-aware OCC, per block.
    let mut s = base.clone();
    let t = Instant::now();
    for (txs, graph) in reordered.iter().zip(dep_graphs.iter()) {
        let mut engine = GraphPevm::default();
        let r = engine
            .execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
            )
            .expect("graph occ failed");
        row.graph_diag.add(&engine.last_diagnostics);
        update_storage_with_results(&mut s, r);
    }
    row.graph_s = t.elapsed().as_secs_f64();

    // ── 5. Omakase (integrated groups).
    let mut s = base.clone();
    let t = Instant::now();
    for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
        let mut engine = GraphPevm::default();
        let r = engine
            .execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
            )
            .expect("omakase failed");
        row.integ_diag.add(&engine.last_diagnostics);
        update_storage_with_results(&mut s, r);
    }
    row.integ_s = t.elapsed().as_secs_f64();
    row.phases.execute = Duration::from_secs_f64(row.integ_s);
    row.logical_integ = logical_digest(&s);
    let integ_total_balance = total_balance(&s);

    // ── 6. Vegeta: its schedule on the graph-aware OCC engine, per block.
    let mut s = base.clone();
    let t = Instant::now();
    for (txs, graph) in vegeta_blocks.iter() {
        let mut engine = GraphPevm::default();
        let r = engine
            .execute_revm_parallel(
                &chain, &s, spec_id, BlockEnv::default(), txs.clone(), concurrency, graph.clone(),
            )
            .expect("vegeta failed");
        row.vegeta_diag.add(&engine.last_diagnostics);
        update_storage_with_results(&mut s, r);
    }
    row.vegeta_s = t.elapsed().as_secs_f64();
    row.logical_vegeta = logical_digest(&s);
    let vegeta_total_balance = total_balance(&s);

    // Correctness.
    //
    // Block-STM and the concatenated-block baseline preserve the original
    // transaction order, so they must reproduce the sequential state
    // exactly, balances included.
    //
    // Omakase and Vegeta both reorder. Every effect here is a commuting
    // `state[k] += 1`, so slot values and nonces must still match exactly;
    // balances legitimately differ because reordering moves which
    // transaction pays SSTORE_SET rather than SSTORE_RESET. Total ether is
    // still conserved, which we check separately.
    if cfg.check_state {
        assert_eq!(row.digest_par, row.digest_seq,
            "batch {batch_idx}: Block-STM state != sequential");
        assert_eq!(row.digest_concat, row.digest_seq,
            "batch {batch_idx}: concatenated-block Block-STM state != sequential");
        assert_eq!(digest_cgraph, row.digest_seq,
            "batch {batch_idx}: concat+graph state != sequential");
        assert_eq!(row.logical_integ, row.logical_seq,
            "batch {batch_idx}: Omakase slot/nonce state != sequential");
        assert_eq!(row.logical_vegeta, row.logical_seq,
            "batch {batch_idx}: Vegeta slot/nonce state != sequential");
        assert_eq!(integ_total_balance, seq_total_balance,
            "batch {batch_idx}: Omakase did not conserve total balance");
        assert_eq!(vegeta_total_balance, seq_total_balance,
            "batch {batch_idx}: Vegeta did not conserve total balance");
    }
    row
}

fn print_row(prefix: &str, row: &Row, cum_s: f64) {
    let tp = |s: f64| if s > 0.0 { row.num_txs as f64 / s } else { 0.0 };
    println!(
        "  {prefix} txs={} | seq={:.0} par={:.0} concat={:.0} concat+g={:.0} graph={:.0} omakase={:.0} vegeta={:.0} tx/s \
         | prep: pre={:.3}s graph={:.3}s integ={:.3}s | groups={} | cum {:.1}s",
        row.num_txs,
        tp(row.seq_s), tp(row.par_s), tp(row.concat_s), tp(row.cgraph_s), tp(row.graph_s), tp(row.integ_s), tp(row.vegeta_s),
        row.phases.pre_execute.as_secs_f64(), row.phases.graph_build.as_secs_f64(),
        row.phases.integrate.as_secs_f64(), row.num_integrated_groups, cum_s,
    );
}

fn open_csv(output: &str) -> std::io::BufWriter<std::fs::File> {
    use std::io::Write;
    if let Some(dir) = std::path::Path::new(output).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(output).expect("create output"));
    writeln!(w, "{CSV_HEADER}").unwrap();
    w.flush().unwrap();
    w
}

/// The artificial workload: conflict structure is dialed by `K` (inter-block)
/// and `M` (intra-block), no real-block dataset required.
#[test]
fn test_rebuttal_artificial() {
    use std::io::Write;

    let num_batches = env_usize("NUM_BATCHES", 20);
    let blocks_per_batch = env_usize("BLOCKS_PER_BATCH", 50);
    let txns_per_block = env_usize("TXNS_PER_BLOCK", 100);
    let k = env_usize("K", 4);
    let m_pct = env_u64("M", 40);
    let target = env_u64("TARGET", 100);
    let writes_per_tx = env_usize("WRITES_PER_TX", 3);
    let hot_set_size = env_usize("HOT_SET_SIZE", 5);
    let cfg = SynthCfg::from_env();
    let output = std::env::var("OUTPUT")
        .unwrap_or_else(|_| "experiments/rebuttal/artificial.csv".to_string());

    println!(
        "rebuttal/artificial: batches={num_batches} blocks={blocks_per_batch} \
         txs/block={txns_per_block} K={k} M={m_pct}% hot_set={hot_set_size} {cfg:?}"
    );
    println!("CSV -> {output}");
    let mut w = open_csv(&output);
    let overall = Instant::now();

    for batch_idx in 0..num_batches {
        let (state, bytecodes, _sim, blocks_txs) = tx_simulator::build_artificial_blocks(
            batch_idx as u64, blocks_per_batch, txns_per_block, k, m_pct, target,
            writes_per_tx, hot_set_size,
        );
        let base = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
        let row = run_synthetic_batch(batch_idx, &base, &blocks_txs, &cfg);
        write_row(&mut w, &row);
        w.flush().unwrap();
        print_row(&format!("batch {}/{}", batch_idx + 1, num_batches), &row, overall.elapsed().as_secs_f64());
    }
    println!("\n✓ done in {:.1}s | CSV -> {output}", overall.elapsed().as_secs_f64());
}

/// The paper's synthetic workload (§7.2.2): real blocks' access sets and
/// calibrated execution times replayed through the TxSimulator contract, loaded
/// from the `rw_time_<N>.json` files that `test_generate_rw_time` produces.
#[test]
fn test_rebuttal_v2() {
    use std::io::Write;

    let rw_time_dir = std::env::var("RW_TIME_DIR").unwrap_or_else(|_| {
        "/home/ubuntu/Omakase/eth-block-downloader/test_data/rw_time".to_string()
    });
    let batch_size = env_usize("BATCH_SIZE", 100);
    let max_batches = env_usize("MAX_BATCHES", 150);
    let cfg = SynthCfg::from_env();
    let output = std::env::var("OUTPUT")
        .unwrap_or_else(|_| "experiments/rebuttal/v2.csv".to_string());

    let mut files: Vec<_> = std::fs::read_dir(&rw_time_dir)
        .expect("rw_time_dir not readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no rw_time_*.json in {rw_time_dir}");
    let total_batches = ((files.len() + batch_size - 1) / batch_size).min(max_batches);

    println!("rebuttal/v2: dir={rw_time_dir} files={} batch={batch_size} batches={total_batches} {cfg:?}", files.len());
    println!("CSV -> {output}");
    let mut w = open_csv(&output);
    let overall = Instant::now();

    for (batch_idx, chunk) in files.chunks(batch_size).enumerate() {
        if batch_idx >= max_batches {
            break;
        }
        // The loader picks its slice from START_INDEX (same convention as
        // `test_v2_all_batches`).
        std::env::set_var("START_INDEX", (batch_idx * batch_size).to_string());
        let (state, bytecodes, _addr, blocks_txs) =
            match tx_simulator::load_n_rw_time_blocks_with_callers(&rw_time_dir, chunk.len()) {
                Ok(t) => t,
                Err(e) => {
                    println!("  batch {batch_idx}: load failed ({e}), skipping");
                    continue;
                }
            };
        let base = InMemoryStorage::new(state, Arc::new(bytecodes), Default::default());
        let row = run_synthetic_batch(batch_idx, &base, &blocks_txs, &cfg);
        write_row(&mut w, &row);
        w.flush().unwrap();
        print_row(&format!("batch {}/{}", batch_idx + 1, total_batches), &row, overall.elapsed().as_secs_f64());
    }
    println!("\n✓ done in {:.1}s | CSV -> {output}", overall.elapsed().as_secs_f64());
}

// ─────────────────────────────────────────────────────────────────────────────
// Real Ethereum mainnet blocks
// ─────────────────────────────────────────────────────────────────────────────

use pevm::storage::block_loader::{create_multi_block_storage, get_spec_id, load_block_for_execution};
use pevm::utils::nonce_tracker::NonceTracker;

/// Real mainnet block gas limit: 30M pre-Pectra, 36M after.
const REAL_BLOCK_GAS_LIMIT: u64 = 36_000_000;

fn real_block_env() -> BlockEnv {
    let mut e = BlockEnv::default();
    e.gas_limit = revm::primitives::U256::from(REAL_BLOCK_GAS_LIMIT);
    e
}

/// The concatenated-block baseline needs a gas budget covering every block it
/// fused, otherwise revm rejects the tail of the batch.
fn concat_block_env(num_blocks: usize) -> BlockEnv {
    let mut e = BlockEnv::default();
    e.gas_limit =
        revm::primitives::U256::from(REAL_BLOCK_GAS_LIMIT.saturating_mul(num_blocks as u64));
    e
}

/// Same measurements as `test_rebuttal_artificial`, on real mainnet blocks.
///
/// State digests are recorded but *not* asserted across reordering engines:
/// real EVM execution is state-dependent, so a reordered replay legitimately
/// diverges from history (this is exactly why the paper's synthetic workload
/// exists). Block-STM and the concatenated baseline preserve order, so those
/// two are still checked against sequential.
#[test]
fn test_rebuttal_real() {
    use std::io::Write;

    let blocks_dir = std::env::var("BLOCKS_DIR").unwrap_or_else(|_| {
        "/home/ubuntu/Omakase/eth-block-downloader/test_data/blocks_rw".to_string()
    });
    let start_block = env_u64("START_BLOCK", 16_774_645);
    let batch_size = env_usize("BATCH_SIZE", 100);
    let max_batches = env_usize("MAX_BATCHES", 10);
    let concurrency = NonZeroUsize::new(env_usize("NUM_THREADS", 8)).expect("NUM_THREADS>0");
    let tau_cv = env_f64("TAU_CV", 0.5);
    let hot_key_threshold = env_f64("HOT_KEY_THRESHOLD", 1.5);
    let merge_cap = env_usize("MERGE_CAP", 10);
    let run_vegeta = env_usize("RUN_VEGETA", 1) == 1;
    let output = std::env::var("OUTPUT")
        .unwrap_or_else(|_| "experiments/rebuttal/real.csv".to_string());

    if let Some(dir) = std::path::Path::new(&output).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    println!(
        "rebuttal/real: dir={blocks_dir} start={start_block} batch={batch_size} \
         max_batches={max_batches} threads={} tau_cv={tau_cv} hot_kt={hot_key_threshold} vegeta={run_vegeta}",
        concurrency.get()
    );
    println!("CSV -> {output}");

    let mut w = std::io::BufWriter::new(std::fs::File::create(&output).expect("create output"));
    writeln!(w, "{CSV_HEADER}").unwrap();
    w.flush().unwrap();

    let chain = PevmEthereum::mainnet();
    let overall = Instant::now();
    let mut done_batches = 0usize;

    for batch_idx in 0..max_batches {
        let first = start_block + (batch_idx * batch_size) as u64;
        let block_numbers: Vec<u64> = (first..first + batch_size as u64).collect();

        // Skip a batch whose files are not all present rather than aborting the
        // sweep — chunk boundaries and the handful of blocks that fail
        // single-block replay would otherwise stop the run.
        if block_numbers
            .iter()
            .any(|n| !std::path::Path::new(&format!("{blocks_dir}/block_{n}.json")).exists())
        {
            println!("  batch {batch_idx}: incomplete block files, skipping");
            continue;
        }

        let spec_id = get_spec_id(block_numbers[0]);
        let Ok(storage) = create_multi_block_storage(&block_numbers, &blocks_dir) else {
            println!("  batch {batch_idx}: storage build failed, skipping");
            continue;
        };

        let mut blocks_txs: Vec<Vec<TxEnv>> = Vec::new();
        let mut nonce_tracker = NonceTracker::new();
        let mut load_ok = true;
        for &bn in &block_numbers {
            match load_block_for_execution(&format!("{blocks_dir}/block_{bn}.json"), true) {
                Ok((_, block_storage, txenvs)) => {
                    nonce_tracker.record_from_prestate(bn, &block_storage, &txenvs);
                    blocks_txs.push(txenvs);
                }
                Err(e) => {
                    println!("  batch {batch_idx}: load failed for {bn}: {e:?}, skipping");
                    load_ok = false;
                    break;
                }
            }
        }
        if !load_ok || blocks_txs.is_empty() {
            continue;
        }
        // `update_txenv_nonces` mutates the tracker, so each engine that
        // reorders needs its own copy of the freshly-recorded prestate nonces.
        // Sharing one tracker across Omakase and Vegeta made the second caller
        // assign nonces on top of the first's, producing "nonce too high".
        let nonce_snapshot = nonce_tracker.nonces.clone();
        let total_txs: usize = blocks_txs.iter().map(|b| b.len()).sum();

        let mut row = Row {
            batch_idx,
            num_blocks: blocks_txs.len(),
            num_txs: total_txs,
            threads: concurrency.get(),
            merge_cap,
            calldata_bytes: calldata_bytes(&blocks_txs),
            ..Default::default()
        };

        // ── Sequential baseline. A batch whose sequential replay errors is
        //    unusable, so bail out of it rather than reporting a bad speedup.
        let mut s = storage.clone();
        let t = Instant::now();
        let mut seq_ok = true;
        for txs in blocks_txs.clone() {
            match execute_revm_sequential(&chain, &s, spec_id, real_block_env(), txs) {
                Ok(r) => update_storage_with_results(&mut s, r),
                Err(e) => {
                    println!("  batch {batch_idx}: sequential failed ({e:?}), skipping");
                    seq_ok = false;
                    break;
                }
            }
        }
        if !seq_ok {
            continue;
        }
        row.seq_s = t.elapsed().as_secs_f64();
        row.digest_seq = state_digest(&s);
        row.logical_seq = logical_digest(&s);

        // ── Phase 1: sequential pre-execution (proposer-side pre-pass).
        let mut prep = storage.clone();
        let t = Instant::now();
        for txs in blocks_txs.iter() {
            if let Ok((results, _sets)) = execute_revm_sequential_with_access_sets(
                &chain, &prep, spec_id, real_block_env(), txs.clone(),
            ) {
                update_storage_with_results(&mut prep, results);
            }
        }
        row.phases.pre_execute = t.elapsed();

        // ── Phase 2: per-block conflict graph + intra-block reorder.
        let mut prep = storage.clone();
        let mut dep_graphs = Vec::new();
        let mut reordered = Vec::new();
        let mut base_graphs = Vec::new();
        let t = Instant::now();
        let mut graph_ok = true;
        for (i, txs) in blocks_txs.iter().enumerate() {
            match GraphPevm::construct_graph_pevm_by_sequential(
                &chain, &prep, spec_id, real_block_env(), txs.clone(), i as u64,
            ) {
                Ok((mut graph, results)) => {
                    update_storage_with_results(&mut prep, results);
                    base_graphs.push(graph.clone());
                    let (rtxs, g) = GraphPevm::reorder_txs_by_dependency_graph(
                        txs.clone(), &mut graph, concurrency.get(),
                    );
                    dep_graphs.push(g);
                    reordered.push(rtxs);
                }
                Err(e) => {
                    println!("  batch {batch_idx}: graph build failed ({e:?}), skipping");
                    graph_ok = false;
                    break;
                }
            }
        }
        if !graph_ok {
            continue;
        }
        row.phases.graph_build = t.elapsed().saturating_sub(row.phases.pre_execute);
        for g in &mut dep_graphs {
            g.set_hot_key_threshold(hot_key_threshold);
        }
        row.omakase_graph_bytes = dep_graphs.iter().map(vegeta::graph_proposal_bytes).sum();

        // ── Phase 3: greedy inter-block integration.
        let integrator = GreedyIntegrator::new(GreedyIntegratorConfig {
            num_threads: concurrency.get(),
            tau_cv,
            max_group_blocks: merge_cap,
        });
        let t = Instant::now();
        let (mut integrated_txns, integrated_graphs) =
            integrator.integrate_pevm_graphs(dep_graphs.clone(), reordered.clone());
        row.phases.integrate = t.elapsed();
        row.num_integrated_groups = integrated_graphs.len();
        for txs in integrated_txns.iter_mut() {
            nonce_tracker.update_txenv_nonces(txs);
        }

        // ── 2. Block-STM, per block.
        let mut s = storage.clone();
        let mut engine = Pevm::default();
        let t = Instant::now();
        let mut ok = true;
        for txs in blocks_txs.clone() {
            match engine.execute_revm_parallel(
                &chain, &s, spec_id, real_block_env(), txs, concurrency,
            ) {
                Ok(r) => {
                    row.par_diag.add(&engine.last_diagnostics);
                    update_storage_with_results(&mut s, r);
                }
                Err(e) => {
                    println!("  batch {batch_idx}: block-stm failed ({e:?}), skipping");
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        row.par_s = t.elapsed().as_secs_f64();
        row.digest_par = state_digest(&s);

        // ── 3. Concatenated-block Block-STM (R4-O2).
        let concat: Vec<TxEnv> = blocks_txs.iter().flat_map(|b| b.iter().cloned()).collect();
        let mut s = storage.clone();
        let mut engine = Pevm::default();
        let t = Instant::now();
        match engine.execute_revm_parallel(
            &chain, &s, spec_id, concat_block_env(blocks_txs.len()), concat.clone(), concurrency,
        ) {
            Ok(r) => {
                row.concat_diag.add(&engine.last_diagnostics);
                update_storage_with_results(&mut s, r);
                row.concat_s = t.elapsed().as_secs_f64();
                row.digest_concat = state_digest(&s);
            }
            Err(e) => println!("  batch {batch_idx}: concat block-stm failed ({e:?})"),
        }

        // ── 3b. Concat + graph: graph-aware OCC over the whole round.
        let rg = round_graph(&base_graphs, hot_key_threshold);
        let mut s = storage.clone();
        let mut engine = GraphPevm::default();
        let t = Instant::now();
        match engine.execute_revm_parallel(
            &chain, &s, spec_id, concat_block_env(blocks_txs.len()), concat, concurrency, rg,
        ) {
            Ok(r) => {
                row.cgraph_diag.add(&engine.last_diagnostics);
                update_storage_with_results(&mut s, r);
                row.cgraph_s = t.elapsed().as_secs_f64();
            }
            Err(e) => println!("  batch {batch_idx}: concat+graph failed ({e:?})"),
        }

        // ── 4. Graph-aware OCC, per block.
        let mut s = storage.clone();
        let t = Instant::now();
        for (txs, graph) in reordered.iter().zip(dep_graphs.iter()) {
            let mut engine = GraphPevm::default();
            match engine.execute_revm_parallel(
                &chain, &s, spec_id, real_block_env(), txs.clone(), concurrency, graph.clone(),
            ) {
                Ok(r) => {
                    row.graph_diag.add(&engine.last_diagnostics);
                    update_storage_with_results(&mut s, r);
                }
                Err(e) => println!("  batch {batch_idx}: graph occ failed ({e:?})"),
            }
        }
        row.graph_s = t.elapsed().as_secs_f64();

        // ── 5. Omakase.
        let mut s = storage.clone();
        let t = Instant::now();
        for (txs, graph) in integrated_txns.iter().zip(integrated_graphs.iter()) {
            let mut engine = GraphPevm::default();
            match engine.execute_revm_parallel(
                &chain, &s, spec_id, concat_block_env(blocks_txs.len()), txs.clone(),
                concurrency, graph.clone(),
            ) {
                Ok(r) => {
                    row.integ_diag.add(&engine.last_diagnostics);
                    update_storage_with_results(&mut s, r);
                }
                Err(e) => println!("  batch {batch_idx}: omakase failed ({e:?})"),
            }
        }
        row.integ_s = t.elapsed().as_secs_f64();
        row.phases.execute = Duration::from_secs_f64(row.integ_s);
        row.logical_integ = logical_digest(&s);

        // ── 6. Vegeta: Rule-1 schedule on the graph-aware OCC engine, per block.
        if run_vegeta {
            let t = Instant::now();
            let mut vegeta_blocks: Vec<(Vec<TxEnv>, pevm::dependency_graph::TransactionGraph)> =
                blocks_txs
                    .iter()
                    .zip(base_graphs.iter())
                    .map(|(txs, g)| vegeta::speculate_graph(txs.clone(), g))
                    .collect();
            // Vegeta reorders within a block, so nonces need the same
            // reassignment Omakase's integrated groups get — from an
            // independent tracker.
            let mut vegeta_nonces = NonceTracker { nonces: nonce_snapshot.clone() };
            for (txs, _) in vegeta_blocks.iter_mut() {
                vegeta_nonces.update_txenv_nonces(txs);
            }
            row.vegeta_spec_s = t.elapsed().as_secs_f64();
            row.vegeta_sched_bytes =
                vegeta_blocks.iter().map(|(_, g)| vegeta::graph_proposal_bytes(g)).sum();

            let mut s = storage.clone();
            let t = Instant::now();
            for (txs, graph) in vegeta_blocks.iter() {
                let mut engine = GraphPevm::default();
                match engine.execute_revm_parallel(
                    &chain, &s, spec_id, real_block_env(), txs.clone(), concurrency, graph.clone(),
                ) {
                    Ok(r) => {
                        row.vegeta_diag.add(&engine.last_diagnostics);
                        update_storage_with_results(&mut s, r);
                    }
                    Err(e) => println!("  batch {batch_idx}: vegeta failed ({e:?})"),
                }
            }
            row.vegeta_s = t.elapsed().as_secs_f64();
            row.logical_vegeta = logical_digest(&s);
        }

        // State comparison on real blocks is reported, not asserted.
        //
        // Even the order-preserving engines are not bit-identical to the
        // sequential replay here: the parallel engines settle the block
        // beneficiary lazily and the concatenated baseline fuses 100 blocks
        // into one, so block-granular reward accounting differs even though
        // the transaction effects do not. Real EVM execution is also
        // state-dependent, so the reordering engines diverge by design (this
        // is precisely why the paper's synthetic workload exists). The
        // artificial workload in `test_rebuttal_artificial` is where exact
        // equivalence is actually asserted.
        if row.digest_par != row.digest_seq {
            println!("    note: batch {batch_idx} Block-STM digest != sequential (expected: lazy beneficiary settlement)");
        }
        if row.concat_s > 0.0 && row.digest_concat != row.digest_par {
            println!("    note: batch {batch_idx} concat digest != per-block Block-STM (expected: fused block boundaries)");
        }

        write_row(&mut w, &row);
        w.flush().unwrap();
        done_batches += 1;

        let tp = |s: f64| if s > 0.0 { total_txs as f64 / s } else { 0.0 };
        println!(
            "  batch {} blocks {}..{} txs={} | seq={:.0} par={:.0} concat={:.0} concat+g={:.0} graph={:.0} \
             omakase={:.0} vegeta={:.0} tx/s | prep: pre={:.2}s graph={:.2}s integ={:.2}s \
             | groups={} | cum {:.0}s",
            batch_idx, block_numbers[0], block_numbers[block_numbers.len() - 1], total_txs,
            tp(row.seq_s), tp(row.par_s), tp(row.concat_s), tp(row.cgraph_s), tp(row.graph_s),
            tp(row.integ_s), tp(row.vegeta_s),
            row.phases.pre_execute.as_secs_f64(), row.phases.graph_build.as_secs_f64(),
            row.phases.integrate.as_secs_f64(), row.num_integrated_groups,
            overall.elapsed().as_secs_f64(),
        );
    }

    println!("\n✓ {done_batches} batch(es) in {:.1}s | CSV -> {output}",
        overall.elapsed().as_secs_f64());
}
