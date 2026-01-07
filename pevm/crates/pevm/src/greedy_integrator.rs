// greedy_integrator.rs

use crate::dependency_graph::{TransactionGraph, SimulationResult};
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
            tau_cv: 0.3,  // Default threshold - adjust as needed
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
        
        // println!(
        //     "Checking if graph is decent: CV={:.4}, τ_CV={:.4}, decent={}",
        //     cv, self.config.tau_cv, is_decent
        // );
        
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
            
            println!("Group {}: merging graphs {:?}", group_idx, source_indices);
            
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
            println!("  Group {}: {:?} (CV: {:.4})", i, integrated_result.source_indices, cv);
        }
        
        (integrated_txns, integrated_graphs)
    }
}

#[cfg(test)]
mod greedy_tests {
    use super::*;
    use crate::dependency_graph::{TransactionGraph, TransactionNode, TransactionId};
    use std::collections::HashSet;

    /// Helper function to create a simple transaction node
    fn create_test_node(
        id: u64,
        replica: u64,
        round: u64,
        execution_time: u64,
        read_set: Vec<u64>,
        write_set: Vec<u64>,
    ) -> TransactionNode {
        TransactionNode {
            id,
            replica,
            round,
            execution_time,
            read_set: read_set.into_iter().collect(),
            write_set: write_set.into_iter().collect(),
            longest_suffix: 0,
            children_indices: Vec::new(),
            parent_indices: HashSet::new(),
        }
    }

    /// Verify that a graph is a DAG (Directed Acyclic Graph) using DFS
    fn is_dag(graph: &TransactionGraph) -> bool {
        if graph.nodes.is_empty() {
            return true;
        }

        let n = graph.nodes.len();
        let mut color = vec![0; n]; // 0: white (unvisited), 1: gray (visiting), 2: black (visited)

        fn dfs_check_cycle(
            node_idx: usize,
            graph: &TransactionGraph,
            color: &mut Vec<u8>,
        ) -> bool {
            color[node_idx] = 1; // Mark as gray (visiting)

            // Check all children
            for &child_idx in &graph.nodes[node_idx].children_indices {
                if color[child_idx] == 1 {
                    // Gray node found - back edge detected, cycle exists
                    return false;
                }
                if color[child_idx] == 0 {
                    // White node - continue DFS
                    if !dfs_check_cycle(child_idx, graph, color) {
                        return false;
                    }
                }
            }

            color[node_idx] = 2; // Mark as black (visited)
            true
        }

        // Check from all nodes (in case of disconnected components)
        for i in 0..n {
            if color[i] == 0 {
                if !dfs_check_cycle(i, graph, &mut color) {
                    return false; // Cycle detected
                }
            }
        }

        true // No cycles found
    }

    /// Helper to verify graph is DAG and print error if not
    fn assert_is_dag(graph: &TransactionGraph, graph_name: &str) {
        assert!(is_dag(graph), "{} is not a DAG (contains cycles)!", graph_name);
        println!("✓ {} is a valid DAG", graph_name);
    }

    /// Create a simple linear chain graph: tx1 -> tx2 -> tx3 -> tx4 -> tx5
    /// All transactions access the same address (0)
    fn create_linear_graph(replica: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        // All transactions write to address 0, creating a linear dependency chain
        for i in 1..=5 {
            let node = create_test_node(
                i,           // id
                replica,           // replica
                i,           // round
                100,         // execution_time (constant)
                vec![],      // read_set
                vec![0],     // write_set (all write to address 0)
            );
            graph.add_transaction(node).unwrap();
        }
        
        assert_is_dag(&graph, "Linear graph");
        graph
    }

