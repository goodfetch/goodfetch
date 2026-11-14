//! The writer's open batch at a level that batches attempts (ADR 0142):
//! without `run_state.batch.per_attempt`, the writer keeps one transaction
//! open across its groups. A write that needs no sync before it is answered,
//! an attempt's ([`Syncing::Attempt`]) or a deferred one, rides it: it is
//! answered once its savepoint is released, and a reader sees it only once
//! the batch commits. A deferred write that charges before the file
//! declares its charges unsynced ([`super::Charges`]) declares it, a
//! barrier. Any other write is a barrier: the batch commits at the
//! end of its group, and the barrier is answered then. The batch also
//! commits once the attempts it ended reach `run_state.batch.pages`, its
//! writes' bytes `run_state.batch.bytes`, or its age
//! `run_state.batch.interval`, which a timer of its own checks while the
//! writer is idle, and as the writer ends. It is one transaction, synced as
//! `run_state.batch.sync` says: a crash before its commit loses it whole,
//! and the file stays at the batch before.

use std::time::Duration;

use goodfetch_counted_work::Row;
use goodfetch_protocol::ops::RUN_STATE_BATCH_ROWS;
use goodfetch_protocol::run::durability::BatchThreshold;
use opentelemetry::metrics::Histogram;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::{
    BatchSync, Completion, Job, Marks, Queued, RunStateError, RunStateOptions, Syncing, Writer,
    begin, finish, roll_back,
};
use crate::clock::Clock;
use crate::metrics::Metrics;
use crate::tasks::TaskOwner;

/// The writer's batching: its thresholds and its open batch.
pub(super) struct Batching {
    pages: BatchThreshold,
    bytes: BatchThreshold,
    interval: Option<Duration>,
    clock: Clock,
    /// When the open batch began, which its timer watches.
    opened: watch::Sender<Option<Instant>>,
    open: Option<OpenBatch>,
    /// `goodfetch.run_state.batch_rows`.
    rows: Histogram<u64>,
}

/// The batch whose transaction is open on the writer's connection.
struct OpenBatch {
    began: Instant,
    /// Whether it began on the connection that syncs.
    synced: bool,
    /// The attempts it ended.
    pages: u64,
    /// The bytes its attempts' writes wrote.
    bytes: u64,
    /// The writes it holds.
    rows: u64,
    /// The answers its barriers wait for, sent once it commits.
    owed: Vec<Completion>,
    /// Whether its writes ask its commit to declare how the Run's charges
    /// reach the file.
    declare: bool,
}

impl Batching {
    /// The writer's batching at `options`' Durability, with the timer that
    /// commits an idle writer's batch once `run_state.batch.interval` passed,
    /// one of `owner`'s tasks queuing ticks into `mailbox`: none when each
    /// attempt is a batch of its own.
    pub(super) fn of(
        options: &RunStateOptions,
        owner: &TaskOwner,
        mailbox: &mpsc::Sender<Queued>,
    ) -> Option<Self> {
        let durability = options.durability;
        if durability.per_attempt {
            return None;
        }
        let interval = match durability.interval {
            BatchThreshold::At(milliseconds) => Some(Duration::from_millis(milliseconds.get())),
            BatchThreshold::Never => None,
        };
        let (opened, watched) = watch::channel(None);
        if let Some(every) = interval {
            batch_timer(owner, every, options.clock, mailbox, watched);
        }
        let metrics = options.metrics.clone().unwrap_or_else(Metrics::none);
        let rows = metrics
            .meter()
            .u64_histogram(RUN_STATE_BATCH_ROWS.name)
            .with_description(RUN_STATE_BATCH_ROWS.description)
            .with_unit(RUN_STATE_BATCH_ROWS.unit)
            .build();
        Some(Self {
            pages: durability.pages,
            bytes: durability.bytes,
            interval,
            clock: options.clock,
            opened,
            open: None,
            rows,
        })
    }

