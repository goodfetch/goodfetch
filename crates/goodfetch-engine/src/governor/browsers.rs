//! The browsers the governor counts and sheds (ADR 0040): each browser a
//! Run's pool launched joins through the Run's share with its process,
//! and reports how many pages it holds open and whether one is held. At
//! each sample the governor measures every one, the process and its
//! descendants as it counts the engine's (PSS on Linux); at critical it
//! sheds them by the shedding ladder's rule (`ladder`), and the pool closes
//! or recycles each browser shed. A Run's footprint, which the last resort
//! weighs, is its browsers', since Runs never share a browser process. A
//! Borrowed browser never joins, so it is neither shed nor counted.
#![expect(
    clippy::disallowed_types,
    reason = "a browser leaves the table as its handle drops, which cannot wait: one Mutex guards the table for a few reads and writes, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

use super::ladder::{self, Candidate, Weight};

/// Whether a browser holds a page that keeps it from being shed, and who
/// holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageHold {
    /// None of its pages is held.
    #[default]
    None,
    /// A page in a Human pause or under a raw CDP attach bound to its work
    /// item's lease: the browser is never shed while it holds one.
    Held,
    /// A page under a person's control in the Live view: the browser is
    /// never shed, and its Run is stopped last, only once the person hands
    /// back or leaves.
    Person,
}

/// What a browser reports of its use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrowserUse {
    /// The pages it holds open.
    pub open_pages: usize,
    /// Whether one of them is held.
    pub held: PageHold,
}

/// Every browser that joined, which clones share.
#[derive(Clone, Debug, Default)]
pub(crate) struct Browsers(Arc<Mutex<Table>>);

#[derive(Debug, Default)]
struct Table {
    next: u64,
    list: BTreeMap<u64, Entry>,
}

#[derive(Debug)]
struct Entry {
    /// The Run's share it joined through.
    run: u64,
    process: u32,
    usage: BrowserUse,
    /// What it used at the last sample, once measured.
    footprint: Option<u64>,
    /// Whether the governor shed it.
    shed: watch::Sender<bool>,
}

/// A browser of a Run's pool, as the governor counts and sheds it, until
/// the value drops.
#[derive(Debug)]
pub struct GovernedBrowser {
    browsers: Browsers,
    id: u64,
    shed: watch::Receiver<bool>,
}

impl Browsers {
    fn table(&self) -> MutexGuard<'_, Table> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The browser running as `process` joins for the Run `run`.
    pub(crate) fn join(&self, run: u64, process: u32) -> GovernedBrowser {
        let (sender, shed) = watch::channel(false);
        let mut table = self.table();
        table.next += 1;
        let id = table.next;
        table.list.insert(
            id,
            Entry {
                run,
                process,
                usage: BrowserUse::default(),
                footprint: None,
                shed: sender,
            },
        );
        GovernedBrowser {
            browsers: self.clone(),
            id,
            shed,
        }
    }

    /// Every browser's process, by its number, to measure.
    pub(crate) fn processes(&self) -> Vec<(u64, u32)> {
        self.table()
            .list
            .iter()
            .map(|(id, entry)| (*id, entry.process))
            .collect()
    }

    /// Keeps what each browser, by its number, used at this sample.
    pub(crate) fn measured(&self, footprints: &[(u64, u64)]) {
        let mut table = self.table();
        for (id, footprint) in footprints {
            if let Some(entry) = table.list.get_mut(id) {
                entry.footprint = Some(*footprint);
            }
        }
    }

    /// Sheds the browsers the ladder names at critical, telling each, and
    /// answers their processes.
    pub(crate) fn shed_at_critical(&self) -> Vec<u32> {
        let table = self.table();
        let candidates: Vec<Candidate> = table
            .list
            .iter()
            .map(|(id, entry)| Candidate {
                id: *id,
                open_pages: entry.usage.open_pages,
                held: entry.usage.held != PageHold::None,
                footprint: entry.footprint.unwrap_or(0),
                shed: *entry.shed.borrow(),
            })
            .collect();
        ladder::browsers_to_shed(&candidates)
            .into_iter()
            .filter_map(|id| {
                let entry = table.list.get(&id)?;
                entry.shed.send_replace(true);
                Some(entry.process)
            })
            .collect()
    }

    /// The Run `run`'s weight: its browsers' footprint, and whether a
    /// person holds a page of one.
    pub(crate) fn weight(&self, run: u64) -> Weight {
        let table = self.table();
        let of_run = table.list.values().filter(|entry| entry.run == run);
        Weight {
            footprint: of_run
                .clone()
                .map(|entry| entry.footprint.unwrap_or(0))
                .sum(),
            held_by_person: of_run
                .clone()
                .any(|entry| entry.usage.held == PageHold::Person),
        }
    }
}

impl GovernedBrowser {
    /// Reports how many pages it holds open and whether one is held.
    pub fn report(&self, usage: BrowserUse) {
        if let Some(entry) = self.browsers.table().list.get_mut(&self.id) {
            entry.usage = usage;
        }
    }

    /// Answers once the governor shed it: the pool closes it when it holds
    /// no open page, and recycles it otherwise. The future holds no borrow
    /// of the browser, so a task may wait on it while the pool keeps it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: a later call answers at once.
    pub fn shed(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut shed = self.shed.clone();
        async move {
            let _ = shed.wait_for(|shed| *shed).await;
        }
    }
}

impl Drop for GovernedBrowser {
    fn drop(&mut self) {
        self.browsers.table().list.remove(&self.id);
    }
}
