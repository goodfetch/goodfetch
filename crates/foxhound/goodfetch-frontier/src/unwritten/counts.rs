//! The Bounds' counts memory holds (#2169): the requests admitted against
//! `crawl.max_requests` and each path budget's, read from the file once,
//! on the writer, as the crawl opens or by the first admission that
//! writes, then counted here as each admission writes its own to the file.

use std::collections::HashMap;

use super::Holding;

/// The Bounds' counts the Frontier keeps in memory (#2169): the requests
/// admitted, and each path budget's, by its pattern, as the run file holds
/// them with the writes not committed yet. Written with the batch and
/// never removed; the refusals' counts are only written.
#[derive(Clone, Debug, Default)]
pub(crate) struct Counts {
    admitted: u64,
    paths: HashMap<Box<str>, u64>,
}

impl Counts {
    /// The counts of `admitted` requests, and of each path budget's, by
    /// its pattern.
    pub(crate) const fn new(admitted: u64, paths: HashMap<Box<str>, u64>) -> Self {
        Self { admitted, paths }
    }

    /// The requests admitted.
    pub(crate) const fn admitted(&self) -> u64 {
        self.admitted
    }

    /// The requests the path budget of `pattern` admitted.
    pub(crate) fn path_admitted(&self, pattern: &str) -> u64 {
        self.paths.get(pattern).copied().unwrap_or_default()
    }

    /// Counts one request admitted, against the path budget of `pattern`
    /// when one counts it.
    pub(crate) fn count(&mut self, pattern: Option<&str>) {
        self.admitted = self.admitted.saturating_add(1);
        if let Some(pattern) = pattern {
            let path = self.paths.entry(pattern.into()).or_default();
            *path = path.saturating_add(1);
        }
    }
}

/// The Bounds' counts memory held before a writer job first changed them.
#[derive(Debug, Default)]
pub(super) enum CountsBefore {
    /// The job changed none.
    #[default]
    Unchanged,
    /// Memory held none: the job read them from the file.
    Unread,
    /// Memory held these.
    Held(Counts),
}

impl Holding<'_> {
    /// The Bounds' counts memory holds: as `read` reads them from the
    /// writer's connection, as the crawl opens or by the first call of an
    /// admission that `writes`, a read of the Frontier's state
    /// (`F3.state_reads`); none before it, when the counts are the file's.
    ///
    /// # Errors
    ///
    /// When `read` cannot read the run file.
    pub(crate) fn counts(
        &mut self,
        writes: bool,
        read: impl FnOnce() -> rusqlite::Result<Counts>,
    ) -> rusqlite::Result<Option<&Counts>> {
        if self.held.counts.is_none() && writes {
            self.reads_state();
            let counts = read()?;
            let held = &mut *self.held;
            let journal = held.journal.under(self.committed);
            if matches!(journal.counts, CountsBefore::Unchanged) {
                journal.counts = CountsBefore::Unread;
            }
            held.counts = Some(counts);
        }
        Ok(self.held.counts.as_ref())
    }

    /// Counts one request a writer job admitted, against the path budget
    /// of `pattern` when one counts it, as it writes the count to the
    /// file in the batch it runs in.
    pub(crate) fn counted(&mut self, pattern: Option<&str>) {
        let held = &mut *self.held;
        if let Some(counts) = &mut held.counts {
            let journal = held.journal.under(self.committed);
            if matches!(journal.counts, CountsBefore::Unchanged) {
                journal.counts = CountsBefore::Held(counts.clone());
            }
            counts.count(pattern);
        }
    }
}
