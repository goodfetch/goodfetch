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
//! admission runs once, on the run file as it stands, inside a savepoint
//! that is then rolled back, against what the attempt's earlier
//! admissions did, which the Frontier's `Staged` keeps beside the entry,
//! so a link the attempt already enqueued is a `duplicate` and the bounds
//! count what the attempt staged; a rule an earlier admission of the
//! attempt would teach is learned at the commit. Its added requests'
//! positions, which are their IDs, are reserved in the run file at once,
//! so the commit, which runs every staged admission again in order, gives
//! each the ID its enqueue answered. A request another attempt's commit
//! added meanwhile, or a rule learned since refuses, is not added, and the
//! outcome of the commit names its ID, with why; a discarded attempt
//! leaves a gap among the IDs. An attempt stages at most
//! `crawl.max_staged_enqueues` admissions: one more fails with
//! `staging_full`.
//!
//! While a declared Effect's body runs in an attempt, a call that would
//! stage an output with it fails with `effect_staging_refused` (ADR 0106).
#![expect(
    clippy::disallowed_types,
    reason = "a lease's end discards its attempt's staging as it drops, which cannot wait, and the run-state writer's thread stages inside its writes: one Mutex guards the entries for a lookup or a push, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_dataset::Checked;
use goodfetch_engine::CallError;
use goodfetch_engine::staging::{AttemptWrites, StagedKey, StagedWrite, Unstaged};
use goodfetch_protocol::catalog::ErrorCodeDefinition;
use goodfetch_protocol::foxhound::crawl::{
    ATTEMPT_ENDED, CRAWL_MAX_STAGED_ENQUEUES, EFFECT_STAGING_REFUSED, STAGING_FULL,
    StagingFullDetails,
};
use goodfetch_protocol::foxhound::outcomes::NotAddedAtCommit;
use goodfetch_protocol::foxhound::provenance::RobotsTxtMark;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::work_items::{Fetched, FieldsRead, Positions};

/// An admission through the Frontier that an attempt staged, run again at
/// its commit.
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
    /// What its admissions did beside the run file, the Frontier
    /// identities of the requests they added among it, which its later
    /// admissions read.
    staged: Arc<goodfetch_frontier::Staged>,
    /// The requests its admissions added through a link suspected a trap,
    /// each with its URL, as they answered when staged.
    suspected: Vec<(String, String)>,
    records: Checked,
    writes: Vec<StagedWrite>,
    fetched: Fetched,
    fields: FieldsRead,
    /// How many declared Effects' bodies run in it.
    effects: u32,
}

/// The `fix` of `effect_staging_refused`.
const STAGE_AFTER: &str = "Stage it after the `effect` declaration returns, from the output its body returned: a re-run that finds the Effect in the Effect journal does not run the body again.";

/// An attempt of a crawl, by what holds its lease: its work item and the
/// lease epoch it holds it under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AttemptKey {
    work_item: String,
    lease_epoch: u64,
}

impl AttemptKey {
    fn of(work_item: &str, lease_epoch: u64) -> Self {
        Self {
            work_item: work_item.to_owned(),
            lease_epoch,
        }
    }
}

/// The staging of every attempt of one crawl that holds its lease, by its
/// work item and lease epoch, each staging at most `max` admissions
/// (`crawl.max_staged_enqueues`).
#[derive(Clone)]
pub(crate) struct Staging {
    entries: Arc<Mutex<HashMap<AttemptKey, Entry>>>,
    max: u64,
}

impl Default for Staging {
    /// A staging under `crawl.max_staged_enqueues`'s default.
    fn default() -> Self {
        Self::bounded(
            (CRAWL_MAX_STAGED_ENQUEUES.default)()
                .as_u64()
                .expect("invariant: the setting's default is a count"),
        )
    }
}

