//! A Run's opening beside the Runs' registry (ADR 0145, issue 2258): the
//! disk work of a start or a resume, from the read of its run file or
//! Checkpoint to its lock and its next run epoch, runs in a task of its
//! own, which owns what it reads. The registry's loop keeps every
//! decision: the task hands each to it ([`Decisions`]), so only the loop
//! reads the Lifecycle's table and changes what the registry keeps, and it
//! answers the calls on every other Run meanwhile.
//!
//! Size: one concern, a Run's opening, kept whole: a start's and a
//! resume's disk work share each step, from the read of what the folder
//! holds to the lock and the run file opened, and each step's decision
//! goes back to the loop at the same seam.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::RunBinding;
use goodfetch_protocol::run::{
    Lifecycle, Presence, ProgressMessage, RunAxes, RunId, RunIndexRow, RunKind, RunStarted,
    StartInput, StartedAs, StopReason,
};
use tokio::sync::{mpsc, oneshot, watch};

use super::folder::{
    self, Compared, FixedSettings, Found, Recorded, RunRow, canonical_inputs, input_differences,
    masked_tags, read_found,
};
use super::lifecycle::{self, Next, RunEvent};
use super::writer::{retried, try_lock};
use super::{
    Command, ConnectionId, Ended, Keys, Limits, Live, Registry, Resume, Resumed, Run, RunError,
    Start, StartError, Started, StorageExhausted, UncleanDiscard, critical_among, discard,
    durability_of, joined_weakened, ledger_read_interval, periodic_checkpoints, progress,
    warn_over_charged,
};
use crate::checkpoint;
use crate::clock::Clock;
use crate::engine_context::CALLER;
use crate::entropy::Entropy;
use crate::events::Events;
use crate::governor::{Filesystem, Governor};
use crate::ledger::{Charging, Refusal};
use crate::mask::Scope;
use crate::run_index::RunIndex;
use crate::run_key::StoredKey;
use crate::run_state::install::InstallDatabase;
use crate::run_state::{
    self, Durability, RUN_FILE, RunState, RunStateError, RunStateOptions, RunStateTables, Syncing,
};
use crate::secrets::RunSecrets;
use crate::settings::RunSettings;
use crate::tasks::{OwnerKind, TaskOwner};
use crate::trace::Link;

/// The registry's loop is gone: the engine stops, and so does the work
/// beside it.
pub(super) struct Gone;

impl From<Gone> for StartError {
    fn from(_: Gone) -> Self {
        Self::Stopped
    }
}

impl From<Gone> for RunError {
    fn from(_: Gone) -> Self {
        Self::Stopped
    }
}

/// The registry's loop, as the work beside it hands it each decision.
#[derive(Clone)]
pub(super) struct Decisions(mpsc::WeakSender<Command>);

impl Decisions {
    /// What `decision` answers, taken in the loop on what the registry
    /// keeps.
    pub(super) async fn decide<T: Send + 'static>(
        &self,
        decision: impl FnOnce(&mut Registry) -> T + Send + 'static,
    ) -> Result<T, Gone> {
        let (answer, answered) = oneshot::channel();
        self.send(Command::Decide(Box::new(move |registry| {
            let _ = answer.send(decision(registry));
        })))
        .await?;
        answered.await.map_err(|_| Gone)
    }

    /// The machine's identity, which the loop reads once (ADR 0056).
    async fn machine(&self) -> Result<String, RunError> {
        let (answer, answered) = oneshot::channel();
        self.send(Command::Machine(answer)).await?;
        answered.await.map_err(|_| RunError::Stopped)?
    }

    /// Sends `command` to the loop, holding its mailbox only meanwhile, so
    /// the work beside it never keeps the registry from ending.
    async fn send(&self, command: Command) -> Result<(), Gone> {
        let registry = self.0.upgrade().ok_or(Gone)?;
        registry.send(command).await.map_err(|_| Gone)
    }
}

/// What a Run's disk work beside the registry reads and writes with.
pub(super) struct Disk {
    run_id: RunId,
    folder: PathBuf,
    /// Under whose blocking policy it runs.
    owner: Arc<TaskOwner>,
    /// What its retries wait on.
    clock: Clock,
    /// The VFS run files open through.
    vfs: Option<String>,
    /// The install database, in the per-install layout.
    install: Option<Arc<InstallDatabase>>,
    /// The Run index, as this engine opened it.
    index: RunIndex,
}

impl Disk {
    /// A start's refusal for `error`, of the Run's folder.
    fn unavailable(&self, error: String) -> StartError {
        StartError::Unavailable {
            run_id: self.run_id.clone(),
            folder: self.folder.clone(),
            error,
        }
    }