    /// Create a parallel graph: 5 independent transactions on different addresses
    /// tx1(addr:1), tx2(addr:2), tx3(addr:3), tx4(addr:4), tx5(addr:5)
    fn create_parallel_graph(replica: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        // Each transaction writes to a different address (no dependencies)
        for i in 1..=5 {
            let node = create_test_node(
                i,           // id
                replica,           // replica
                i,           // round
                100,         // execution_time
                vec![],      // read_set
                vec![i],     // write_set (each writes to different address)
            );
            graph.add_transaction(node).unwrap();
        }
        
        assert_is_dag(&graph, "Parallel graph");
        graph
    }

    /// Create a diamond-shaped dependency graph:
    ///     tx1 (addr:0)
    ///    /   \
    ///  tx2   tx3 (both read addr:0, write to addr:1 and addr:2)
    ///    \   /
    ///     tx4 (reads addr:1,2, writes addr:3)
    ///      |
    ///     tx5 (reads addr:3, writes addr:4)
    fn create_diamond_graph(replica: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        // tx1: writes to address 0
        let tx1 = create_test_node(1, replica, 1, 100, vec![], vec![0]);
        graph.add_transaction(tx1).unwrap();
        
        // tx2: reads address 0, writes to address 1
        let tx2 = create_test_node(2, replica, 2, 150, vec![0], vec![1]);
        graph.add_transaction(tx2).unwrap();
        
        // tx3: reads address 0, writes to address 2
        let tx3 = create_test_node(3, replica, 2, 150, vec![0], vec![2]);
        graph.add_transaction(tx3).unwrap();
        
        // tx4: reads addresses 1,2, writes to address 3
        let tx4 = create_test_node(4, replica, 3, 200, vec![1, 2], vec![3]);
        graph.add_transaction(tx4).unwrap();
        
        // tx5: reads address 3, writes to address 4
        let tx5 = create_test_node(5, replica, 4, 100, vec![3], vec![4]);
        graph.add_transaction(tx5).unwrap();
        
        assert_is_dag(&graph, "Diamond graph");
        graph
    }

    /// Create a graph with high variance in execution times
    fn create_high_variance_graph(replica: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        let execution_times = vec![50, 200, 100, 300, 150];
        
        for (i, &exec_time) in execution_times.iter().enumerate() {
            let id = (i + 1) as u64;
            let node = create_test_node(
                id,
                replica,
                id,
                exec_time,
                vec![],
                vec![id],  // Each writes to different address
            );
            graph.add_transaction(node).unwrap();
        }
        
        assert_is_dag(&graph, "High variance graph");
        graph
    }

    /// Create a graph with mixed dependencies
    /// tx1(addr:0) -> tx2(addr:0,1) -> tx3(addr:1)
    /// tx4(addr:2) -> tx5(addr:2)
    fn create_mixed_graph(replica: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        // Chain 1: tx1 -> tx2 -> tx3
        let tx1 = create_test_node(1, replica, 1, 100, vec![], vec![0]);
        graph.add_transaction(tx1).unwrap();

        let tx2 = create_test_node(2, replica, 2, 100, vec![0], vec![0, 1]);
        graph.add_transaction(tx2).unwrap();
        
        let tx3 = create_test_node(3, replica, 3, 100, vec![1], vec![1]);
        graph.add_transaction(tx3).unwrap();
        
        // Chain 2: tx4 -> tx5 (independent from chain 1)
        let tx4 = create_test_node(4, replica, 1, 100, vec![], vec![2]);
        graph.add_transaction(tx4).unwrap();
        
        let tx5 = create_test_node(5, replica, 2, 100, vec![2], vec![2]);
        graph.add_transaction(tx5).unwrap();
        
        assert_is_dag(&graph, "Mixed graph");
        graph
    }

    #[test]
    fn test_dag_verification() {
        println!("\n=== Testing DAG Verification ===");
        
        let linear = create_linear_graph(0);
        assert!(is_dag(&linear), "Linear graph should be a DAG");
        
        let parallel = create_parallel_graph(1);
        assert!(is_dag(&parallel), "Parallel graph should be a DAG");
        
        let diamond = create_diamond_graph(2);
        assert!(is_dag(&diamond), "Diamond graph should be a DAG");
        
        let mixed = create_mixed_graph(3);
        assert!(is_dag(&mixed), "Mixed graph should be a DAG");
        
        println!("✓ All graphs verified as DAGs");
    }

