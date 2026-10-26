//! What an attempt stages (ADRs 0003, 0093, 0103 and 0106): a handler's
//! enqueues, the Records it pushes, once checked, the seeds it adds
//! through its `ctx.run` and the Key-value store writes it makes through it
//! are staged with its attempt while its lease holds, and commit in the one
//! write that commits the Records of its answer and its work item's
//! success, or are discarded once it ends any other way, so
//! a failed, retried or escalated attempt leaves no stray link, seed or key.
//! Beside them it keeps how its run's document was fetched, the tier and
//! the encoding, which the Provenance of each Record it commits names
//! (ADR 0123); a run that restages, an escalation or a sample's browser
//! run, records its own.
//! Nothing staged reaches the run file before that commit, so a crash
//! between the staging and the commit leaves the Frontier as the last
//! commit left it.
//!
//! Each staged enqueue is answered as the Frontier will admit it: its
//! admission runs on the run file as it stands, after the attempt's
//! earlier ones, inside a savepoint that is then rolled back, so a link the
//! attempt already enqueued is a `duplicate` and the bounds count what the
//! attempt staged. Its added requests' positions, which are their IDs, are
//! reserved in the run file at once, so the commit, which runs every staged
//! admission again in order, gives each the ID its enqueue answered. A
//! request another attempt's commit added meanwhile is not added twice; a
//! discarded attempt leaves a gap among the IDs.
//!
//! While a declared Effect's body runs in an attempt, a call that would
//! stage an output with it fails with `effect_staging_refused` (ADR 0106).
#![expect(
    clippy::disallowed_types,
    reason = "a lease's end discards its attempt's staging as it drops, which cannot wait, and the run-state writer's thread stages inside its writes: one Mutex guards the entries for a lookup or a push, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_dataset::Checked;
use goodfetch_engine::CallError;
use goodfetch_engine::staging::{AttemptWrites, StagedKey, StagedWrite, Unstaged};
use goodfetch_protocol::catalog::ErrorCodeDefinition;
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, EFFECT_STAGING_REFUSED};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::work_items::{Fetched, Positions};

/// An admission through the Frontier that an attempt staged, run again at
/// each later staging of the attempt and at its commit.
type Admission = Arc<dyn Fn(&Connection, &mut Positions) -> rusqlite::Result<()> + Send + Sync>;

/// What a staged call answers inside its write: `None` once its attempt
/// ended, else its answer or the call's own refusal.
pub(crate) type Answered<T> = rusqlite::Result<Option<Result<T, CallError>>>;

/// One staged admission, with the position it gave each URL it added.
#[derive(Clone)]
struct Admitted {
    admission: Admission,
    positions: BTreeMap<String, u64>,
}

/// What one attempt staged so far.
#[derive(Default)]
struct Entry {
    admitted: Vec<Admitted>,
    records: Checked,
    writes: Vec<StagedWrite>,
    fetched: Fetched,
    /// How many declared Effects' bodies run in it.
    effects: u32,
}

/// The `fix` of `effect_staging_refused`.
const STAGE_AFTER: &str = "Stage it after the `effect` declaration returns, from the output its body returned: a re-run that finds the Effect in the Effect journal does not run the body again.";

/// The staging of every attempt of one crawl that holds its lease, by its
/// work item and lease epoch.
#[derive(Clone, Default)]
pub(crate) struct Staging(Arc<Mutex<HashMap<(String, u64), Entry>>>);

/// What an attempt staged, taken out to commit.
#[derive(Default)]
pub(crate) struct Staged {
    admitted: Vec<Admitted>,
    records: Checked,
    writes: Vec<StagedWrite>,
    fetched: Fetched,
}

impl Staged {
    /// Admits every staged request again, in order, in `connection`'s
    /// transaction, each at the position its staging answered, and answers
    /// the Key-value store writes, in order, to apply in it too.
    pub(crate) fn admit(&self, connection: &Connection) -> rusqlite::Result<&[StagedWrite]> {
        let mut positions = Positions::of(connection)?;
        for admitted in &self.admitted {
            positions.keeping(admitted.positions.clone());
            (admitted.admission)(connection, &mut positions)?;
        }
        Ok(&self.writes)
    }

    /// How the attempt's run fetched its document.
    pub(crate) fn fetched(&self) -> &Fetched {
        &self.fetched
    }

    /// The Records the attempt pushed, in order.
    pub(crate) fn records(self) -> Checked {
        self.records
    }
}

