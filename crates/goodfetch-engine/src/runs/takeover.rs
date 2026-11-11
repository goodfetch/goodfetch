//! Taking the writer role of a Run no engine of this one's runs, to abort it
//! (ADRs 0003, 0031 and 0098): the Run's writer lock, then the next run
//! epoch, then the end the Run's table gives an abort (ADR 0128), written
//! under it. A held lock goes first to the
//! owner the Run index names, then is retried with backoff, since a
//! reader's test holds it for a moment, before the abort fails as busy. A
//! Run that needs a Product this engine is not is refused before anything
//! is written.

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{ProductVersion, StoppedBy};
use goodfetch_protocol::run::{
    Lifecycle, Presence, RunAxes, RunId, RunIndexRow, RunReport, StopReason,
};

use super::folder::{Recorded, read_recorded, write_end};
use super::lifecycle::{self, Next, RunEvent};
use super::report::{self, EndFacts, ReportPart};
use super::writer::{retried, try_lock};
use super::{Forwarder, RUN_FILE, RunError};
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::events::Events;
use crate::run_index;
use crate::run_state::{self, RunState, RunStateOptions, RunStateTables};
use crate::tasks::TaskOwner;

/// An abort that takes the writer role of a Run, and what it needs.
pub(super) struct TakeOver {
    pub(super) run_id: RunId,
    pub(super) folder: PathBuf,
    pub(super) location: String,
    pub(super) by: StoppedBy,
    pub(super) product: Product,
    pub(super) run_state: RunStateOptions,
    pub(super) tables: Vec<RunStateTables>,
    pub(super) clock: Clock,
    pub(super) entropy: Entropy,
    pub(super) index: PathBuf,
    /// This engine's Product version, which the Run index's marker holds
    /// to its lowest minor (ADR 0098).
    pub(super) version: ProductVersion,
    pub(super) events: Events,
    pub(super) forwarder: Arc<dyn Forwarder>,
    pub(super) owner: Arc<TaskOwner>,
    /// What the composition's areas add to the Run's report.
    pub(super) parts: Vec<Arc<dyn ReportPart>>,
}

/// What an abort does to the Run its state holds.
enum Decided {
    /// It ended already: its report.
    Report(Box<RunReport>),
    /// It stops, in this Lifecycle.
    Stop(Box<Recorded>, Lifecycle),
}

/// What taking the writer's lock came to.
enum Taken {
    /// This engine holds it.
    Lock(File),
    /// The owner the Run index names stopped the Run.
    Forwarded(Box<RunReport>),
    /// Another kept it through every retry.
    Busy,
}

impl TakeOver {
    /// Aborts the Run: answers the report of one that ended, has its owner
    /// stop one another engine writes, or takes its writer role and stops
    /// it.
    pub(super) async fn abort(mut self) -> Result<RunReport, RunError> {
        let Some(recorded) = self.recorded().await? else {
            return Err(RunError::NotFound(self.run_id));
        };
        let recorded = match self.decide(recorded)? {
            Decided::Report(report) => return Ok(*report),
            Decided::Stop(recorded, _) => *recorded,
        };
        if !self.product.contains(recorded.needs) {
            return Err(RunError::NotInProduct {
                run_id: self.run_id,
                needs: recorded.needs,
            });
        }
        let lock = match self.take_lock().await? {
            Taken::Lock(lock) => lock,
            Taken::Forwarded(report) => return Ok(*report),
            Taken::Busy => {
                return Err(RunError::Busy {
                    run_id: self.run_id,
                    folder: self.folder,
                });
            }
        };
        // Another engine may have ended it before this one took the lock.
        let Some(recorded) = self.recorded().await? else {
            return Err(RunError::NotFound(self.run_id));
        };
        let (recorded, lifecycle) = match self.decide(recorded)? {
            Decided::Report(report) => return Ok(*report),
            Decided::Stop(recorded, lifecycle) => (*recorded, lifecycle),
        };
        let report = self.stop(&recorded, lifecycle).await?;
        let row = RunIndexRow {
            run_id: self.run_id.clone(),
            location: self.location.clone(),
            kind: recorded.kind,
            needs: recorded.needs,
            tags: recorded.tags,
            lifecycle: report.lifecycle,
            verdict: Some(report.verdict),
            completeness: Some(report.completeness),
            started_at: report.started_at.clone(),
            ended_at: Some(report.ended_at.clone()),
            owner: None,
            presence: Presence::Present,
            progress_message: recorded.progress_message.clone(),
            stop_reason: report.stop_reason.clone(),
            replayed: report.replayed(),
        };
        let index = self.index.clone();
        let version = self.version.clone();
        if let Err(error) = self
            .owner
            .blocking(move || run_index::write(&index, &version, &row))
            .await
        {
            tracing::warn!(%error, "the Run index row was not written; `runs reindex` restores it");
        }
        drop(lock);
        self.events
            .publish_axes(RunAxes {
                run_id: self.run_id,
                lifecycle: report.lifecycle,
                verdict: Some(report.verdict),
                completeness: Some(report.completeness),
            })
            .await;
        Ok(report)
    }

