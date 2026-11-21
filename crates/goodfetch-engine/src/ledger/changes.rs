//! How many times this process changed the Limits of one ledger file (ADR
//! 0140, #1888): the count a run compares at each charge to know that the
//! Limits it holds in memory are still the ledger's, with no look at the
//! file. Every [`Ledger`](super::Ledger) this process makes of one path
//! shares one count, whichever engine made it. Engines that name one file
//! by different paths count apart, as do engines of other processes: their
//! changes reach a run when it reads its Limits again, before a Slice or
//! after its `usage.ledger_read_interval` (ADR 0041, issue 2602).

#![expect(
    clippy::disallowed_types,
    reason = "a count of changes nothing else synchronizes on, and the registry that shares it, locked for one lookup with nothing else under it: no lock order for loom to test, and no loom model opens a ledger (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// The changes made to the Limits of one ledger file through this
/// process's ledgers.
#[derive(Debug, Default)]
pub(super) struct Changes(AtomicU64);

impl Changes {
    /// The changes of the ledger file at `path`, one count for every
    /// ledger this process makes of that path, as SQLite keeps one state
    /// for a process's connections to one file.
    pub(super) fn of(path: &Path) -> Arc<Self> {
        static FILES: Mutex<BTreeMap<PathBuf, Weak<Changes>>> = Mutex::new(BTreeMap::new());
        let mut files = FILES.lock().unwrap_or_else(PoisonError::into_inner);
        files.retain(|_, changes| changes.strong_count() > 0);
        if let Some(changes) = files.get(path).and_then(Weak::upgrade) {
            return changes;
        }
        let changes = Arc::new(Self::default());
        files.insert(path.to_owned(), Arc::downgrade(&changes));
        changes
    }

    /// How many there were so far.
    pub(super) fn count(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    /// Counts one more, committed.
    pub(super) fn add(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}