impl std::fmt::Debug for Staging {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Staging")
            .field("attempts", &self.entries().len())
            .finish()
    }
}

impl Staging {
    fn entries(&self) -> MutexGuard<'_, HashMap<(String, u64), Entry>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the staging of the attempt of `work_item` under `lease_epoch`,
    /// as its lease is granted.
    pub(crate) fn open(&self, work_item: &str, lease_epoch: u64) {
        self.entries()
            .insert((work_item.to_owned(), lease_epoch), Entry::default());
    }

    /// Discards what the attempt staged, as it ends without committing,
    /// and answers the Records it had pushed; every later staging call of
    /// it fails with `attempt_ended`.
    pub(crate) fn discard(&self, work_item: &str, lease_epoch: u64) -> Checked {
        self.entries()
            .remove(&(work_item.to_owned(), lease_epoch))
            .map(|entry| entry.records)
            .unwrap_or_default()
    }

    /// The Records the attempt pushed so far, which stay staged.
    pub(crate) fn pushed(&self, work_item: &str, lease_epoch: u64) -> Checked {
        self.entries()
            .get(&(work_item.to_owned(), lease_epoch))
            .map(|entry| entry.records.clone())
            .unwrap_or_default()
    }

    /// Takes out what the attempt staged, to commit it; every later staging
    /// call of it fails with `attempt_ended`.
    pub(crate) fn take(&self, work_item: &str, lease_epoch: u64) -> Staged {
        self.entries()
            .remove(&(work_item.to_owned(), lease_epoch))
            .map(|entry| Staged {
                admitted: entry.admitted,
                records: entry.records,
                writes: entry.writes,
                fetched: entry.fetched,
            })
            .unwrap_or_default()
    }

    /// Notes how the attempt of `work_item` under `lease_epoch` fetched its
    /// run's document, in place of what an earlier run of it noted; nothing
    /// once the attempt ended.
    pub(crate) fn fetched(&self, work_item: &str, lease_epoch: u64, fetched: Fetched) {
        if let Some(entry) = self.entries().get_mut(&(work_item.to_owned(), lease_epoch)) {
            entry.fetched = fetched;
        }
    }

    /// Notes that a declared Effect's body begins in the attempt, which
    /// stages nothing until it ends (ADR 0106); `false` once the attempt
    /// ended.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "`effect.open` calls it once the Effect journal lands (#1550)"
        )
    )]
    pub(crate) fn begin_effect(&self, work_item: &str, lease_epoch: u64) -> bool {
        self.entries()
            .get_mut(&(work_item.to_owned(), lease_epoch))
            .map(|entry| entry.effects += 1)
            .is_some()
    }

    /// Notes that a declared Effect's body ended in the attempt.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "`effect.close` calls it once the Effect journal lands (#1550)"
        )
    )]
    pub(crate) fn end_effect(&self, work_item: &str, lease_epoch: u64) {
        if let Some(entry) = self.entries().get_mut(&(work_item.to_owned(), lease_epoch)) {
            entry.effects = entry.effects.saturating_sub(1);
        }
    }

    /// Whether the attempt of `work_item` under `lease_epoch` still
    /// stages.
    pub(crate) fn holds(&self, work_item: &str, lease_epoch: u64) -> bool {
        self.entries()
            .contains_key(&(work_item.to_owned(), lease_epoch))
    }

    /// Stages `records`, as their Datasets checked them, with the attempt of
    /// `work_item` under `lease_epoch`, after those it staged before:
    /// `None` once the attempt ended, or the refusal of a declared
    /// Effect's body.
    pub(crate) fn push(
        &self,
        work_item: &str,
        lease_epoch: u64,
        records: Checked,
    ) -> Option<Result<(), CallError>> {
        let mut entries = self.entries();
        let entry = entries.get_mut(&(work_item.to_owned(), lease_epoch))?;
        if entry.effects > 0 {
            return Some(Err(refused_in_effect(work_item, lease_epoch)));
        }
        entry.records.append(records);
        Some(Ok(()))
    }

    /// Stages `admission` with the attempt of `work_item` under
    /// `lease_epoch`, inside the write of `connection`, and answers what it
    /// answers there after the attempt's earlier admissions, the run file
    /// left as it was but for the positions it reserves. An admission that
    /// answers the attempt ended or a refusal of its own is not staged.
    pub(crate) fn admit<T>(
        &self,
        connection: &Connection,
        work_item: &str,
        lease_epoch: u64,
        admission: impl Fn(&Connection, &mut Positions) -> Answered<T> + Send + Sync + 'static,
    ) -> Answered<T> {
        let key = (work_item.to_owned(), lease_epoch);
        let earlier = match self.entries().get(&key) {
            None => return Ok(None),
            Some(entry) if entry.effects > 0 => {
                return Ok(Some(Err(refused_in_effect(work_item, lease_epoch))));
            }
            Some(entry) => entry.admitted.clone(),
        };
        let mut positions = Positions::of(connection)?;
        connection.execute_batch("SAVEPOINT staging")?;
        let answered = (|| {
            for admitted in &earlier {
                positions.keeping(admitted.positions.clone());
                (admitted.admission)(connection, &mut positions)?;
            }
            positions.keeping(BTreeMap::new());
            admission(connection, &mut positions)
        })();
        connection.execute_batch("ROLLBACK TO staging; RELEASE staging")?;
        let answered = answered?;
        if !matches!(answered, Some(Ok(_))) {
            return Ok(answered);
        }
        positions.reserve(connection)?;
        let admission = Arc::new(admission);
        let admitted = Admitted {
            admission: Arc::new(move |connection: &Connection, positions: &mut Positions| {
                admission(connection, positions).map(drop)
            }),
            positions: positions.taken(),
        };
        match self.entries().get_mut(&key) {
            Some(entry) => entry.admitted.push(admitted),
            None => return Ok(None),
        }
        Ok(answered)
    }
}

