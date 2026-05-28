// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Per-commit scratch collections for retroactive validation.
//!
//! When `commit_layer` validates a new layer's effect on lower-layer
//! resources, it accumulates four collections — a dedup set of IRIs
//! already revalidated, a FIFO queue of IRIs pending revalidation, a
//! set of IRIs the cascade plans to tombstone, and a buffer of
//! validation errors. For deep cascades (e.g., a foundational
//! `core:Property` redef touching millions of lower-layer resources)
//! these collections can grow large.
//!
//! Three trait surfaces capture the operations the commit code
//! actually needs ([`IriSet`], [`IriQueue`], [`ViolationCollector`]),
//! plus a bundle ([`CommitWorkingSet`]) that ties them together. v1
//! ships in-memory implementations with a per-collection capacity cap
//! (default [`DEFAULT_WORKING_SET_CAP`] = 1M entries) that surfaces a
//! [`WorkingSetExhausted`] error before OOM. Future revisions can swap
//! in spill-to-disk implementations behind the same trait surface
//! without touching the commit code.
//!
//! Callers that drive many short commits in succession (e.g., the
//! gRPC server) should use [`CommitWorkingSetPool`] for RAII checkout
//! and automatic reset between commits. Single-shot callers can just
//! allocate with [`CommitWorkingSet::in_memory()`].

use crate::ontology::iri::Iri;
use crate::validation::ValidationError;
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

/// Default per-collection capacity for `CommitWorkingSet::in_memory()`.
///
/// Per-collection rather than total: each of `pending`, `revalidated`,
/// and `cascade_tombstones` can hold up to this many distinct IRIs.
/// The violation collector stores up to this many errors past which
/// it drops new ones but still increments its total count.
///
/// Sized for ontologies up to ~1M resources. Larger ontologies should
/// either use a smaller-cap working set with a stricter commit policy
/// or switch to a spilling implementation when that lands.
pub const DEFAULT_WORKING_SET_CAP: usize = 1_000_000;

/// Error returned when an [`IriSet`] or [`IriQueue`] would exceed its
/// configured capacity. Surfaces from `commit_layer` as
/// `CommitError::WorkingSetExhausted` so the caller can either raise
/// the cap or back off the commit.
///
/// The error names the collection that hit the cap so diagnostics can
/// pinpoint which side of the working set overflowed (e.g., a deep
/// cascade might exhaust `cascade_tombstones` first, while a wide
/// triple-index lookup might exhaust `pending`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingSetExhausted {
    pub collection: &'static str,
    pub cap: usize,
}

impl fmt::Display for WorkingSetExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "working-set collection '{}' exhausted at cap {}",
            self.collection, self.cap
        )
    }
}

impl std::error::Error for WorkingSetExhausted {}

// ─── Traits ────────────────────────────────────────────────────────────

/// Deduped set of IRIs with bounded capacity.
///
/// Used by the commit pass for `revalidated` (already-validated dedup)
/// and `cascade_tombstones` (the running tombstone set the cascade
/// plans to apply).
///
/// `insert` returns `Ok(true)` for a newly-added IRI, `Ok(false)` for
/// a duplicate, and `Err(WorkingSetExhausted)` when the cap is
/// reached.
pub trait IriSet: Send {
    fn insert(&mut self, iri: Iri) -> Result<bool, WorkingSetExhausted>;
    fn contains(&self, iri: &Iri) -> bool;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Iterate every IRI currently in the set, in ascending order by
    /// the underlying storage's natural ordering. The returned
    /// iterator must not outlive the next mutating call.
    fn iter(&self) -> Box<dyn Iterator<Item = Iri> + '_>;
    /// Drop every entry. Used by pool implementations to recycle the
    /// allocation between commits.
    fn clear(&mut self);
}

/// FIFO work queue with dedup. `push` is a no-op for IRIs the queue
/// has already seen (whether still pending or already popped). `pop`
/// returns each IRI at most once.
///
/// The dedup discipline matters: in a fixpoint cascade the same IRI
/// might be discovered as a revalidation target through multiple
/// paths (e.g., it's both an instance of a redefined class and the
/// target of a tombstoned IRI reference). The queue ensures we
/// validate each at most once per commit.
pub trait IriQueue: IriSet {
    fn push(&mut self, iri: Iri) -> Result<bool, WorkingSetExhausted>;
    fn pop(&mut self) -> Option<Iri>;
}

