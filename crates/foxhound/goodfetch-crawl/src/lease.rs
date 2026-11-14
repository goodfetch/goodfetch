//! A work item's lease (ADRs 0003 and 0087): the right of one attempt to
//! commit its outputs, numbered by an epoch that only goes up, granted for
//! the handler's timeout plus a margin and renewed only by lengthening.
//! Whether it still holds is a pure function of the `Clock`'s monotonic
//! reading (ADR 0059). While its attempt runs, the engine holds it as a
//! [`WorkItemLease`]: the attempt waits on it, it backs the page's
//! [`LeaseCheck`] once a handler gets its page, and an attempt's commit
//! passes through it, so a commit after its end is refused with
//! `lease_lost` and the run file's later lease of the work item fences it
//! too. Foxhound's: a Collie page has no work item.
//!
//! Each transition of the run fencing model (ADR 0060) is a `tracing`
//! event under [`TARGET`], its fields the model's variables: `transition`
//! (`grant`, `renew`, `expire`, `release`, `commit` or `refuse_commit`),
//! `run_id`, `run_epoch`, `work_item`, `lease_epoch`, `now` and
//! `expires_at` (milliseconds of the monotonic clock since the Run's
//! delivery began), and a refused commit's `reason` (`lease_lost`, or
//! `run_epoch` when a later writer holds the run file).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::CountedWork;
use goodfetch_dataset::Datasets;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::Keys;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::{RunState, RunStateError};
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, PushedRecord};
use goodfetch_protocol::foxhound::provenance::RobotsTxtMark;
use goodfetch_protocol::run::PointInTime;
use tokio::sync::watch;
use tokio::time::Instant;

pub(crate) use crate::staging::SoFar;
use crate::staging::{SetAside, Staging};
use crate::work_items::{self, Changed, CleanUpRules, Fetched, FieldsRead};

/// The `tracing` target of the run fencing model's transitions.
pub(crate) const TARGET: &str = "goodfetch::lease";

/// One attempt's lease of a work item, as a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lease {
    epoch: u64,
    expires_at: Instant,
}

impl Lease {
    /// The lease of epoch `epoch`, granted at `now` for `length`.
    pub(crate) fn granted(epoch: u64, now: Instant, length: Duration) -> Self {
        Self {
            epoch,
            expires_at: now + length,
        }
    }

    /// Its epoch.
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// When it ends.
    pub(crate) fn expires_at(&self) -> Instant {
        self.expires_at
    }

    /// Whether it still holds at `now`.
    pub(crate) fn holds_at(&self, now: Instant) -> bool {
        now < self.expires_at
    }

    /// Renews it at `now` for `length`: a renewal never shortens it, and
    /// never revives one that ended.
    #[must_use]
    pub(crate) fn renewed(self, now: Instant, length: Duration) -> Self {
        if !self.holds_at(now) {
            return self;
        }
        Self {
            expires_at: self.expires_at.max(now + length),
            ..self
        }
    }
}

/// Where a Run's leases are granted: its clock, and what each transition
/// it traces names.
#[derive(Clone, Debug)]
pub(crate) struct Leases {
    clock: Clock,
    origin: Instant,
    run_id: Arc<str>,
    run_epoch: u64,
    staging: Staging,
    work: CountedWork,
    datasets: Datasets,
    /// How each work item a commit adds is keyed, as the crawl's gate
    /// keys it; `None` leaves them for the delivery to key.
    keys: Option<Keys>,
    /// The Clean-up rules the Run holds to, which its first commit that
    /// keeps Records reads for every one after it.
    clean_up_rules: CleanUpRules,
}

impl Leases {
    /// The leases of the Run `run_id` under run epoch `run_epoch`, whose
    /// traced times count from now.
    pub(crate) fn new(clock: Clock, run_id: &str, run_epoch: u64) -> Self {
        Self {
            origin: clock.now(),
            clock,
            run_id: Arc::from(run_id),
            run_epoch,
            staging: Staging::default(),
            work: CountedWork::default(),
            datasets: Datasets::default(),
            keys: None,
            clean_up_rules: CleanUpRules::default(),
        }
    }

