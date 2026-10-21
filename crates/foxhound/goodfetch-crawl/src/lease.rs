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

use goodfetch_engine::clock::Clock;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::run_state::{RunState, RunStateError};
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::run::PointInTime;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::work_items;

/// The `tracing` target of the run fencing model's transitions.
pub(crate) const TARGET: &str = "goodfetch::lease";

/// One attempt's lease of a work item, as a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    epoch: u64,
    expires_at: Instant,
}

impl Lease {
    /// The lease of epoch `epoch`, granted at `now` for `length`.
    pub fn granted(epoch: u64, now: Instant, length: Duration) -> Self {
        Self {
            epoch,
            expires_at: now + length,
        }
    }

    /// Its epoch.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// When it ends.
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }

    /// Whether it still holds at `now`.
    pub fn holds_at(&self, now: Instant) -> bool {
        now < self.expires_at
    }

    /// Renews it at `now` for `length`: a renewal never shortens it, and
    /// never revives one that ended.
    #[must_use]
    pub fn renewed(self, now: Instant, length: Duration) -> Self {
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
        }
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
        let (state, _) = watch::channel(Held {
            lease,
            released: false,
            expired: false,
        });
        let granted = WorkItemLease(Arc::new(Shared {
            leases: self.clone(),
            work_item: Arc::from(work_item),
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
    state: watch::Sender<Held>,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    lease: Lease,
    released: bool,
    /// Whether its end was traced.
    expired: bool,
}

/// Why an attempt's commit did not count.
#[derive(Debug)]
pub(crate) enum CommitError {
    /// The lease ended first, or the run file holds a later one of its
    /// work item: `lease_lost`.
    LeaseLost,
    /// The run file refused or failed the write.
    RunState(RunStateError),
}

impl WorkItemLease {
    /// Renews it for `length` from now; one that ended stays ended.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the engine renews it while a permit's wait holds the attempt (ADR 0093), which the page's navigation gate brings (#720)"
        )
    )]
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
        if let Some(lease) = released {
            self.trace("release", &lease, None);
        }
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

    /// Commits `records` as its attempt's outputs, once, with its work
    /// item's outcome ended at `ended_at`, when it still holds and the run
    /// file holds it too.
    ///
    /// # Errors
    ///
    /// `LeaseLost` when it ended first or the run file holds a later lease
    /// of its work item, and nothing is committed; `RunState` when the
    /// write failed or a later writer holds the run file.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write was queued may still commit it.
    pub(crate) async fn commit(
        &self,
        state: &RunState,
        records: Vec<PushedRecord>,
        ended_at: PointInTime,
    ) -> Result<(), CommitError> {
        let lease = self.0.state.borrow().lease;
        if self.check().is_err() {
            self.trace("refuse_commit", &lease, Some("lease_lost"));
            return Err(CommitError::LeaseLost);
        }
        let work_item = self.0.work_item.to_string();
        let epoch = lease.epoch();
        match state
            .write(move |connection| {
                work_items::commit(connection, &work_item, epoch, &records, &ended_at)
            })
            .await
        {
            Ok(true) => {
                self.trace("commit", &lease, None);
                Ok(())
            }
            Ok(false) => {
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
            self.trace("expire", &held.lease, None);
        }
        Err(LeaseEnded::Lost)
    }
}

#[cfg(test)]
mod tests;
