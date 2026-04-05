use std::collections::{HashSet, HashMap, BinaryHeap};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId {
    pub id: u64,
    pub replica: u64,
}

impl TransactionId {
    pub fn new(
        id: u64,
        replica: u64,
    ) -> Self {
        Self{
            id,
            replica,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionNode {
    pub id: u64,
    pub replica: u64,
    pub round: u64,
    pub execution_time: u64,
    pub read_set: HashSet<u64>,
    pub write_set: HashSet<u64>,
    pub longest_suffix: u64,
    pub children_indices: Vec<usize>,
    pub parent_indices: HashSet<usize>,
}

impl TransactionNode {
    /// Creates a new TransactionNode with the given parameters
    pub fn new(
        id: u64,
        replica: u64,
        round: u64,
        execution_time: u64,
        read_set: HashSet<u64>,
        write_set: HashSet<u64>,
    ) -> Self {
        Self {
            id,
            replica,
            round,
            execution_time,
            children_indices: Vec::new(),
            read_set,
            write_set,
            longest_suffix: execution_time,
            parent_indices: HashSet::new(),
        }
    }

    pub fn transaction_id(&self) -> TransactionId {
        TransactionId::new(self.id, self.replica)
    }

}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeapEntry {
    pub longest_suffix: u64,        // Primary sort key (first field)
    pub tx_id: TransactionId,       // Secondary sort key
}

impl HeapEntry{
    fn new(tx_id: TransactionId, longest_suffix: u64) -> Self {
        Self {
            longest_suffix,
            tx_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EstimatedCompletion {
    time: u64,
    thread_id: usize,
}

impl Ord for EstimatedCompletion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse ordering for min-heap
        other.time.cmp(&self.time)
            .then_with(|| other.thread_id.cmp(&self.thread_id))
    }
}

impl PartialOrd for EstimatedCompletion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}


#[derive(Debug, Clone)]
pub struct HotKeyStats {
    pub total_keys: usize,
    pub hot_keys_count: usize,
    pub hot_key_ratio: f64,
    pub threshold_value: f64,
    pub avg_thread_completion: f64,
}


#[derive(Debug, Clone)]
pub struct TransactionGraph {
    pub nodes: Vec<TransactionNode>,
    pub id_to_index: HashMap<TransactionId, usize>,
    pub head_txns: HashMap<u64, TransactionId>,    // The Map from Address to the head Transaction Nodes in the graph
    pub tail_txns: HashMap<u64, TransactionId>,    // The Map from Address to the tail Transaction Nodes in the graph
    pub txns_without_parent: BinaryHeap<HeapEntry>,  // A max_heap of TransactionId of TransactionNodes without parents, where the nodes are ordered by their longest_suffix 
    pub simulation_result: Option<SimulationResult>,
    pub temp_parents: Vec<HashSet<usize>>, // Temporary storage for parent transactions during simulation
    pub hot_key_threshold: f64, // Threshold to classify hot keys based on access frequency
    pub hot_keys: HashSet<u64>,
}

impl TransactionGraph {

    // Initialize a new empty transaction graph
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            id_to_index: HashMap::new(),
            head_txns: HashMap::new(),
            tail_txns: HashMap::new(),
            txns_without_parent: BinaryHeap::new(),
            simulation_result: None,
            temp_parents: Vec::new(),
            hot_key_threshold: 1.5,
            hot_keys: HashSet::new(),
        }
    }

    /// Set the hot key threshold
    pub fn set_hot_key_threshold(&mut self, threshold: f64) {
        self.hot_key_threshold = threshold;
    }

    /// Gets a reference to a node by TransactionId
    pub fn get_node(&self, tx_id: &TransactionId) -> Option<&TransactionNode> {
        self.id_to_index.get(tx_id).map(|&idx| &self.nodes[idx])
    }

    /// Gets a mutable reference to a node by TransactionId
    pub fn get_node_mut(&mut self, tx_id: &TransactionId) -> Option<&mut TransactionNode> {
        self.id_to_index.get(tx_id).map(|&idx| &mut self.nodes[idx])
    }

    pub fn add_node(&mut self, node: TransactionNode) -> usize {
        let tx_id = node.transaction_id();
        self.nodes.push(node);
        let index = self.nodes.len() - 1;
        self.id_to_index.insert(tx_id, index);
        index
    }

    /// Checks if a directed edge exists from parent to child
    /// Returns Error if parent or child node doesn't exist
    pub fn has_edge(&self, parent_id: &TransactionId, child_id: &TransactionId) -> Result<bool, String> {
        let parent_idx = self.id_to_index.get(parent_id)
            .ok_or_else(|| format!("Parent node ({}, {}) not found", parent_id.id, parent_id.replica))?;
        
        let child_idx = self.id_to_index.get(child_id)
            .ok_or_else(|| format!("Child node ({}, {}) not found", child_id.id, child_id.replica))?;
        
        Ok(self.nodes[*parent_idx].children_indices.contains(child_idx))
    }
    
    /// Adds a directed edge from parent to child
    /// Checks if edge exists before adding to avoid duplicates
    pub fn add_edge(&mut self, parent_id: TransactionId, child_id: TransactionId) -> Result<(), String> {
        let parent_idx = *self.id_to_index.get(&parent_id)
            .ok_or_else(|| format!("Parent node ({}, {}) not found", parent_id.id, parent_id.replica))?;
        let child_idx = *self.id_to_index.get(&child_id)
            .ok_or_else(|| format!("Child node ({}, {}) not found", child_id.id, child_id.replica))?;
        
        // Check if edge already exists
        if !self.nodes[parent_idx].children_indices.contains(&child_idx) {
            self.nodes[parent_idx].children_indices.push(child_idx);
            self.nodes[child_idx].parent_indices.insert(parent_idx);
        }
        
        Ok(())
    }

    pub fn add_transaction(&mut self, node: TransactionNode) -> Result<usize, String> {
        let tx_id = node.transaction_id();
        // println!("Adding Txn {}", tx_id.id);
        // Check if transaction already exists
        if self.id_to_index.contains_key(&tx_id) {
            return Err(format!("Transaction {:?} already exists in graph", tx_id));
        }

        // Collect all addresses this transaction touches
        // Collect all addresses this transaction touches
        let mut all_addresses: HashSet<u64> = HashSet::new();
        all_addresses.extend(node.read_set.iter().cloned());
        all_addresses.extend(node.write_set.iter().cloned());
        
        // Step 1: Add the node to the graph
        let new_idx = self.add_node(node.clone());
        
        // Step 2: Find dependencies (parent transactions) based on conflicts
        let mut parent_transactions: HashSet<TransactionId> = HashSet::new();

        for addr in &all_addresses {
            // Check if there's a tail transaction for this address
            if let Some(tail_tx_id) = self.tail_txns.get(addr).cloned() {
                // Verify the transaction still exists and check for conflicts
                if let Some(tail_node) = self.get_node(&tail_tx_id) {
                    // Check for conflicts
                    let has_conflict =
                        // Write-After-Write conflict only.
                        // RAW and WAR are handled by OCC validation and don't need
                        // explicit graph edges — including them over-constrains the graph
                        // and kills parallelism.
                        !tail_node.write_set.is_disjoint(&node.write_set);
                    if has_conflict {
                        // println!("Conflicting with Txn {}", tail_tx_id.id);
                        parent_transactions.insert(tail_tx_id.clone());
                    }
                }
            }
        }

        // Step 3: Add edges from parents to this new node
        for parent_tx_id in &parent_transactions {
            self.add_edge(parent_tx_id.clone(), tx_id.clone())?;
        }

        // Step 4: Update head/tail mappings for all addresses
        for addr in &all_addresses {
            // If no head exists for this address, set this as head
            if !self.head_txns.contains_key(addr) {
                self.head_txns.insert(*addr, tx_id.clone());
            }
            
            // Always update tail to this new transaction
            self.tail_txns.insert(*addr, tx_id.clone());
        }

        Ok(new_idx)
    }

    pub fn update_longest_suffix_postorder(&mut self) {
        let n = self.nodes.len();
        if n == 0 {
            return;
        }
        
        // Step 1: Find all root nodes (nodes with no parents)
        let mut has_parent = vec![false; n];
        for node in &self.nodes {
            for &child_idx in &node.children_indices {
                has_parent[child_idx] = true;
            }
        }
        
        let mut root_nodes = Vec::new();
        for (idx, &has_p) in has_parent.iter().enumerate() {
            if !has_p {
                root_nodes.push(idx);
            }
        }
        
        // Step 2: Clear the existing heap
        self.txns_without_parent.clear();
        
        // Step 3: Perform post-order DFS from each root and add to heap
        let mut visited = vec![false; n];
        
        for &root_idx in &root_nodes {
            // Compute longest_suffix for this root and its subtree
            self.postorder_dfs(root_idx, &mut visited);
            
            // After computing, add root to heap
            let root_node = &self.nodes[root_idx];
            let tx_id = root_node.transaction_id();
            let entry = HeapEntry::new(tx_id, root_node.longest_suffix);
            self.txns_without_parent.push(entry);
        }
    }
    
    /// Post-order DFS: process children before parent
    fn postorder_dfs(&mut self, node_idx: usize, visited: &mut [bool]) -> u64 {
        if visited[node_idx] {
            return self.nodes[node_idx].longest_suffix;
        }
        
        visited[node_idx] = true;
        
        // First, recursively process all children
        let children_indices = self.nodes[node_idx].children_indices.clone();
        let mut max_child_suffix = 0u64;
        
        for &child_idx in &children_indices {
            let child_suffix = self.postorder_dfs(child_idx, visited);
            max_child_suffix = max_child_suffix.max(child_suffix);
        }
        
        // Then update current node
        let execution_time = self.nodes[node_idx].execution_time;
        let new_suffix = execution_time + max_child_suffix;
        self.nodes[node_idx].longest_suffix = new_suffix;
        
        new_suffix
    }
    
    /// Pops the transaction with the highest longest_suffix from the heap
    pub fn pop_txn_without_parent(&mut self) -> Option<TransactionId> {
        self.txns_without_parent.pop().map(|entry| entry.tx_id)
    }

    /// Simulate parallel execution with k threads using a min-heap for completion times
    pub fn simulate_parallel_execution(&mut self, k: usize) -> (){
        // Initialize temporary parent storage
        self.temp_parents = self.nodes.iter()
            .map(|node| node.parent_indices.clone()).collect();
        
        // Initialize threads
        let mut threads: Vec<ThreadState> = (0..k)
            .map(|i| ThreadState::new(i))
            .collect();
        
        // Min-heap of completion times
        let mut completion_heap: BinaryHeap<EstimatedCompletion> = BinaryHeap::new();
        
        // Track execution timeline
        let mut execution_order = Vec::new();
        let mut current_time = 0u64;
        
        // Initialize the heap with root transactions
        self.update_longest_suffix_postorder();
        
        loop {
            // Assign new transactions to idle threads
            let mut assigned_any = false;
            
            for thread in &mut threads {
                if thread.is_idle() {
                    if let Some(tx_id) = self.pop_txn_without_parent() {
                        let keys = {
                            let node = self.get_node(&tx_id).unwrap();
                            let mut key_set: std::collections::HashSet<u64> = HashSet::new();
                            key_set.extend(&node.write_set);
                            key_set
                        };
                        let execution_time = self.get_node(&tx_id)
                            .map(|node| node.execution_time)
                            .unwrap_or(0);
                        
                        let expected_completion_time = current_time + execution_time;
                        
                        // println!(
                        //     "Time {}: Thread {} starts transaction {} ({}, {}) [execution_time: {}, expected_completion: {}] keys: {:?}",
                        //     current_time,
                        //     thread.thread_id,
                        //     self.id_to_index[&tx_id],
                        //     tx_id.id,
                        //     tx_id.replica,
                        //     execution_time,
                        //     expected_completion_time,
                        //     keys
                        // );
                        
                        thread.assign_transaction(tx_id.clone(), current_time, execution_time);
                        execution_order.push((
                            thread.thread_id,
                            tx_id,
                            current_time,
                            expected_completion_time,
                        ));
                        
                        // Add completion event to min-heap
                        completion_heap.push(EstimatedCompletion {
                            time: expected_completion_time,
                            thread_id: thread.thread_id,
                        });
                        
                        assigned_any = true;
                    }
                }
            }
            
            // If we assigned any transactions, continue to next assignment round
            if assigned_any && !self.txns_without_parent.is_empty() {
                continue;
            }
            
            // Pop the earliest completion time from min-heap
            if let Some(event) = completion_heap.pop() {
                // Advance time to the completion event if it's in the future
                if event.time > current_time {
                    current_time = event.time;
                }
                
                // println!(
                //     "Time {}: Thread {} completed transaction ({}, {})",
                //     current_time,
                //     event.thread_id,
                //     threads[event.thread_id].current_transaction.as_ref().unwrap().id,
                //     threads[event.thread_id].current_transaction.as_ref().unwrap().replica
                // );
                
                // Complete the transaction on this thread
                let completed_tx_id = threads[event.thread_id].complete_transaction().unwrap();
                
                // Update the graph: remove transaction and free its children
                self.remove_transaction_and_update(&completed_tx_id);
            } else {
                // No more events and no more transactions
                break;
            }
        }
        
        self.simulation_result = Some(SimulationResult::new(current_time, threads, execution_order));
        // println!("simulation_result: {:#?}", self.simulation_result.clone().unwrap().thread_results);
        self.analyze_key_access_spans();
    }
    
    /// Remove a transaction and add its newly-freed children to the heap
    fn remove_transaction_and_update(&mut self, tx_id: &TransactionId) {
        let tx_idx = match self.id_to_index.get(tx_id) {
            Some(&idx) => idx,
            None => return,
        };
        
        let children_indices = self.nodes[tx_idx].children_indices.clone();
        
        // Check each child to see if it's now free (no other parents)
        for &child_idx in &children_indices {
            self.temp_parents[child_idx].remove(&tx_idx);
            if self.temp_parents[child_idx].len() == 0 {
                let child_node = &self.nodes[child_idx];
                let child_tx_id = child_node.transaction_id();
                let child_suffix = child_node.longest_suffix;
                
                // println!(
                //     "  -> Freeing child transaction ({}, {}) with longest_suffix: {}",
                //     child_tx_id.id, child_tx_id.replica, child_suffix
                // );
                
                self.txns_without_parent.push(HeapEntry::new(child_tx_id, child_suffix));
            }
        }
    }

    pub fn integrate_graph(&mut self, other: TransactionGraph) -> Result<usize, String> {
        let mut edges_added = 0;
        
        // Step 1: Collect all addresses from both graphs
        let mut all_addresses: HashSet<u64> = HashSet::new();
        all_addresses.extend(self.head_txns.keys().cloned());
        all_addresses.extend(self.tail_txns.keys().cloned());
        all_addresses.extend(other.head_txns.keys().cloned());
        all_addresses.extend(other.tail_txns.keys().cloned());
        
        // Step 2: Add all nodes from G2 to G1
        let mut old_to_new_index: HashMap<usize, usize> = HashMap::new();
        
        for (old_idx, node) in other.nodes.iter().enumerate() {
            let new_idx = self.nodes.len();
            old_to_new_index.insert(old_idx, new_idx);
            
            let tx_id = node.transaction_id();
            self.id_to_index.insert(tx_id, new_idx);
            self.nodes.push(node.clone());
        }
        
        // Step 3: Update children and parent indices for G2 nodes (remap to new indices)
        for (old_idx, &new_idx) in &old_to_new_index {
            let old_children = other.nodes[*old_idx].children_indices.clone();
            let old_parents = other.nodes[*old_idx].parent_indices.clone();
            let new_children: Vec<usize> = old_children
                .iter()
                .map(|&old_child_idx| old_to_new_index[&old_child_idx])
                .collect();
            let new_parents: HashSet<usize> = old_parents
                .iter()
                .map(|&old_parent_idx| old_to_new_index[&old_parent_idx])
                .collect();
            
            self.nodes[new_idx].children_indices = new_children;
            self.nodes[new_idx].parent_indices = new_parents;
        }

        // Step 4: For each address, connect G1's tail to G2's head
        for addr in &all_addresses {
            let g1_tail = self.tail_txns.get(addr).cloned();
            let g2_head = other.head_txns.get(addr).cloned();
            
            match (g1_tail, g2_head) {
                (Some(tail_tx_id), Some(head_tx_id)) => {

                    if let Ok(false) = self.has_edge(&tail_tx_id, &head_tx_id) {
                        // Add edge from G1's tail to G2's head
                        // println!(
                        //     "Connecting address {:?}: G1 tail ({}, {}) -> G2 head ({}, {})",
                        //     addr, tail_tx_id.id, tail_tx_id.replica, head_tx_id.id, head_tx_id.replica
                        // );

                        self.add_edge(tail_tx_id.clone(), head_tx_id.clone())?;
                        edges_added += 1;
                    }
                    
                    // Update tail to G2's tail
                    if let Some(g2_tail) = other.tail_txns.get(addr) {
                        self.tail_txns.insert(*addr, g2_tail.clone());
                    }
                }
                (None, Some(head_tx_id)) => {
                    // G1 has no transactions for this address, set G2's head as G1's head
                    self.head_txns.insert(*addr, head_tx_id);
                    if let Some(g2_tail) = other.tail_txns.get(addr) {
                        self.tail_txns.insert(*addr, g2_tail.clone());
                    }
                }
                (Some(_), None) => {
                    // G2 has no transactions for this address, keep G1's mappings
                }
                (None, None) => {
                    // Neither graph has transactions for this address (shouldn't happen)
                }
            }
        }
        // println!("edges_added :{}", edges_added);
        Ok(edges_added)
    }

    /// Detect and store hot keys
    fn detect_hot_keys(&mut self) {
        self.hot_keys.clear();
        
        if let Some(result) = &self.simulation_result {
            let avg_completion = self.average_thread_completion_time();
            let threshold = avg_completion * self.hot_key_threshold;
            
            for (key, span) in &result.key_access_spans {
                let lifespan = span.latest_finish - span.earliest_start;
                if lifespan as f64 > threshold {
                    self.hot_keys.insert(*key);
                }
            }
        }
    }

    /// Analyze key access spans from simulation results
    fn analyze_key_access_spans(&mut self) {
        if let Some(result) = &mut self.simulation_result {
            // Pre-build index for O(1) lookup
            let node_map: HashMap<u64, &TransactionNode> = self.nodes.iter()
                .map(|n| (n.id, n))
                .collect();
            
            let mut key_spans: HashMap<u64, KeyAccessSpan> = HashMap::new();
            
            for (_thread_id, tx_id, start_time, end_time) in &result.execution_order {
                if let Some(&node) = node_map.get(&(tx_id.id as u64)) {
                    // Collect all keys accessed by this transaction
                    // Combine both read_set and write_set
                    let all_keys: HashSet<u64> = node.read_set.iter()
                        .chain(node.write_set.iter())
                        .copied()
                        .collect();
                    
                    for key in all_keys {
                        key_spans.entry(key)
                            .and_modify(|span| {
                                span.earliest_start = span.earliest_start.min(*start_time);
                                span.latest_finish = span.latest_finish.max(*end_time);
                                span.access_count += 1;
                            })
                            .or_insert(KeyAccessSpan {
                                key,
                                earliest_start: *start_time,
                                latest_finish: *end_time,
                                access_count: 1,
                            });
                    }
                }
            }
            
            result.key_access_spans = key_spans;
        }
        // After analyzing key access spans, detect hot keys
        self.detect_hot_keys();
    }

    /// Calculate average thread completion time
    fn average_thread_completion_time(&self) -> f64 {
        if let Some(result) = &self.simulation_result {
            if result.thread_results.is_empty() {
                return 0.0;
            }
            
            result.sum_completion_times as f64 / result.thread_results.len() as f64
        } else {
            0.0
        }
    }
    
    /// Check if a specific key is hot
    pub fn is_hot_key(&self, key: u64) -> bool {
        self.hot_keys.contains(&key)
    }
    
    /// Get all hot keys (returns reference to the set)
    pub fn get_hot_keys(&self) -> &HashSet<u64> {
        &self.hot_keys
    }

    /// Get hot key details (with spans)
    pub fn get_hot_key_spans(&self) -> Vec<&KeyAccessSpan> {
        if let Some(result) = &self.simulation_result {
            self.hot_keys.iter()
                .filter_map(|key| result.key_access_spans.get(key))
                .collect()
        } else {
            Vec::new()
        }
    }
    
    /// Get top N hottest keys (by lifespan)
    pub fn get_hottest_keys(&self, n: usize) -> Vec<&KeyAccessSpan> {
        let mut hot_key_spans = self.get_hot_key_spans();
        
        // Sort by lifespan descending
        hot_key_spans.sort_by(|a, b| {
            let lifespan_a = a.latest_finish - a.earliest_start;
            let lifespan_b = b.latest_finish - b.earliest_start;
            lifespan_b.cmp(&lifespan_a)
        });
        
        hot_key_spans.into_iter().take(n).collect()
    }

    /// Check if two graphs have common hot keys
    pub fn has_common_hot_keys(&self, other: &TransactionGraph) -> bool {
        !self.hot_keys.is_disjoint(&other.hot_keys)
    }
    
    /// Get hot key statistics
    pub fn hot_key_statistics(&self) -> HotKeyStats {
        let all_keys = if let Some(result) = &self.simulation_result {
            result.key_access_spans.len()
        } else {
            0
        };
        
        let hot_count = self.hot_keys.len();
        
        let avg_completion = self.average_thread_completion_time();
        let threshold = avg_completion * self.hot_key_threshold;
        
        HotKeyStats {
            total_keys: all_keys,
            hot_keys_count: hot_count,
            hot_key_ratio: if all_keys > 0 { 
                hot_count as f64 / all_keys as f64 
            } else { 
                0.0 
            },
            threshold_value: threshold,
            avg_thread_completion: avg_completion,
        }
    }
    
    /// Print hot key analysis
    pub fn print_hot_key_report(&self) {
        let stats = self.hot_key_statistics();
        
        println!("\n╔═══════════════════════════════════════════════════════════════════╗");
        println!("║                      HOT KEY ANALYSIS                             ║");
        println!("╠═══════════════════════════════════════════════════════════════════╣");
        println!("║ Hot Key Threshold (k):        {:>6.2}                            ║", 
                 self.hot_key_threshold);
        println!("║ Avg Thread Completion Time:   {:>10.2}                          ║",
                 stats.avg_thread_completion);
        println!("║ Threshold Value (k × avg):    {:>10.2}                          ║",
                 stats.threshold_value);
        println!("╠═══════════════════════════════════════════════════════════════════╣");
        println!("║ Total Keys:                   {:>6}                              ║",
                 stats.total_keys);
        println!("║ Hot Keys:                     {:>6}                              ║",
                 stats.hot_keys_count);
        println!("║ Hot Key Ratio:                {:>6.2}%                            ║",
                 stats.hot_key_ratio * 100.0);
        println!("╠═══════════════════════════════════════════════════════════════════╣");
        
        let hottest = self.get_hottest_keys(10);
        if !hottest.is_empty() {
            println!("║ Top 10 Hottest Keys:                                              ║");
            for (i, span) in hottest.iter().enumerate() {
                let lifespan = span.latest_finish - span.earliest_start;
                let ratio = lifespan as f64 / stats.avg_thread_completion;
                println!("║ {}. Key {:>5}: lifespan {:>6} ({:>4.2}× avg), {} accesses      ║",
                         i + 1, span.key, lifespan, ratio, span.access_count);
            }
        } else {
            println!("║ No hot keys detected.                                             ║");
        }
        
        println!("╚═══════════════════════════════════════════════════════════════════╝");
    }

    /// Check if the graph has cycles
pub fn has_cycle(&self) -> bool {
    let n = self.nodes.len();
    let mut visited = vec![false; n];
    let mut rec_stack = vec![false; n];
    
    fn dfs(
        node_idx: usize,
        graph: &TransactionGraph,
        visited: &mut Vec<bool>,
        rec_stack: &mut Vec<bool>,
        path: &mut Vec<usize>,  // 记录路径用于调试
    ) -> bool {
        visited[node_idx] = true;
        rec_stack[node_idx] = true;
        path.push(node_idx);
        
        // 遍历所有子节点
        for &child_idx in &graph.nodes[node_idx].children_indices {
            if !visited[child_idx] {
                if dfs(child_idx, graph, visited, rec_stack, path) {
                    return true;
                }
            } else if rec_stack[child_idx] {
                // 找到循环！
                println!("⚠️  Cycle detected:");
                let cycle_start = path.iter().position(|&idx| idx == child_idx).unwrap();
                for i in cycle_start..path.len() {
                    let tx = &graph.nodes[path[i]];
                    println!("  -> Tx {} (id={}, replica={})", 
                             path[i], tx.id, tx.replica);
                }
                println!("  -> Tx {} (back to start)", child_idx);
                return true;
            }
        }
        
        rec_stack[node_idx] = false;
        path.pop();
        false
    }
    
    for i in 0..n {
        if !visited[i] {
            let mut path = Vec::new();
            if dfs(i, self, &mut visited, &mut rec_stack, &mut path) {
                return true;
            }
        }
    }
    
    false
}

/// Get the dependencies (parent transactions) of a transaction
pub fn dependencies(&self, tx_idx: usize) -> Vec<usize> {
    if tx_idx < self.nodes.len() {
        self.nodes[tx_idx].parent_indices.iter().copied().collect()
    } else {
        Vec::new()
    }
}

/// Get node count
pub fn node_count(&self) -> usize {
    self.nodes.len()
}

/// Get edge count
pub fn edge_count(&self) -> usize {
    self.nodes.iter()
        .map(|node| node.children_indices.len())
        .sum()
}

}


#[derive(Debug, Clone)]
pub struct ThreadState {
    pub thread_id: usize,
    pub current_transaction: Option<TransactionId>,
    pub completion_time: u64,
    pub completed_transactions: Vec<TransactionId>,
}

impl ThreadState {
    pub fn new(thread_id: usize) -> Self {
        Self {
            thread_id,
            current_transaction: None,
            completion_time: 0,
            completed_transactions: Vec::new(),
        }
    }
    