    /// The same, each commit keying the work items it adds under `keys`,
    /// the crawl's gate's, as it adds them (ADR 0138).
    #[must_use]
    pub(crate) fn keyed_by(mut self, keys: Keys) -> Self {
        self.keys = Some(keys);
        self
    }

    /// The same, each commit's Records counting their work on `work`
    /// (ADR 0140).
    #[must_use]
    pub(crate) fn counting_on(mut self, work: CountedWork) -> Self {
        self.work = work;
        self
    }

    /// The same, each attempt staging its outputs in `staging` while its
    /// lease holds (ADR 0093).
    #[must_use]
    pub(crate) fn staging_in(mut self, staging: Staging) -> Self {
        self.staging = staging;
        self
    }

    /// The same, each commit checking its Records against `datasets`, the
    /// crawl's, built once as it opened (ADR 0138).
    #[must_use]
    pub(crate) fn checking_against(mut self, datasets: Datasets) -> Self {
        self.datasets = datasets;
        self
    }

    /// Grants the lease of epoch `lease_epoch` on `work_item`, which the
    /// run file already holds, for `length` from now.
    pub(crate) fn grant(
        &self,
        work_item: &str,
        lease_epoch: u64,
        length: Duration,
    ) -> WorkItemLease {
        let lease = Lease::granted(lease_epoch, self.clock.now(), length);
        self.staging.open(work_item, lease_epoch);
        let (state, _) = watch::channel(Held {
            lease,
            released: false,
            expired: false,
        });
        let granted = WorkItemLease(Arc::new(Shared {
            leases: self.clone(),
            work_item: Arc::from(work_item),
            length,
            state,
        }));
        granted.trace("grant", &lease, None);
        granted
    }
}

/// An attempt's lease while the attempt runs, which any of its tasks may
/// clone.
#[derive(Clone, Debug)]
pub(crate) struct WorkItemLease(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    leases: Leases,
    work_item: Arc<str>,
    /// How long it was granted for, which each renewal grants again.
    length: Duration,
    state: watch::Sender<Held>,
}

impl Drop for Shared {
    /// What its attempt staged and never committed goes with it.
    fn drop(&mut self) {
        let epoch = self.state.borrow().lease.epoch();
        self.leases.staging.discard(&self.work_item, epoch);
    }
}

#[derive(Clone, Copy, Debug)]
struct Held {
    lease: Lease,
    released: bool,
    /// Whether its end was traced.
    expired: bool,
}

/// What an attempt's commit committed, for its caller to finish.
#[derive(Debug, Default)]
pub(crate) struct Committed {
    /// The kept files of the Run's folder the staged Key-value store
    /// writes took out, which the caller forgets.
    pub(crate) files: Vec<String>,
    /// The requests the attempt added through a link suspected a trap and
    /// kept marked, each with its URL and mark, whose `trap_link_suspected`
    /// Provenance entries the caller writes (ADR 0035).
    pub(crate) suspected: Vec<crate::staging::Marked>,
    /// Each work item it added, keyed as it was added, in order, as its
    /// first lease takes it, for the delivery to offer its domain's queue
    /// (ADR 0138) and lease without reading the run file, whose readers do
    /// not see it before its batch is in the file (ADR 0142).
    pub(crate) keyed: Vec<work_items::Leased>,
}

/// What an attempt's commit writes, taken from its lease while it held
/// ([`WorkItemLease::committing`]), for one write of run state to run.
pub(crate) struct Commit {
    work_item: String,
    epoch: u64,
    staging: Staging,
    counted_work: CountedWork,
    declared: Datasets,
    keys: Option<Keys>,
    masks: Masks,
    clean_up_rules: CleanUpRules,
    folder: std::path::PathBuf,
    records: Vec<PushedRecord>,
    ended_at: PointInTime,
}

impl Commit {
    /// Its attempt's work item and lease epoch.
    pub(crate) fn attempt(&self) -> (&str, u64) {
        (&self.work_item, self.epoch)
    }

    /// When its work item's outcome ends.
    pub(crate) fn ended_at(&self) -> &PointInTime {
        &self.ended_at
    }

