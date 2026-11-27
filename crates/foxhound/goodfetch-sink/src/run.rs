//! A crawl's Sinks on its Run: [`Sinks`], the value Foxhound's composition
//! passes the crawl, refuses at the start a Sink this engine cannot deliver
//! to yet, probes each other Sink at the Run's start against the Run's
//! `crawl.min_sink_guarantee` (ADR 0026), and delivers its Datasets from
//! the run file while the Run goes. A Sink reads only committed Records, so
//! it waits for each of the run file's commits, one per Run-state batch,
//! and never forces one while the Run goes (spec #2138); the Run's end
//! writes the last batch and waits for a last delivery. [`contribution`]
//! answers `sink.probe`, a probe of one Sink outside any Run.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use goodfetch_engine::mask::Masks;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::runs::{RunKeys, RunParts, Runs};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::{CallError, Contribution, Handler, named};
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::report::SinkStatus;
use goodfetch_protocol::foxhound::sink::{
    CRAWL_MIN_SINK_GUARANTEE, CRAWL_ON_SINK_FAILURE, DeliveredAs, OnSinkFailure, SINK_DELIVER,
    SINK_LIST, SINK_PROBE, SINKS, SinkSettings, SinkType, WriteMode,
};
use goodfetch_protocol::run::{Lifecycle, Presence, RunKind, RunListParams};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SETTING_INVALID, SettingInvalidDetails,
    SettingsLayer, TIMEOUT_SINK_WRITE,
};
use rusqlite::Connection;
use tokio::sync::{Mutex as AsyncMutex, OnceCell, watch};

use crate::contract::{Failure, GuaranteeClass, RunCopy};
use crate::dead_letters;
use crate::driver::{Driver, RetryOutcome, Status};
use crate::guarantee;
use crate::report::{self, SinkReportPart, SinkStateRow};
use crate::sync::Mutex;
use crate::throughput::DEFAULT_WRITE_TIMEOUT;

mod built_in;
mod deliver;
mod ops;

use built_in::{BuiltInSink, Destination};

/// Goodfetch-sink's composition value (ADR 0087): what the crawl's Run
/// asks of its Sinks, the catalog's in-binary destinations this engine
/// delivers to, SQLite so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sinks;

impl Sinks {
    /// The Sinks this engine delivers to.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Refuses `settings` when one of their `sinks` asks for what this
    /// engine does not deliver yet, rather than run with its Records going
    /// nowhere (ADR 0083), or is keyed on no key of the Datasets
    /// `declarations` declare (ADR 0103).
    ///
    /// # Errors
    ///
    /// `capability_unavailable` naming the first such Sink's key: its
    /// `type` when it is not `sqlite` or is left out for a program's own
    /// handler, its `deliver` when not `records`. `setting_invalid` naming
    /// its `key`, when its mode is keyed, it names none and a Dataset it
    /// receives has none, or when it names a field a schema of one of them
    /// does not require.
    pub fn check(
        self,
        settings: &Resolved,
        declarations: &[DatasetDeclaration],
    ) -> Result<(), CallError> {
        let layer = layer(settings);
        entries(settings).try_for_each(|(name, sink)| {
            deliverable(&name, &sink, layer)?;
            webhook_checked(&name, &sink, layer)?;
            keyed(&name, &sink, layer, declarations)
        })
    }