    /// Discards the state of the Run, which this engine does not run, when
    /// its run file is one a crash left at `off` (ADR 0138), the Run's
    /// Durability level now being `durability`: under its writer lock, as
    /// a discard does, so another engine's live Run is never touched. Its
    /// Run index row is marked *erased* then, as a discard marks it.
    #[cfg_attr(
        windows,
        expect(
            clippy::result_large_err,
            reason = "a Windows path is wider than a Unix one, and the error is a start's rare failure"
        )
    )]
    async fn discard_unclean(
        &self,
        entropy: &mut Entropy,
        durability: Durability,
    ) -> Result<bool, UncleanDiscard> {
        let discarding = discard::Discarding {
            owner: &self.owner,
            clock: &self.clock,
            entropy,
        };
        let discarded = discard::discard_unclean(
            discarding,
            self.run_id.as_str(),
            &self.folder,
            self.vfs.as_deref(),
            self.install.as_ref(),
            durability,
        )
        .await
        .map_err(|error| UncleanDiscard {
            run_id: self.run_id.clone(),
            folder: self.folder.clone(),
            error,
        })?;
        if discarded {
            let run_id = &self.run_id;
            tracing::warn!(run_id = %run_id, "the Run's run file was left by a crash at `run_state.durability = off`: it was discarded, and the Run starts again");
            // A later start under its Run ID then begins it rather than
            // finding a listed Run whose state is missing (ADR 0102).
            if let Err(error) = self.index.erase(run_id.clone()).await {
                tracing::warn!(%error, run_id = %run_id, "the discarded Run's Run index row was not marked erased");
            }
        }
        Ok(discarded)
    }

    /// The Run its folder holds.
    pub(super) async fn existing(&self) -> Result<Option<Found>, String> {
        let (file, vfs) = (self.folder.join(RUN_FILE), self.vfs.clone());
        self.owner
            .blocking(move || read_found(&file, vfs.as_deref()))
            .await
    }

    /// Refuses with `Unavailable` a start under the ID of a Run the Run
    /// index lists, not erased, whose state was not found: a start never
    /// begins afresh a Run whose state is lost (ADR 0102). An index that
    /// cannot be read lists nothing, as a start's check of it says.
    async fn listed_present(&self) -> Result<(), StartError> {
        match self.index.read(self.run_id.clone()).await {
            Ok(Some(row)) if row.presence != Presence::Erased => Err(self
                .unavailable("the Run index lists the Run, but its state is missing".to_owned())),
            Ok(_) => Ok(()),
            Err(error) => {
                tracing::warn!(%error, run_id = %self.run_id, "the Run index cannot be read; the Run starts without it");
                Ok(())
            }
        }
    }

    /// Refuses a start when the Run index's marker refuses this engine
    /// (ADR 0098): an engine that finds its lowest minor raised past itself
    /// takes no new Run. An index that cannot be opened stops no Run; its
    /// row is restored by `runs reindex`.
    async fn check_index(&self) -> Result<(), StartError> {
        match self.index.check().await {
            Err(RunStateError::Format {
                format,
                found,
                compatibility,
                install,
            }) => Err(StartError::Format {
                format,
                found,
                compatibility,
                install,
            }),
            Err(error) => {
                tracing::warn!(%error, "the Run index cannot be read; the Run starts without it");
                Ok(())
            }
            Ok(()) => Ok(()),
        }
    }

    /// The first of `folders` whose filesystem is at `critical` now, read
    /// afresh rather than from the last sample (ADR 0122), with it.
    async fn critical_among(
        &self,
        governor: &Governor,
        folders: Vec<PathBuf>,
    ) -> Option<(PathBuf, Filesystem)> {
        let governor = governor.clone();
        self.owner
            .blocking(move || critical_among(&governor, folders))
            .await
    }

    /// Takes the Run's lock, which no other engine may hold while it runs
    /// there; one found held, perhaps only by a reader's test, is retried
    /// as every writer retries it (ADR 0031).
    async fn lock(&self, entropy: &mut Entropy) -> Result<File, StartError> {
        let (owner, folder) = (&*self.owner, self.folder.as_path());
        let taken = match try_lock(owner, folder).await {
            Ok(Some(lock)) => Ok(Some(lock)),
            // Only a retry draws from the `Entropy`.
            Ok(None) => retried(&self.clock, entropy, || try_lock(owner, folder)).await,
            Err(error) => Err(error),
        };
        match taken {
            Ok(Some(lock)) => Ok(lock),
            Ok(None) => Err(StartError::InUse {
                run_id: self.run_id.clone(),
                folder: self.folder.clone(),
            }),
            Err(error) => Err(self.unavailable(error)),
        }
    }

    /// The charging of the new Run, started on `connection`, once it may
    /// start: every Limit that holds it has something left, and none it
    /// cannot be placed outside of is one this engine cannot evaluate (ADR
    /// 0041). A ledger that cannot be read refuses it. The Run then holds
    /// the Limits this read, on the connection it opened, so its charges
    /// open nothing (ADR 0140's E3), and reads them again as its resolved
    /// `settings` say.
    async fn may_start(
        &self,
        limits: &Limits,
        connection: Option<ConnectionId>,
        settings: Option<&RunSettings>,
    ) -> Result<Charging, StartError> {
        let now = limits.now();
        let mut charging = limits.charging(
            &self.run_id,
            &self.folder,
            1,
            limits.placement(&self.run_id, connection),
            ledger_read_interval(settings),
        );
        let may_start = self
            .owner
            .blocking(move || charging.may_start(now).map(|()| charging))
            .await;
        match may_start {
            Ok(charging) => Ok(charging),
            Err(Refusal::Reached(reached)) => Err(StartError::LimitReached(reached)),
            Err(Refusal::NotUnderstood(limit)) => Err(StartError::LimitNotUnderstood(limit)),
            Err(Refusal::Ledger(error)) => Err(StartError::Unavailable {
                run_id: self.run_id.clone(),
                folder: limits.ledger.path().to_owned(),
                error: error.to_string(),
            }),
        }
    }

    /// What the Checkpoint `checkpoint`, or else the Run's run file, holds
    /// of the Run, refusing a version this engine cannot read.
    pub(super) async fn held(
        &self,
        checkpoint: Option<&Path>,
    ) -> Result<checkpoint::Held, RunError> {
        let (run_id, folder) = (&self.run_id, &self.folder);
        let held = if let Some(checkpoint) = checkpoint {
            let path = checkpoint.to_owned();
            self.owner
                .blocking(move || checkpoint::read(&checkpoint::open_checkpoint(&path)?, &path))
                .await
        } else {
            let found = self
                .existing()
                .await
                .map_err(|error| RunError::Unavailable {
                    run_id: run_id.clone(),
                    folder: folder.clone(),
                    error,
                })?;
            if found.is_none() {
                return Err(RunError::NotFound(run_id.clone()));
            }
            let (path, vfs) = (folder.join(RUN_FILE), self.vfs.clone());
            self.owner
                .blocking(move || {
                    checkpoint::read(&run_state::open_reader(&path, vfs.as_deref())?, &path)
                })
                .await
        }
        .map_err(|error| RunError::of_state(run_id, folder, error))?;
        if held.run_id != run_id.as_str() {
            return Err(RunError::Unavailable {
                run_id: run_id.clone(),
                folder: folder.clone(),
                error: format!("the Checkpoint holds the Run `{}`", held.run_id),
            });
        }
        Ok(held)
    }

    /// What [`Self::held`] reads, refused when its wrapped data key is not
    /// `stored`, the one a resume unwrapped before the registry took it.
    async fn held_with_key(
        &self,
        checkpoint: Option<&Path>,
        stored: Option<StoredKey>,
    ) -> Result<checkpoint::Held, RunError> {
        let held = self.held(checkpoint).await?;
        if held.key != stored {
            return Err(RunError::Unavailable {
                run_id: self.run_id.clone(),
                folder: self.folder.clone(),
                error: "its data key changed while it resumed; resume it again".to_owned(),
            });
        }
        Ok(held)
    }

    /// What the Run's run file records of it.
    async fn recorded(&self) -> Result<Recorded, RunError> {
        let (read, vfs) = (self.folder.join(RUN_FILE), self.vfs.clone());
        self.owner
            .blocking(move || folder::read_recorded(&read, vfs.as_deref()))
            .await
            .map_err(|error| RunError::Unavailable {
                run_id: self.run_id.clone(),
                folder: self.folder.clone(),
                error: error.to_string(),
            })?
            .ok_or_else(|| RunError::NotFound(self.run_id.clone()))
    }

    /// Takes the writer role of the Run, `held` as its state says: its
    /// lock, which another writer's holding refuses as the Run's table says
    /// (`WriterFound`), the Checkpoint `checkpoint` made its run file when
    /// given, then the next run epoch, under which `writer` opens it with
    /// `options` and marks it running again.
    async fn reopen(
        &self,
        entropy: &mut Entropy,
        held: Lifecycle,
        checkpoint: Option<PathBuf>,
        (options, tables): (&RunStateOptions, &[RunStateTables]),
        writer: &TaskOwner,
    ) -> Result<(File, RunState), RunError> {
        let (run_id, folder) = (&self.run_id, &self.folder);
        let file = folder.join(RUN_FILE);
        let lock = self
            .lock(entropy)
            .await
            .map_err(|error| writer_found(run_id, error, held))?;
        let mut after = 0;
        if let Some(checkpoint) = checkpoint {
            // The run epoch of a run file of this Run already here, which the
            // Checkpoint replaces: the resume's goes past it too.
            if let Ok(Some(_)) = self.existing().await
                && let Ok(here) = self.held(None).await
            {
                after = here.run_epoch;
            }
            let (into, vfs) = (file.clone(), self.vfs.clone());
            self.owner
                .blocking(move || checkpoint::install(&checkpoint, &into, vfs.as_deref()))
                .await
                .map_err(|error| RunError::of_write(run_id, folder, error))?;
        }
        let (taking, opened, next) = (options.clone(), tables.to_vec(), file.clone());
        let epoch = self
            .owner
            .blocking(move || run_state::next_epoch(&next, &taking, &opened, after))
            .await
            .map_err(|error| RunError::of_state(run_id, folder, error))?;
        tracing::info!(
            target: "goodfetch::lease",
            transition = "take_epoch",
            run_id = %run_id,
            run_epoch = epoch,
            "took the Run's writer role to resume it"
        );
        let state = RunState::open(file, options, tables.to_vec(), epoch, writer)
            .await
            .map_err(|error| RunError::of_state(run_id, folder, error))?;
        state
            .write(|connection| {
                connection
                    .prepare_cached(
                        "UPDATE run SET lifecycle = ?1, stop_reason = NULL, verdict = NULL,
                         completeness = NULL, ended_at = NULL",
                    )?
                    .execute([Lifecycle::Running.as_str()])
                    .map(drop)
            })
            .await
            .map_err(|error| RunError::of_state(run_id, folder, error))?;
        Ok((lock, state))
    }
}

