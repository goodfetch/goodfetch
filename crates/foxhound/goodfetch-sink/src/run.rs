//! A crawl's Sinks on its Run: [`Sinks`], the value Foxhound's composition
//! passes the crawl, refuses at the start a Sink this engine cannot deliver
//! to yet, probes each other Sink at the Run's start against the Run's
//! `crawl.min_sink_guarantee` (ADR 0026), and delivers its Datasets from
//! the run file while the Run goes. A Sink reads only committed Records, so
//! it waits for each of the run file's commits, one per Run-state batch,
//! and never forces one while the Run goes (spec #2138); the Run's end
//! writes the last batch and waits for a last delivery. [`contribution`]
//! answers `sink.probe`, a probe of one Sink outside any Run.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run::{run_error, run_layer, settings_error};
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::runs::{RunError, RunKeys, RunParts, Runs};
use goodfetch_engine::secrets::{Purpose, RunSecrets};
use goodfetch_engine::settings::{Opening, Resolved};
use goodfetch_engine::tasks::TaskStarter;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler, named};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::report::SinkStatus;
use goodfetch_protocol::foxhound::sink::{
    CRAWL_MIN_SINK_GUARANTEE, DeliveredAs, SINK_LIST, SINK_PROBE, SINKS, SinkList, SinkListEntry,
    SinkListParams, SinkProbe, SinkProbeOutcome, SinkProbeParams, SinkSettings, SinkType,
    WriteMode,
};
use goodfetch_protocol::run::{Lifecycle, Presence, RunId, RunKind, RunListParams};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, ProjectSettings, RunOrigin,
    SETTING_INVALID, SettingInvalidDetails, SettingsLayer,
};
use rusqlite::Connection;
use secrecy::ExposeSecret as _;
use tokio::sync::{Mutex as AsyncMutex, OnceCell, watch};

use crate::contract::{Failure, GuaranteeClass, RunCopy};
use crate::dead_letters;
use crate::driver::{Driver, RetryOutcome, Status};
use crate::guarantee;
use crate::report::{self, SinkReportPart, SinkStateRow};
use crate::sqlite::SqliteSink;
use crate::sync::Mutex;

/// The table a SQLite Sink writes when its `target` names none.
const SQLITE_TARGET: &str = "records";

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
            keyed(&name, &sink, layer, declarations)
        })
    }

    /// The deliveries of the Run `run`'s Datasets to each of the Sinks its
    /// `settings` name, which [`Delivery::start`] probes and starts and
    /// [`Delivery::finish`] finishes before the Run ends. `settings` passed
    /// [`Sinks::check`]; `runs` lets each Sink retry dead letters left by
    /// earlier ended Runs (ADR 0026).
    #[must_use]
    pub fn deliver(self, run: &RunParts, settings: &Resolved, runs: &Runs) -> Delivery {
        let layer = layer(settings);
        let sinks = entries(settings)
            .map(|(name, settings)| {
                Arc::new(Sink {
                    name,
                    settings,
                    layer,
                    identity: run.identity.clone(),
                    state: run.state.clone(),
                    keys: run.keys.clone(),
                    runs: runs.clone(),
                    driver: OnceCell::new(),
                    delivering: AsyncMutex::new(()),
                    retried_dead_letters: OnceCell::new(),
                    stopped: watch::Sender::new(false),
                })
            })
            .collect();
        Delivery {
            sinks,
            minimum: minimum(settings),
        }
    }
}

/// The deliveries of one Run to its Sinks.
pub struct Delivery {
    sinks: Vec<Arc<Sink>>,
    /// The weakest Guarantee class the Run accepts of each Sink, its
    /// `crawl.min_sink_guarantee`; none when it accepts any.
    minimum: Option<GuaranteeClass>,
}