/// What an attempt staged so far, as a sample comparison weighs it (ADR
/// 0103): the Records it pushed, in order, and the Frontier identities of
/// the requests it added.
#[derive(Clone, Debug, Default)]
pub(crate) struct SoFar {
    pub(crate) records: Checked,
    pub(crate) enqueues: BTreeSet<String>,
}

/// What an attempt staged, set aside while another run of it stages.
pub(crate) struct SetAside(Entry);

/// What an attempt staged, taken out to commit.
#[derive(Default)]
pub(crate) struct Staged {
    admitted: Vec<Admitted>,
    suspected: Vec<(String, String)>,
    records: Checked,
    writes: Vec<StagedWrite>,
    fetched: Fetched,
    fields: FieldsRead,
}

/// What a commit admitted of what its attempt staged: the Key-value store
/// writes to apply in the same transaction, in order, and each request
/// its staging answered `added` that the commit did not add.
pub(crate) struct Replayed<'a> {
    pub(crate) writes: &'a [StagedWrite],
    pub(crate) not_added: Vec<NotAddedAtCommit>,
}

impl Staged {
    /// Admits every staged request again, in order, in `connection`'s
    /// transaction, each at the position its staging answered, and answers
    /// the Key-value store writes, in order, to apply in it too, with each
    /// request it did not add.
    pub(crate) fn admit(&self, connection: &Connection) -> rusqlite::Result<Replayed<'_>> {
        let mut positions = Positions::committing(connection)?;
        let mut not_added = Vec::new();
        for staged in &self.admitted {
            positions.keeping(staged.positions.clone());
            (staged.admission)(connection, &mut positions)?;
            not_added.extend(positions.not_added());
        }
        Ok(Replayed {
            writes: &self.writes,
            not_added,
        })
    }

    /// The requests the attempt added through a link suspected a trap,
    /// each with its URL, whose mark `connection`'s transaction holds once
    /// [`Staged::admit`] ran in it: a request another commit added first,
    /// or one a later link the attempt staged cleared, is left out
    /// (ADR 0035).
    ///
    /// # Errors
    ///
    /// The run state's.
    pub(crate) fn suspected(
        &self,
        connection: &Connection,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        let mut kept = Vec::new();
        for (id, url) in &self.suspected {
            if goodfetch_frontier::trap_links::suspected(connection, id)? {
                kept.push((id.clone(), url.clone()));
            }
        }
        Ok(kept)
    }

    /// How the attempt's run fetched its document.
    pub(crate) fn fetched(&self) -> &Fetched {
        &self.fetched
    }

    /// What the attempt read of its page's fields.
    pub(crate) fn fields(&self) -> &FieldsRead {
        &self.fields
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
    /// A staging whose attempts each stage at most `max` admissions.
    pub(crate) fn bounded(max: u64) -> Self {
        Self {
            entries: Arc::default(),
            max,
        }
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<AttemptKey, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the staging of the attempt of `work_item` under `lease_epoch`,
    /// as its lease is granted.
    pub(crate) fn open(&self, work_item: &str, lease_epoch: u64) {
        self.entries()
            .insert(AttemptKey::of(work_item, lease_epoch), Entry::default());
    }

    /// Discards what the attempt staged, as it ends without committing,
    /// and answers what it had staged; every later staging call of it
    /// fails with `attempt_ended`.
    pub(crate) fn discard(&self, work_item: &str, lease_epoch: u64) -> SoFar {
        self.entries()
            .remove(&AttemptKey::of(work_item, lease_epoch))
            .map(|entry| SoFar {
                enqueues: enqueues_of(&entry.staged),
                records: entry.records,
            })
            .unwrap_or_default()
    }

    /// What the attempt staged so far, which stays staged.
    pub(crate) fn pushed(&self, work_item: &str, lease_epoch: u64) -> SoFar {
        self.entries()
            .get(&AttemptKey::of(work_item, lease_epoch))
            .map(|entry| SoFar {
                records: entry.records.clone(),
                enqueues: enqueues_of(&entry.staged),
            })
            .unwrap_or_default()
    }

    /// Sets aside what the attempt of `work_item` under `lease_epoch`
    /// staged so far and stages anew, while a run whose staging is then
    /// discarded runs after it: a sample's `discovered_api` run, which
    /// needs the browser's run before it (ADR 0103). `None` once the
    /// attempt ended.
    pub(crate) fn set_aside(&self, work_item: &str, lease_epoch: u64) -> Option<SetAside> {
        let mut entries = self.entries();
        let entry = entries.get_mut(&AttemptKey::of(work_item, lease_epoch))?;
        Some(SetAside(std::mem::take(entry)))
    }

    /// Puts back what [`Staging::set_aside`] set aside, in place of what
    /// the attempt staged since, which is discarded; nothing once the
    /// attempt ended.
    pub(crate) fn put_back(&self, work_item: &str, lease_epoch: u64, set_aside: SetAside) {
        if let Some(entry) = self
            .entries()
            .get_mut(&AttemptKey::of(work_item, lease_epoch))
        {
            *entry = set_aside.0;
        }
    }

    /// Takes out what the attempt staged, to commit it; every later staging
    /// call of it fails with `attempt_ended`.
    pub(crate) fn take(&self, work_item: &str, lease_epoch: u64) -> Staged {
        self.entries()
            .remove(&AttemptKey::of(work_item, lease_epoch))
            .map(|entry| Staged {
                admitted: entry.admitted,
                suspected: entry.suspected,
                records: entry.records,
                writes: entry.writes,
                fetched: entry.fetched,
                fields: entry.fields,
            })
            .unwrap_or_default()
    }

    /// Notes how the attempt of `work_item` under `lease_epoch` fetched its
    /// run's document, in place of what an earlier run of it noted, its
    /// robots.txt mark kept unless `fetched` names one; nothing once the
    /// attempt ended.
    pub(crate) fn fetched(&self, work_item: &str, lease_epoch: u64, fetched: Fetched) {
        if let Some(entry) = self
            .entries()
            .get_mut(&AttemptKey::of(work_item, lease_epoch))
        {
            let robots_txt = fetched.robots_txt.or(entry.fetched.robots_txt);
            entry.fetched = Fetched {
                robots_txt,
                ..fetched
            };
        }
    }

    /// Notes that an admission the attempt of `work_item` under
    /// `lease_epoch` staged added `suspected`, each request with its URL,
    /// through links suspected traps, for the Provenance its commit writes;
    /// nothing once the attempt ended.
    pub(crate) fn suspected(
        &self,
        work_item: &str,
        lease_epoch: u64,
        suspected: impl IntoIterator<Item = (String, String)>,
    ) {
        if let Some(entry) = self
            .entries()
            .get_mut(&AttemptKey::of(work_item, lease_epoch))
        {
            entry.suspected.extend(suspected);
        }
    }

    /// Notes what the attempt of `work_item` under `lease_epoch` read of
    /// its page's fields, in place of what an earlier run of it noted;
    /// nothing once the attempt ended.
    pub(crate) fn fields_read(&self, work_item: &str, lease_epoch: u64, fields: FieldsRead) {
        if let Some(entry) = self
            .entries()
            .get_mut(&AttemptKey::of(work_item, lease_epoch))
        {
            entry.fields = fields;
        }
    }

    /// The robots.txt mark the attempt of `work_item` under `lease_epoch`
    /// noted, if any.
    pub(crate) fn robots_txt(&self, work_item: &str, lease_epoch: u64) -> Option<RobotsTxtMark> {
        self.entries()
            .get(&AttemptKey::of(work_item, lease_epoch))
            .and_then(|entry| entry.fetched.robots_txt)
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
            .get_mut(&AttemptKey::of(work_item, lease_epoch))
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
        if let Some(entry) = self
            .entries()
            .get_mut(&AttemptKey::of(work_item, lease_epoch))
        {
            entry.effects = entry.effects.saturating_sub(1);
        }
    }

    /// Whether the attempt of `work_item` under `lease_epoch` still
    /// stages.
    pub(crate) fn holds(&self, work_item: &str, lease_epoch: u64) -> bool {
        self.entries()
            .contains_key(&AttemptKey::of(work_item, lease_epoch))
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
        let entry = entries.get_mut(&AttemptKey::of(work_item, lease_epoch))?;
        if entry.effects > 0 {
            return Some(Err(refused_in_effect(work_item, lease_epoch)));
        }
        entry.records.append(records);
        Some(Ok(()))
    }

    /// Stages `admission` with the attempt of `work_item` under
    /// `lease_epoch`, inside the write of `connection`, and answers what it
    /// answers there after the attempt's earlier admissions, which it reads
    /// from what they did beside the run file, the run file left as it was
    /// but for the positions it reserves. An admission that answers the
    /// attempt ended or a refusal of its own is not staged; one past the
    /// attempt's `crawl.max_staged_enqueues` fails with `staging_full`.
    pub(crate) fn admit<T>(
        &self,
        connection: &Connection,
        work_item: &str,
        lease_epoch: u64,
        admission: impl Fn(&Connection, &mut Positions) -> Answered<T> + Send + Sync + 'static,
    ) -> Answered<T> {
        let key = AttemptKey::of(work_item, lease_epoch);
        let earlier = match self.entries().get(&key) {
            None => return Ok(None),
            Some(entry) if entry.effects > 0 => {
                return Ok(Some(Err(refused_in_effect(work_item, lease_epoch))));
            }
            Some(entry) if u64::try_from(entry.admitted.len()).unwrap_or(u64::MAX) >= self.max => {
                return Ok(Some(Err(full(work_item, lease_epoch, self.max))));
            }
            Some(entry) => Arc::clone(&entry.staged),
        };
        let mut positions = Positions::staging(connection, earlier)?;
        connection.execute_batch("SAVEPOINT staging")?;
        let answered = admission(connection, &mut positions);
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
        // Its `earlier` goes with it, so the entry's own is the one left.
        let now = positions.staged();
        match self.entries().get_mut(&key) {
            Some(entry) => {
                entry.admitted.push(admitted);
                Arc::make_mut(&mut entry.staged).absorb(now);
            }
            None => return Ok(None),
        }
        Ok(answered)
    }
}

/// The Frontier identities of the requests `staged`'s admissions added.
fn enqueues_of(staged: &goodfetch_frontier::Staged) -> BTreeSet<String> {
    staged.identities().map(str::to_owned).collect()
}

/// `staging_full` for a call of the attempt of `work_item` under
/// `lease_epoch`, which staged `max` admissions.
fn full(work_item: &str, lease_epoch: u64, max: u64) -> CallError {
    CallError::new(
        &STAGING_FULL,
        format!(
            "Attempt {lease_epoch} of crawl request `{work_item}` already staged {max} admissions, as many as `{}` lets one attempt stage, so nothing more was staged with it.",
            CRAWL_MAX_STAGED_ENQUEUES.name
        ),
        &StagingFullDetails {
            limit: CRAWL_MAX_STAGED_ENQUEUES.name.to_owned(),
            max,
        },
    )
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
        .map_or(StagedKey::Untouched, StagedWrite::staged)
}

impl AttemptWrites for Staging {
    fn stage(
        &self,
        crawl_request_id: &str,
        lease_epoch: u64,
        write: StagedWrite,
    ) -> Result<StagedKey, Unstaged> {
        let mut entries = self.entries();
        let Some(entry) = entries.get_mut(&AttemptKey::of(crawl_request_id, lease_epoch)) else {
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
            .get(&AttemptKey::of(crawl_request_id, lease_epoch))
            .map(|entry| last_of(&entry.writes, key))
            .ok_or_else(|| ended(crawl_request_id, lease_epoch))
    }
}

#[cfg(test)]
mod tests;