/// A resume's refusal for `error`, its lock's: another writer's holding
/// refuses the Run `run_id` as its table says for `held` (`WriterFound`).
fn writer_found(run_id: &RunId, error: StartError, held: Lifecycle) -> RunError {
    match error {
        StartError::InUse { run_id, folder } => match lifecycle::apply(held, RunEvent::WriterFound)
        {
            Err(refused) => RunError::refused(&run_id, &folder, refused),
            Ok(_) => RunError::InUse { run_id, folder },
        },
        StartError::Unavailable {
            run_id,
            folder,
            error,
        } => RunError::Unavailable {
            run_id,
            folder,
            error,
        },
        StartError::Internal { run_id, message } => RunError::Internal { run_id, message },
        StartError::Format {
            format,
            found,
            compatibility,
            install,
        } => RunError::Format {
            run_id: run_id.clone(),
            format,
            found,
            compatibility,
            install,
        },
        StartError::ResourcesExhausted(exhausted) => RunError::ResourcesExhausted(exhausted),
        StartError::Discarding { run_id, folder } => RunError::Discarding { run_id, folder },
        StartError::FixedSetting(changed) => RunError::FixedSetting(changed),
        StartError::Conflict { .. }
        | StartError::Live { .. }
        | StartError::LimitReached(_)
        | StartError::LimitNotUnderstood(_)
        | StartError::Draining { .. }
        | StartError::Secret(_)
        | StartError::Refused(_)
        | StartError::Stopped => RunError::Stopped,
    }
}

