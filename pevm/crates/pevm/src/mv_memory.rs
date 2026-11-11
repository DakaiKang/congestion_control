use std::{
    collections::{BTreeMap, HashSet},
    sync::{atomic::{AtomicUsize, Ordering}, Mutex},
};

use alloy_primitives::{Address, B256};
use dashmap::DashMap;
use revm::primitives::Bytecode;

use crate::{
    BuildIdentityHasher, BuildSuffixHasher, MemoryEntry, MemoryLocationHash, ReadOrigin, ReadSet, TxIncarnation,
    TxIdx, TxVersion, WriteSet,
};

#[derive(Default, Debug)]
struct LastLocations {
    read: ReadSet,
    // Consider [SmallVec] since most transactions explicitly write to 2 locations!
    write: Vec<MemoryLocationHash>,
}

type LazyAddresses = HashSet<Address, BuildSuffixHasher>;

/// The `MvMemory` contains shared memory in a form of a multi-version data
/// structure for values written and read by different transactions. It stores
/// multiple writes for each memory location, along with a value and an associated
/// version of a corresponding transaction.
#[derive(Debug)]
pub struct MvMemory {
    /// The list of transaction incarnations and written values for each memory location
    pub(crate) data: DashMap<MemoryLocationHash, BTreeMap<TxIdx, MemoryEntry>, BuildIdentityHasher>,
    /// Last read & written locations of each transaction
    last_locations: Vec<Mutex<LastLocations>>,
    /// Lazy addresses that need full evaluation at the end of the block
    lazy_addresses: Mutex<LazyAddresses>,
    /// New bytecodes deployed in this block
    pub(crate) new_bytecodes: DashMap<B256, Bytecode, BuildSuffixHasher>,

    // ── Diagnostics (only compiled with feature = "diagnostics") ────────────
    #[cfg(feature = "diagnostics")]
    pub(crate) reexec_total: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) reexec_read_changed: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) reexec_write_changed: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) cascade_aborts: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) total_aborts: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) blocking_estimate: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) blocking_nonce: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) blocking_retry: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) wrote_new_location: AtomicUsize,
    /// (location, tx) -> incarnation at which that transaction *first* wrote
    /// the location (reset if a later incarnation stops writing it). Lets
    /// validation tell whether the write that invalidated a read came from a
    /// first execution (ordinary optimistic abort) or a re-execution (cascade).
    #[cfg(feature = "diagnostics")]
    pub(crate) first_write_inc: DashMap<(MemoryLocationHash, TxIdx), TxIncarnation>,
    /// Block id (graph node `replica`) of every transaction, when the engine
    /// knows it; lets validation tell cross-block from intra-block aborts.
    #[cfg(feature = "diagnostics")]
    pub(crate) block_of: std::sync::OnceLock<Vec<u32>>,
    #[cfg(feature = "diagnostics")]
    pub(crate) cross_block_aborts: AtomicUsize,
    #[cfg(feature = "diagnostics")]
    pub(crate) cross_block_cascade: AtomicUsize,
    /// Whether tx_idx has ever recorded a (successful) incarnation. A
    /// transaction whose incarnation 0 was *blocked* (ESTIMATE / nonce) never
    /// records, so its first successful execution would otherwise be counted as
    /// "wrote a new location" against an empty previous write set even though
    /// nothing diverged. `wrote_new_location` only counts when a previous
    /// incarnation actually recorded.
    #[cfg(feature = "diagnostics")]
    pub(crate) recorded_once: Vec<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "diagnostics")]
    pub(crate) incarnation0_keys: Vec<Mutex<Option<(HashSet<MemoryLocationHash>, HashSet<MemoryLocationHash>)>>>,
}