/// `effect_staging_refused` for a call of the attempt of `work_item` under
/// `lease_epoch`.
fn refused_in_effect(work_item: &str, lease_epoch: u64) -> CallError {
    let (message, details) = in_effect(work_item, lease_epoch);
    CallError::new(&EFFECT_STAGING_REFUSED, message, &details)
}

fn in_effect(work_item: &str, lease_epoch: u64) -> (String, Map<String, Value>) {
    let message = format!(
        "Attempt {lease_epoch} of crawl request `{work_item}` runs a declared Effect's body, so nothing was staged with it."
    );
    let Value::Object(details) = json!({ "fix": STAGE_AFTER }) else {
        unreachable!("invariant: a JSON object literal is an object")
    };
    (message, details)
}

fn unstaged(
    code: &'static ErrorCodeDefinition,
    message: String,
    details: Map<String, Value>,
) -> Unstaged {
    Unstaged {
        code,
        message,
        details,
    }
}

/// `attempt_ended` for a write through an attempt that is not staging.
fn ended(work_item: &str, lease_epoch: u64) -> Unstaged {
    unstaged(
        &ATTEMPT_ENDED,
        format!(
            "Attempt {lease_epoch} of crawl request `{work_item}` already ended, so nothing was written to the Key-value store."
        ),
        Map::new(),
    )
}

/// What `writes` leave `key` holding, last write winning.
fn last_of(writes: &[StagedWrite], key: &str) -> StagedKey {
    writes
        .iter()
        .rev()
        .find(|write| write.key() == key)
        .map_or(StagedKey::Untouched, |write| match write.value() {
            Some(value) => StagedKey::Set(value.clone()),
            None => StagedKey::Deleted,
        })
}

impl AttemptWrites for Staging {
    fn stage(
        &self,
        crawl_request_id: &str,
        lease_epoch: u64,
        write: StagedWrite,
    ) -> Result<StagedKey, Unstaged> {
        let mut entries = self.entries();
        let Some(entry) = entries.get_mut(&(crawl_request_id.to_owned(), lease_epoch)) else {
            return Err(ended(crawl_request_id, lease_epoch));
        };
        if entry.effects > 0 {
            let (message, details) = in_effect(crawl_request_id, lease_epoch);
            return Err(unstaged(&EFFECT_STAGING_REFUSED, message, details));
        }
        let before = last_of(&entry.writes, write.key());
        entry.writes.push(write);
        Ok(before)
    }

    fn staged(
        &self,
        crawl_request_id: &str,
        lease_epoch: u64,
        key: &str,
    ) -> Result<StagedKey, Unstaged> {
        self.entries()
            .get(&(crawl_request_id.to_owned(), lease_epoch))
            .map(|entry| last_of(&entry.writes, key))
            .ok_or_else(|| ended(crawl_request_id, lease_epoch))
    }
}

#[cfg(test)]
mod tests;