/// Writes `row` to `index`; one not written is restored by `runs reindex`.
pub(super) async fn write_index(index: &RunIndex, row: RunIndexRow) {
    if let Err(error) = index.write(row).await {
        tracing::warn!(%error, "the Run index row was not written; `runs reindex` restores it");
    }
}

/// What a start declares of its Run, which a Run found under its Run ID
/// must match.
struct Declared {
    kind: RunKind,
    needs: Product,
    /// Its inputs, as given.
    given: String,
    /// Its inputs, masked as run state keeps them.
    inputs: String,
    /// The settings fixed for the Run's life, masked.
    fixed: FixedSettings,
}

/// What a start found under its Run ID, once it matched what the start
/// declared.
enum FoundRun {
    /// A finished Run, answered as it ended.
    Ended(RunStarted),
    /// An interrupted Run, which its caller resumes.
    Interrupted,
}

/// What a start or a resume opens its Run with, beside the registry.
struct Opening {
    disk: Disk,
    /// Drawn from only by a retry of its lock or its discard's.
    entropy: Entropy,
    decisions: Decisions,
    connection: Option<ConnectionId>,
    limits: Arc<Limits>,
    governor: Governor,
    /// The folders of the essential writes it reads afresh (ADR 0122).
    essential: Vec<PathBuf>,
    /// How its run file opens.
    options: RunStateOptions,
    tables: Vec<RunStateTables>,
    /// The owner its writer runs under.
    writer: Arc<TaskOwner>,
    events: Events,
}

impl Registry {
    /// The disk of the Run `run_id`, for its work beside the loop.
    pub(super) fn disk_of(&self, run_id: &RunId) -> Disk {
        Disk {
            run_id: run_id.clone(),
            folder: self.folder(run_id),
            owner: Arc::clone(&self.disk),
            clock: self.setup.engine.clock(),
            vfs: self.setup.run_state.vfs.clone(),
            install: self.setup.install.clone(),
            index: self.setup.engine.run_index().clone(),
        }
    }

    /// The loop, as the work beside it hands it each decision.
    pub(super) fn decisions(&self) -> Decisions {
        Decisions(self.runs.clone())
    }

    /// What the Run `run_id`'s start or resume opens it with, under its
    /// resolved `settings`, started on `connection`.
    fn opening(
        &mut self,
        run_id: &RunId,
        connection: Option<ConnectionId>,
        essential: Vec<PathBuf>,
        settings: Option<&RunSettings>,
    ) -> Opening {
        Opening {
            disk: self.disk_of(run_id),
            entropy: self.setup.entropy.fork(),
            decisions: self.decisions(),
            connection,
            limits: Arc::clone(&self.limits),
            governor: self.setup.governor.clone(),
            essential,
            options: self.opened_with(settings),
            tables: self.setup.tables.clone(),
            writer: Arc::new(self.owner.child_for(OwnerKind::Run)),
            events: self.setup.events.clone(),
        }
    }