    /// The deliveries of the Run `run`'s Datasets to each of the Sinks its
    /// `settings` name, which [`Delivery::start`] probes and starts and
    /// [`Delivery::finish`] finishes before the Run ends. `settings` passed
    /// [`Sinks::check`]; `runs` lets each Sink retry dead letters left by
    /// earlier ended Runs, and `daemon` (present on a Daemon engine) lets
    /// behind Sinks keep delivering a finished Run's backlog after the Run
    /// ends (ADR 0026).
    #[must_use]
    pub fn deliver(
        self,
        run: &RunParts,
        settings: &Resolved,
        runs: &Runs,
        daemon: Option<Arc<TaskOwner>>,
    ) -> Delivery {
        let layer = layer(settings);
        let on_failure = on_sink_failure(settings);
        let write_timeout = sink_write_timeout(settings);
        let endpoint = NotificationEndpoint::of(
            settings,
            run.identity.clock(),
            run.identity.owner().blocking_threads(),
        );
        let stop_requested = Arc::new(watch::Sender::new(None));
        let sinks = entries(settings)
            .map(|(name, settings)| {
                Arc::new(Sink {
                    name,
                    settings,
                    layer,
                    write_timeout,
                    endpoint: endpoint.clone(),
                    identity: run.identity.clone(),
                    state: run.state.clone(),
                    keys: run.keys.clone(),
                    runs: runs.clone(),
                    driver: OnceCell::new(),
                    delivering: AsyncMutex::new(()),
                    retried_dead_letters: OnceCell::new(),
                    stopped: watch::Sender::new(false),
                    on_failure,
                    stop_requested: Arc::clone(&stop_requested),
                })
            })
            .collect();
        Delivery {
            sinks,
            minimum: minimum(settings),
            stop_requested,
            daemon,
        }
    }
}

/// The deliveries of one Run to its Sinks.
pub struct Delivery {
    sinks: Vec<Arc<Sink>>,
    /// The weakest Guarantee class the Run accepts of each Sink, its
    /// `crawl.min_sink_guarantee`; none when it accepts any.
    minimum: Option<GuaranteeClass>,
    /// Set to a failure reason when `crawl.on_sink_failure` is `stop` and any
    /// Sink fails (ADR 0026).
    stop_requested: Arc<watch::Sender<Option<String>>>,
    /// The Daemon engine's task owner, when running on a Daemon, so behind
    /// Sinks keep delivering a finished Run's backlog after the Run ends.
    daemon: Option<Arc<TaskOwner>>,
}

impl Delivery {
    /// Returns the Sink failure reason if `crawl.on_sink_failure` is `stop`
    /// and a Sink has already failed.
    #[must_use]
    pub fn failure_reason(&self) -> Option<String> {
        self.stop_requested.borrow().clone()
    }

    /// Resolves once a Sink fails under `crawl.on_sink_failure: stop` (pends
    /// forever when `crawl.on_sink_failure` is `continue`).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels only the watch subscription.
    pub async fn failed(&self) -> String {
        let mut rx = self.stop_requested.subscribe();
        let _ = rx.wait_for(Option::is_some).await;
        rx.borrow()
            .clone()
            .unwrap_or_else(|| "a Sink failed".to_owned())
    }

    /// Probes each Sink (ADR 0026), then starts delivering to each, as
    /// tasks of the Run, each waking at the run file's commits; what the
    /// Run does before any other work.
    ///
    /// # Errors
    ///
    /// `sink_guarantee_unmet` for the first Sink whose probe proved a
    /// weaker class than the Run's `crawl.min_sink_guarantee`, or failed
    /// under one: no Sink then delivers anything.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves no delivery started; a probe already on
    /// its blocking thread runs to its end.
    pub async fn start(&self) -> Result<(), CallError> {
        for sink in &self.sinks {
            let probed = sink.probe().await;
            if let Err(cause) = &probed {
                tracing::warn!(sink = %sink.name, %cause, "a Sink's probe at the Run's start failed");
            }
            if let Err(error) = guarantee::require(self.minimum, &sink.name, probed.clone()) {
                for sink in &self.sinks {
                    sink.stopped.send_replace(true);
                }
                return Err(error);
            }
            let initial = match &probed {
                Ok(class) => SinkStateRow {
                    sink: sink.name.clone(),
                    guarantee_class: *class,
                    status: SinkStatus::Complete,
                    paused_reason: None,
                    outside_dedup_window: 0,
                    undeclared_datasets: Vec::new(),
                },
                Err(cause) => {
                    sink.note_failure(cause);
                    SinkStateRow {
                        sink: sink.name.clone(),
                        guarantee_class: GuaranteeClass::AtLeastOnce,
                        status: SinkStatus::Failed,
                        paused_reason: None,
                        outside_dedup_window: 0,
                        undeclared_datasets: Vec::new(),
                    }
                }
            };
            sink.persist_state(initial, Vec::new(), Vec::new()).await;
        }
        for sink in &self.sinks {
            if !*sink.stopped.borrow() {
                sink.identity.spawn(keep_delivering(Arc::clone(sink)));
            }
        }
        Ok(())
    }

