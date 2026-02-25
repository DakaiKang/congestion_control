// greedy_integrator.rs

use crate::dependency_graph::{TransactionGraph};
use std::collections::HashSet;
use revm::primitives::{TxEnv};
use crate::graph_pevm::GraphPevm;

/// Configuration for greedy integration algorithm
#[derive(Debug, Clone)]
pub struct GreedyIntegratorConfig {
    /// Maximum acceptable CV for a "decent" graph (τ_CV threshold)
    pub tau_cv: f64,
    /// Number of threads for simulation
    pub num_threads: usize,
}

impl Default for GreedyIntegratorConfig {
    fn default() -> Self {
        Self {
            tau_cv: 0.1,  // Default threshold - adjust as needed
            num_threads: std::thread::available_parallelism()
                .unwrap_or(std::num::NonZeroUsize::MIN)
                .get(),
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
        mut graphs: Vec<TransactionGraph>
    ) -> Vec<IntegratedGraphWithIndices> {
        // Store original indices before sorting
        let mut indexed_graphs: Vec<(usize, TransactionGraph)> = graphs.into_iter()
            .enumerate()
            .collect();
        
        // Ensure all graphs have simulation results for CV calculation
        for (_, graph) in &mut indexed_graphs {
            if graph.simulation_result.is_none() {
                graph.simulate_parallel_execution(self.config.num_threads);
            }
        }
    
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
    
        let mut l = indexed_graphs; // L: list of (index, graph) pairs
        let mut output = Vec::new(); // O: output list
    
        while !l.is_empty() {
            // G ← L[0] (initialize with highest-CV graph)
            let (first_idx, mut g) = l.remove(0);
            
            // Track which original graphs were merged into G (in integration order)
            let mut source_indices = vec![first_idx];
            
            // S: set of positions in L that were merged
            let mut s_positions = HashSet::new();
            
            // Keep track of which graphs have been integrated
            let mut integrated_graphs = vec![g.clone()];
    
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
                    continue; // Skip graphs with common hot keys
                }

                // Check if integration is decent and get the result
                let (is_decent, integrated_graph) = self.decent_integration(&g, &l[i].1, &integrated_graphs);
                
                if is_decent {
                    // Use the already-computed integrated graph
                    g = integrated_graph;
                    s_positions.insert(i);
                    source_indices.push(l[i].0); // Record original index (keep integration order!)
                    integrated_graphs.push(l[i].1.clone());
                    
                    println!(
                        "Integrated graph {} (original index {}) into G. New CV: {:.4}",
                        i,
                        l[i].0,
                        g.simulation_result.as_ref()
                            .map(|r| r.coefficient_of_variation)
                            .unwrap_or(0.0)
                    );

                    if source_indices.len() > 10 {
                        println!("Warning: Integrated more than 5 graphs into one group. Possible excessive merging.");
                        break;
                    }
                }
            }
            
            // Append G with its source indices to output
            output.push(IntegratedGraphWithIndices {
                graph: g,
                source_indices,  // Preserves integration order: e.g., [5, 3, 8, 1]
            });
    
            // Remove all graphs in S from L (in reverse order to maintain positions)
            let mut positions_to_remove: Vec<_> = s_positions.into_iter().collect();
            positions_to_remove.sort_by(|a, b| b.cmp(a)); // Sort in descending order
            
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

    /// Check if integrating graph_b into graph_a is "decent"
    /// 
    /// Returns: (is_decent, integrated_graph)
    /// - If decent: returns (true, integrated graph with simulation result)
    /// - If not decent: returns (false, original graph_a)
    fn decent_integration(
        &self,
        graph_a: &TransactionGraph,
        graph_b: &TransactionGraph,
        integrated_graphs: &[TransactionGraph]
    ) -> (bool, TransactionGraph) {
        // Create a temporary merged graph to test
        let mut test_graph = graph_a.clone();
        
        if let Ok(_) = test_graph.integrate_graph(graph_b.clone()) {
            // Simulate the merged graph
            test_graph.simulate_parallel_execution(self.config.num_threads);
            
            // Get CV of the integrated graph
            let cv_g = test_graph.simulation_result.as_ref()
                .map(|r| r.coefficient_of_variation)
                .unwrap_or(f64::INFINITY);

            // Calculate CV_base using weighted average
            // All graphs that would be integrated (existing + new)
            let mut all_graphs = integrated_graphs.to_vec();
            all_graphs.push(graph_b.clone());
            
            // Calculate μ (sum of completion times) for each graph
            let mut mu_values = Vec::new();
            let mut sum_mu = 0.0;
            
            for graph in &all_graphs {
                // Directly use the cached sum_completion_times
                let mu_i = graph.simulation_result.as_ref()
                    .map(|r| r.sum_completion_times as f64)
                    .unwrap_or(0.0);
                mu_values.push(mu_i);
                sum_mu += mu_i;
            }
            
            // Calculate CV_base = Σ(w_i * CV_i)
            let mut cv_base = 0.0;
            for (i, graph) in all_graphs.iter().enumerate() {
                let cv_i = graph.simulation_result.as_ref()
                    .map(|r| r.coefficient_of_variation)
                    .unwrap_or(0.0);
                let w_i = if sum_mu > 0.0 {
                    mu_values[i] / sum_mu
                } else {
                    1.0 / all_graphs.len() as f64
                };
                cv_base += w_i * cv_i;
            }
            
            let is_decent = cv_g <= cv_base;
            
            // println!(
            //     "Testing integration: CV_G={:.4}, CV_base={:.4}, decent={}",
            //     cv_g, cv_base, is_decent
            // );

            // Return the integrated graph if decent, otherwise return original
            if is_decent {
                (true, test_graph)
            } else {
                (false, graph_a.clone())
            }
        } else {
            (false, graph_a.clone())
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
        
        for (group_idx, integrated_result) in integrated_results.iter().enumerate() {
            let source_indices = &integrated_result.source_indices;
            
            // println!("Group {}: merging graphs {:?}", group_idx, source_indices);
            
            // Merge transactions in integration order
            let mut merged_txns = Vec::new();
            for &idx in source_indices {
                merged_txns.extend(reordered_blocks_txs[idx].clone());
            }
            
            // Merge graphs in integration order
            let mut merged_graph = dependency_graphs[source_indices[0]].clone();
            for i in 1..source_indices.len() {
                let idx = source_indices[i];
                merged_graph.integrate_graph(dependency_graphs[idx].clone());
            }
            
            println!("  Merged graph has {} nodes", merged_graph.nodes.len());
            
            // Reorder transactions based on integrated graph
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