    /// Starts a Run as [`super::Runs::start`] says: a Run this engine runs
    /// under its Run ID is compared in the loop, and one its folder holds,
    /// or a new one, beside it, which answers `answer` once its Run began
    /// or was refused.
    pub(super) fn start(
        &mut self,
        start: Box<(Start, RunSecrets, Link)>,
        connection: Option<ConnectionId>,
        answer: oneshot::Sender<Result<Started, StartError>>,
    ) {
        let (mut start, secrets, link) = *start;
        let run_id = match &start.run_id {
            Some(run_id) => run_id.clone(),
            None => self.generated_id(),
        };
        // Its tags and inputs pass through the `Mask` before run state and
        // the Run index keep them (spec #564), as every writer does.
        let given = canonical_inputs(&start.inputs);
        let mut kept = start.inputs.clone();
        for value in kept.values_mut() {
            self.setup.masks.apply_value(value, Scope::Kept);
        }
        let inputs = canonical_inputs(&kept);
        start.tags = masked_tags(&self.setup.masks, &start.tags);
        let declared = Declared {
            kind: start.kind,
            needs: start.needs,
            given,
            inputs: inputs.clone(),
            fixed: FixedSettings::of(start.settings.as_ref(), &self.setup.masks),
        };
        // The start table (ADR 0102): a Run this engine runs is in use.
        if let Some(live) = self.live.get(&run_id) {
            let found = Found {
                kind: live.kind,
                needs: live.needs,
                caller: CALLER.to_owned(),
                inputs: live.inputs.clone(),
                lifecycle: live.lifecycle,
                fixed_settings: live.fixed_settings.clone(),
            };
            let folder = live.folder.clone();
            let refused = self
                .found(&run_id, &declared, &found)
                .and(Err(StartError::Live { run_id, folder }));
            let _ = answer.send(refused);
            return;
        }
        let row = self.run_row(&run_id, &start, inputs, self.now());
        let syncing = self.start_syncing(start.kind);
        let essential = self.engine_folders().essential();
        let opening = self.opening(&run_id, connection, essential, start.settings.as_ref());
        self.beside(Some(run_id), async move {
            let started = opening
                .start(declared, start, secrets, (row, syncing))
                .await;
            if let Ok(Started::New(run)) = &started {
                link.add_to(run.identity.span());
            }
            let _ = answer.send(started);
        });
    }

    /// Resumes an interrupted Run as [`super::Runs::resume`] says: one this
    /// engine runs is answered in the loop, and any other beside it, which
    /// answers `answer` once the Run went on or was refused.
    pub(super) fn resume(
        &mut self,
        resume: Box<(Resume, Link, Keys, Option<StoredKey>)>,
        connection: Option<ConnectionId>,
        answer: oneshot::Sender<Result<Resumed, RunError>>,
    ) {
        let (resume, link, keys, stored) = *resume;
        // Fixed keys the resume gives keep their values, in any state (ADR 0102).
        let fixed = FixedSettings::of(resume.settings.as_ref(), &self.setup.masks);
        if let Some(live) = self.resumed_live(&resume.run_id, &fixed) {
            let _ = answer.send(live);
            return;
        }
        let run_id = resume.run_id.clone();
        let essential = vec![
            self.run_state_of(&self.folder(&run_id)),
            self.setup.engine.home().root().to_owned(),
        ];
        let opening = self.opening(&run_id, connection, essential, resume.settings.as_ref());
        self.beside(Some(run_id), async move {
            let resumed = opening.resume(resume, fixed, keys, stored).await;
            if let Ok(Resumed::New { run, .. }) = &resumed {
                link.add_to(run.identity.span());
            }
            let _ = answer.send(resumed);
        });
    }

    /// Whether the Run `run_id` that a start found, `found`, matches what
    /// the start `declared`, and what the start table (ADR 0102) answers
    /// of it: a finished one is answered as it ended, and any other is
    /// interrupted, which its caller resumes.
    fn found(
        &self,
        run_id: &RunId,
        declared: &Declared,
        found: &Found,
    ) -> Result<FoundRun, StartError> {
        let mut differences = Vec::new();
        if found.needs != declared.needs {
            differences.push(StartInput::Product);
        }
        if found.kind != declared.kind {
            differences.push(StartInput::Kind);
        }
        if found.caller != CALLER {
            differences.push(StartInput::Caller);
        }
        // A value that left the masks since the Run kept its inputs
        // masked is no difference: an input differs when it differs
        // both masked and as given.
        let unmasked = input_differences(&found.inputs, &declared.given);
        differences.extend(
            input_differences(&found.inputs, &declared.inputs)
                .into_iter()
                .filter(|input| unmasked.contains(input)),
        );
        if !differences.is_empty() {
            return Err(StartError::Conflict {
                run_id: run_id.clone(),
                differences,
            });
        }
        // A start declares the whole Run again: every key fixed for
        // its life keeps the value it began with (ADRs 0102 and 0120).
        declared
            .fixed
            .unchanged(run_id, &found.fixed_settings, Compared::Every)
            .map_err(StartError::FixedSetting)?;
        if let Err(refused) = lifecycle::apply(found.lifecycle, RunEvent::Start) {
            let folder = self.folder(run_id);
            return Err(match RunError::refused(run_id, &folder, refused) {
                RunError::InUse { run_id, folder } => StartError::InUse { run_id, folder },
                other => StartError::Internal {
                    run_id: run_id.clone(),
                    message: other.to_string(),
                },
            });
        }
        if found.lifecycle != Lifecycle::Finished {
            return Ok(FoundRun::Interrupted);
        }
        Ok(FoundRun::Ended(RunStarted {
            run_id: run_id.clone(),
            started_as: StartedAs::Ended,
            lifecycle: found.lifecycle,
            needs: found.needs,
        }))
    }