impl MvMemory {
    pub(crate) fn new(
        block_size: usize,
        estimated_locations: impl IntoIterator<Item = (MemoryLocationHash, Vec<TxIdx>)>,
        lazy_addresses: impl IntoIterator<Item = Address>,
    ) -> Self {
        let data = DashMap::default();
        for (location_hash, estimated_tx_idxs) in estimated_locations {
            data.insert(
                location_hash,
                estimated_tx_idxs
                    .into_iter()
                    .map(|tx_idx| (tx_idx, MemoryEntry::Estimate))
                    .collect(),
            );
        }
        Self {
            data,
            last_locations: (0..block_size).map(|_| Mutex::default()).collect(),
            lazy_addresses: Mutex::new(LazyAddresses::from_iter(lazy_addresses)),
            new_bytecodes: DashMap::default(),
            #[cfg(feature = "diagnostics")]
            reexec_total: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            reexec_read_changed: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            reexec_write_changed: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            cascade_aborts: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            first_write_inc: DashMap::default(),
            #[cfg(feature = "diagnostics")]
            block_of: std::sync::OnceLock::new(),
            #[cfg(feature = "diagnostics")]
            cross_block_aborts: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            cross_block_cascade: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            total_aborts: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            blocking_estimate: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            blocking_nonce: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            blocking_retry: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            wrote_new_location: AtomicUsize::new(0),
            #[cfg(feature = "diagnostics")]
            recorded_once: (0..block_size).map(|_| std::sync::atomic::AtomicBool::new(false)).collect(),
            #[cfg(feature = "diagnostics")]
            incarnation0_keys: (0..block_size).map(|_| Mutex::new(None)).collect(),
        }
    }

    pub(crate) fn add_lazy_addresses(&self, new_lazy_addresses: impl IntoIterator<Item = Address>) {
        let mut lazy_addresses = self.lazy_addresses.lock().unwrap();
        for address in new_lazy_addresses {
            lazy_addresses.insert(address);
        }
    }

