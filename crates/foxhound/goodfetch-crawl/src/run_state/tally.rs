//! The crawl's requests by state as its Run state counts them, in memory
//! (ADR 0142; #2167): how many work items are left to deliver, how the
//! ended requests ended, and each sitemap seed's report. The delivery
//! loop, which owns the Run state, reads them once from the run file as
//! the crawl opens, then counts what each job of the run-state writer
//! answers that it changed ([`Changed`]): the jobs it queued itself and
//! the lesson a handler's attempt teaches the Frontier in a job of its
//! own. A live crawl's progress is built from them and reads no count
//! from the run file.
//!
//! The counts are signed. A handler attempt's lesson runs beside the
//! loop's jobs, and what it merged is counted when its attempt settles,
//! so a request a later job gave back may leave a count before it joined
//! it. What a value of the progress shows never goes below zero.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_frontier::sitemap;
use goodfetch_protocol::foxhound::outcomes::OutcomeKind;
use goodfetch_protocol::foxhound::report::SitemapReport;
use rusqlite::Connection;

use crate::work_items::{self, Changed, CountedAs};

/// The crawl's requests by state, as its Run state counts them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// The work items left to deliver: pending, or leased to an attempt.
    left: i64,
    /// The ended requests, by kind, then by what each counts under: its
    /// skip reason, Error code or Block cause, none for `succeeded`. None
    /// counts zero.
    ended: BTreeMap<OutcomeKind, BTreeMap<Arc<str>, i64>>,
    /// Each sitemap seed's report, in the order the seeds were given.
    sitemaps: Arc<[SitemapReport]>,
}

impl Tally {
    /// The counts the run file `connection` reads holds: what a crawl's
    /// Run state starts from as it opens.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let mut tally = Self {
            left: i64::try_from(work_items::left(connection)?).unwrap_or(i64::MAX),
            ended: BTreeMap::new(),
            sitemaps: sitemap::reports(connection)?.into(),
        };
        let (ended, _) = work_items::ended_counts(connection)?;
        for (kind, name, count) in ended {
            let Some(kind) = OutcomeKind::ALL
                .into_iter()
                .find(|known| known.as_str() == kind)
            else {
                continue;
            };
            let count = i64::try_from(count).unwrap_or(i64::MAX);
            tally.count(&(kind, name), count);
        }
        Ok(tally)
    }

    /// Counts what a job `changed`: each work item it added is left to
    /// deliver, each request it ended left that count for its ending's,
    /// each sitemap's fetch it ended left it for none, each ended request
    /// or failed fetch it made pending again came back, and the sitemap
    /// seeds' reports are those it read, when it changed them.
    pub(crate) fn change(&mut self, changed: Changed) {
        let signed = |count: u64| i64::try_from(count).unwrap_or(i64::MAX);
        self.left += signed(changed.added) + signed(changed.fetches_reopened)
            - signed(changed.fetches_ended);
        for ended in &changed.ended {
            self.left -= 1;
            self.count(ended, 1);
        }
        for reopened in &changed.reopened {
            self.left += 1;
            self.count(reopened, -1);
        }
        if let Some(sitemaps) = changed.sitemaps {
            self.sitemaps = sitemaps.into();
        }
    }

    /// Counts `by` more requests ended as `ended`, less when negative.
    fn count(&mut self, (kind, name): &CountedAs, by: i64) {
        let of_kind = self.ended.entry(kind.clone()).or_default();
        let counted = if let Some(count) = of_kind.get_mut(name.as_str()) {
            *count += by;
            *count
        } else {
            of_kind.insert(Arc::from(name.as_str()), by);
            by
        };
        if counted == 0 {
            of_kind.remove(name.as_str());
            if of_kind.is_empty() {
                self.ended.remove(kind);
            }
        }
    }

    /// How many work items are left to deliver: pending, or leased to an
    /// attempt.
    pub(crate) fn left(&self) -> u64 {
        u64::try_from(self.left).unwrap_or(0)
    }

    /// The ended requests of `kind`, by what each counts under, in the
    /// order of those names, with how many there are of each: those that
    /// count at least one.
    pub(crate) fn ended(&self, kind: &OutcomeKind) -> impl Iterator<Item = (&str, u64)> {
        self.ended
            .get(kind)
            .into_iter()
            .flatten()
            .filter_map(|(name, count)| {
                let count = u64::try_from(*count).ok().filter(|count| *count > 0)?;
                Some((&**name, count))
            })
    }

    /// Each sitemap seed's report, in the order the seeds were given.
    pub(crate) fn sitemaps(&self) -> &[SitemapReport] {
        &self.sitemaps
    }
}

#[cfg(test)]
mod tests;