    /// Forgets the end of the Run `run_id`, whose state is gone.
    pub(super) fn forget(&mut self, run_id: &RunId) {
        self.ended.forget(run_id);
        self.found_ended.remove(run_id);
        self.unrecorded.remove(run_id);
    }

    /// Whether this engine may resume the Run `run_id` that `held` holds:
    /// one bound to its machine only there, `machine` (ADR 0056), one of
    /// its Product only, and none while the filesystem of its run state or
    /// of the Goodfetch home is at `critical`, as `critical` read it (ADR
    /// 0122); then whether its `fixed` settings are unchanged, and the
    /// Lifecycle the Run's table takes it to, none when it leaves it as it
    /// is.
    fn may_resume(
        &self,
        run_id: &RunId,
        held: &checkpoint::Held,
        (machine, critical): (Option<String>, Option<(PathBuf, Filesystem)>),
        fixed: &FixedSettings,
        event: RunEvent,
    ) -> Result<Option<Lifecycle>, RunError> {
        if let Some(RunBinding::Machine { machine: bound }) = &held.binding
            && Some(bound) != machine.as_ref()
        {
            return Err(RunError::BoundToMachine(run_id.clone()));
        }
        if !self.setup.product.contains(held.needs) {
            return Err(RunError::NotInProduct {
                run_id: run_id.clone(),
                needs: held.needs,
            });
        }
        if let Some((folder, filesystem)) = critical {
            return Err(RunError::ResourcesExhausted(Box::new(StorageExhausted {
                run_id: run_id.clone(),
                folder,
                filesystem,
            })));
        }
        fixed.unchanged(run_id, &held.fixed_settings, Compared::Given)?;
        match lifecycle::apply(held.lifecycle, event) {
            Ok(Next::To(next)) => Ok(Some(next)),
            Ok(Next::Unchanged) => Ok(None),
            Err(refused) => Err(RunError::refused(run_id, &self.folder(run_id), refused)),
        }
    }

    /// Takes `live`, which went live with `timeout` and a Checkpoint every
    /// `interval`, among the live Runs, and answers its Run.
    fn went_live(
        &mut self,
        live: Live,
        timeout: Option<std::time::Duration>,
        interval: Option<std::time::Duration>,
    ) -> Run {
        let run = live
            .run
            .clone()
            .expect("invariant: a Run goes live with its writer");
        super::watch_safe_points(&run, timeout, live.ended_by);
        self.setup.metrics.run_live(live.kind);
        self.live.insert(live.id.clone(), live);
        if let Some(interval) = interval {
            run.spawn(periodic_checkpoints(run.clone(), interval));
        }
        run
    }
}

impl Opening {
    /// The start of the Run that `declared` and `start` declare, resolving
    /// its Secret references through `secrets`, its `row` written as its
    /// run file opens with `syncing`.
    async fn start(
        mut self,
        declared: Declared,
        start: Start,
        secrets: RunSecrets,
        (row, syncing): (RunRow, Syncing),
    ) -> Result<Started, StartError> {
        let run_id = self.disk.run_id.clone();
        // A run file a crash left at `off` is discarded, never read: the Run
        // starts again (ADR 0138).
        let discarded = self
            .disk
            .discard_unclean(&mut self.entropy, durability_of(start.settings.as_ref()))
            .await?;
        if discarded {
            let id = run_id.clone();
            self.decisions
                .decide(move |registry| registry.forget(&id))
                .await?;
        }
        let found = self
            .disk
            .existing()
            .await
            .map_err(|error| self.disk.unavailable(error))?;
        if let Some(found) = found {
            let id = run_id.clone();
            let found = self
                .decisions
                .decide(move |registry| {
                    let found = registry.found(&id, &declared, &found);
                    if let Ok(FoundRun::Ended(_)) = &found {
                        registry.found_ended.insert(id);
                    }
                    found
                })
                .await??;
            return match found {
                FoundRun::Ended(started) => Ok(Started::Ended(started)),
                FoundRun::Interrupted => {
                    self.may_resume_after_limit().await?;
                    Ok(Started::Interrupted(run_id))
                }
            };
        }
        // A Run the Run index lists whose state is gone is never started
        // afresh (ADR 0102); one discarded, on purpose or after a crash at
        // `off`, starts again. A generated Run ID names no listed Run.
        if !discarded && start.run_id.is_some() {
            self.disk.listed_present().await?;
        }
        let essential = std::mem::take(&mut self.essential);
        if let Some((folder, filesystem)) =
            self.disk.critical_among(&self.governor, essential).await
        {
            return Err(StartError::ResourcesExhausted(Box::new(StorageExhausted {
                run_id,
                folder,
                filesystem,
            })));
        }
        self.begin(start, Keys { secrets, key: None }, row, syncing)
            .await
    }

