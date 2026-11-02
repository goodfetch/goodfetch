//! The one atomic of this crate (ADR 0060), the one place `disallowed-types`
//! lets it be named. Under `goodfetch_loom` this module would map it to loom;
//! a count is a relaxed `fetch_add` nothing synchronizes on, which orders
//! nothing for loom to explore, so no loom test reaches it and the crate takes
//! no dependency on loom (agent decision of #1840, ADR 0140's notes).
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its atomic is named (ADR 0060)"
)]

use std::sync::atomic::{AtomicU64, Ordering};

/// A count one thread may add to while another reads.
#[derive(Debug, Default)]
pub(crate) struct Count(AtomicU64);

impl Count {
    /// Adds one.
    pub(crate) fn add_one(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// What it holds: some count that was true.
    pub(crate) fn read(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}