    pub(crate) fn record(
        &self,
        tx_version: &TxVersion,
        read_set: ReadSet,
        storage_read_keys: HashSet<MemoryLocationHash>,
        write_set: WriteSet,
    ) -> bool {
        let mut last_locations = index_mutex!(self.last_locations, tx_version.tx_idx);

        #[cfg(feature = "diagnostics")]
        {
            // Save incarnation-0 storage-only read keys + write keys for divergence analysis
            if tx_version.tx_incarnation == 0 {
                let w: HashSet<MemoryLocationHash> = write_set.iter().map(|(l, _)| *l).collect();
                *index_mutex!(self.incarnation0_keys, tx_version.tx_idx) = Some((storage_read_keys, w));
            }

            // Track access-set changes across re-executions
            if tx_version.tx_incarnation > 0 {
                self.reexec_total.fetch_add(1, Ordering::Relaxed);
                let prev_read: HashSet<MemoryLocationHash> =
                    last_locations.read.keys().copied().collect();
                let new_read: HashSet<MemoryLocationHash> = read_set.keys().copied().collect();
                if prev_read != new_read {
                    self.reexec_read_changed.fetch_add(1, Ordering::Relaxed);
                }
                let prev_write: HashSet<MemoryLocationHash> =
                    last_locations.write.iter().copied().collect();
                let new_write: HashSet<MemoryLocationHash> =
                    write_set.iter().map(|(l, _)| *l).collect();
                if prev_write != new_write {
                    self.reexec_write_changed.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        // suppress unused warning when diagnostics is off
        #[cfg(not(feature = "diagnostics"))]
        let _ = storage_read_keys;

        last_locations.read = read_set;

        let mut last_location_idx = 0;
        while last_location_idx < last_locations.write.len() {
            let prev_location = unsafe { last_locations.write.get_unchecked(last_location_idx) };
            if write_set.iter().all(|(l, _)| l != prev_location) {
                if let Some(mut written_transactions) = self.data.get_mut(prev_location) {
                    written_transactions.remove(&tx_version.tx_idx);
                }
                #[cfg(feature = "diagnostics")]
                self.first_write_inc.remove(&(*prev_location, tx_version.tx_idx));
                last_locations.write.swap_remove(last_location_idx);
            } else {
                last_location_idx += 1;
            }
        }

        let mut wrote_new_location = false;
        for (location, value) in write_set {
            self.data.entry(location).or_default().insert(
                tx_version.tx_idx,
                MemoryEntry::Data(tx_version.tx_incarnation, value),
            );
            if !last_locations.write.contains(&location) {
                last_locations.write.push(location);
                wrote_new_location = true;
                #[cfg(feature = "diagnostics")]
                self.first_write_inc.insert((location, tx_version.tx_idx), tx_version.tx_incarnation);
            }
        }

        #[cfg(feature = "diagnostics")]
        {
            let previously_recorded =
                self.recorded_once[tx_version.tx_idx].swap(true, Ordering::Relaxed);
            if wrote_new_location && tx_version.tx_incarnation > 0 && previously_recorded {
                self.wrote_new_location.fetch_add(1, Ordering::Relaxed);
            }
        }

        wrote_new_location
    }

    /// Snapshot the diagnostic counters into a plain struct. Every field but
    /// `re_executions` (filled in by the caller, which owns the scheduler)
    /// stays 0 unless `feature = "diagnostics"` is enabled.
    pub(crate) fn diagnostics(&self, block_size: usize) -> crate::ExecDiagnostics {
        #[allow(unused_mut)]
        let mut d = crate::ExecDiagnostics {
            block_size,
            ..Default::default()
        };
        #[cfg(feature = "diagnostics")]
        {
            d.validation_aborts = self.total_aborts.load(Ordering::Relaxed);
            d.cascade_aborts = self.cascade_aborts.load(Ordering::Relaxed);
            d.cross_block_aborts = self.cross_block_aborts.load(Ordering::Relaxed);
            d.cross_block_cascade = self.cross_block_cascade.load(Ordering::Relaxed);
            d.wrote_new_location = self.wrote_new_location.load(Ordering::Relaxed);
            d.blocking_estimate = self.blocking_estimate.load(Ordering::Relaxed);
            d.blocking_nonce = self.blocking_nonce.load(Ordering::Relaxed);
            d.blocking_retry = self.blocking_retry.load(Ordering::Relaxed);
        }
        d
    }

    /// Diagnostics: count a validation failure, and whether it is a *cascade*
    /// abort, i.e. the invalidating write was produced by a **re-execution** of
    /// a lower-indexed transaction (incarnation > 0, an ESTIMATE marker left by
    /// an aborted incarnation, or a write that a later incarnation no longer
    /// produces). A failure caused by the *first* execution of a lower-indexed
    /// writer that simply had not run yet is an ordinary optimistic abort.
    #[cfg(feature = "diagnostics")]
    fn record_abort(&self, cascade: bool, reader: TxIdx, writer: Option<TxIdx>) {
        self.total_aborts.fetch_add(1, Ordering::Relaxed);
        if cascade {
            self.cascade_aborts.fetch_add(1, Ordering::Relaxed);
        }
        if let (Some(ids), Some(w)) = (self.block_of.get(), writer) {
            if ids.get(w) != ids.get(reader) {
                self.cross_block_aborts.fetch_add(1, Ordering::Relaxed);
                if cascade {
                    self.cross_block_cascade.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
    #[cfg(not(feature = "diagnostics"))]
    #[inline(always)]
    fn record_abort(&self, _cascade: bool, _reader: TxIdx, _writer: Option<TxIdx>) {}

    /// Diagnostics: tell validation which block each transaction came from.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn set_block_ids(&self, ids: Vec<u32>) {
        let _ = self.block_of.set(ids);
    }
    #[cfg(not(feature = "diagnostics"))]
    #[inline(always)]
    pub(crate) fn set_block_ids(&self, _ids: Vec<u32>) {}

    /// Diagnostics: did `writer` first write `location` in a re-execution?
    #[cfg(feature = "diagnostics")]
    fn written_by_reexecution(&self, location: &MemoryLocationHash, writer: TxIdx) -> bool {
        self.first_write_inc.get(&(*location, writer)).map(|v| *v > 0).unwrap_or(false)
    }
    #[cfg(not(feature = "diagnostics"))]
    #[inline(always)]
    fn written_by_reexecution(&self, _location: &MemoryLocationHash, _writer: TxIdx) -> bool {
        false
    }

    pub(crate) fn validate_read_locations(&self, tx_idx: TxIdx) -> bool {
        for (location, prior_origins) in &index_mutex!(self.last_locations, tx_idx).read {
            if let Some(written_transactions) = self.data.get(location) {
                let mut iter = written_transactions.range(..tx_idx);
                for prior_origin in prior_origins {
                    let closest = iter.next_back();
                    match prior_origin {
                        ReadOrigin::MvMemory(prior_version) => match closest {
                            Some((closest_idx, MemoryEntry::Data(tx_incarnation, ..))) => {
                                if closest_idx != &prior_version.tx_idx
                                    || &prior_version.tx_incarnation != tx_incarnation
                                {
                                    // Same writer, new incarnation: its re-execution
                                    // changed the value we read -> cascade. A different
                                    // writer got in between: cascade only if it first
                                    // wrote this location in a re-execution; a first
                                    // execution that had not run yet is an ordinary abort.
                                    let cascade = closest_idx == &prior_version.tx_idx
                                        || self.written_by_reexecution(location, *closest_idx);
                                    self.record_abort(cascade, tx_idx, Some(*closest_idx));
                                    return false;
                                }
                            }
                            // ESTIMATE left by an aborted incarnation of the writer.
                            Some((closest_idx, MemoryEntry::Estimate)) => {
                                let cascade = closest_idx == &prior_version.tx_idx
                                    || self.written_by_reexecution(location, *closest_idx);
                                self.record_abort(cascade, tx_idx, Some(*closest_idx));
                                return false;
                            }
                            // The version we read has vanished: its writer re-executed
                            // and no longer writes here.
                            None => {
                                self.record_abort(true, tx_idx, Some(prior_version.tx_idx));
                                return false;
                            }
                        },
                        ReadOrigin::Storage => match closest {
                            None => {}
                            // We read storage believing no lower-indexed writer
                            // existed. Cascade iff the writer first produced this
                            // location in a re-execution; a first execution that had
                            // not run yet is the ordinary optimistic abort.
                            Some((closest_idx, _)) => {
                                self.record_abort(self.written_by_reexecution(location, *closest_idx), tx_idx, Some(*closest_idx));
                                return false;
                            }
                        },
                    }
                }
            } else if prior_origins.len() != 1 || prior_origins.last() != Some(&ReadOrigin::Storage)
            {
                // We read a multi-version entry that no longer exists at all: its
                // writer re-executed and dropped the location.
                let writer = prior_origins.iter().find_map(|o| match o {
                    ReadOrigin::MvMemory(v) => Some(v.tx_idx),
                    _ => None,
                });
                self.record_abort(true, tx_idx, writer);
                return false;
            }
        }
        true
    }

    pub(crate) fn convert_writes_to_estimates(&self, tx_idx: TxIdx) {
        for location in &index_mutex!(self.last_locations, tx_idx).write {
            if let Some(mut written_transactions) = self.data.get_mut(location) {
                written_transactions.insert(tx_idx, MemoryEntry::Estimate);
            }
        }
    }

    pub(crate) fn consume_lazy_addresses(&self) -> impl IntoIterator<Item = Address> {
        std::mem::take(&mut *self.lazy_addresses.lock().unwrap()).into_iter()
    }

    /// Return incarnation-0 (first parallel execution) access set keys per tx.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn get_incarnation0_keys(
        &self,
    ) -> Vec<Option<(HashSet<MemoryLocationHash>, HashSet<MemoryLocationHash>)>> {
        self.incarnation0_keys
            .iter()
            .map(|m| m.lock().unwrap().clone())
            .collect()
    }
}