    /// Refuses the start of the interrupted Run that a Limit stopped while
    /// a Limit that holds it still has nothing left: no person raised it
    /// and its period has not started again (ADR 0102's start table), as a
    /// new Run is refused. A Run stopped for another reason resumes
    /// whatever its Limits hold; its next charge reads them.
    async fn may_resume_after_limit(&self) -> Result<(), StartError> {
        let run_id = self.disk.run_id.clone();
        let report = self
            .decisions
            .decide(move |registry| registry.report_of(&run_id))
            .await?;
        // A Run whose run file holds no report, killed while it ran, was
        // stopped by no Limit; one whose file cannot be read fails the
        // resume that follows with that cause.
        let Ok(ended) = report.resolve().await else {
            return Ok(());
        };
        let stopped_by_limit = matches!(
            &*ended.borrow(),
            Some(Ended::Report(report) | Ended::Unwritten(report))
                if matches!(report.stop_reason, Some(StopReason::Limit(_)))
        );
        if !stopped_by_limit {
            return Ok(());
        }
        // Only whether it may start: its charging, and the interval it
        // would read at, go with this check.
        self.disk
            .may_start(&self.limits, self.connection, None)
            .await
            .map(drop)
    }

    /// Begins the new Run that `start` declares, with `keys`, its `row`
    /// written as its run file opens with `syncing`.
    async fn begin(
        mut self,
        start: Start,
        keys: Keys,
        mut row: RunRow,
        syncing: Syncing,
    ) -> Result<Started, StartError> {
        let run_id = self.disk.run_id.clone();
        self.disk.check_index().await?;
        let charging = self
            .disk
            .may_start(&self.limits, self.connection, start.settings.as_ref())
            .await?;
        let lock = self.disk.lock(&mut self.entropy).await?;
        let state = RunState::open(
            self.disk.folder.join(RUN_FILE),
            &self.options,
            self.tables.clone(),
            1,
            &self.writer,
        )
        .await
        .map_err(|error| self.disk.unavailable(error.to_string()))?;
        let Start {
            kind,
            needs,
            tags,
            ended_by,
            initialize,
            policy_version,
            weakened,
            checkpoint_interval,
            timeout,
            settings,
            ..
        } = start;
        row.started_at = self.disk.clock.point_in_time();
        let (started_at, inputs) = (row.started_at, row.inputs.clone());
        let fixed_settings = row.fixed_settings.clone();
        let written = tags.clone();
        state
            .write_with(syncing, move |connection| {
                row.insert(connection)?;
                for (position, tag) in written.iter().enumerate() {
                    connection
                        .prepare_cached("INSERT INTO tags (position, tag) VALUES (?1, ?2)")?
                        .execute((i64::try_from(position).unwrap_or(i64::MAX), tag.as_str()))?;
                }
                match initialize {
                    Some(initialize) => initialize(connection),
                    None => Ok(()),
                }
            })
            .await
            .map_err(|error| self.disk.unavailable(error.to_string()))?;
        let (connection, writer) = (self.connection, Arc::clone(&self.writer));
        let folder = self.disk.folder.clone();
        let times = (timeout, checkpoint_interval);
        let run = self
            .go_live(Lifecycle::Running, times, move |registry| {
                let (run, let_finish) =
                    registry.new_run(&run_id, kind, state, connection, settings, keys);
                run.shared.charging.hold(charging);
                Live {
                    id: run_id,
                    progress: run.shared.progress.clone(),
                    run: Some(run),
                    let_finish,
                    folder,
                    kind,
                    needs,
                    tags,
                    inputs,
                    fixed_settings,
                    ended_by,
                    started_at,
                    policy_version,
                    weakened,
                    report: watch::Sender::new(None),
                    writer,
                    lock: Arc::new(lock),
                    connection,
                    checkpoint: None,
                    progress_message: None,
                    lifecycle: Lifecycle::Running,
                    storage: None,
                    warned_low: std::collections::BTreeSet::new(),
                    sampled_after: registry.setup.governor.storage_samples_begun(),
                    raw_endings: Vec::new(),
                }
            })
            .await?;
        Ok(Started::New(run))
    }

    /// The Run that `live` makes live at `lifecycle`, with its timeout and
    /// its Checkpoint interval: its Run index row written, then its axes
    /// published.
    async fn go_live(
        &self,
        lifecycle: Lifecycle,
        (timeout, interval): (Option<std::time::Duration>, Option<std::time::Duration>),
        live: impl FnOnce(&mut Registry) -> Live + Send + 'static,
    ) -> Result<Run, Gone> {
        let (live, row) = self
            .decisions
            .decide(move |registry| {
                let live = live(registry);
                let row = registry.index_row(&live, lifecycle, None);
                (live, row)
            })
            .await?;
        write_index(&self.disk.index, row).await;
        let run_id = live.id.clone();
        let run = self
            .decisions
            .decide(move |registry| registry.went_live(live, timeout, interval))
            .await?;
        self.events
            .publish_axes(RunAxes {
                run_id,
                lifecycle,
                verdict: None,
                completeness: None,
            })
            .await;
        Ok(run)
    }