    /// Writes the run file's open batch, then delivers once more to each
    /// Sink still delivering what the run file has committed, then stops
    /// its deliveries: what the Run's end waits for, so a Sink holds the
    /// Records the Run committed before its end, the last batch's included.
    /// When the Run `finished`, a Sink that delivered all of them then
    /// swaps in what it wrote apart, as `replace` does (ADR 0026). On a
    /// Daemon, a Sink still behind on retries continues delivering the
    /// finished Run's backlog on the Daemon's task owner (ADR 0026).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the Sinks it had not reached stopped,
    /// with what they last delivered.
    pub async fn finish(&self, finished: bool) {
        let Some(first) = self.sinks.first() else {
            return;
        };
        if let Err(error) = first.state.seal().await {
            tracing::warn!(%error, "the run file's last batch did not commit before the Sinks' last delivery");
        }
        for sink in &self.sinks {
            if sink.stopped.send_replace(true) {
                continue;
            }
            let next = match sink.pending_retry().await {
                Some(wait) => Next::Retry(wait),
                None => sink.deliver().await,
            };
            match next {
                Next::Wait if finished => sink.finish().await,
                Next::Retry(wait) if finished => {
                    if let Some(engine_owner) = &self.daemon {
                        deliver::spawn_backlog(Arc::clone(sink), engine_owner, wait, finished);
                    }
                }
                Next::Wait | Next::Retry(_) | Next::Stop => {}
            }
        }
    }
}

/// Goodfetch-sink's contribution (ADR 0087): `sink.probe`, `sink.list`,
/// `sink.deliver`, the `sink_dead_letters`, `sink_state` and
/// `sink_notifications` run-state tables, and the Run report's `sinks` part
/// (ADRs 0026 and 0102).
#[must_use]
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-sink")
        .with_run_state_tables(dead_letters::TABLES)
        .with_run_state_tables(report::TABLES)
        .with_report_part(Arc::new(SinkReportPart))
        .with_handler(Handler::new(&SINK_PROBE, ops::probe))
        .with_handler(Handler::new(&SINK_LIST, ops::list))
        .with_handler(Handler::new(&SINK_DELIVER, deliver::deliver_op))
}

/// One Sink of a Run: its entry and what its delivery reads and writes.
struct Sink {
    name: String,
    settings: SinkSettings,
    layer: SettingsLayer,
    write_timeout: Duration,
    endpoint: NotificationEndpoint,
    identity: RunIdentity,
    state: RunState,
    keys: RunKeys,
    /// The engine's Runs registry, used once per Sink to retry dead
    /// letters left by earlier ended Runs (ADR 0026).
    runs: Runs,
    /// Its delivery driver, once its destination opened; why it did not
    /// otherwise.
    driver: OnceCell<Result<Arc<Mutex<Driver<BuiltInSink>>>, String>>,
    /// Serializes delivery passes (including persisting dead letters into
    /// `state`) so `Delivery::finish` never overtakes an in-flight pass
    /// before `RunEnd::end` closes the run-state writer.
    delivering: AsyncMutex<()>,
    /// Set once dead letters from this Run and earlier ended Runs have
    /// been retried.
    retried_dead_letters: OnceCell<()>,
    /// Whether its delivery stopped: failed, superseded, refused at the
    /// Run's start or finished; its delivery task watches it while it
    /// waits, since the run file's commits go on until the Run's tasks end.
    stopped: watch::Sender<bool>,
    /// Whether a failure of this Sink should stop the Run.
    on_failure: OnSinkFailure,
    /// Shared signal telling the crawl loop to stop when `on_failure == Stop`.
    stop_requested: Arc<watch::Sender<Option<String>>>,
}