/// Result of [`ViolationCollector::drain`].
///
/// `errors` holds up to the caller's `max` cap. `total` is the
/// cumulative count of `push` calls across the commit, including any
/// errors the collector dropped past its own capacity. The
/// `total - errors.len()` delta tells the caller how many errors
/// were truncated.
#[derive(Debug, Clone)]
pub struct DrainedViolations {
    pub errors: Vec<ValidationError>,
    pub total: usize,
}

/// Accumulator for validation errors during a commit's retroactive
/// pass.
///
/// Unlike [`IriSet`] / [`IriQueue`], hitting the collector's internal
/// cap is *not* a hard error — the implementation drops the error
/// payload but continues to increment `len()`. This is deliberate:
/// validation errors are diagnostic, the caller always supplies a
/// truncation cap on top, and silently dropping extras past that cap
/// is what callers expect.
pub trait ViolationCollector: Send {
    fn push(&mut self, error: ValidationError);
    /// Total count of errors pushed, including any dropped past the
    /// implementation's internal cap.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Return up to `max` stored errors along with the true total
    /// count of errors pushed. Drains the collector to empty (next
    /// `len()` is 0).
    fn drain(&mut self, max: usize) -> DrainedViolations;
    fn clear(&mut self);
}

// ─── Bundle ────────────────────────────────────────────────────────────

/// Bundle of the four collections the commit's retroactive validation
/// pass needs. Constructed via [`Self::in_memory`] for the standard
/// in-memory implementation, or by hand from custom impls.
pub struct CommitWorkingSet {
    pub pending: Box<dyn IriQueue>,
    pub revalidated: Box<dyn IriSet>,
    pub cascade_tombstones: Box<dyn IriSet>,
    pub violations: Box<dyn ViolationCollector>,
}

impl CommitWorkingSet {
    /// In-memory bundle with the default per-collection capacity
    /// ([`DEFAULT_WORKING_SET_CAP`] = 1M entries each).
    pub fn in_memory() -> Self {
        Self::in_memory_with_cap(DEFAULT_WORKING_SET_CAP)
    }

    /// In-memory bundle with a custom per-collection capacity. Each
    /// of `pending`, `revalidated`, and `cascade_tombstones` will
    /// reject inserts past `cap` with [`WorkingSetExhausted`]; the
    /// violation collector silently truncates past `cap` while still
    /// counting.
    pub fn in_memory_with_cap(cap: usize) -> Self {
        Self {
            pending: Box::new(InMemoryIriQueue::new(cap)),
            revalidated: Box::new(InMemoryIriSet::new("revalidated", cap)),
            cascade_tombstones: Box::new(InMemoryIriSet::new("cascade_tombstones", cap)),
            violations: Box::new(InMemoryViolationCollector::new(cap)),
        }
    }

    /// Reset all four collections in place. Used by
    /// [`CommitWorkingSetPool`] to recycle a working set between
    /// commits without dropping its underlying allocations.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.revalidated.clear();
        self.cascade_tombstones.clear();
        self.violations.clear();
    }
}

impl fmt::Debug for CommitWorkingSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommitWorkingSet")
            .field("pending_len", &self.pending.len())
            .field("revalidated_len", &self.revalidated.len())
            .field("cascade_tombstones_len", &self.cascade_tombstones.len())
            .field("violations_len", &self.violations.len())
            .finish()
    }
}

// ─── In-memory implementations ─────────────────────────────────────────

/// `BTreeSet`-backed [`IriSet`] with a cap.
pub struct InMemoryIriSet {
    name: &'static str,
    cap: usize,
    set: BTreeSet<Iri>,
}

impl InMemoryIriSet {
    pub fn new(name: &'static str, cap: usize) -> Self {
        Self {
            name,
            cap,
            set: BTreeSet::new(),
        }
    }
}