    /// Writes it in `connection`'s transaction: `None` when the run file no
    /// longer holds its lease, the refusal of a Record that does not hold
    /// for its Dataset, nothing written then, or what it committed, which
    /// `changed` counts among the crawl's requests by state.
    ///
    /// # Errors
    ///
    /// The run state's.
    pub(crate) fn write(
        self,
        connection: &rusqlite::Connection,
        changed: &mut Changed,
    ) -> rusqlite::Result<Option<Result<Committed, CallError>>> {
        let Self {
            work_item,
            epoch,
            staging,
            counted_work,
            declared,
            keys,
            masks,
            clean_up_rules,
            folder,
            records,
            ended_at,
        } = self;
        let mut staged = staging.take(&work_item, epoch);
        if work_items::attempt(connection, &work_item, epoch)?.is_none() {
            return Ok(None);
        }
        let answered = match declared.check(records, &counted_work) {
            Ok(answered) => answered,
            Err(refused) => return Ok(Some(Err(refused))),
        };
        let replayed = staged.admit(connection, keys)?;
        let (writes, not_added, keyed) = (replayed.writes, replayed.not_added, replayed.keyed);
        changed.and(replayed.changed);
        let files = goodfetch_engine::key_value_store::apply(connection, &folder, writes)?;
        let suspected = staged.suspected(connection)?;
        let fetched = staged.fetched().clone();
        let fields = staged.fields().clone();
        let mut records = staged.records();
        records.append(answered);
        let committed = work_items::Committed {
            records: &records,
            fetched: &fetched,
            fields: &fields,
            masks: &masks,
            ended_at: &ended_at,
            not_added: &not_added,
            work: &counted_work,
            keeps_records: declared.keeps_records(),
            clean_up_rules: &clean_up_rules,
        };
        if !work_items::commit(connection, &work_item, epoch, committed, changed)? {
            return Ok(None);
        }
        let keyed = work_items::leasable_added(connection, &keyed)?;
        Ok(Some(Ok(Committed {
            files,
            suspected,
            keyed,
        })))
    }
}

/// Why an attempt's commit did not count.
#[derive(Debug)]
pub(crate) enum CommitError {
    /// The lease ended first, or the run file holds a later one of its
    /// work item: `lease_lost`.
    LeaseLost,
    /// A Record of the attempt's answer does not hold for its Dataset
    /// (`dataset_unknown`, `record_invalid`): nothing is committed, and the
    /// attempt fails with that error.
    Refused(CallError),
    /// The run file refused or failed the write.
    RunState(RunStateError),
}

impl WorkItemLease {
    /// Renews it for `length` from now; one that ended stays ended.
    pub(crate) fn renew(&self, length: Duration) {
        let now = self.0.leases.clock.now();
        let mut renewed = None;
        self.0.state.send_if_modified(|held| {
            if held.released || !held.lease.holds_at(now) {
                return false;
            }
            let lease = held.lease.renewed(now, length);
            renewed = Some(lease);
            let lengthened = lease != held.lease;
            held.lease = lease;
            lengthened
        });
        if let Some(lease) = renewed {
            self.trace("renew", &lease, None);
        }
    }

    /// How long it was granted for.
    pub(crate) fn length(&self) -> Duration {
        self.0.length
    }

    /// Gives it back as its attempt ends; it then lets nothing more leave.
    pub(crate) fn release(&self) {
        let now = self.0.leases.clock.now();
        let mut released = None;
        self.0.state.send_if_modified(|held| {
            if held.released || !held.lease.holds_at(now) {
                return false;
            }
            held.released = true;
            released = Some(held.lease);
            true
        });
        self.discard();
        if let Some(lease) = released {
            self.trace("release", &lease, None);
        }
    }

    /// Discards what its attempt staged so far and stages anew, as its
    /// handler runs again on another tier with its first answer discarded:
    /// an escalation or a sample's browser run (ADR 0093). Answers what
    /// the discarded run had staged, which a sample compares.
    /// Its robots.txt mark, which holds for its request's URL whatever
    /// the tier, is kept.
    pub(crate) fn restage(&self) -> SoFar {
        let epoch = self.0.state.borrow().lease.epoch();
        let staging = &self.0.leases.staging;
        let robots_txt = staging.robots_txt(&self.0.work_item, epoch);
        let pushed = staging.discard(&self.0.work_item, epoch);
        if self.check().is_ok() {
            staging.open(&self.0.work_item, epoch);
            if robots_txt.is_some() {
                staging.fetched(
                    &self.0.work_item,
                    epoch,
                    Fetched {
                        robots_txt,
                        ..Fetched::default()
                    },
                );
            }
        }
        pushed
    }