/// What a Sink's delivery does after an attempt.
enum Next {
    /// Waits for the run file's next commit.
    Wait,
    /// Tries again after the wait.
    Retry(Duration),
    /// Stops.
    Stop,
}

impl Sink {
    /// Signals the crawl loop to stop if `crawl.on_sink_failure` is `stop`.
    fn note_failure(&self, cause: &str) {
        if self.on_failure == OnSinkFailure::Stop && self.stop_requested.borrow().is_none() {
            self.stop_requested
                .send_replace(Some(format!("Sink `{}` failed: {cause}", self.name)));
        }
    }

    /// Waits for any in-flight delivery pass, then returns the remaining retry
    /// wait if the driver is already scheduled for a future retry.
    async fn pending_retry(&self) -> Option<Duration> {
        let _turn = self.delivering.lock().await;
        let driver = self.driver.get()?.as_ref().ok()?;
        driver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending_retry()
    }

    /// Probes its destination, opening it first: the class it proves, or
    /// why it proved none.
    async fn probe(&self) -> Result<GuaranteeClass, String> {
        let driver = self.driver().await?;
        self.identity
            .owner()
            .blocking(move || {
                driver
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .probe()
            })
            .await
            .map_err(|failure| probe_failed(&failure))
    }

    /// Delivers what the run file has committed and says what follows.
    /// On the first delivery pass, retries any dead letters stored for this
    /// Sink in this Run or in earlier ended Runs (ADR 0026).
    async fn deliver(&self) -> Next {
        let _turn = self.delivering.lock().await;
        let driver = match self.driver().await {
            Ok(driver) => driver,
            Err(cause) => {
                tracing::warn!(sink = %self.name, %cause, "a Sink's destination did not open: it delivers nothing");
                self.note_failure(&cause);
                self.persist_state(
                    SinkStateRow {
                        sink: self.name.clone(),
                        guarantee_class: GuaranteeClass::AtLeastOnce,
                        status: SinkStatus::Failed,
                        paused_reason: None,
                        outside_dedup_window: 0,
                        undeclared_datasets: Vec::new(),
                    },
                    Vec::new(),
                    Vec::new(),
                )
                .await;
                self.stopped.send_replace(true);
                return Next::Stop;
            }
        };
        let () = *self
            .retried_dead_letters
            .get_or_init(|| self.retry_dead_letters(&driver))
            .await;
        let driver_for_read = Arc::clone(&driver);
        let sink_name = self.name.clone();
        let delivered = self
            .state
            .read(move |run_file| deliver::deliver_pass(&driver_for_read, &sink_name, run_file))
            .await;
        let next = match delivered {
            Ok(deliver::PassOutput {
                status,
                rejected,
                notifications,
                state_row,
            }) => {
                self.persist_state(state_row, rejected, notifications).await;
                match status {
                    Status::Complete | Status::CompleteWithRejections => Next::Wait,
                    Status::Behind {
                        failure, retry_in, ..
                    } => {
                        tracing::info!(sink = %self.name, %failure, ?retry_in, "a Sink's delivery failed and is retried");
                        Next::Retry(retry_in)
                    }
                    Status::Paused { reason, failure } => {
                        tracing::warn!(sink = %self.name, ?reason, %failure, "a Sink was paused");
                        self.note_failure(&failure.to_string());
                        Next::Stop
                    }
                    Status::Failed(failure) => {
                        tracing::warn!(sink = %self.name, %failure, "a Sink failed: it delivers nothing more");
                        self.note_failure(&failure.to_string());
                        Next::Stop
                    }
                    Status::Superseded { run_epoch } => {
                        tracing::info!(sink = %self.name, run_epoch, "a later copy of the Run delivers to this Sink: this copy stops");
                        Next::Stop
                    }
                }
            }
            Err(error) => {
                tracing::warn!(sink = %self.name, %error, "a Sink's delivery could not read the run file: it stops");
                Next::Stop
            }
        };
        if matches!(next, Next::Stop) {
            self.stopped.send_replace(true);
        }
        next
    }