impl IriSet for InMemoryIriSet {
    fn insert(&mut self, iri: Iri) -> Result<bool, WorkingSetExhausted> {
        if self.set.contains(&iri) {
            return Ok(false);
        }
        if self.set.len() >= self.cap {
            return Err(WorkingSetExhausted {
                collection: self.name,
                cap: self.cap,
            });
        }
        self.set.insert(iri);
        Ok(true)
    }
    fn contains(&self, iri: &Iri) -> bool {
        self.set.contains(iri)
    }
    fn len(&self) -> usize {
        self.set.len()
    }
    fn iter(&self) -> Box<dyn Iterator<Item = Iri> + '_> {
        Box::new(self.set.iter().cloned())
    }
    fn clear(&mut self) {
        self.set.clear();
    }
}

/// `VecDeque`-backed [`IriQueue`] with a `BTreeSet` dedup side-table.
///
/// Memory budget: roughly two copies of each IRI inserted (one in the
/// queue, one in the dedup set), bounded by `cap`.
pub struct InMemoryIriQueue {
    cap: usize,
    queue: VecDeque<Iri>,
    seen: BTreeSet<Iri>,
}

impl InMemoryIriQueue {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            queue: VecDeque::new(),
            seen: BTreeSet::new(),
        }
    }
}

impl IriSet for InMemoryIriQueue {
    fn insert(&mut self, iri: Iri) -> Result<bool, WorkingSetExhausted> {
        self.push(iri)
    }
    fn contains(&self, iri: &Iri) -> bool {
        self.seen.contains(iri)
    }
    fn len(&self) -> usize {
        self.seen.len()
    }
    fn iter(&self) -> Box<dyn Iterator<Item = Iri> + '_> {
        Box::new(self.seen.iter().cloned())
    }
    fn clear(&mut self) {
        self.queue.clear();
        self.seen.clear();
    }
}

impl IriQueue for InMemoryIriQueue {
    fn push(&mut self, iri: Iri) -> Result<bool, WorkingSetExhausted> {
        if self.seen.contains(&iri) {
            return Ok(false);
        }
        if self.seen.len() >= self.cap {
            return Err(WorkingSetExhausted {
                collection: "pending",
                cap: self.cap,
            });
        }
        self.seen.insert(iri.clone());
        self.queue.push_back(iri);
        Ok(true)
    }
    fn pop(&mut self) -> Option<Iri> {
        // `seen` retains the popped IRI so re-pushes are no-ops for
        // the duration of the commit — that's the dedup discipline
        // documented on the trait.
        self.queue.pop_front()
    }
}

/// `Vec`-backed [`ViolationCollector`] with a soft cap.
///
/// Stores at most `cap` errors. Pushes past the cap drop the payload
/// but continue to bump `total`, so `len()` always reports the true
/// count and `drain` can surface the truncation gap.
pub struct InMemoryViolationCollector {
    cap: usize,
    stored: Vec<ValidationError>,
    total: usize,
}

impl InMemoryViolationCollector {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            stored: Vec::new(),
            total: 0,
        }
    }
}

impl ViolationCollector for InMemoryViolationCollector {
    fn push(&mut self, error: ValidationError) {
        self.total += 1;
        if self.stored.len() < self.cap {
            self.stored.push(error);
        }
    }
    fn len(&self) -> usize {
        self.total
    }
    fn drain(&mut self, max: usize) -> DrainedViolations {
        let take = max.min(self.stored.len());
        let errors: Vec<ValidationError> = self.stored.drain(..take).collect();
        let total = self.total;
        self.stored.clear();
        self.total = 0;
        DrainedViolations { errors, total }
    }
    fn clear(&mut self) {
        self.stored.clear();
        self.total = 0;
    }
}

// ─── Pool ──────────────────────────────────────────────────────────────

/// Pool of pre-allocated [`CommitWorkingSet`] instances.
///
/// Sessions that drive many short commits (the gRPC server is the
/// canonical caller) take a pool at startup and acquire one set per
/// commit. Each acquire returns a [`PooledWorkingSet`] RAII guard;
/// on drop the working set is `reset()` and returned to the pool, so
/// the next acquire skips reallocation.
///
/// The pool is thread-safe; multiple commits can `acquire()` in
/// parallel, each getting their own working set (a fresh allocation
/// if the pool is empty).
pub struct CommitWorkingSetPool {
    inner: Mutex<Vec<CommitWorkingSet>>,
    factory: Box<dyn Fn() -> CommitWorkingSet + Send + Sync>,
}