    /// Sets aside what its attempt staged so far, its run that commits,
    /// while a run whose staging is discarded runs after it (a sample's
    /// `discovered_api` run); `None` once the attempt ended.
    pub(crate) fn set_aside(&self) -> Option<SetAside> {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0.leases.staging.set_aside(&self.0.work_item, epoch)
    }

    /// Puts back what [`WorkItemLease::set_aside`] set aside, what was
    /// staged since discarded.
    pub(crate) fn put_back(&self, set_aside: SetAside) {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0
            .leases
            .staging
            .put_back(&self.0.work_item, epoch, set_aside);
    }

    /// Notes that its request's URL is one its origin's robots.txt
    /// disallows, fetched while `site_preferences.respect_robots_txt` was
    /// off, which the Provenance of each Record it commits marks (ADR
    /// 0086).
    pub(crate) fn robots_txt_ignored(&self) {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0.leases.staging.fetched(
            &self.0.work_item,
            epoch,
            Fetched {
                robots_txt: Some(RobotsTxtMark::Ignored),
                ..Fetched::default()
            },
        );
    }

    /// Notes how its attempt's run fetched its document, which the
    /// Provenance of each Record it commits names.
    pub(crate) fn fetched(&self, fetched: Fetched) {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0
            .leases
            .staging
            .fetched(&self.0.work_item, epoch, fetched);
    }

    /// Notes what its attempt read of its page's fields, which the
    /// Provenance of the Record it commits names (ADR 0123).
    pub(crate) fn fields_read(&self, fields: FieldsRead) {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0
            .leases
            .staging
            .fields_read(&self.0.work_item, epoch, fields);
    }

    /// What its attempt staged so far, which stays staged.
    pub(crate) fn pushed(&self) -> SoFar {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0.leases.staging.pushed(&self.0.work_item, epoch)
    }

    /// Discards what its attempt staged and did not commit.
    fn discard(&self) {
        let epoch = self.0.state.borrow().lease.epoch();
        self.0.leases.staging.discard(&self.0.work_item, epoch);
    }

    /// Waits until it ends: until its attempt releases it, or until the
    /// monotonic clock passes its end, renewals included.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future only stops waiting.
    pub(crate) async fn ended(&self) -> LeaseEnded {
        let mut changes = self.0.state.subscribe();
        loop {
            if let Err(ended) = self.check() {
                return ended;
            }
            let expires_at = changes.borrow_and_update().lease.expires_at();
            tokio::select! {
                () = self.0.leases.clock.sleep_until(expires_at) => {}
                _ = changes.changed() => {}
            }
        }
    }

    /// Commits `records`, its attempt's answer, once, after the Records the
    /// attempt pushed and with what else it staged and its work item's
    /// outcome ended at `ended_at`, when it still holds, the run file
    /// holds it too and each of `records` holds for the Dataset it names,
    /// each Record naming its Provenance and keeping every value `masks`
    /// holds as its `%name%`; answers what it committed: the kept files of
    /// the Run's folder the staged Key-value store writes took out, which
    /// the caller forgets, the requests it added through a link suspected a
    /// trap and kept marked, whose Provenance the caller writes (ADR 0035),
    /// and the work items it added keyed. A job of its own: an attempt's
    /// outcome writes [`WorkItemLease::committing`]'s [`Commit`] in its one
    /// job instead (ADR 0138), which answers the crawl's Run state what it
    /// changed of its requests by state. This one answers it to nobody: a
    /// running crawl calls it only for a late answer, whose lease ended
    /// first, so it is refused before any write.
    ///
    /// # Errors
    ///
    /// `LeaseLost` when it ended first or the run file holds a later lease
    /// of its work item, and nothing is committed; `Refused` when a Record
    /// of `records` does not hold, and nothing is committed; `RunState`
    /// when the write failed or a later writer holds the run file.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write was queued may still commit it.
    pub(crate) async fn commit(
        &self,
        state: &RunState,
        masks: &Masks,
        records: Vec<PushedRecord>,
        ended_at: PointInTime,
    ) -> Result<Committed, CommitError> {
        let commit = self.committing(state, masks, records, ended_at)?;
        let written = state
            .write(move |connection| commit.write(connection, &mut Changed::default()))
            .await;
        self.committed(written)
    }

