//! The learned rules memory holds (#2169), so an admission keys a URL
//! under them, and asks whether one refuses it, with no read of the file.
//!
//! The first admission that writes reads them all, on the writer, once it
//! has the writer's connection tell memory the lowest ID of each rule it
//! changes: learned, stopped by a contradiction, or rolled back. The next
//! admission that writes forgets the rules from that ID on and reads them
//! again from the file, which holds what the writer did, rolled back or
//! not. Until it does, memory's rules are behind the writer's, and a
//! lookup reads the file as before; a reader's snapshot may not hold the
//! writer's newest rules, so only the writer reads them into memory.
//!
//! The rules count in memory's bytes, and never leave it: none is read
//! from the file again unless it changed. At a budget of 0 memory holds
//! none, and each lookup reads the file.
//!
//! The rules themselves, their kinds and the lookups that read them, are
//! `rules`': this module only holds the rows.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use rusqlite::Connection;
use rusqlite::hooks::Action;

use super::{Holding, table};

/// One learned rule, as the file's row holds it.
#[derive(Debug)]
pub(crate) struct HeldRule {
    pub(crate) id: i64,
    pub(crate) kind: Box<str>,
    pub(crate) path: Box<str>,
    pub(crate) parameter: Box<str>,
    pub(crate) form: Box<str>,
    /// Whether a contradiction stopped it.
    pub(crate) stopped: bool,
}

impl HeldRule {
    /// The bytes its texts allocate.
    fn texts(&self) -> usize {
        self.kind.len() + self.path.len() + self.parameter.len() + self.form.len()
    }
}

/// The learned rules, by host, each host's in the order of their IDs.
#[derive(Debug, Default)]
pub(crate) struct LearnedRules {
    by_host: HashMap<Box<str>, Vec<HeldRule>>,
    /// The bytes the hosts, the rules' texts and each host's list take;
    /// the table holding the lists adds its own ([`Self::bytes`]).
    entries: usize,
}

impl LearnedRules {
    /// The rules learned on `host`, in the order of their IDs.
    pub(crate) fn on(&self, host: &str) -> &[HeldRule] {
        self.by_host.get(host).map_or(&[], Vec::as_slice)
    }

    /// The bytes held.
    pub(super) fn bytes(&self) -> u64 {
        let lists = table::<(Box<str>, Vec<HeldRule>)>(self.by_host.capacity());
        u64::try_from(self.entries.saturating_add(lists)).unwrap_or(u64::MAX)
    }

    /// Forgets each rule whose ID is `from` or later.
    fn forget_from(&mut self, from: i64) {
        let entries = &mut self.entries;
        self.by_host.retain(|host, rules| {
            let kept = rules.partition_point(|rule| rule.id < from);
            for rule in rules.drain(kept..) {
                *entries = entries.saturating_sub(rule.texts());
            }
            if rules.is_empty() {
                let list = host.len() + rules.capacity() * size_of::<HeldRule>();
                *entries = entries.saturating_sub(list);
                return false;
            }
            true
        });
    }

    /// Holds `rule`, learned on `host` after every rule held there.
    fn add(&mut self, host: &str, rule: HeldRule) {
        self.entries += rule.texts();
        if !self.by_host.contains_key(host) {
            self.entries += host.len();
        }
        let rules = self.by_host.entry(host.into()).or_default();
        let room = rules.capacity();
        rules.push(rule);
        self.entries += (rules.capacity() - room) * size_of::<HeldRule>();
    }
}

/// The lowest ID of a rule the writer changed since memory last read the
/// rules, or none, [`i64::MAX`].
pub(super) fn unchanged() -> Arc<AtomicI64> {
    Arc::new(AtomicI64::new(i64::MAX))
}

impl Holding<'_> {
    /// The learned rules memory holds, while it holds each one as the
    /// writer last changed it; none at a budget of 0, before the first
    /// admission that writes read them, or once the writer changed one
    /// since: a lookup then reads the file.
    pub(crate) fn rules(&self) -> Option<&LearnedRules> {
        self.held
            .rules
            .as_ref()
            .filter(|_| self.held.rules_changed.load(Ordering::SeqCst) == i64::MAX)
    }

    /// Brings the learned rules memory holds up to the writer's, for an
    /// admission that writes on `connection`, the writer's: the first
    /// reads them all, with `read`, once the connection tells memory each
    /// change of `rules`, their table; a later one reads, from the file,
    /// those from the lowest ID the writer changed since. A job whose work
    /// fails has the next read again what it read. Nothing is held at a
    /// budget of 0.
    ///
    /// # Errors
    ///
    /// When `read` cannot read the run file, or the connection's changes
    /// cannot be watched.
    pub(crate) fn keep_rules(
        &mut self,
        connection: &Connection,
        rules: &'static str,
        read: impl FnOnce(i64) -> rusqlite::Result<Vec<(String, HeldRule)>>,
    ) -> rusqlite::Result<()> {
        let held = &mut *self.held;
        if held.budget == 0 {
            return Ok(());
        }
        let changed = &held.rules_changed;
        let from = if held.rules.is_none() {
            let watched = Arc::clone(changed);
            connection.update_hook(Some(move |_: Action, _: &str, table: &str, id: i64| {
                if table == rules {
                    watched.fetch_min(id, Ordering::SeqCst);
                }
            }))?;
            changed.store(i64::MAX, Ordering::SeqCst);
            i64::MIN
        } else {
            let from = changed.swap(i64::MAX, Ordering::SeqCst);
            if from == i64::MAX {
                return Ok(());
            }
            from
        };
        let rows = read(from).inspect_err(|_| {
            changed.fetch_min(from, Ordering::SeqCst);
        })?;
        let learned = held.rules.get_or_insert_default();
        learned.forget_from(from);
        for (host, rule) in rows {
            learned.add(&host, rule);
        }
        let journal = held.journal.under(self.committed);
        journal.rules_from = Some(journal.rules_from.map_or(from, |earlier| earlier.min(from)));
        held.fit();
        Ok(())
    }
}