    /// Whether the resume of this Opening's Run may go on, read with
    /// `checkpoint`, the data key `stored` and its `fixed` settings for
    /// `event`: the Lifecycle it takes the Run to, none when it leaves it
    /// as it is, with what its file holds. A run file a crash left at
    /// `off`, under `durability`, is discarded first, never read: the Run
    /// is then not found, and starts again (ADR 0138).
    async fn may_resume(
        &mut self,
        checkpoint: Option<&Path>,
        (fixed, stored): (FixedSettings, Option<StoredKey>),
        durability: Durability,
        event: RunEvent,
    ) -> Result<(Option<Lifecycle>, checkpoint::Held), RunError> {
        let run_id = self.disk.run_id.clone();
        if self
            .disk
            .discard_unclean(&mut self.entropy, durability)
            .await?
        {
            let id = run_id.clone();
            self.decisions
                .decide(move |registry| registry.forget(&id))
                .await?;
        }
        let held = self.disk.held_with_key(checkpoint, stored).await?;
        let machine = match &held.binding {
            Some(RunBinding::Machine { .. }) => Some(self.decisions.machine().await?),
            _ => None,
        };
        let essential = std::mem::take(&mut self.essential);
        let critical = self.disk.critical_among(&self.governor, essential).await;
        let (decided, held) = self
            .decisions
            .decide(move |registry| {
                let decided =
                    registry.may_resume(&run_id, &held, (machine, critical), &fixed, event);
                (decided, held)
            })
            .await?;
        Ok((decided?, held))
    }

    /// The resume `resume` of an interrupted Run, its `fixed` settings
    /// compared, its Secret references resolved through `keys`, the data
    /// key its file holds being `stored`.
    async fn resume(
        mut self,
        resume: Resume,
        fixed: FixedSettings,
        keys: Keys,
        stored: Option<StoredKey>,
    ) -> Result<Resumed, RunError> {
        let Resume {
            run_id,
            checkpoint,
            checkpoint_interval,
            timeout,
            settings,
            retry_failed,
        } = resume;
        let (decided, held) = self
            .may_resume(
                checkpoint.as_deref(),
                (fixed, stored),
                durability_of(settings.as_ref()),
                Resume::event(retry_failed),
            )
            .await?;
        let Some(lifecycle) = decided else {
            return Ok(Resumed::unchanged(run_id, &held));
        };
        let (lock, state) = self
            .disk
            .reopen(
                &mut self.entropy,
                held.lifecycle,
                checkpoint,
                (&self.options, &self.tables),
                &self.writer,
            )
            .await?;
        let recorded = self.disk.recorded().await?;
        let id = run_id.clone();
        self.decisions
            .decide(move |registry| {
                registry.ended.forget(&id);
                registry.unrecorded.remove(&id);
            })
            .await?;
        let progress_message = recorded
            .progress_message
            .as_deref()
            .and_then(ProgressMessage::parse);
        let listed = recorded.weakened.clone();
        let folder = self.disk.folder.clone();
        let weakened = joined_weakened(&listed, settings.as_ref(), &state)
            .await
            .map_err(|error| RunError::of_state(&run_id, &folder, error))?;
        let (id, kind, connection) = (run_id.clone(), recorded.kind, self.connection);
        let (run, let_finish) = self
            .decisions
            .decide(move |registry| registry.new_run(&id, kind, state, connection, settings, keys))
            .await?;
        run.shared.charging.read_limits(&self.disk.owner).await;
        if held.lifecycle == Lifecycle::Running {
            warn_over_charged(&run).await;
        }
        progress(&run).resumed(lifecycle, progress_message.clone());
        let (writer, needs) = (Arc::clone(&self.writer), recorded.needs);
        let run = self
            .go_live(lifecycle, (timeout, checkpoint_interval), move |registry| {
                Live {
                    id: run_id,
                    progress: run.shared.progress.clone(),
                    run: Some(run),
                    let_finish,
                    folder,
                    kind,
                    needs,
                    tags: recorded.tags,
                    inputs: recorded.inputs,
                    fixed_settings: held.fixed_settings,
                    ended_by: registry.driving(kind).ended_by,
                    started_at: recorded.started_at,
                    policy_version: recorded.policy_version,
                    weakened,
                    report: watch::Sender::new(None),
                    writer,
                    lock: Arc::new(lock),
                    connection,
                    checkpoint: None,
                    progress_message,
                    lifecycle,
                    storage: None,
                    warned_low: std::collections::BTreeSet::new(),
                    sampled_after: registry.setup.governor.storage_samples_begun(),
                    raw_endings: Vec::new(),
                }
            })
            .await?;
        Ok(Resumed::New {
            run: Box::new(run),
            kind,
            needs,
            listed,
        })
    }
}