impl CommitWorkingSetPool {
    /// Pool that allocates `CommitWorkingSet::in_memory()` instances
    /// on demand. The default factory.
    pub fn in_memory() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            factory: Box::new(CommitWorkingSet::in_memory),
        }
    }

    /// Pool with a custom factory — for callers that want non-default
    /// caps or alternative working-set implementations.
    pub fn with_factory(factory: impl Fn() -> CommitWorkingSet + Send + Sync + 'static) -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            factory: Box::new(factory),
        }
    }

    /// Acquire a working set. Returns an existing one from the pool
    /// if available, otherwise allocates via the factory. The
    /// returned guard `reset()`s and returns to the pool on drop.
    pub fn acquire(&self) -> PooledWorkingSet<'_> {
        let inner = self
            .inner
            .lock()
            .expect("CommitWorkingSetPool poisoned")
            .pop()
            .unwrap_or_else(|| (self.factory)());
        PooledWorkingSet {
            inner: Some(inner),
            pool: self,
        }
    }

    /// Number of working sets currently sitting in the pool. Diagnostic.
    /// Excludes any that are checked out via [`acquire`].
    pub fn idle_count(&self) -> usize {
        self.inner
            .lock()
            .expect("CommitWorkingSetPool poisoned")
            .len()
    }
}

impl Default for CommitWorkingSetPool {
    fn default() -> Self {
        Self::in_memory()
    }
}

/// RAII guard returned by [`CommitWorkingSetPool::acquire`]. Derefs to
/// [`CommitWorkingSet`]; on drop, the set is reset and returned to
/// the pool for the next caller.
pub struct PooledWorkingSet<'a> {
    inner: Option<CommitWorkingSet>,
    pool: &'a CommitWorkingSetPool,
}

impl<'a> Deref for PooledWorkingSet<'a> {
    type Target = CommitWorkingSet;
    fn deref(&self) -> &Self::Target {
        self.inner
            .as_ref()
            .expect("PooledWorkingSet already dropped")
    }
}

impl<'a> DerefMut for PooledWorkingSet<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
            .as_mut()
            .expect("PooledWorkingSet already dropped")
    }
}

