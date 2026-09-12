// greedy_integrator.rs

use crate::dependency_graph::{TransactionGraph};
use std::collections::HashSet;
use std::sync::Arc;
use revm::primitives::{TxEnv};
use crate::graph_pevm::GraphPevm;

/// Configuration for greedy integration algorithm
#[derive(Debug, Clone)]
pub struct GreedyIntegratorConfig {
    /// Maximum acceptable CV for a "decent" graph (τ_CV threshold)
    pub tau_cv: f64,
    /// Number of threads for simulation
    pub num_threads: usize,
    /// Maximum number of source graphs (blocks) absorbed into one integrated
    /// group. Historically hard-coded to 10; exposed so the rebuttal can sweep
    /// it — an unbounded cap with no hot-key filtering degenerates to the
    /// "concatenate the whole round" baseline, so this knob is what places
    /// Omakase between per-block execution and full concatenation.
    pub max_group_blocks: usize,
}

impl Default for GreedyIntegratorConfig {
    fn default() -> Self {
        Self {
            // Tuned via full 100-batch × 100-block sweep over real ETH
            // mainnet workload (NUM_THREADS=16, hot_key_threshold=1.5):
            //   tau_cv  mean integrated speedup
            //   0.1     1.88×
            //   0.3     1.89×
            //   0.5     1.89×   ← chosen
            //   1.0     1.86×
            //   2.0     1.79×
            // 0.3-0.5 are statistically indistinguishable. 0.5 picked because
            // it lets the inner-loop early-exit fire just a little sooner than
            // 0.3, saving a few simulate calls without measurable quality loss.
            tau_cv: 0.5,
            num_threads: std::thread::available_parallelism()
                .unwrap_or(std::num::NonZeroUsize::MIN)
                .get(),
            max_group_blocks: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IntegratedGraphWithIndices {
    pub graph: TransactionGraph,
    pub source_indices: Vec<usize>,  // Indices of original graphs that were merged (in integration order)
}

pub struct GreedyIntegrator {
    config: GreedyIntegratorConfig,
}

impl GreedyIntegrator {
    pub fn new(config: GreedyIntegratorConfig) -> Self {
        Self { config }
    }

/// Main greedy integration algorithm with source indices tracking
    /// Input: A set of dependency graphs
    /// Output: A list of integrated graphs with their source indices (preserving integration order)
    pub fn greedy_decent_integration_with_indices(
        &self,
        graphs: Vec<TransactionGraph>
    ) -> Vec<IntegratedGraphWithIndices> {
        // Simulate any graph that lacks a simulation_result, then move into Arc.
        // Wrapping in Arc lets us hand out cheap refcount-bump clones for the
        // greedy bookkeeping (candidates in `l`, accepted source graphs in
        // `integrated_arcs`) instead of cloning the full graph each time.
        let mut indexed_graphs: Vec<(usize, Arc<TransactionGraph>)> = graphs
            .into_iter()
            .enumerate()
            .map(|(i, mut g)| {
                if g.simulation_result.is_none() {
                    g.simulate_parallel_execution(self.config.num_threads);
                }
                (i, Arc::new(g))
            })
            .collect();

        // Sort graphs in descending order of CV (keeping track of original indices)
        indexed_graphs.sort_by(|a, b| {
            let cv_a = a.1.simulation_result.as_ref()
                .map(|r| r.coefficient_of_variation)
                .unwrap_or(0.0);
            let cv_b = b.1.simulation_result.as_ref()
                .map(|r| r.coefficient_of_variation)
                .unwrap_or(0.0);
            cv_b.partial_cmp(&cv_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut l = indexed_graphs; // L: list of (index, Arc<graph>) pairs
        let mut output = Vec::new(); // O: output list

        while !l.is_empty() {
            // G ← L[0] (initialize with highest-CV graph). We must own G to
            // mutate it during integration, so clone the underlying graph once.
            let (first_idx, first_arc) = l.remove(0);
            let mut g: TransactionGraph = (*first_arc).clone();

            let mut source_indices = vec![first_idx];
            let mut s_positions = HashSet::new();
            // Accepted source graphs (just references — used only for CV_base).
            let mut integrated_arcs: Vec<Arc<TransactionGraph>> = vec![first_arc];

            // For i from |L| down to 1 (in reverse order)
            let mut i = l.len();
            while i > 0 {
                i -= 1;

                // Check if G is already decent - if so, no need to continue
                if self.decent_graph(&g) {
                    println!("Graph G reached decent state. Breaking.");
                    break;
                }

                if g.has_common_hot_keys(&l[i].1) {
                    println!("Skipping graph {} due to common hot keys", i);
                    continue;
                }

                // Try the merge; on success returns the new merged graph,
                // on failure returns None (we don't clone graph_a wastefully).
                let (is_decent, maybe_integrated) =
                    self.decent_integration(&g, &l[i].1, &integrated_arcs);

                if is_decent {
                    g = maybe_integrated.expect("decent => Some(merged)");
                    s_positions.insert(i);
                    source_indices.push(l[i].0);
                    integrated_arcs.push(Arc::clone(&l[i].1));

                    println!(
                        "Integrated graph {} (original index {}) into G. New CV: {:.4}",
                        i,
                        l[i].0,
                        g.simulation_result.as_ref()
                            .map(|r| r.coefficient_of_variation)
                            .unwrap_or(0.0)
                    );

                    if source_indices.len() > self.config.max_group_blocks {
                        break;
                    }
                }
            }

            output.push(IntegratedGraphWithIndices {
                graph: g,
                source_indices,
            });

            // Remove all graphs in S from L (in reverse order to maintain positions)
            let mut positions_to_remove: Vec<_> = s_positions.into_iter().collect();
            positions_to_remove.sort_by(|a, b| b.cmp(a));
            for &pos in &positions_to_remove {
                if pos < l.len() {
                    l.remove(pos);
                }
            }
        }

        output
    }
    
    /// Original method for backward compatibility
    /// Returns only the integrated graphs without source indices
    pub fn greedy_decent_integration(
        &self,
        graphs: Vec<TransactionGraph>
    ) -> Vec<TransactionGraph> {
        self.greedy_decent_integration_with_indices(graphs)
            .into_iter()
            .map(|integrated| integrated.graph)
            .collect()
    }

    /// Check if integrating graph_b into graph_a is "decent".
    ///
    /// Returns `(is_decent, Some(merged))` if integration is acceptable, else
    /// `(false, None)`. The caller already has `graph_a`, so on the non-decent
    /// path we no longer clone graph_a just to hand it back.
    fn decent_integration(
        &self,
        graph_a: &TransactionGraph,
        graph_b: &TransactionGraph,
        integrated_graphs: &[Arc<TransactionGraph>],
    ) -> (bool, Option<TransactionGraph>) {
        // We must clone graph_a here — integrate_graph mutates self, and we
        // mustn't disturb the caller's G unless we end up accepting.
        let mut test_graph = graph_a.clone();
        if test_graph.integrate_graph(graph_b).is_err() {
            return (false, None);
        }
        test_graph.simulate_parallel_execution(self.config.num_threads);

        let cv_g = test_graph.simulation_result.as_ref()
            .map(|r| r.coefficient_of_variation)
            .unwrap_or(f64::INFINITY);

        // CV_base = Σ w_i * CV_i, where w_i = μ_i / Σμ. Iterate by reference
        // over integrated_graphs ⨯ {graph_b} — no clones, no temp Vec.
        let candidate_iter = || integrated_graphs
            .iter()
            .map(|a| a.as_ref())
            .chain(std::iter::once(graph_b));

        let sum_mu: f64 = candidate_iter()
            .map(|g| g.simulation_result.as_ref()
                .map(|r| r.sum_completion_times as f64)
                .unwrap_or(0.0))
            .sum();
        let n = integrated_graphs.len() + 1;

        let cv_base: f64 = candidate_iter()
            .map(|g| {
                let cv_i = g.simulation_result.as_ref()
                    .map(|r| r.coefficient_of_variation).unwrap_or(0.0);
                let mu_i = g.simulation_result.as_ref()
                    .map(|r| r.sum_completion_times as f64).unwrap_or(0.0);
                let w_i = if sum_mu > 0.0 { mu_i / sum_mu } else { 1.0 / n as f64 };
                w_i * cv_i
            })
            .sum();

        if cv_g <= cv_base {
            (true, Some(test_graph))
        } else {
            (false, None)
        }
    }

    /// Check if a graph is "decent"
    /// 
    /// A graph is decent if: CV ≤ τ_CV
    fn decent_graph(&self, graph: &TransactionGraph) -> bool {
        let cv = graph.simulation_result.as_ref()
            .map(|r| r.coefficient_of_variation)
            .unwrap_or(f64::INFINITY);
        
        let is_decent = cv <= self.config.tau_cv;
        
        println!(
            "Checking if graph is decent: CV={:.4}, τ_CV={:.4}, decent={}",
            cv, self.config.tau_cv, is_decent
        );
        
        is_decent
    }

    /// Integrate PEVM dependency graphs using greedy algorithm
    /// Returns integrated transactions and PEVM graphs
    pub fn integrate_pevm_graphs(
        &self,
        dependency_graphs: Vec<TransactionGraph>,
        reordered_blocks_txs: Vec<Vec<TxEnv>>,
    ) -> (Vec<Vec<TxEnv>>, Vec<TransactionGraph>) {
        
        println!("\n=== Greedy PEVM Graph Integration ===");
        println!("Input graphs: {}", dependency_graphs.len());
        println!("CV threshold (tau_cv): {}", self.config.tau_cv);
        println!("Concurrency level: {}\n", self.config.num_threads);
        
        // Run greedy integration directly
        let integration_start = std::time::Instant::now();
        let integrated_results = self.greedy_decent_integration_with_indices(dependency_graphs.clone());
        let integration_time = integration_start.elapsed();
        
        println!("Greedy integration completed in {:.2} s", integration_time.as_secs_f64());
        println!("Output graphs: {} (reduced from {})\n", 
                 integrated_results.len(), 
                 dependency_graphs.len());
        
        // Merge transactions and graphs based on source indices
        let mut integrated_txns = Vec::new();
        let mut integrated_graphs = Vec::new();
        
        println!("=== Merging Transactions and Graphs ===");
        
        for integrated_result in integrated_results.iter() {
            let source_indices = &integrated_result.source_indices;

            // Merge transactions in integration order (this part is still needed).
            let mut merged_txns = Vec::new();
            for &idx in source_indices {
                merged_txns.extend(reordered_blocks_txs[idx].clone());
            }

            // Reuse the graph greedy already produced — it is exactly the
            // serial-integrate_graph of the same `source_indices` in the same
            // order, and its `simulation_result` is already populated. Cloning
            // once here costs less than re-merging k-1 source graphs.
            let mut merged_graph = integrated_result.graph.clone();

            println!("  Merged graph has {} nodes", merged_graph.nodes.len());

            // Reorder transactions based on integrated graph. reorder_txs_*
            // skips its internal simulate when simulation_result is present.
            let (final_reordered_txns, final_graph) = GraphPevm::reorder_txs_by_dependency_graph(
                merged_txns,
                &mut merged_graph,
                self.config.num_threads,
            );

            integrated_txns.push(final_reordered_txns);
            integrated_graphs.push(final_graph);
        }
        
        // Print summary
        println!("\n=== Integration Summary ===");
        println!("Original graphs: {}", dependency_graphs.len());
        println!("Integrated graphs: {}", integrated_graphs.len());
        println!("Reduction: {:.1}%", 
                 (1.0 - integrated_graphs.len() as f64 / dependency_graphs.len() as f64) * 100.0);
        
        // Print details of each group
        for (i, integrated_result) in integrated_results.iter().enumerate() {
            let cv = integrated_result.graph.simulation_result.as_ref()
                .map(|r| r.coefficient_of_variation)
                .unwrap_or(0.0);
            // println!("Group {}: {:?} (CV: {:.4})", i, integrated_result.source_indices, cv);
        }
        
        (integrated_txns, integrated_graphs)
    }
}