impl Delivery {
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
            let initial = match probed {
                Ok(class) => SinkStateRow {
                    sink: sink.name.clone(),
                    guarantee_class: class,
                    status: SinkStatus::Complete,
                    paused_reason: None,
                    outside_dedup_window: 0,
                    undeclared_datasets: Vec::new(),
                },
                Err(_) => SinkStateRow {
                    sink: sink.name.clone(),
                    guarantee_class: GuaranteeClass::AtLeastOnce,
                    status: SinkStatus::Failed,
                    paused_reason: None,
                    outside_dedup_window: 0,
                    undeclared_datasets: Vec::new(),
                },
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
    /// swaps in what it wrote apart, as `replace` does (ADR 0026). A Sink
    /// behind on its retries stays behind; a resumed copy of the Run
    /// delivers again.
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
            if !sink.stopped.send_replace(true)
                && matches!(sink.deliver().await, Next::Wait)
                && finished
            {
                sink.finish().await;
            }
        }
    }
}

/// Goodfetch-sink's contribution (ADR 0087): `sink.probe`, `sink.list`, the
/// `sink_dead_letters`, `sink_state` and `sink_notifications` run-state
/// tables, and the Run report's `sinks` part (ADRs 0026 and 0102).
#[must_use]
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-sink")
        .with_run_state_tables(dead_letters::TABLES)
        .with_run_state_tables(report::TABLES)
        .with_report_part(Arc::new(SinkReportPart))
        .with_handler(Handler::new(&SINK_PROBE, probe))
        .with_handler(Handler::new(&SINK_LIST, list))
}

