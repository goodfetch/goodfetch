//! The process Plugins whose footprint the Resource governor measures at each
//! sample (ADRs 0033, 0040 and 0140, issue 867): `plugin_host` registers each
//! started Plugin instance's OS process with [`Governor::register_plugin`],
//! which returns a [`GovernedPlugin`] handle. At each `governor.sample_interval`,
//! the governor's sampler reads every registered Plugin's footprint once,
//! together with the browsers', and publishes it on the handle's `watch`
//! channel. Dropping the handle when the instance stops unregisters it.
#![expect(
    clippy::disallowed_types,
    reason = "a Plugin leaves the table as its handle drops, which cannot wait: one Mutex guards the table for a few reads and writes, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

/// Every registered process Plugin, shared by the governor and its sampler.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plugins(Arc<Mutex<Table>>);

#[derive(Debug, Default)]
struct Table {
    next: u64,
    list: BTreeMap<u64, Entry>,
}

#[derive(Debug)]
struct Entry {
    process: u32,
    footprint: watch::Sender<Option<u64>>,
}

/// One registered process Plugin instance's footprint watch handle. Dropping
/// it unregisters the process from the governor's sampler.
#[derive(Debug)]
pub struct GovernedPlugin {
    plugins: Plugins,
    id: u64,
    footprint: watch::Receiver<Option<u64>>,
}

impl Plugins {
    fn table(&self) -> MutexGuard<'_, Table> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `process` to be measured once per governor sample until the
    /// returned [`GovernedPlugin`] drops.
    pub(crate) fn register(&self, process: u32) -> GovernedPlugin {
        let (sender, footprint) = watch::channel(None);
        let mut table = self.table();
        table.next += 1;
        let id = table.next;
        table.list.insert(
            id,
            Entry {
                process,
                footprint: sender,
            },
        );
        GovernedPlugin {
            plugins: self.clone(),
            id,
            footprint,
        }
    }

    /// Every registered Plugin's `(id, pid)` pair to measure in this sample.
    pub(crate) fn processes(&self) -> Vec<(u64, u32)> {
        self.table()
            .list
            .iter()
            .map(|(id, entry)| (*id, entry.process))
            .collect()
    }

    /// Publishes the measured footprint in bytes for each registered Plugin.
    pub(crate) fn measured(&self, footprints: &[(u64, u64)]) {
        let table = self.table();
        for (id, bytes) in footprints {
            if let Some(entry) = table.list.get(id) {
                entry.footprint.send_replace(Some(*bytes));
            }
        }
    }
}

impl GovernedPlugin {
    /// The last physical footprint the governor measured for this Plugin
    /// process, in bytes; `None` before its first sample.
    #[must_use]
    pub fn footprint(&self) -> Option<u64> {
        *self.footprint.borrow()
    }

    /// A `watch` receiver updated once per governor sample with this Plugin
    /// process's latest physical footprint in bytes.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<Option<u64>> {
        self.footprint.clone()
    }
}

impl Drop for GovernedPlugin {
    fn drop(&mut self) {
        self.plugins.table().list.remove(&self.id);
    }
}
