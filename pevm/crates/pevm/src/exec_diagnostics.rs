//! Programmatic abort / re-execution counters for one parallel execution.
//!
//! The engines already print these under `feature = "diagnostics"`; this module
//! makes the same numbers readable by benchmark harnesses so they can be
//! aggregated into per-batch CSVs instead of scraped from stderr.
//!
//! Only `re_executions` is populated without `feature = "diagnostics"`; every
//! other field stays 0 because the underlying counters are not compiled in.

/// Abort and re-execution counts for a single parallel block/group execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecDiagnostics {
    /// Transactions executed (block or integrated-group size).
    pub block_size: usize,
    /// Total re-executions, i.e. the sum of every transaction's final incarnation.
    pub re_executions: usize,
    /// Validation failures: a transaction's re-read of its read set disagreed
    /// with what it originally observed.
    pub validation_aborts: usize,
    /// Subset of `validation_aborts` caused by the *re-execution* of a
    /// lower-indexed transaction: the invalidating write carries incarnation
    /// > 0, is an ESTIMATE left by an aborted incarnation, or is a version the
    /// transaction read that a later incarnation no longer produces. Aborts
    /// caused by a lower-indexed writer's *first* execution are ordinary
    /// optimistic aborts, not cascades.
    pub cascade_aborts: usize,
    /// Validation failures whose invalidating writer belongs to a different
    /// block (graph node `replica`) than the reader: dependencies a block
    /// boundary resolved before integration. Graph-driven engines only.
    pub cross_block_aborts: usize,
    /// Subset of `cross_block_aborts` that are cascades.
    pub cross_block_cascade: usize,
    /// Re-executions that wrote a memory location the previous incarnation had
    /// not written. These are the re-executions that can invalidate *other*
    /// transactions, i.e. the trigger for a cascade.
    pub wrote_new_location: usize,
    /// Number of optimistic windows the engine abandoned and re-executed
    /// sequentially (e.g. a read of an account self-destructed in the same
    /// window, or a scheduler abort). Rounds with any fallback are excluded
    /// from the reported statistics (implementation artifact).
    pub fallbacks: usize,
    /// Aborts caused by blocking on an ESTIMATE marker or an out-of-order nonce.
    pub blocking_aborts: usize,
    pub blocking_estimate: usize,
    pub blocking_nonce: usize,
    /// Blocking attempts that resolved without an abort (the blocker finished first).
    pub blocking_retry: usize,
}

impl ExecDiagnostics {
    /// Re-executions per transaction — the abort-rate figure to compare engines by.
    pub fn re_exec_rate(&self) -> f64 {
        if self.block_size == 0 {
            0.0
        } else {
            self.re_executions as f64 / self.block_size as f64
        }
    }

    /// Share of validation aborts attributable to a newly-appeared prior write.
    pub fn cascade_share(&self) -> f64 {
        if self.validation_aborts == 0 {
            0.0
        } else {
            self.cascade_aborts as f64 / self.validation_aborts as f64
        }
    }

    /// Accumulate another execution's counts (for summing over a batch).
    pub fn add(&mut self, o: &ExecDiagnostics) {
        self.block_size += o.block_size;
        self.re_executions += o.re_executions;
        self.validation_aborts += o.validation_aborts;
        self.cascade_aborts += o.cascade_aborts;
        self.cross_block_aborts += o.cross_block_aborts;
        self.cross_block_cascade += o.cross_block_cascade;
        self.wrote_new_location += o.wrote_new_location;
        self.fallbacks += o.fallbacks;
        self.blocking_aborts += o.blocking_aborts;
        self.blocking_estimate += o.blocking_estimate;
        self.blocking_nonce += o.blocking_nonce;
        self.blocking_retry += o.blocking_retry;
    }
}