    /// The threshold `open` reached, if any: its pages or its age, which
    /// the batch's plan seals it at, before its bytes, which seal it early.
    fn reached(&self, open: &OpenBatch) -> Option<Sealed> {
        let at = |threshold: BatchThreshold, count: u64| matches!(threshold, BatchThreshold::At(at) if count >= at.get());
        let aged = self
            .interval
            .is_some_and(|every| self.clock.now() >= open.began + every);
        if at(self.pages, open.pages) || aged {
            Some(Sealed::Planned)
        } else if at(self.bytes, open.bytes) {
            Some(Sealed::Early)
        } else {
            None
        }
    }
}

/// Why the writer commits its open batch, which `F2.batches` counts by
/// (ADR 0140's note on batching).
#[derive(Clone, Copy)]
enum Sealed {
    /// At its plan: its pages reached `run_state.batch.pages`, its age
    /// `run_state.batch.interval`, or a sync fell due at
    /// `run_state.commit_interval`.
    Planned,
    /// Before it: a barrier, a seal, its bytes, or the writer's end.
    Early,
}

/// How a write in the open batch is answered.
enum Answered {
    /// Once its savepoint is released, counting `ended` attempts and
    /// `bytes` towards the thresholds.
    Riding { ended: u64, bytes: u64 },
    /// Once the batch commits, which it commits at the end of its group.
    Barrier { declaring: bool },
}

/// How `queued` is answered in the open batch while the file `owes` the
/// writer's declaration of its charges or not.
fn answered(queued: &Queued, owes: bool) -> Answered {
    match (&queued.job, queued.syncing) {
        (Job::Write, Syncing::Attempt { ended, bytes }) => Answered::Riding { ended, bytes },
        (Job::Write, Syncing::Deferred { charged }) if charged > 0 && owes => {
            Answered::Barrier { declaring: true }
        }
        (Job::Write, Syncing::Deferred { .. }) => Answered::Riding { ended: 0, bytes: 0 },
        (_, Syncing::Declaring) => Answered::Barrier { declaring: true },
        _ => Answered::Barrier { declaring: false },
    }
}

impl Writer {
    /// Runs `group` in the open batch, opening one when none is, and
    /// commits the batch when the group holds a barrier, a sync falls due,
    /// or a threshold is reached; a group that holds the writer's `last`
    /// write commits it synced at every level.
    pub(super) fn batch(&mut self, group: Vec<Queued>, ticked: bool, last: bool) {
        let sync_due = self
            .interval
            .as_ref()
            .is_some_and(|interval| interval.plan(ticked, last).synced);
        let unsynced_stop =
            last && matches!(self.durability.sync, BatchSync::Never | BatchSync::Off);
        let open_unsynced = self.open_batch().is_some_and(|open| !open.synced);
        if unsynced_stop && open_unsynced {
            self.stop_in_open_batch(group);
            return;
        }
        // A sync falls due: a batch open on a connection that does not sync
        // commits first, since a connection switches to syncing outside a
        // transaction only.
        if sync_due && open_unsynced {
            self.commit_batch(Sealed::Planned, false, false);
        }
        let synced_stop = self.synced_stop(last);
        let clean = self.marked && synced_stop;
        let synced = match self.durability.sync {
            BatchSync::Each => true,
            BatchSync::Interval(_) => sync_due,
            BatchSync::Never | BatchSync::Off => synced_stop,
        };
        let commit = sync_due || last;
        if group.is_empty() && !commit {
            self.commit_reached();
            return;
        }
        let open = if self.open_batch().is_none() {
            self.open(synced, group)
        } else {
            self.run_in_batch(group)
        };
        if !open {
            return;
        }
        // The open batch's writes ran: any barrier among them commits it.
        let barrier = self
            .open_batch()
            .is_some_and(|open| !open.owed.is_empty() || open.declare);
        if sync_due && !last {
            self.commit_batch(Sealed::Planned, clean, last);
        } else if commit || barrier {
            self.commit_batch(Sealed::Early, clean, last);
        } else {
            self.commit_reached();
        }
    }

    fn open_batch(&self) -> Option<&OpenBatch> {
        self.batching
            .as_ref()
            .and_then(|batching| batching.open.as_ref())
    }