    #[test]
    fn test_linear_graph_simulation() {
        println!("\n=== Linear Graph Test ===");
        let mut graph = create_linear_graph(0);
        graph.simulate_parallel_execution(2);
        
        let result = graph.simulation_result.as_ref().unwrap();
        println!("Total time: {}, CV: {:.4}", 
                 result.total_time, result.coefficient_of_variation);
        
        assert!(result.total_time > 0);
        assert!(result.coefficient_of_variation >= 0.0);
    }

    #[test]
    fn test_parallel_graph_simulation() {
        println!("\n=== Parallel Graph Test ===");
        let mut graph = create_parallel_graph(0);
        graph.simulate_parallel_execution(2);
        
        let result = graph.simulation_result.as_ref().unwrap();
        println!("Total time: {}, CV: {:.4}", 
                 result.total_time, result.coefficient_of_variation);
        
        // Parallel graph should have lower CV than linear graph
        assert!(result.coefficient_of_variation < 0.5);
    }

    #[test]
    fn test_diamond_graph_simulation() {
        println!("\n=== Diamond Graph Test ===");
        let mut graph = create_diamond_graph(0);
        graph.simulate_parallel_execution(3);
        
        let result = graph.simulation_result.as_ref().unwrap();
        println!("Total time: {}, CV: {:.4}", 
                 result.total_time, result.coefficient_of_variation);
        
        assert!(result.total_time > 0);
    }

    #[test]
    fn test_greedy_integration_with_parallel_graphs() {
        println!("\n=== Greedy Integration: Parallel Graphs ===");
        let config = GreedyIntegratorConfig {
            tau_cv: 0.3,
            num_threads: 2,
        };
        let integrator = GreedyIntegrator::new(config);
        
        // Create multiple parallel graphs
        let graphs = vec![
            create_parallel_graph(0),
            create_parallel_graph(1),
            create_parallel_graph(2),
        ];
        
        let result = integrator.greedy_decent_integration(graphs);
        
        println!("Integration result: {} graphs", result.len());
        for (i, graph) in result.iter().enumerate() {
            assert_is_dag(graph, &format!("Integrated graph {}", i));
            let sim_result = graph.simulation_result.as_ref().unwrap();
            println!("Graph {}: nodes={}, CV={:.4}", 
                     i, graph.nodes.len(), sim_result.coefficient_of_variation);
        }
        
        assert!(!result.is_empty());
    }

    #[test]
    fn test_greedy_integration_with_mixed_graphs() {
        println!("\n=== Greedy Integration: Mixed Graphs ===");
        let config = GreedyIntegratorConfig {
            tau_cv: 0.4,
            num_threads: 3,
        };
        let integrator = GreedyIntegrator::new(config);
        
        let graphs = vec![
            create_linear_graph(0),      // High CV
            create_parallel_graph(1),    // Low CV
            create_diamond_graph(2),     // Medium CV
        ];
        
        let result = integrator.greedy_decent_integration(graphs);
        
        for (i, graph) in result.iter().enumerate() {
            assert_is_dag(graph, &format!("Mixed integrated graph {}", i));
            let sim_result = graph.simulation_result.as_ref().unwrap();
            println!("Result graph {}: nodes={}, total_time={}, CV={:.4}", 
                     i, graph.nodes.len(), sim_result.total_time, 
                     sim_result.coefficient_of_variation);
        }
        
        assert!(!result.is_empty());
    }

