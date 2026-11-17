//! The Run's memory budget, `run_state.memory.budget` (#2169), which the
//! Frontier's visits share with the Run state's pending work items: each
//! holder keeps what it holds within the budget less what the other holds
//! now, so the two together stay within it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The Run's memory budget, shared by the Frontier's visits and the Run
/// state's pending work items. Clones share the bytes the pending work
/// items hold.
#[derive(Clone, Debug)]
pub struct SharedBudget {
    /// The budget's bytes.
    bytes: u64,
    /// The bytes the Run state's pending work items hold now.
    pending: Arc<AtomicU64>,
}

impl SharedBudget {
    /// A budget of `bytes`, none of it held by pending work items yet.
    #[must_use]
    pub fn new(bytes: u64) -> Self {
        Self {
            bytes,
            pending: Arc::default(),
        }
    }

    /// The budget's bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Tells that the Run state's pending work items hold `bytes` now.
    pub fn pending_hold(&self, bytes: u64) {
        self.pending.store(bytes, Ordering::Relaxed);
    }

    /// The bytes the Run state's pending work items hold now.
    #[must_use]
    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Relaxed)
    }

    /// The bytes the Frontier's visits may hold now: the budget less what
    /// the pending work items hold.
    pub(crate) fn room(&self) -> u64 {
        self.bytes.saturating_sub(self.pending())
    }
}

/// The bytes a hash table with room for `capacity` entries of `T` takes,
/// as hashbrown's lays it out, std's too: for each bucket a slot and a
/// control byte, one bucket in eight kept empty past seven, and a group
/// of control bytes more. A table never gives back the room it grew to,
/// and grows past what it holds as entries leave and others come, so its
/// room, not its entries, is what it costs: each holder of the budget
/// counts its tables so.
#[must_use]
pub const fn hash_table_bytes<T>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = if capacity < 8 {
        capacity + 1
    } else {
        capacity / 7 * 8
    };
    buckets * (size_of::<T>() + 1) + CONTROL_GROUP
}

/// The control bytes a hash table holds past its buckets.
const CONTROL_GROUP: usize = 16;