    /// Persists `state_row` into `sink_state`, plus any new `rejected` dead
    /// letters and `notifications`, in a single run-state write.
    async fn persist_state(
        &self,
        state_row: SinkStateRow,
        rejected: Vec<dead_letters::DeadLetter>,
        notifications: Vec<crate::schedule::SinkNotification>,
    ) {
        let masks = self.keys.secrets().mask().masks().clone();
        if let Err(error) = self
            .state
            .write(move |connection| {
                for entry in &rejected {
                    dead_letters::record(connection, entry, &masks)?;
                }
                for notification in &notifications {
                    report::record_notification(connection, notification, &masks)?;
                }
                report::record_state(connection, &state_row)
            })
            .await
        {
            tracing::warn!(sink = %self.name, %error, "Sink state could not be recorded in the run file");
        }
    }

    /// Retries dead letters stored for this Sink in this Run (on resume)
    /// and in earlier ended Crawl Runs, removing each that succeeds and
    /// updating each that is rejected again (ADR 0026).
    async fn retry_dead_letters(&self, driver: &Arc<Mutex<Driver<BuiltInSink>>>) {
        let masks = self.keys.secrets().mask().masks().clone();
        self.retry_own_dead_letters(driver, &masks).await;
        self.retry_past_runs_dead_letters(driver, &masks).await;
    }

    async fn retry_own_dead_letters(
        &self,
        driver: &Arc<Mutex<Driver<BuiltInSink>>>,
        masks: &Masks,
    ) {
        let sink_name = self.name.clone();
        let driver = Arc::clone(driver);
        let outcome = self
            .state
            .read(move |run_file| retry_in(run_file, &sink_name, &driver))
            .await;
        let Ok(Some(outcome)) = outcome else {
            return;
        };
        if outcome.delivered.is_empty() && outcome.still_rejected.is_empty() {
            return;
        }
        let sink_name = self.name.clone();
        let masks = masks.clone();
        if let Err(error) = self
            .state
            .write(move |connection| apply_retry_outcome(connection, &sink_name, &outcome, &masks))
            .await
        {
            tracing::warn!(sink = %self.name, %error, "dead-letter retry results could not be saved");
        }
    }

    async fn retry_past_runs_dead_letters(
        &self,
        driver: &Arc<Mutex<Driver<BuiltInSink>>>,
        masks: &Masks,
    ) {
        let Ok(rows) = self.runs.list(RunListParams::default()).await else {
            return;
        };
        for row in rows {
            if row.run_id == *self.identity.id()
                || row.presence != Presence::Present
                || row.lifecycle == Lifecycle::Running
                || row.kind != RunKind::Crawl
            {
                continue;
            }
            let Ok(mut reader) = self.runs.reader(row.run_id.clone()).await else {
                continue;
            };
            let sink_name = self.name.clone();
            let driver = Arc::clone(driver);
            let outcome = reader
                .read(move |run_file| retry_in(run_file, &sink_name, &driver))
                .await;
            let Ok(Some(outcome)) = outcome else {
                continue;
            };
            if outcome.delivered.is_empty() && outcome.still_rejected.is_empty() {
                continue;
            }
            let sink_name = self.name.clone();
            let masks = masks.clone();
            if let Err(error) = self
                .runs
                .update_ended(row.run_id.clone(), move |connection| {
                    apply_retry_outcome(connection, &sink_name, &outcome, &masks)?;
                    report::refresh_stored_report(connection)
                })
                .await
            {
                tracing::warn!(sink = %self.name, run_id = %row.run_id, %error, "earlier Run's dead letters could not be updated");
            }
        }
    }