    #[test]
    fn test_greedy_integration_high_variance() {
        println!("\n=== Greedy Integration: High Variance ===");
        let config = GreedyIntegratorConfig {
            tau_cv: 0.5,
            num_threads: 4,
        };
        let integrator = GreedyIntegrator::new(config);
        
        let graphs = vec![
            create_high_variance_graph(0),
            create_high_variance_graph(1),
        ];
        
        let result = integrator.greedy_decent_integration(graphs);
        
        for (i, graph) in result.iter().enumerate() {
            assert_is_dag(graph, &format!("High variance integrated graph {}", i));
            let sim_result = graph.simulation_result.as_ref().unwrap();
            println!("Result graph {}: CV={:.4}, std_dev={:.2}", 
                     i, sim_result.coefficient_of_variation, 
                     sim_result.standard_deviation);
        }
        
        assert!(!result.is_empty());
    }

    #[test]
    fn test_decent_graph_threshold() {
        println!("\n=== Decent Graph Threshold Test ===");
        let config = GreedyIntegratorConfig {
            tau_cv: 0.2,
            num_threads: 2,
        };
        let integrator = GreedyIntegrator::new(config);
        
        let mut parallel_graph = create_parallel_graph(0);
        parallel_graph.simulate_parallel_execution(2);
        
        let mut linear_graph = create_linear_graph(1);
        linear_graph.simulate_parallel_execution(2);
        
        let parallel_decent = integrator.decent_graph(&parallel_graph);
        let linear_decent = integrator.decent_graph(&linear_graph);
        
        println!("Parallel graph decent: {}, CV: {:.4}", 
                 parallel_decent, 
                 parallel_graph.simulation_result.as_ref().unwrap().coefficient_of_variation);
        println!("Linear graph decent: {}, CV: {:.4}", 
                 linear_decent,
                 linear_graph.simulation_result.as_ref().unwrap().coefficient_of_variation);
        
        assert!(parallel_graph.simulation_result.as_ref().unwrap().coefficient_of_variation 
                <= linear_graph.simulation_result.as_ref().unwrap().coefficient_of_variation);
    }

    #[test]
    fn test_integration_preserves_dag() {
        println!("\n=== Integration Preserves DAG Test ===");
        let mut graph1 = create_parallel_graph(0);
        let graph2 = create_parallel_graph(1);
        println!("graph1: {:#?}", graph1.id_to_index);
        println!("graph2: {:#?}", graph2.id_to_index);
        
        let nodes_before = graph1.nodes.len();
        graph1.integrate_graph(graph2).unwrap();
        let nodes_after = graph1.nodes.len();
        
        println!("Nodes before: {}, after: {}", nodes_before, nodes_after);
        assert_eq!(nodes_after, nodes_before * 2);

        
        
        // Verify the integrated graph is still a DAG
        assert_is_dag(&graph1, "Integrated graph");
        
        graph1.simulate_parallel_execution(2);
        let result = graph1.simulation_result.as_ref().unwrap();
        println!("Integrated graph CV: {:.4}", result.coefficient_of_variation);
    }