    /// Opens a batch, `synced` or not, holding `group`'s writes: whether
    /// it opened, its writes having all run. When it cannot open, every
    /// write of `group` is answered with the failure.
    fn open(&mut self, synced: bool, group: Vec<Queued>) -> bool {
        let Some(batching) = &mut self.batching else {
            return false;
        };
        let connection = self.connection.for_group(synced);
        if let Err(error) = begin(connection, &self.path, self.epoch) {
            self.signals.failed(&self.path, &error);
            for queued in group {
                (queued.write)(None).0(Err(error.clone()));
            }
            return false;
        }
        let began = batching.clock.now();
        batching.open = Some(OpenBatch {
            began,
            synced,
            pages: 0,
            bytes: 0,
            rows: 0,
            owed: Vec::new(),
            declare: false,
        });
        batching.opened.send_replace(Some(began));
        self.run_in_batch(group)
    }

    /// Runs `group`'s writes in the open batch: whether it is still open,
    /// a write having ended it on a full disk or an I/O error.
    fn run_in_batch(&mut self, group: Vec<Queued>) -> bool {
        let owes = self.deferral.owes();
        let Some(open) = self
            .batching
            .as_mut()
            .and_then(|batching| batching.open.as_mut())
        else {
            return false;
        };
        let connection = self.connection.for_group(open.synced);
        let mut jobs = group.into_iter();
        let mut ended = None;
        while let Some(queued) = jobs.next() {
            let answered = answered(&queued, owes);
            let (completion, ends_batch) = (queued.write)(Some((connection, &self.path)));
            open.rows += 1;
            if let Some(error) = ends_batch {
                open.owed.push(completion);
                // The rest of the group never runs: in a transaction SQLite
                // ended, each of its statements would commit on its own.
                let refused = jobs.by_ref().map(|queued| (queued.write)(None).0);
                open.owed.extend(refused);
                ended = Some(error);
                break;
            }
            match answered {
                Answered::Riding { ended, bytes } => {
                    open.pages = open.pages.saturating_add(ended);
                    open.bytes = open.bytes.saturating_add(bytes);
                    completion(Ok(()));
                }
                Answered::Barrier { declaring } => {
                    open.declare |= declaring;
                    open.owed.push(completion);
                }
            }
        }
        match ended {
            Some(error) => {
                self.fail_batch(&error);
                false
            }
            None => true,
        }
    }

    /// Commits the open batch, if one is open, and answers each of `seals`
    /// once it is in the file; with none open, at once.
    pub(super) fn seal(&mut self, seals: Vec<super::QueuedWrite>) {
        if seals.is_empty() {
            return;
        }
        let completions = seals.into_iter().map(|seal| seal(None).0);
        match self
            .batching
            .as_mut()
            .and_then(|batching| batching.open.as_mut())
        {
            Some(open) => {
                open.owed.extend(completions);
                self.commit_batch(Sealed::Early, false, false);
            }
            None => {
                for completion in completions {
                    completion(Ok(()));
                }
            }
        }
    }

    /// Commits the open batch when it reached a threshold.
    fn commit_reached(&mut self) {
        let reached = self.batching.as_ref().and_then(|batching| {
            batching
                .open
                .as_ref()
                .and_then(|open| batching.reached(open))
        });
        if let Some(sealed) = reached {
            self.commit_batch(sealed, false, false);
        }
    }

    /// Commits the open batch, `sealed` at its plan or before it, clearing
    /// the "unclean" mark when `clean`, as the writer's `last` commit or
    /// not, then answers its barriers.
    fn commit_batch(&mut self, sealed: Sealed, clean: bool, last: bool) {
        if let Some((answer, owed)) = self.write_open(sealed, clean, last) {
            for completion in owed {
                completion(answer.clone());
            }
        }
    }

    /// Stops the writer at a level that syncs no batch while a batch is
    /// open on a connection that does not sync: `group`, which holds the
    /// writer's last write, rides the open batch, which commits as the
    /// last batch; then [`Writer::sync_stop`] syncs it. The batch's
    /// barriers, the last write's among them, are answered once that sync
    /// is in the file.
    fn stop_in_open_batch(&mut self, group: Vec<Queued>) {
        if !self.run_in_batch(group) {
            return;
        }
        let Some((answer, owed)) = self.write_open(Sealed::Early, false, false) else {
            return;
        };
        let answer = answer.and_then(|()| self.sync_stop());
        for completion in owed {
            completion(answer.clone());
        }
    }