/// One Sink of a Run: its entry and what its delivery reads and writes.
struct Sink {
    name: String,
    settings: SinkSettings,
    layer: SettingsLayer,
    identity: RunIdentity,
    state: RunState,
    keys: RunKeys,
    /// The engine's Runs registry, used once per Sink to retry dead
    /// letters left by earlier ended Runs (ADR 0026).
    runs: Runs,
    /// Its delivery driver, once its destination opened; why it did not
    /// otherwise.
    driver: OnceCell<Result<Arc<Mutex<Driver<SqliteSink>>>, String>>,
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
        let delivered = self
            .state
            .read(move |run_file| {
                let mut guard = driver_for_read
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let status = guard.deliver(run_file)?;
                let rejected = guard.take_dead_letters();
                let notifications = guard.take_notifications();
                let state_row = SinkStateRow {
                    sink: String::new(),
                    guarantee_class: guard.class().unwrap_or(GuaranteeClass::AtLeastOnce),
                    status: guard.report_status(),
                    paused_reason: guard.paused_reason(),
                    outside_dedup_window: guard.outside_dedup_window(),
                    undeclared_datasets: Vec::new(),
                };
                Ok((status, rejected, notifications, state_row))
            })
            .await;
        let next = match delivered {
            Ok((status, rejected, notifications, mut state_row)) => {
                state_row.sink.clone_from(&self.name);
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
                        Next::Stop
                    }
                    Status::Failed(failure) => {
                        tracing::warn!(sink = %self.name, %failure, "a Sink failed: it delivers nothing more");
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
    async fn retry_dead_letters(&self, driver: &Arc<Mutex<Driver<SqliteSink>>>) {
        let masks = self.keys.secrets().mask().masks().clone();
        self.retry_own_dead_letters(driver, &masks).await;
        self.retry_past_runs_dead_letters(driver, &masks).await;
    }

    async fn retry_own_dead_letters(&self, driver: &Arc<Mutex<Driver<SqliteSink>>>, masks: &Masks) {
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
        driver: &Arc<Mutex<Driver<SqliteSink>>>,
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
                    apply_retry_outcome(connection, &sink_name, &outcome, &masks)
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
    async fn driver(&self) -> Result<Arc<Mutex<Driver<SqliteSink>>>, String> {
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
                    run_id: Some(self.identity.id()),
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
    driver: &Mutex<Driver<SqliteSink>>,
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

/// A Sink's destination, as its entry names it.
struct Destination<'a> {
    name: &'a str,
    settings: &'a SinkSettings,
    /// The layer that set its entry.
    layer: SettingsLayer,
    /// The Run it delivers for; none for a probe outside any Run.
    run_id: Option<&'a RunId>,
}

impl Destination<'_> {
    /// Its delivery driver for the copy `run` of a Run, its Secret
    /// reference resolved through `secrets` and its database opened on
    /// `owner`'s blocking threads; why not, without the resolved value,
    /// when either fails.
    async fn open(
        &self,
        secrets: &RunSecrets,
        owner: &TaskStarter,
        clock: Clock,
        run: RunCopy,
    ) -> Result<Driver<SqliteSink>, String> {
        let destination =
            self.settings.destination.as_deref().expect(
                "invariant: `SinkSettings::read` refuses a typed Sink with no `destination`",
            );
        let reference = SecretReference::parse(destination).expect(
            "invariant: `SinkSettings::read` refuses a `destination` that is no Secret reference",
        );
        let placeholder = format!("{}.{}.destination", SINKS.name, self.name);
        let purpose = Purpose {
            placeholder: &placeholder,
            domain: None,
            domains: &[],
            layer: Some(self.layer),
            run_id: self.run_id,
        };
        let path = secrets
            .resolve(&reference, &purpose)
            .await
            .map_err(|error| format!("its destination `{destination}` did not resolve: {error}"))?
            .expose_secret()
            .to_owned();
        let connection = owner
            .blocking(move || Connection::open(path))
            .await
            .map_err(|error| {
                format!(
                    "its SQLite database did not open ({:?})",
                    error.sqlite_error_code()
                )
            })?;
        let target = self.settings.target.as_deref().unwrap_or(SQLITE_TARGET);
        let mode = self.settings.write_mode();
        let schema = self.settings.schema.clone().unwrap_or_default();
        let driver = Driver::new(
            self.name,
            SqliteSink::new(connection, target)
                .with_mode(mode)
                .with_schema(schema),
            run,
            self.settings.datasets.clone(),
        )
        .with_clock(clock)
        .with_masks(secrets.mask().masks().clone());
        Ok(if mode == WriteMode::Append {
            driver
        } else {
            driver.keyed(self.settings.key.clone())
        })
    }
}

/// Resolves `settings` and `project` over the connection's settings as a
/// Foxhound crawl Run would.
async fn open_settings(
    settings: &serde_json::Map<String, serde_json::Value>,
    project: Option<ProjectSettings>,
    context: &CallContext,
) -> Result<Resolved, CallError> {
    let (opened, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Foxhound),
            project,
            recipe: None,
            settings: run_layer(settings, context)?,
            origin: Some(RunOrigin::Settings),
            registered: context.registered_settings().clone(),
            acting_for_model: context.surface().acts_for_model(),
            exposed: context.exposed(),
        })
        .await
        .map_err(|error| settings_error(&error, context))?;
    Ok(opened.resolved().clone())
}

/// Probes one configured Sink outside any Run.
async fn probe_configured(
    sink: &str,
    entry: &SinkSettings,
    layer: SettingsLayer,
    context: &CallContext,
) -> SinkProbeOutcome {
    let (_, secrets) = context.runs().outside_a_run();
    let owner = context.engine().owner();
    let destination = Destination {
        name: sink,
        settings: entry,
        layer,
        run_id: None,
    };
    let run = RunCopy {
        run_id: String::new(),
        run_epoch: 0,
    };
    match destination
        .open(&secrets, owner, Clock::system(), run)
        .await
    {
        Ok(mut driver) => match owner.blocking(move || driver.probe()).await {
            Ok(class) => SinkProbeOutcome::Proved { class },
            Err(failure) => SinkProbeOutcome::Failed {
                cause: probe_failed(&failure),
            },
        },
        Err(cause) => SinkProbeOutcome::Failed { cause },
    }
}

/// `sink.probe`: probes the Sink the params name, among the `sinks` their
/// settings resolve to as a crawl's Run would.
async fn probe(mut params: SinkProbeParams, context: CallContext) -> Result<SinkProbe, CallError> {
    let settings = open_settings(&params.settings, params.project.take(), &context).await?;
    let layer = layer(&settings);
    let sink = params.sink;
    let Some((_, entry)) = entries(&settings).find(|(name, _)| *name == sink) else {
        let named: Vec<String> = entries(&settings)
            .map(|(name, _)| format!("`{name}`"))
            .collect();
        let cause = if named.is_empty() {
            format!("the settings name no Sink: `{}` is empty", SINKS.name)
        } else {
            format!(
                "the settings name no Sink `{sink}`, only {}",
                named.join(", ")
            )
        };
        return Ok(SinkProbe {
            sink,
            outcome: SinkProbeOutcome::Failed { cause },
        });
    };
    deliverable(&sink, &entry, layer)?;
    let outcome = probe_configured(&sink, &entry, layer, &context).await;
    Ok(SinkProbe { sink, outcome })
}

/// `sink.list`: lists each Sink with its Guarantee class, dead-letter count
/// and deliveries outside the destination's deduplication window (ADRs 0026
/// and 0099).
async fn list(mut params: SinkListParams, context: CallContext) -> Result<SinkList, CallError> {
    if let Some(run_id) = params.run_id {
        let failed = |error: RunError| run_error(error, &context);
        let mut reader = context.runs().reader(run_id).await.map_err(failed)?;
        let reports = reader.read(report::reports).await.map_err(failed)?;
        let sinks = reports
            .into_iter()
            .map(|entry| SinkListEntry {
                name: entry.name,
                guarantee_class: Some(entry.guarantee_class),
                dead_letters: entry.dead_letters,
                outside_dedup_window: entry.outside_dedup_window,
                status: Some(entry.status),
                probe_cause: None,
            })
            .collect();
        return Ok(SinkList { sinks });
    }
    let settings = open_settings(&params.settings, params.project.take(), &context).await?;
    let layer = layer(&settings);
    let totals = aggregate_sink_counts(context.runs()).await;
    let mut sinks = Vec::new();
    for (name, entry) in entries(&settings) {
        let (dead_letters, outside_dedup_window) = totals.get(&name).copied().unwrap_or((0, 0));
        let (guarantee_class, probe_cause) = match deliverable(&name, &entry, layer) {
            Ok(()) => match probe_configured(&name, &entry, layer, &context).await {
                SinkProbeOutcome::Proved { class } => (Some(class), None),
                SinkProbeOutcome::Failed { cause } => (None, Some(cause)),
            },
            Err(error) => (None, Some(error.to_string())),
        };
        sinks.push(SinkListEntry {
            name,
            guarantee_class,
            dead_letters,
            outside_dedup_window,
            status: None,
            probe_cause,
        });
    }
    sinks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(SinkList { sinks })
}

/// Aggregates `(dead_letters, outside_dedup_window)` per Sink across every
/// known Crawl Run.
async fn aggregate_sink_counts(runs: &Runs) -> BTreeMap<String, (u64, u64)> {
    let mut totals: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let Ok(rows) = runs.list(RunListParams::default()).await else {
        return totals;
    };
    for row in rows {
        if row.presence != Presence::Present || row.kind != RunKind::Crawl {
            continue;
        }
        let Ok(mut reader) = runs.reader(row.run_id).await else {
            continue;
        };
        let Ok(reports) = reader.read(report::reports).await else {
            continue;
        };
        for entry in reports {
            let counts = totals.entry(entry.name).or_insert((0, 0));
            counts.0 = counts.0.saturating_add(entry.dead_letters);
            counts.1 = counts.1.saturating_add(entry.outside_dedup_window);
        }
    }
    totals
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

/// The key of `sink`'s entry that asks for what this engine does not
/// deliver yet, with the feature it asks for; none when it delivers it.
fn unavailable(sink: &SinkSettings) -> Option<(&'static str, String)> {
    let Some(sink_type) = sink.sink_type else {
        return Some(("type", "program_sink".to_owned()));
    };
    if sink_type != SinkType::Sqlite {
        return Some(("type", wire_name(serde_json::to_value(sink_type))));
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

/// The wire name of a value of one of the Sink entry's enums.
fn wire_name(value: serde_json::Result<serde_json::Value>) -> String {
    value
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("invariant: a Sink entry's enums encode as their snake_case names")
}