    /// What committing `records` writes, as [`WorkItemLease::commit`]
    /// says, for a write of `state` to run with [`Commit::write`], whose
    /// answer [`WorkItemLease::committed`] then reads.
    ///
    /// # Errors
    ///
    /// `LeaseLost` when it ended first: nothing is to be committed.
    pub(crate) fn committing(
        &self,
        state: &RunState,
        masks: &Masks,
        records: Vec<PushedRecord>,
        ended_at: PointInTime,
    ) -> Result<Commit, CommitError> {
        let lease = self.0.state.borrow().lease;
        if self.check().is_err() {
            self.trace("refuse_commit", &lease, Some("lease_lost"));
            return Err(CommitError::LeaseLost);
        }
        let leases = &self.0.leases;
        Ok(Commit {
            work_item: self.0.work_item.to_string(),
            epoch: lease.epoch(),
            staging: leases.staging.clone(),
            counted_work: leases.work.clone(),
            declared: leases.datasets.clone(),
            keys: leases.keys.clone(),
            masks: masks.clone(),
            clean_up_rules: leases.clean_up_rules.clone(),
            folder: state
                .path()
                .parent()
                .expect("invariant: a run file lies in its Run's folder")
                .to_owned(),
            records,
            ended_at,
        })
    }

    /// What the write of a [`Commit`] of it answered, `written`, as
    /// [`WorkItemLease::commit`] answers it, each transition traced.
    ///
    /// # Errors
    ///
    /// As [`WorkItemLease::commit`]'s.
    pub(crate) fn committed(
        &self,
        written: Result<Option<Result<Committed, CallError>>, RunStateError>,
    ) -> Result<Committed, CommitError> {
        let lease = self.0.state.borrow().lease;
        match written {
            Ok(Some(Ok(committed))) => {
                self.trace("commit", &lease, None);
                Ok(committed)
            }
            Ok(Some(Err(refused))) => Err(CommitError::Refused(refused)),
            Ok(None) => {
                self.trace("refuse_commit", &lease, Some("lease_lost"));
                Err(CommitError::LeaseLost)
            }
            Err(error) => {
                if let RunStateError::Fenced { .. } = error {
                    self.trace("refuse_commit", &lease, Some("run_epoch"));
                }
                Err(CommitError::RunState(error))
            }
        }
    }

    /// Traces one transition of the run fencing model.
    fn trace(&self, transition: &'static str, lease: &Lease, reason: Option<&'static str>) {
        let leases = &self.0.leases;
        let since = |moment: Instant| {
            u64::try_from(moment.saturating_duration_since(leases.origin).as_millis())
                .unwrap_or(u64::MAX)
        };
        tracing::trace!(
            target: TARGET,
            transition,
            run_id = &*leases.run_id,
            run_epoch = leases.run_epoch,
            work_item = &*self.0.work_item,
            lease_epoch = lease.epoch(),
            now = since(leases.clock.now()),
            expires_at = since(lease.expires_at()),
            reason,
            "a work item's lease changed"
        );
    }
}

impl LeaseCheck for WorkItemLease {
    fn check(&self) -> Result<(), LeaseEnded> {
        let held = *self.0.state.borrow();
        if held.released {
            return Err(LeaseEnded::Released);
        }
        if held.lease.holds_at(self.0.leases.clock.now()) {
            return Ok(());
        }
        let first = self
            .0
            .state
            .send_if_modified(|held| !std::mem::replace(&mut held.expired, true));
        if first {
            self.discard();
            self.trace("expire", &held.lease, None);
        }
        Err(LeaseEnded::Lost)
    }

    /// `attempt_ended`: the handler's page sent nothing more once its
    /// attempt's lease ended (ADRs 0003 and 0087).
    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        CallError::new(
            &ATTEMPT_ENDED,
            format!(
                "the page's attempt ended with its lease ({ended}), so it sent nothing more (`{method}` withheld)"
            ),
            &serde_json::json!({}),
        )
    }
}

#[cfg(test)]
mod tests;