    pub fn is_idle(&self) -> bool {
        self.current_transaction.is_none()
    }
    
    pub fn assign_transaction(&mut self, tx_id: TransactionId, current_time: u64, execution_time: u64) {
        self.current_transaction = Some(tx_id);
        self.completion_time = current_time + execution_time;
    }
    
    pub fn complete_transaction(&mut self) -> Option<TransactionId> {
        if let Some(tx_id) = self.current_transaction.take() {
            self.completed_transactions.push(tx_id.clone());
            Some(tx_id)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct KeyAccessSpan {
    pub key: u64,
    pub earliest_start: u64,  // Earliest time this key is accessed
    pub latest_finish: u64,   // Latest time this key finishes being accessed
    pub access_count: usize,  // Number of transactions accessing this key
}


#[derive(Debug, Clone, Default)]
pub struct SimulationResult {
    pub total_time: u64,
    pub thread_results: Vec<ThreadState>,
    pub execution_order: Vec<(usize, TransactionId, u64, u64)>, // (thread_id, tx_id, start_time, end_time)
    pub standard_deviation: f64,
    pub coefficient_of_variation: f64,
    pub sum_completion_times: u64,
    pub key_access_spans: HashMap<u64, KeyAccessSpan>,
}

impl SimulationResult {
    pub fn new(
        total_time: u64, 
        thread_results: Vec<ThreadState>, 
        execution_order: Vec<(usize, TransactionId, u64, u64)>
    ) -> Self {
        // Calculate sum of all thread completion times
        let sum_completion_times: u64 = thread_results.iter()
            .map(|thread| thread.completion_time)
            .sum();

        // Calculate standard deviation and coefficient of variation
        let (standard_deviation, coefficient_of_variation) = 
            Self::calculate_std_dev_and_cv(&thread_results, sum_completion_times);
        
        Self {
            total_time,
            thread_results,
            execution_order,
            standard_deviation,
            coefficient_of_variation,
            sum_completion_times,
            key_access_spans: HashMap::new(),
        }
    }

    /// Internal helper to calculate std dev and CV from thread results
    fn calculate_std_dev_and_cv(
        thread_results: &[ThreadState],
        sum_completion_times: u64
    ) -> (f64, f64) {
        if thread_results.is_empty() {
            return (0.0, 0.0);
        }
        
        let mean = sum_completion_times as f64 / thread_results.len() as f64;
        
        // Calculate variance
        let variance: f64 = thread_results
            .iter()
            .map(|thread| {
                let diff = thread.completion_time as f64 - mean;
                diff * diff
            })
            .sum::<f64>() / thread_results.len() as f64;
        
        // Standard deviation is the square root of variance
        let standard_deviation = variance.sqrt();
        let coefficient_of_variation = if mean > 0.0 {
            standard_deviation / mean
        } else {
            0.0
        };
        
        (standard_deviation, coefficient_of_variation)
    }

    /// Calculates the standard deviation of the end time (completion time) of all threads
    /// Updates internal fields and returns the calculated values
    pub fn calculate_thread_end_time_std_dev_and_coef_dev(&mut self) -> Result<(f64, f64), String> {
        let (std_dev, cv) = Self::calculate_std_dev_and_cv(&self.thread_results, self.sum_completion_times);
        self.standard_deviation = std_dev;
        self.coefficient_of_variation = cv;
        Ok((std_dev, cv))
    }
}


#[derive(Debug, Clone)]
pub struct ThreadEndTimeStats {
    pub mean: f64,
    pub std_dev: f64,
    pub min: u64,
    pub max: u64,
    pub range: u64,
}

impl std::fmt::Display for ThreadEndTimeStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Thread End Time Statistics:\n\
             Mean: {:.2}\n\
             Std Dev: {:.2}\n\
             Min: {}\n\
             Max: {}\n\
             Range: {}",
            self.mean, self.std_dev, self.min, self.max, self.range
        )
    }
}