    /// Syncs what the writer committed unsynced before it stops, in a
    /// commit of its own that writes no batch's rows and clears the
    /// "unclean" mark: a connection switches to syncing outside a
    /// transaction only, so the batch committed before the switch. Without
    /// the switch, the stop syncs nothing and the mark stays, as a crash
    /// would leave it.
    fn sync_stop(&mut self) -> Result<(), RunStateError> {
        if !self.synced_stop(true) {
            return Ok(());
        }
        let connection = &self.connection.connection;
        let marks = Marks {
            declare: self.deferral.declaring(false),
            clean: self.marked,
        };
        let committed = begin(connection, &self.path, self.epoch)
            .and_then(|()| finish(connection, &self.path, marks));
        match committed {
            Ok(declared) => {
                self.deferral.declared(declared);
                self.signals.commits.send_modify(|count| *count += 1);
                self.committed(true, true);
                Ok(())
            }
            Err(error) => {
                self.signals.failed(&self.path, &error);
                Err(error)
            }
        }
    }

    /// Commits the open batch, if one is open, as [`Writer::commit_batch`]
    /// does: what its barriers are owed, and those barriers, unanswered.
    fn write_open(
        &mut self,
        sealed: Sealed,
        clean: bool,
        last: bool,
    ) -> Option<(Result<(), RunStateError>, Vec<Completion>)> {
        let batching = self.batching.as_mut()?;
        let open = batching.open.take()?;
        batching.opened.send_replace(None);
        let connection = self.connection.for_group(open.synced);
        let marks = Marks {
            declare: self.deferral.declaring(open.declare),
            clean,
        };
        let committed = finish(connection, &self.path, marks);
        let answer = match &committed {
            Ok(declared) => {
                batching.rows.record(open.rows, &[]);
                self.deferral.declared(*declared);
                self.signals.commits.send_modify(|count| *count += 1);
                if matches!(sealed, Sealed::Early) {
                    self.signals.work.count(Row::RunBatches);
                }
                Ok(())
            }
            Err(error) => {
                self.signals.failed(&self.path, error);
                Err(error.clone())
            }
        };
        if committed.is_ok() {
            self.committed(open.synced, last);
        }
        Some((answer, open.owed))
    }

    /// Rolls the open batch back on `error`, which its barriers are
    /// answered with: the writes that rode it were answered already, and
    /// are lost as a crash would lose them.
    fn fail_batch(&mut self, error: &RunStateError) {
        let Some(batching) = &mut self.batching else {
            return;
        };
        let Some(open) = batching.open.take() else {
            return;
        };
        batching.opened.send_replace(None);
        roll_back(self.connection.for_group(open.synced));
        self.signals.failed(&self.path, error);
        for completion in open.owed {
            completion(Err(error.clone()));
        }
    }
}

/// Starts the timer of the writer's open batch, one of `owner`'s tasks:
/// once `every` passed on `clock` since the batch `opened`, it queues a
/// tick into `mailbox`, so the writer commits a batch no later write
/// commits. It holds no handle on the writer, and ends with the writer's
/// batching or once `owner` is cancelled.
fn batch_timer(
    owner: &TaskOwner,
    every: Duration,
    clock: Clock,
    mailbox: &mpsc::Sender<Queued>,
    mut opened: watch::Receiver<Option<Instant>>,
) {
    let mailbox = mailbox.downgrade();
    let cancelled = owner.token().clone();
    owner.spawn(async move {
        loop {
            let began = *opened.borrow_and_update();
            if let Some(began) = began {
                tokio::select! {
                    biased;
                    () = cancelled.cancelled() => break,
                    changed = opened.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        continue;
                    }
                    () = clock.sleep_until(began + every) => {}
                }
                let Some(mailbox) = mailbox.upgrade() else {
                    break;
                };
                // A full mailbox holds a group already, which commits the
                // batch past its interval.
                let _ = mailbox.try_send(Queued::tick());
            }
            tokio::select! {
                biased;
                () = cancelled.cancelled() => break,
                changed = opened.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

impl Drop for Writer {
    /// A writer whose every handle is gone commits its open batch.
    fn drop(&mut self) {
        self.commit_batch(Sealed::Early, false, true);
    }
}
