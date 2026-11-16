//! The learned identities memory holds (#2169), so a duplicate check asks
//! whether a request's learned identity is a visited one's with no read of
//! the file.
//!
//! The first admission that writes reads them all, on the writer, as it
//! reads the learned rules (`rules`); each one a writer job visits is then
//! held as a visited identity is, and kept within the budget once the file
//! holds it. The writer changes them otherwise only as a rule that merges
//! is learned or stopped, re-keying its hosts' visited set (`identity`):
//! the next admission that writes, which reads the rules the writer
//! changed, reads the learned identities of those rules' hosts again.
//! Until it does, while memory's rules are behind the writer's, a lookup
//! reads the file as before, then the identities a writer job noted that
//! the file may not hold yet. A job whose work fails takes back what it
//! read, and its rules are read again, so their hosts' identities are too.
//!
//! Past the budget, those the oldest batches wrote leave first, and a miss
//! reads the file again. At a budget of 0 memory holds none the file
//! holds, and each lookup reads the file.

use std::sync::Arc;

use rusqlite::{Connection, Rows};

use super::{Held, Holding, Space, Whole};

/// The statement memory reads every learned identity with, named in the
/// statements a Run prepares as it opens.
pub(crate) const EVERY_LEARNED: &str =
    "SELECT identity, rule, crawl_request FROM learned_identities";

/// The statement memory reads the learned identities from `?1` to before
/// `?2` with, named in the statements a Run prepares as it opens.
pub(crate) const LEARNED_WITHIN: &str =
    "SELECT identity, rule, crawl_request FROM learned_identities
     WHERE identity >= ?1 AND identity < ?2";

impl Holding<'_> {
    /// The rule and crawl request the learned identity `identity` was
    /// visited with: memory's answer while it holds the learned identities
    /// as the writer last changed them, and keeps this one or every one
    /// the file holds; otherwise `read`'s from the file, counted as a
    /// spilled lookup (`goodfetch.run_state.spilled_lookups`) once memory
    /// read them.
    ///
    /// # Errors
    ///
    /// When `read` fails.
    pub(crate) fn learned_or(
        &self,
        identity: &str,
        read: impl FnOnce() -> rusqlite::Result<Option<(i64, String)>>,
    ) -> rusqlite::Result<Option<(i64, String)>> {
        let held = &*self.held;
        if !held.learned_read || self.rules().is_none() {
            return read();
        }
        if let Some((rule, visit)) = held.learned.get(identity) {
            return Ok(Some((*rule, String::from(&*visit.crawl_request))));
        }
        if held.every_learned.holds() {
            return Ok(None);
        }
        self.visits.spilled_lookups.add(1);
        read()
    }

    /// The rule and crawl request of the learned identity `identity`,
    /// while the file may not hold it: a writer job noted it in a batch
    /// not committed yet, or a chain's end holds it.
    pub(crate) fn learned(&self, identity: &str) -> Option<(i64, &str)> {
        self.held
            .learned
            .get(identity)
            .filter(|(_, visit)| visit.written.is_none_or(|tag| tag >= self.committed))
            .map(|(rule, visit)| (*rule, &*visit.crawl_request))
    }

    /// Whether memory read the learned identities the file holds.
    pub(crate) fn learned_read(&self) -> bool {
        self.held.learned_read
    }

    /// Reads again, from the file on `connection`, the writer's, the
    /// learned identities `within` those ranges of the key, or every one,
    /// in place of those memory holds there that the file holds, until the
    /// next would take memory past the budget. A chain's end held there
    /// that the file holds stays once its job is dropped. Nothing is held
    /// at a budget of 0.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub(crate) fn relearn(
        &mut self,
        connection: &Connection,
        within: Option<&[(String, String)]>,
    ) -> rusqlite::Result<()> {
        let tag = self.committed;
        let held = &mut *self.held;
        if held.budget.bytes() == 0 {
            return Ok(());
        }
        let inside = |identity: &str| {
            within.is_none_or(|ranges| {
                ranges
                    .iter()
                    .any(|(low, high)| identity >= low.as_str() && identity < high.as_str())
            })
        };
        let dropped: Vec<Arc<str>> = held
            .learned
            .iter()
            .filter(|(identity, (_, visit))| visit.written.is_some() && inside(identity))
            .map(|(identity, _)| Arc::clone(identity))
            .collect();
        for identity in dropped {
            held.remove(Space::Learned, &identity);
        }
        let whole = match within {
            None => held.hold_learned(
                &mut connection.prepare_cached(EVERY_LEARNED)?.query([])?,
                tag,
            )?,
            Some(ranges) => {
                let mut whole = true;
                for (low, high) in ranges {
                    let mut statement = connection.prepare_cached(LEARNED_WITHIN)?;
                    if !held.hold_learned(&mut statement.query((low, high))?, tag)? {
                        whole = false;
                        break;
                    }
                }
                whole
            }
        };
        match within {
            None => {
                held.learned_read = true;
                held.every_learned = Whole {
                    read: whole,
                    spilled: false,
                };
                held.journal.under(tag).learned_read = true;
            }
            Some(_) => held.every_learned.spilled |= !whole,
        }
        held.fit();
        Ok(())
    }
}

impl Held {
    /// Holds each learned identity `rows` reads, with its rule and crawl
    /// request, as a writer job under `tag` noted it, the job taking it
    /// back if its work fails, until the next would take memory past the
    /// budget; answers whether it held them all.
    fn hold_learned(&mut self, rows: &mut Rows<'_>, tag: u64) -> rusqlite::Result<bool> {
        while let Some(row) = rows.next()? {
            let identity = row.get_ref(0)?.as_str()?;
            let rule = row.get_ref(1)?.as_i64()?;
            let crawl_request = row.get_ref(2)?.as_str()?;
            // Held as a chain's pending end, the file holds it: it is the
            // file's from now on.
            self.remove(Space::Learned, identity);
            let identity: Arc<str> = identity.into();
            self.insert(Space::Learned, (&identity, rule), crawl_request, Some(tag));
            self.written
                .push_back((tag, Space::Learned, Arc::clone(&identity)));
            if self
                .entries
                .saturating_add(self.kept())
                .saturating_add(self.least(0))
                > self.budget.room()
            {
                self.written.pop_back();
                self.remove(Space::Learned, &identity);
                return Ok(false);
            }
            self.journal
                .under(tag)
                .changes
                .push((Space::Learned, identity, None));
        }
        Ok(true)
    }
}