    /// Ends its delivery of a Run that finished, once it delivered every
    /// Record: a failure leaves the destination as it was, and is logged.
    async fn finish(&self) {
        let Ok(driver) = self.driver().await else {
            return;
        };
        let finished = self
            .identity
            .owner()
            .blocking(move || {
                driver
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .finish()
            })
            .await;
        match finished {
            Status::Complete | Status::CompleteWithRejections => {}
            Status::Superseded { run_epoch } => {
                tracing::info!(sink = %self.name, run_epoch, "a later copy of the Run finishes this Sink's delivery");
            }
            Status::Behind { failure, .. }
            | Status::Paused { failure, .. }
            | Status::Failed(failure) => {
                tracing::warn!(sink = %self.name, %failure, "a Sink did not finish its delivery: its destination keeps what it held");
            }
        }
    }

    /// Its delivery driver, opened by the first probe or delivery to reach
    /// it.
    async fn driver(&self) -> Result<Arc<Mutex<Driver<BuiltInSink>>>, String> {
        self.driver
            .get_or_init(|| async {
                let run = RunCopy {
                    run_id: self.identity.id().as_str().to_owned(),
                    run_epoch: self.state.epoch(),
                };
                let destination = Destination {
                    name: &self.name,
                    settings: &self.settings,
                    layer: self.layer,
                    write_timeout: self.write_timeout,
                    run_id: Some(self.identity.id()),
                    endpoint: &self.endpoint,
                };
                destination
                    .open(
                        self.keys.secrets(),
                        self.identity.owner(),
                        self.identity.clock(),
                        run,
                    )
                    .await
                    .map(|driver| Arc::new(Mutex::new(driver)))
            })
            .await
            .clone()
    }
}

/// Retries every dead letter of `sink_name` stored in `run_file` through
/// `driver`, or `None` when none are stored there.
fn retry_in(
    run_file: &Connection,
    sink_name: &str,
    driver: &Mutex<Driver<BuiltInSink>>,
) -> rusqlite::Result<Option<RetryOutcome>> {
    let entries = dead_letters::list(run_file, sink_name)?;
    if entries.is_empty() {
        return Ok(None);
    }
    driver
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retry_dead_letters(run_file, &entries)
        .map(Some)
}

/// Applies `outcome` to `connection`'s `sink_dead_letters`: removes each
/// delivered dead letter and updates each still-rejected one.
fn apply_retry_outcome(
    connection: &Connection,
    sink_name: &str,
    outcome: &RetryOutcome,
    masks: &Masks,
) -> rusqlite::Result<()> {
    for id in &outcome.delivered {
        dead_letters::remove(connection, sink_name, id)?;
    }
    for entry in &outcome.still_rejected {
        dead_letters::record(connection, entry, masks)?;
    }
    Ok(())
}

/// Why a probe that answered `failure` proved nothing, without the
/// destination's own text, which may echo a credential.
fn probe_failed(failure: &Failure) -> String {
    format!("its destination failed the probe ({:?})", failure.kind())
}

/// Delivers to `sink` at each of the run file's commits until it stops,
/// the Run's tasks are cancelled or the run file's writer ends. Its waits
/// end at the stop, never only at a commit: the writer, an actor of the
/// Run, ends only after the Run's tasks, this one among them.
async fn keep_delivering(sink: Arc<Sink>) {
    let mut commits = sink.state.commits();
    let mut stopped = sink.stopped.subscribe();
    let clock = sink.identity.clock();
    let owner = sink.identity.owner().clone();
    while !*stopped.borrow_and_update() {
        let next = sink.deliver().await;
        let goes_on = async {
            match next {
                Next::Wait => commits.changed().await.is_ok(),
                Next::Retry(after) => {
                    clock.sleep(after).await;
                    true
                }
                Next::Stop => false,
            }
        };
        tokio::select! {
            biased;
            _ = stopped.wait_for(|stopped| *stopped) => return,
            () = owner.cancelled() => return,
            goes_on = goes_on => if !goes_on {
                return;
            },
        }
    }
}

