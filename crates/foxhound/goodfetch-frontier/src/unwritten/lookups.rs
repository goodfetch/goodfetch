//! The visited set's lookups a links batch makes on a reader before its
//! job, while memory does not hold every visit (#2169, G-11 Q6): the
//! writer then answers every batch whose identities memory holds while a
//! batch's read of the run file is pending, and runs no read of its own
//! for the identities a batch looked up.
//!
//! A batch marks each identity it reads as being read, so a second batch
//! that holds one waits for that read's answer instead of reading it
//! again. An answer stands until memory notes a visit of its identity,
//! which makes it stale: a job then reads the file itself, as before. The
//! entries take no part in the budget: each lasts until the last batch
//! that uses it is written.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

use super::{Held, Space, UnwrittenVisits};

/// The identities batches look up, each with its answer.
#[derive(Debug, Default)]
pub(super) struct Lookups {
    entries: HashMap<Arc<str>, Entry>,
    /// How many batches looked identities up: the next one's number.
    batches: u64,
}

#[derive(Debug)]
struct Entry {
    answer: Answer,
    /// How many batches not written yet use it.
    users: usize,
}

#[derive(Debug)]
enum Answer {
    /// The batch numbered reads it, and says on `read` once it did.
    Reading {
        batch: u64,
        read: watch::Receiver<bool>,
    },
    /// The crawl request the file holds it for, or none.
    Read(Option<Box<str>>),
    /// Memory noted a visit of it since: a job reads it again.
    Stale,
}

impl Lookups {
    /// The crawl request the file holds `identity` for, `Some(None)` for
    /// none, while a read's answer stands.
    #[expect(
        clippy::option_option,
        reason = "whether a read's answer stands, then the visitor it read"
    )]
    pub(super) fn answer(&self, identity: &str) -> Option<Option<&str>> {
        match &self.entries.get(identity)?.answer {
            Answer::Read(visitor) => Some(visitor.as_deref()),
            Answer::Reading { .. } | Answer::Stale => None,
        }
    }

    /// Notes that memory holds a visit of `identity`: its answer, if
    /// any, no longer stands.
    pub(super) fn visited(&mut self, identity: &str) {
        if let Some(entry) = self.entries.get_mut(identity) {
            entry.answer = Answer::Stale;
        }
    }
}

/// A links batch's lookups, which last until its job is written.
#[derive(Debug)]
pub struct Lookup {
    held: Arc<Mutex<Held>>,
    batch: u64,
    /// Says, once this batch read its identities, that it did.
    read: watch::Sender<bool>,
    /// The identities this batch reads.
    reads: Vec<Arc<str>>,
    /// Each identity it uses, those it reads included.
    uses: Vec<Arc<str>>,
    /// The reads of other batches it waits for.
    waits: Vec<watch::Receiver<bool>>,
}

impl Lookup {
    /// Whether this batch reads any identity itself.
    #[must_use]
    pub fn reads(&self) -> bool {
        !self.reads.is_empty()
    }

    /// Waits until the other batches that read this batch's identities
    /// answered, or were dropped unread.
    pub async fn wait(&mut self) {
        for read in &mut self.waits {
            drop(read.wait_for(|read| *read).await);
        }
    }
}

impl Drop for Lookup {
    fn drop(&mut self) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        for identity in &self.uses {
            let Some(entry) = held.lookups.entries.get_mut(identity) else {
                continue;
            };
            entry.users = entry.users.saturating_sub(1);
            if matches!(entry.answer, Answer::Reading { batch, .. } if batch == self.batch) {
                entry.answer = Answer::Stale;
            }
            if entry.users == 0 {
                drop(held.lookups.entries.remove(identity));
            }
        }
    }
}

impl UnwrittenVisits {
    /// The lookups of the identities of `plain`, a links batch's, that
    /// memory does not hold: none while it holds every visit the file
    /// holds, or each of them. Each one no batch reads, or whose answer
    /// went stale, is this batch's to read; it waits for another's read.
    #[must_use]
    pub fn look_up<'p>(&self, plain: impl IntoIterator<Item = &'p str>) -> Option<Lookup> {
        let mut holding = self.lock();
        let held = &mut *holding.held;
        if held.every_visit.holds() {
            return None;
        }
        let batch = held.lookups.batches;
        let (sender, receiver) = watch::channel(false);
        let (mut reads, mut uses, mut waits) = (Vec::new(), Vec::new(), Vec::new());
        for identity in plain {
            if held.entries.find(Space::Visited, identity).is_some() {
                continue;
            }
            let lookups = &mut held.lookups;
            let key = match lookups.entries.get_key_value(identity) {
                Some((key, _)) => Arc::clone(key),
                None => Arc::from(identity),
            };
            let entry = lookups.entries.entry(Arc::clone(&key)).or_insert(Entry {
                answer: Answer::Stale,
                users: 0,
            });
            match &entry.answer {
                Answer::Reading { batch: by, .. } if *by == batch => continue,
                Answer::Reading { read, .. } => waits.push(read.clone()),
                Answer::Read(_) => {}
                Answer::Stale => {
                    entry.answer = Answer::Reading {
                        batch,
                        read: receiver.clone(),
                    };
                    reads.push(Arc::clone(&key));
                }
            }
            entry.users += 1;
            uses.push(key);
        }
        if uses.is_empty() {
            return None;
        }
        held.lookups.batches += 1;
        drop(holding);
        Some(Lookup {
            held: Arc::clone(&self.held),
            batch,
            read: sender,
            reads,
            uses,
            waits,
        })
    }

    /// Reads `lookup`'s identities, each with `visitor` from the file in
    /// one reader transaction, counted as spilled lookups and in
    /// `F3.state_reads`, and keeps the answers that still stand.
    ///
    /// # Errors
    ///
    /// When `visitor` fails.
    pub(crate) fn read_lookup(
        &self,
        lookup: &Lookup,
        mut visitor: impl FnMut(&str) -> rusqlite::Result<Option<String>>,
    ) -> rusqlite::Result<()> {
        let mut answers = Vec::with_capacity(lookup.reads.len());
        for identity in &lookup.reads {
            self.spilled_lookups.add(1);
            self.reads_state(true);
            answers.push(visitor(identity)?);
        }
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        for (identity, answer) in lookup.reads.iter().zip(answers) {
            if let Some(entry) = held.lookups.entries.get_mut(identity)
                && matches!(entry.answer, Answer::Reading { batch, .. } if batch == lookup.batch)
            {
                entry.answer = Answer::Read(answer.map(String::into_boxed_str));
            }
        }
        drop(held);
        lookup.read.send_replace(true);
        Ok(())
    }
}