    /// What the Run's table makes of the abort of the Run `recorded`
    /// holds: an ended Run's report, which it leaves as it is, or the
    /// Lifecycle a `running` one stops in.
    fn decide(&self, recorded: Recorded) -> Result<Decided, RunError> {
        let state = recorded
            .report
            .as_ref()
            .map_or(Lifecycle::Running, |report| report.lifecycle);
        match lifecycle::apply(state, RunEvent::Abort) {
            Ok(Next::To(lifecycle)) => Ok(Decided::Stop(Box::new(recorded), lifecycle)),
            Ok(Next::Unchanged) => match recorded.report {
                Some(report) => Ok(Decided::Report(Box::new(report))),
                None => Err(RunError::Internal {
                    run_id: self.run_id.clone(),
                    message: "the Run's table left a running Run running on an abort".to_owned(),
                }),
            },
            Err(refused) => Err(RunError::refused(&self.run_id, &self.folder, refused)),
        }
    }

    fn unavailable(&self, error: &impl ToString) -> RunError {
        RunError::Unavailable {
            run_id: self.run_id.clone(),
            folder: self.folder.clone(),
            error: error.to_string(),
        }
    }

    async fn recorded(&self) -> Result<Option<Recorded>, RunError> {
        let file = self.folder.join(RUN_FILE);
        let vfs = self.run_state.vfs.clone();
        self.owner
            .blocking(move || read_recorded(&file, vfs.as_deref()))
            .await
            .map_err(|error| self.unavailable(&error))
    }

    /// Takes the writer's lock; when it is held, asks the owner the Run
    /// index names, then retries it with backoff.
    async fn take_lock(&mut self) -> Result<Taken, RunError> {
        if let Some(lock) = self.try_lock().await? {
            return Ok(Taken::Lock(lock));
        }
        if let Some(report) = self.forward().await {
            return Ok(Taken::Forwarded(Box::new(report)));
        }
        let (owner, folder) = (&self.owner, &self.folder);
        let retried = retried(&self.clock, &mut self.entropy, || try_lock(owner, folder)).await;
        match retried {
            Ok(Some(lock)) => Ok(Taken::Lock(lock)),
            Ok(None) => Ok(Taken::Busy),
            Err(error) => Err(self.unavailable(&error)),
        }
    }

    async fn try_lock(&self) -> Result<Option<File>, RunError> {
        try_lock(&self.owner, &self.folder)
            .await
            .map_err(|error| self.unavailable(&error))
    }

    /// Asks the owner the Run index names to abort the Run: its report, or
    /// `None` when the index names none or it did not stop the Run.
    async fn forward(&self) -> Option<RunReport> {
        let index = self.index.clone();
        let version = self.version.clone();
        let run_id = self.run_id.clone();
        let row = self
            .owner
            .blocking(move || run_index::read(&index, &version, &run_id))
            .await;
        let owner = match row {
            Ok(row) => row?.owner?,
            Err(error) => {
                tracing::warn!(%error, "the Run index cannot be read; the writer lock is retried");
                return None;
            }
        };
        self.forwarder
            .abort(&owner, &self.run_id, &self.owner)
            .await
    }

    /// Under the writer's lock: takes the next run epoch, then writes the
    /// Run's end under it.
    async fn stop(&self, recorded: &Recorded, lifecycle: Lifecycle) -> Result<RunReport, RunError> {
        let file = self.folder.join(RUN_FILE);
        let (options, tables) = (self.run_state.clone(), self.tables.clone());
        let next = file.clone();
        let epoch = self
            .owner
            .blocking(move || run_state::next_epoch(&next, &options, &tables, 0))
            .await
            .map_err(|error| self.unavailable(&error))?;
        tracing::info!(
            target: "goodfetch::lease",
            transition = "take_epoch",
            run_id = %self.run_id,
            run_epoch = epoch,
            "took the Run's writer role to abort it"
        );
        let writer = self.owner.child();
        let state = RunState::open(file, &self.run_state, self.tables.clone(), epoch, &writer)
            .await
            .map_err(|error| self.unavailable(&error))?;
        let facts = EndFacts {
            run_id: self.run_id.clone(),
            lifecycle,
            stop_reason: Some(StopReason::Abort { by: self.by }),
            started_at: recorded.started_at.clone(),
            ended_at: self.clock.point_in_time(),
            policy_version: recorded.policy_version.clone(),
            weakened: recorded.weakened.clone(),
            settings: None,
            checkpoint: None,
            limits: Vec::new(),
            error: None,
            finished: Vec::new(),
        };
        let parts = self.parts.clone();
        let ended = state
            .write(move |connection| {
                let report = report::assemble(connection, &facts, &parts)?;
                write_end(connection, &report)?;
                Ok(report)
            })
            .await;
        drop(state);
        writer.wait().await;
        ended.map_err(|error| self.unavailable(&error))
    }
}