/// Each Sink `settings` name, with its entry.
fn entries(settings: &Resolved) -> impl Iterator<Item = (String, SinkSettings)> + '_ {
    settings
        .value(&SINKS)
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, entry)| {
            let entry = SinkSettings::read(entry)
                .expect("invariant: `sinks` resolved only from entries its shape's check read");
            (name.clone(), entry)
        })
}

/// The weakest Guarantee class `settings` accept of each Sink.
fn minimum(settings: &Resolved) -> Option<GuaranteeClass> {
    serde_json::from_value(settings.value(&CRAWL_MIN_SINK_GUARANTEE).clone())
        .expect("invariant: `crawl.min_sink_guarantee` resolves to one of its choices or null")
}

/// Refuses the Sink `name`, of entry `sink` set in `layer`, when it asks
/// for what this engine does not deliver yet.
///
/// # Errors
///
/// `capability_unavailable` naming the entry's key that asks for it.
fn deliverable(name: &str, sink: &SinkSettings, layer: SettingsLayer) -> Result<(), CallError> {
    let Some((key, kind)) = unavailable(sink) else {
        return Ok(());
    };
    let key = format!("{}.{name}.{key}", SINKS.name);
    Err(CallError::new(
        &CAPABILITY_UNAVAILABLE,
        format!(
            "`{}` asks for a Sink's `{kind}`, which this engine does not deliver yet: deliver to a `type: sqlite` Sink as `records`, or leave the Sink out.",
            named::setting_in(&key, layer),
        ),
        &CapabilityUnavailableDetails {
            key,
            layer,
            kind,
            fix: "`type: sqlite`, `deliver: records`".to_owned(),
        },
    ))
}

/// Refuses the Sink `name`, of entry `sink` set in `layer`, when its keyed
/// write mode has no key for a Dataset it receives among `declarations`,
/// or its own key names a field a schema of one of them does not require
/// (ADR 0103).
///
/// # Errors
///
/// `setting_invalid` naming the entry's `key`.
fn keyed(
    name: &str,
    sink: &SinkSettings,
    layer: SettingsLayer,
    declarations: &[DatasetDeclaration],
) -> Result<(), CallError> {
    if sink.write_mode() == WriteMode::Append {
        return Ok(());
    }
    let mut received = declarations
        .iter()
        .filter(|declaration| sink.datasets.contains(&declaration.name));
    let own = sink.key.as_deref().filter(|own| !own.is_empty());
    let refused = match own {
        None => received
            .find(|declaration| declaration.key.is_empty())
            .map(|declaration| {
                (
                    format!(
                        "its mode is keyed and it names no `key`, while the Dataset `{}` it receives declares none",
                        declaration.name
                    ),
                    format!(
                        "give the Sink a `key` of fields every Dataset it receives requires, declare a `key` on `{}`, or set `mode: append`",
                        declaration.name
                    ),
                )
            }),
        Some(own) => received
            .find_map(|declaration| {
                let required = goodfetch_dataset::required_fields(declaration.schema.as_ref()?);
                let field = own.iter().find(|field| !required.contains(&field.as_str()))?;
                Some((
                    format!(
                        "its `key` names `{field}`, which the schema of the Dataset `{}` it receives does not require",
                        declaration.name
                    ),
                    format!(
                        "key the Sink on fields the schema of `{}` requires, or require `{field}` there",
                        declaration.name
                    ),
                ))
            }),
    };
    let Some((reason, fix)) = refused else {
        return Ok(());
    };
    let key = format!("{}.{name}.key", SINKS.name);
    Err(CallError::new(
        &SETTING_INVALID,
        format!(
            "`{}` is invalid: {reason}; {fix}.",
            named::setting_in(&key, layer)
        ),
        &SettingInvalidDetails {
            key,
            layer: Some(layer),
            reason,
            limit: None,
            fix: Some(fix),
        },
    ))
}