    #[test]
    fn test_greedy_integration_ten_different_graphs() {
        println!("\n=== Greedy Integration: 10 Different Graph Types ===\n");
        
        let config = GreedyIntegratorConfig {
            tau_cv: 0.4,
            num_threads: 4,
        };
        let integrator = GreedyIntegrator::new(config.clone());
        
        // Create 10 different types of graphs with non-overlapping transaction IDs
        let graphs = vec![
            create_linear_graph_with_offset(0, 0),          // Graph 0: Linear, replica 0, ids 1-5
            create_parallel_graph_with_offset(1, 0),        // Graph 1: Parallel, replica 1, ids 1-5
            create_diamond_graph_with_offset(2, 0),         // Graph 2: Diamond, replica 2, ids 1-5
            create_high_variance_graph_with_offset(3, 0),   // Graph 3: High variance, replica 3, ids 1-5
            create_mixed_graph_with_offset(4, 0),           // Graph 4: Mixed, replica 4, ids 1-5
            create_linear_graph_with_offset(5, 0),          // Graph 5: Linear, replica 5, ids 1-5
            create_parallel_graph_with_offset(6, 0),        // Graph 6: Parallel, replica 6, ids 1-5
            create_star_graph_with_offset(7, 0),            // Graph 7: Star, replica 7, ids 1-5
            create_tree_graph_with_offset(8, 0),            // Graph 8: Tree, replica 8, ids 1-5
            create_sequential_graph_with_offset(9, 0),      // Graph 9: Sequential, replica 9, ids 1-5
        ];
        
        // Print initial graph information
        println!("Initial graphs:");
        for (i, graph) in graphs.iter().enumerate() {
            println!("  Graph {}: {} nodes", i, graph.nodes.len());
            assert_is_dag(graph, &format!("Initial graph {}", i));
        }
        println!();
        
        // Perform greedy integration
        let result = integrator.greedy_decent_integration(graphs);
        
        // Verify and print results
        println!("Integration completed: {} output graphs\n", result.len());
        
        let mut total_nodes = 0;
        for (i, graph) in result.iter().enumerate() {
            // Verify each result is a DAG
            assert_is_dag(graph, &format!("Result graph {}", i));
            
            let sim_result = graph.simulation_result.as_ref().unwrap();
            total_nodes += graph.nodes.len();
            
            println!("Result Graph {}:", i);
            println!("  Nodes: {}", graph.nodes.len());
            println!("  Total time: {}", sim_result.total_time);
            println!("  CV: {:.4}", sim_result.coefficient_of_variation);
            println!("  Std Dev: {:.2}", sim_result.standard_deviation);
            println!("  Sum completion times: {}", sim_result.sum_completion_times);
            
            // Verify the graph is decent or is the best we could do
            if sim_result.coefficient_of_variation <= config.tau_cv {
                println!("  Status: ✓ Decent (CV ≤ τ_CV)");
            } else {
                println!("  Status: ⚠ Not decent, but best possible integration");
            }
            println!();
        }
        
        // Verify all nodes are accounted for
        assert_eq!(total_nodes, 50, "Should have 50 total nodes (10 graphs × 5 nodes)");
        println!("✓ All 50 nodes accounted for across {} integrated graphs", result.len());
        
        assert!(!result.is_empty());
    }

    /// Create a linear graph with custom replica ID to avoid TransactionId conflicts
    fn create_linear_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        for i in 1..=5 {
            let node = create_test_node(
                i,
                replica,
                i,
                100,
                vec![],
                vec![addr_offset],  // All write to same address
            );
            graph.add_transaction(node).unwrap();
        }
        