impl<'a> Drop for PooledWorkingSet<'a> {
    fn drop(&mut self) {
        if let Some(mut ws) = self.inner.take() {
            ws.reset();
            self.pool
                .inner
                .lock()
                .expect("CommitWorkingSetPool poisoned")
                .push(ws);
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::ValidationRule;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn err(id: &str) -> ValidationError {
        ValidationError {
            resource_id: Some(iri(id)),
            property: None,
            rule: ValidationRule::MissingRequired,
            message: "test".into(),
        }
    }

    // ── IriSet ──

    #[test]
    fn iri_set_insert_dedup() {
        let mut s = InMemoryIriSet::new("test", 10);
        assert!(s.insert(iri("urn:eigenius:demo:a")).unwrap());
        assert!(!s.insert(iri("urn:eigenius:demo:a")).unwrap());
        assert!(s.insert(iri("urn:eigenius:demo:b")).unwrap());
        assert_eq!(s.len(), 2);
        assert!(s.contains(&iri("urn:eigenius:demo:a")));
        assert!(s.contains(&iri("urn:eigenius:demo:b")));
        assert!(!s.contains(&iri("urn:eigenius:demo:c")));
    }

    #[test]
    fn iri_set_cap_rejects_with_named_error() {
        let mut s = InMemoryIriSet::new("revalidated", 2);
        s.insert(iri("urn:eigenius:demo:a")).unwrap();
        s.insert(iri("urn:eigenius:demo:b")).unwrap();
        let err = s.insert(iri("urn:eigenius:demo:c")).unwrap_err();
        assert_eq!(err.collection, "revalidated");
        assert_eq!(err.cap, 2);
    }

    #[test]
    fn iri_set_cap_allows_dedup_at_cap() {
        // Duplicates of already-stored entries succeed even at cap —
        // they don't grow the set.
        let mut s = InMemoryIriSet::new("test", 2);
        s.insert(iri("urn:eigenius:demo:a")).unwrap();
        s.insert(iri("urn:eigenius:demo:b")).unwrap();
        assert!(!s.insert(iri("urn:eigenius:demo:a")).unwrap());
    }

    #[test]
    fn iri_set_clear_resets_state() {
        let mut s = InMemoryIriSet::new("test", 10);
        s.insert(iri("urn:eigenius:demo:a")).unwrap();
        s.insert(iri("urn:eigenius:demo:b")).unwrap();
        s.clear();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert!(!s.contains(&iri("urn:eigenius:demo:a")));
        // Can re-fill to the cap after clear.
        for i in 0..10 {
            s.insert(iri(&format!("urn:eigenius:demo:{i}"))).unwrap();
        }
    }

    // ── IriQueue ──

    #[test]
    fn iri_queue_fifo_with_dedup() {
        let mut q = InMemoryIriQueue::new(10);
        q.push(iri("urn:eigenius:demo:a")).unwrap();
        q.push(iri("urn:eigenius:demo:b")).unwrap();
        q.push(iri("urn:eigenius:demo:a")).unwrap(); // duplicate, no-op
        q.push(iri("urn:eigenius:demo:c")).unwrap();
        assert_eq!(q.len(), 3);

        assert_eq!(q.pop(), Some(iri("urn:eigenius:demo:a")));
        assert_eq!(q.pop(), Some(iri("urn:eigenius:demo:b")));
        assert_eq!(q.pop(), Some(iri("urn:eigenius:demo:c")));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn iri_queue_pop_does_not_un_dedup() {
        // After popping an IRI, re-pushing it is still a no-op for
        // the duration of this commit. The dedup contract is "at
        // most once per commit," not "at most once concurrently."
        let mut q = InMemoryIriQueue::new(10);
        q.push(iri("urn:eigenius:demo:a")).unwrap();
        assert_eq!(q.pop(), Some(iri("urn:eigenius:demo:a")));
        assert!(!q.push(iri("urn:eigenius:demo:a")).unwrap());
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn iri_queue_cap() {
        let mut q = InMemoryIriQueue::new(2);
        q.push(iri("urn:eigenius:demo:a")).unwrap();
        q.push(iri("urn:eigenius:demo:b")).unwrap();
        let e = q.push(iri("urn:eigenius:demo:c")).unwrap_err();
        assert_eq!(e.collection, "pending");
        assert_eq!(e.cap, 2);
    }

    // ── ViolationCollector ──

    #[test]
    fn violations_basic_push_drain() {
        let mut v = InMemoryViolationCollector::new(10);
        v.push(err("urn:eigenius:demo:a"));
        v.push(err("urn:eigenius:demo:b"));
        assert_eq!(v.len(), 2);
        let drained = v.drain(10);
        assert_eq!(drained.errors.len(), 2);
        assert_eq!(drained.total, 2);
        // Drain leaves the collector empty.
        assert_eq!(v.len(), 0);
        assert!(v.is_empty());
    }

    #[test]
    fn violations_drain_caps_returned_errors_but_reports_full_stored() {
        let mut v = InMemoryViolationCollector::new(100);
        for i in 0..20 {
            v.push(err(&format!("urn:eigenius:demo:{i}")));
        }
        let drained = v.drain(5);
        assert_eq!(drained.errors.len(), 5);
        assert_eq!(drained.total, 20);
    }

    #[test]
    fn violations_cap_drops_payload_but_counts() {
        // Past the internal cap, errors are dropped but `len()` still
        // tracks the true total so callers can surface "X of Y."
        let mut v = InMemoryViolationCollector::new(3);
        for i in 0..10 {
            v.push(err(&format!("urn:eigenius:demo:{i}")));
        }
        assert_eq!(v.len(), 10);
        let drained = v.drain(100);
        assert_eq!(drained.errors.len(), 3); // only 3 actually stored
        assert_eq!(drained.total, 10); // but total reflects all 10
    }

    // ── CommitWorkingSet ──

    #[test]
    fn working_set_reset_clears_all() {
        let mut ws = CommitWorkingSet::in_memory();
        ws.pending.push(iri("urn:eigenius:demo:p")).unwrap();
        ws.revalidated.insert(iri("urn:eigenius:demo:r")).unwrap();
        ws.cascade_tombstones
            .insert(iri("urn:eigenius:demo:t"))
            .unwrap();
        ws.violations.push(err("urn:eigenius:demo:v"));

        ws.reset();

        assert!(ws.pending.is_empty());
        assert!(ws.revalidated.is_empty());
        assert!(ws.cascade_tombstones.is_empty());
        assert!(ws.violations.is_empty());
    }

    #[test]
    fn working_set_in_memory_uses_default_cap() {
        let ws = CommitWorkingSet::in_memory();
        // Smoke: trait surface compiles and the constructors agree.
        assert_eq!(ws.pending.len(), 0);
        assert_eq!(ws.revalidated.len(), 0);
        assert_eq!(ws.cascade_tombstones.len(), 0);
        assert_eq!(ws.violations.len(), 0);
    }

    #[test]
    fn working_set_custom_cap_is_per_collection() {
        let mut ws = CommitWorkingSet::in_memory_with_cap(2);
        ws.pending.push(iri("urn:eigenius:demo:a")).unwrap();
        ws.pending.push(iri("urn:eigenius:demo:b")).unwrap();
        let e = ws.pending.push(iri("urn:eigenius:demo:c")).unwrap_err();
        assert_eq!(e.cap, 2);
        // Other collections still have their own quota.
        ws.revalidated.insert(iri("urn:eigenius:demo:x")).unwrap();
        ws.revalidated.insert(iri("urn:eigenius:demo:y")).unwrap();
    }

    // ── Pool ──

    #[test]
    fn pool_acquire_allocates_when_empty() {
        let pool = CommitWorkingSetPool::in_memory();
        assert_eq!(pool.idle_count(), 0);
        let g = pool.acquire();
        assert_eq!(pool.idle_count(), 0);
        drop(g);
        assert_eq!(pool.idle_count(), 1);
    }

    #[test]
    fn pool_acquire_reuses_existing_after_drop() {
        let pool = CommitWorkingSetPool::in_memory();
        let mut g = pool.acquire();
        g.pending.push(iri("urn:eigenius:demo:a")).unwrap();
        drop(g);
        // Acquire again — should pop the same allocation, reset.
        let g2 = pool.acquire();
        assert!(g2.pending.is_empty());
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn pool_handles_multiple_concurrent_acquires() {
        let pool = CommitWorkingSetPool::in_memory();
        let g1 = pool.acquire();
        let g2 = pool.acquire();
        let g3 = pool.acquire();
        // Three separate working sets, none in the pool.
        assert_eq!(pool.idle_count(), 0);
        drop(g1);
        drop(g2);
        drop(g3);
        assert_eq!(pool.idle_count(), 3);
    }

    #[test]
    fn pool_with_custom_factory() {
        let pool = CommitWorkingSetPool::with_factory(|| CommitWorkingSet::in_memory_with_cap(5));
        let mut g = pool.acquire();
        for i in 0..5 {
            g.pending
                .push(iri(&format!("urn:eigenius:demo:{i}")))
                .unwrap();
        }
        let e = g
            .pending
            .push(iri("urn:eigenius:demo:overflow"))
            .unwrap_err();
        assert_eq!(e.cap, 5);
    }

    #[test]
    fn pool_dropped_guard_resets_borrowed_set() {
        // The guard's Drop calls reset(); the next acquire sees a
        // clean working set even though it's the same allocation.
        let pool = CommitWorkingSetPool::in_memory();
        {
            let mut g = pool.acquire();
            g.pending.push(iri("urn:eigenius:demo:a")).unwrap();
            g.violations.push(err("urn:eigenius:demo:e"));
        }
        let g2 = pool.acquire();
        assert!(g2.pending.is_empty());
        assert!(g2.violations.is_empty());
    }
}