/// Refuses a webhook Sink with no `webhook.signing_secret` at start with
/// `setting_invalid` (ADRs 0008 and 0026; issue 1166).
fn webhook_checked(name: &str, sink: &SinkSettings, layer: SettingsLayer) -> Result<(), CallError> {
    if sink.sink_type != Some(SinkType::Webhook) {
        return Ok(());
    }
    let has_secret = sink
        .webhook
        .as_ref()
        .and_then(|w| w.signing_secret.as_deref())
        .is_some_and(|s| !s.trim().is_empty());
    if has_secret {
        return Ok(());
    }
    let key = format!("{}.{name}.webhook.signing_secret", SINKS.name);
    let reason =
        "a webhook Sink requires `webhook.signing_secret` to sign Standard Webhooks deliveries"
            .to_owned();
    let fix =
        format!("set `{key}` to a Secret reference holding a `whsec_` or `whsk_` signing key");
    Err(CallError::new(
        &SETTING_INVALID,
        format!(
            "`{}` is invalid: {reason}; {fix}.",
            named::setting_in(&key, layer)
        ),
        &SettingInvalidDetails {
            key,
            layer: Some(layer),
            reason,
            limit: None,
            fix: Some(fix),
        },
    ))
}

/// The key of `sink`'s entry that asks for what this engine does not
/// deliver yet, with the feature it asks for; none when it delivers it.
fn unavailable(sink: &SinkSettings) -> Option<(&'static str, String)> {
    let Some(sink_type) = sink.sink_type else {
        return Some(("type", "program_sink".to_owned()));
    };
    if !matches!(
        sink_type,
        SinkType::Sqlite
            | SinkType::Webhook
            | SinkType::S3
            | SinkType::Gcs
            | SinkType::AzureBlob
            | SinkType::Postgres
            | SinkType::Supabase
            | SinkType::Mysql
            | SinkType::Duckdb
            | SinkType::Smtp
            | SinkType::Sftp
            | SinkType::Ftp
    ) {
        return Some(("type", wire_name(serde_json::to_value(sink_type))));
    }
    if matches!(
        sink_type,
        SinkType::S3 | SinkType::Gcs | SinkType::AzureBlob | SinkType::Sftp | SinkType::Ftp
    ) {
        return sink
            .deliver
            .filter(|deliver| *deliver != DeliveredAs::Files)
            .map(|deliver| ("deliver", wire_name(serde_json::to_value(deliver))));
    }
    sink.deliver
        .filter(|deliver| *deliver != DeliveredAs::Records)
        .map(|deliver| ("deliver", wire_name(serde_json::to_value(deliver))))
}

/// The layer that set `sinks`, the Run's when none did.
fn layer(settings: &Resolved) -> SettingsLayer {
    settings
        .effective(&SINKS)
        .map_or(SettingsLayer::Run, |effective| effective.layer)
}

/// The crawl's `crawl.on_sink_failure` choice, defaulting to `continue`.
fn on_sink_failure(settings: &Resolved) -> OnSinkFailure {
    serde_json::from_value(settings.value(&CRAWL_ON_SINK_FAILURE).clone()).unwrap_or_default()
}

/// The Run's `timeout.sink_write` duration, defaulting to 30 s (ADR 0080).
pub(super) fn sink_write_timeout(settings: &Resolved) -> Duration {
    settings
        .value(&TIMEOUT_SINK_WRITE)
        .as_u64()
        .map_or(DEFAULT_WRITE_TIMEOUT, Duration::from_millis)
}

/// The wire name of a value of one of the Sink entry's enums.
fn wire_name(value: serde_json::Result<serde_json::Value>) -> String {
    value
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("invariant: a Sink entry's enums encode as their snake_case names")
}