        graph
    }

    /// Create a parallel graph with custom replica ID
    fn create_parallel_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        for i in 1..=5 {
            let node = create_test_node(
                i,
                replica,
                i,
                100,
                vec![],
                vec![addr_offset + i],  // Each writes to different address
            );
            graph.add_transaction(node).unwrap();
        }
        
        graph
    }

    /// Create a diamond graph with custom replica ID
    fn create_diamond_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        let tx1 = create_test_node(1, replica, 1, 100, vec![], vec![addr_offset]);
        graph.add_transaction(tx1).unwrap();
        
        let tx2 = create_test_node(2, replica, 2, 150, vec![addr_offset], vec![addr_offset + 1]);
        graph.add_transaction(tx2).unwrap();
        
        let tx3 = create_test_node(3, replica, 2, 150, vec![addr_offset], vec![addr_offset + 2]);
        graph.add_transaction(tx3).unwrap();
        
        let tx4 = create_test_node(4, replica, 3, 200, vec![addr_offset + 1, addr_offset + 2], vec![addr_offset + 3]);
        graph.add_transaction(tx4).unwrap();
        
        let tx5 = create_test_node(5, replica, 4, 100, vec![addr_offset + 3], vec![addr_offset + 4]);
        graph.add_transaction(tx5).unwrap();
        
        graph
    }

    /// Create a high variance graph with custom replica ID
    fn create_high_variance_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        let execution_times = vec![50, 200, 100, 300, 150];
        
        for (i, &exec_time) in execution_times.iter().enumerate() {
            let id = (i + 1) as u64;
            let node = create_test_node(
                id,
                replica,
                id,
                exec_time,
                vec![],
                vec![addr_offset + id],
            );
            graph.add_transaction(node).unwrap();
        }
        
        graph
    }

    /// Create a mixed graph with custom replica ID
    fn create_mixed_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        let tx1 = create_test_node(1, replica, 1, 100, vec![], vec![addr_offset]);
        graph.add_transaction(tx1).unwrap();
        
        let tx2 = create_test_node(2, replica, 2, 100, vec![addr_offset], vec![addr_offset, addr_offset + 1]);
        graph.add_transaction(tx2).unwrap();
        
        let tx3 = create_test_node(3, replica, 3, 100, vec![addr_offset + 1], vec![addr_offset + 1]);
        graph.add_transaction(tx3).unwrap();
        
        let tx4 = create_test_node(4, replica, 1, 100, vec![], vec![addr_offset + 2]);
        graph.add_transaction(tx4).unwrap();
        
        let tx5 = create_test_node(5, replica, 2, 100, vec![addr_offset + 2], vec![addr_offset + 2]);
        graph.add_transaction(tx5).unwrap();
        
        graph
    }

    /// Create a star-shaped graph: one central node with 4 dependents
    /// tx1 (center, writes addr:0) -> tx2, tx3, tx4, tx5 (all read addr:0)
    fn create_star_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        // Central node
        let tx1 = create_test_node(1, replica, 1, 100, vec![], vec![addr_offset]);
        graph.add_transaction(tx1).unwrap();
        
        // Four dependent nodes that all read from the center
        for i in 2..=5 {
            let node = create_test_node(
                i,
                replica,
                2,
                100,
                vec![addr_offset],
                vec![addr_offset + i],
            );
            graph.add_transaction(node).unwrap();
        }
        
        graph
    }

    /// Create a binary tree-shaped graph
    ///       tx1
    ///      /   \
    ///    tx2   tx3
    ///    /      \
    ///  tx4      tx5
    fn create_tree_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        // Root
        let tx1 = create_test_node(1, replica, 1, 100, vec![], vec![addr_offset]);
        graph.add_transaction(tx1).unwrap();
        
        // Level 2 - left child
        let tx2 = create_test_node(2, replica, 2, 100, vec![addr_offset], vec![addr_offset + 1]);
        graph.add_transaction(tx2).unwrap();
        
        // Level 2 - right child
        let tx3 = create_test_node(3, replica, 2, 100, vec![addr_offset], vec![addr_offset + 2]);
        graph.add_transaction(tx3).unwrap();
        
        // Level 3 - left grandchild
        let tx4 = create_test_node(4, replica, 3, 100, vec![addr_offset + 1], vec![addr_offset + 3]);
        graph.add_transaction(tx4).unwrap();
        
        // Level 3 - right grandchild
        let tx5 = create_test_node(5, replica, 3, 100, vec![addr_offset + 2], vec![addr_offset + 4]);
        graph.add_transaction(tx5).unwrap();
        
        graph
    }

    /// Create a sequential graph with varied execution times
    fn create_sequential_graph_with_offset(replica: u64, addr_offset: u64) -> TransactionGraph {
        let mut graph = TransactionGraph::new();
        
        let execution_times = vec![80, 120, 90, 110, 100];
        
        for (i, &exec_time) in execution_times.iter().enumerate() {
            let id = (i + 1) as u64;
            let node = create_test_node(
                id,
                replica,
                id,
                exec_time,
                vec![],
                vec![addr_offset],  // All write to same address for sequential dependency
            );
            graph.add_transaction(node).unwrap();
        }
        
        graph
    }
    
}