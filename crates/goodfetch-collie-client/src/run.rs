//! The Run a program drives (ADRs 0078 and 0102): a Connection starts it,
//! the program finishes it for its Run report, and closing it stops it if
//! it still runs. Its members are the ones the catalog gives a Run, which
//! `xtask codegen` writes (ADR 0099); this module adds its start, its
//! close, and the explanation of the settings it runs with, as of the
//! Connection's (ADR 0083). Its handles are cheap clones of one Run, `Send + Sync +
//! 'static`, and closing any of them closes it for all.

mod progress;

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::{
    RUN_PROGRESS_READ, RUN_START, RunId, RunProgress, RunStartParams, RunStarted, StartedAs, Tags,
};
use goodfetch_protocol::settings::{CONFIG_EXPLAIN, ConfigExplainParams, SettingsExplanation};
use serde::Serialize;

use crate::answer::Answer;
use crate::call::Query;
use crate::connection::ClientConnection;
use crate::error::Error;
use crate::event_stream::EventStream;
use crate::generated::handles::{Page, Run};
use crate::host::{HeldRun, Host};
use crate::limits::Limits;
use crate::logging::{self, engine_log_level};
use crate::settings::Settings;

/// The start of a Run (`run.start`), which awaiting sends. Nothing is sent
/// until it is awaited.
#[must_use = "a start is sent only when it is awaited"]
#[derive(Debug)]
pub struct Start {
    connection: ClientConnection,
    params: RunStartParams,
    /// The Run's own settings, which its params carry once checked.
    settings: Settings,
}

impl Start {
    /// The start of a Run on `connection`, with no Run ID: the engine
    /// names it.
    pub(crate) fn new(connection: ClientConnection) -> Self {
        Self {
            connection,
            params: RunStartParams::default(),
            settings: Settings::new(),
        }
    }

    /// Names the Run for good: a start with an existing Run ID resumes the
    /// Run if it was interrupted, or returns it as it ended (ADR 0102),
    /// which [`Run::started_as`] says; the engine names it when none is
    /// given. A Run another engine writes fails with `run_in_use`, one
    /// begun with other inputs with `run_conflict`.
    pub fn run_id(mut self, run_id: &RunId) -> Self {
        self.params.run_id = Some(run_id.as_str().to_owned());
        self
    }

    /// Tags the Run, in place of any tags given before (ADR 0036).
    pub fn tags(mut self, tags: Tags) -> Self {
        self.params.tags = tags;
        self
    }

    /// Sets the Run's own `limits`, in place of any given before: each only
    /// lowers what covers the Run, and a start that names the Run again
    /// lowers its Limits and never raises them (ADRs 0080 and 0102).
    pub fn limits(mut self, limits: Limits) -> Self {
        self.params.limits = limits.into();
        self
    }

    /// Sets the Run's own settings, in place of any given before: what
    /// they leave out the connection's settings give, and the Run's
    /// Sessions and pages inherit them (ADR 0083). A key a Run's start does
    /// not set, as `timeout.run`, fails the start with
    /// `setting_not_applicable` before anything is sent.
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }
}

impl IntoFuture for Start {
    type Output = Result<Run, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Run, Error>> + Send + 'static>>;

    /// Reads the level the program's subscriber records for the engine
    /// on the calling thread, which the start tells the engine first when
    /// it changed (ADR 0078). The start carries the project file the
    /// Connection carries (ADR 0083).
    fn into_future(mut self) -> Self::IntoFuture {
        let settings = self.settings.within(SettingScope::Run);
        let started = settings.map(|settings| {
            self.params.settings = settings;
            self.params.project = self.connection.project().cloned();
            self.connection.start_with(RUN_START.name, self.params)
        });
        Box::pin(async move { started?.await })
    }
}

impl ClientConnection {
    /// Starts a Run with `opener`, a Product's own call that opens a Run
    /// and answers `RunStarted`, such as Foxhound's `crawl.start` (ADR
    /// 0102), sending `params`: the Run it answers is held as a start's
    /// is, closed with the Connection. Reads the level the program's
    /// subscriber records for the engine on the calling thread, which the
    /// start tells the engine first when it changed (ADR 0078).
    ///
    /// # Errors
    ///
    /// What `opener` fails with, such as `invalid_params`, and `closed`
    /// once the Connection has ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before `opener` is answered may leave the Run
    /// started or not; a start under a Run ID of the program's own then
    /// resumes it (ADR 0102).
    pub fn start_with(
        &self,
        opener: &'static str,
        params: impl Serialize + Send + 'static,
    ) -> impl Future<Output = Result<Run, Error>> + Send + 'static {
        self.start_held(opener, params, false)
    }

    /// Starts a Run with `opener` as [`ClientConnection::start_with`]
    /// does, held `detached` when it outlives the program: closing or
    /// dropping its value then sends no `run.abort` (ADR 0102).
    pub(crate) fn start_held(
        &self,
        opener: &'static str,
        params: impl Serialize + Send + 'static,
        detached: bool,
    ) -> impl Future<Output = Result<Run, Error>> + Send + 'static {
        let log_level = engine_log_level();
        let connection = self.clone();
        async move {
            connection.follow_log_level(log_level).await?;
            let started: RunStarted = connection.call(opener, &params).no_timeout().await?;
            log_start(&started);
            let held = HeldRun::new(
                connection,
                started.run_id.clone(),
                started.lifecycle,
                detached,
            );
            Ok(Run {
                host: Host::run(held),
                opened: Arc::new(started),
            })
        }
    }
}

/// Tells the host how a start with an existing Run ID reached its Run:
/// resumed at info, ended at warning, since the Run then takes no new work
/// (ADR 0102). A new Run is not logged.
fn log_start(started: &RunStarted) {
    let run_id = &started.run_id;
    match started.started_as {
        StartedAs::New => {}
        StartedAs::Resumed => tracing::info!(
            target: logging::RUN,
            %run_id,
            started_as = "resumed",
            "The start resumed the Run `{run_id}` from its committed state."
        ),
        StartedAs::Ended => tracing::warn!(
            target: logging::RUN,
            %run_id,
            started_as = "ended",
            "The start found the Run `{run_id}` ended: it is returned as it ended and takes no new work."
        ),
    }
}

impl Run {
    /// Closes the Run for every clone: a Run that still runs is aborted by
    /// its program (`run.abort`), resumable by its Run ID; an ended one
    /// stays as it is. Any later call on it, or on a handle it opened,
    /// fails with `closed`. Closing again does nothing.
    ///
    /// Stops a Run at its next safe point, `stopped` and `incomplete`: no
    /// attempt in flight commits, and its work items return to pending.
    /// Answers its Run report; an ended Run's report is answered as it is.
    /// A Run whose engine died is stopped by this engine, which takes its
    /// writer role at the next run epoch; a Run another engine writes is
    /// stopped by that engine, the owner the Run index names.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// What `run.abort` fails with, unless the Connection is closed: its
    /// engine then stops the Run itself. `deadline_exceeded` once
    /// `timeout.close` has run out, the value closed all the same.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after its first poll leaves the value closed
    /// and the abort sent.
    pub async fn close(&self) -> Result<(), Error> {
        crate::markers::warn_operation("run.abort");
        self.host.held_run().close().await
    }

    /// Reads a Run's progress: what it does and why it waits. Without
    /// `after`, it answers at once; given `after`, the position of the value
    /// the reader last read, it waits for a change and answers the latest
    /// value, so values in between are coalesced. The value whose `last` is
    /// true ends the sequence: the Run ended, and its `lifecycle` and
    /// `stop_reason` say how, or no engine works it (`writer: none`) until a
    /// start with its Run ID resumes it. A Run another engine works is read
    /// through that engine, the owner the Run index names, and one no engine
    /// works from its committed state; a read never takes the Run's writer
    /// lock. A Run ID the Run index does not list fails with
    /// `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The stream (`run.progress`, ADR 0102) sends its first read when it
    /// is first polled; each next read gives `after`, so each next value is
    /// the latest once a newer one comes, read when the program asks for
    /// it. The stream ends after the value whose `last` is true.
    ///
    /// ```no_run
    /// # async fn follow(run: goodfetch_collie_client::Run) -> Result<(), goodfetch_collie_client::Error> {
    /// let mut progress = run.progress();
    /// while let Some(value) = progress.next().await {
    ///     let value = value?;
    ///     tracing::info!(lifecycle = ?value.lifecycle, pauses = value.pauses.len(), "the Run's progress");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// An item is `closed` once the Run or the Connection is closed, or
    /// the engine's error, such as `run_not_found`, after which the stream
    /// ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels its read in flight, and leaves the Run
    /// as it is.
    pub fn progress(&self) -> EventStream<Answer<RunProgress>> {
        crate::markers::warn_operation(RUN_PROGRESS_READ.name);
        EventStream::from_items(progress::Following::new(self.host.clone()))
    }

    /// The handle of the Run's page `page_id`, which the engine opened
    /// rather than the program, such as the page a Product serves a crawl
    /// attempt's document on (ADR 0093). It sends nothing, and dropping it
    /// leaves the page open: what opened the page closes it.
    #[must_use]
    pub fn page_of(&self, page_id: PageId) -> Page {
        Page {
            host: self.host.reached(page_id),
        }
    }

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the Run runs with (ADR 0083), as `config explain` shows it
    /// (`config.explain`), which awaiting asks, naming this Run: each
    /// setting, or those [`Query::keys`] names, with the origin that set
    /// it, its own settings over its Connection's. A Run that is no longer
    /// live fails with `run_not_found`, and a closed Run's value with
    /// `closed`.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        crate::markers::warn_operation(CONFIG_EXPLAIN.name);
        self.host.query(
            CONFIG_EXPLAIN.name,
            ConfigExplainParams {
                run_id: Some(self.host.id().as_str().to_owned()),
                ..ConfigExplainParams::default()
            },
        )
    }
}

impl<T> Query<T, ConfigExplainParams> {
    /// Explains only `keys`, each a key, such as `timeout.wait`, or a
    /// prefix of keys, such as `timeout`, in place of any given before;
    /// left out, every key is explained.
    pub fn keys(self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let keys: Vec<String> = keys.into_iter().map(Into::into).collect();
        self.edit(|params| params.keys = keys)
    }
}

impl ClientConnection {
    /// Leaving the scope or `close()` stops the Run with a Checkpoint;
    /// `finish()` ends it.
    ///
    /// Starts a Run its program drives, and answers once it is started. A
    /// Run ID names the Run for good, and a start under one acts on what
    /// the Run index holds (ADR 0102): no such Run begins it, `new`; an
    /// interrupted Run, stopped, failed, or running with no live writer,
    /// resumes from its committed state at the next run epoch, `resumed`,
    /// as `run.resume` does; a finished Run is answered as it ended with no
    /// handler called, `ended`. A Run live with a writer, in this engine or
    /// another, fails with `run_in_use`; a Run started with other inputs or
    /// by another caller with `run_conflict` listing each difference; a Run
    /// the Run index lists whose state is missing or unreadable with
    /// `run_state_unavailable`, never starting afresh.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a Run this Connection drives (`run.start`), which
    /// awaiting sends.
    pub fn start(&self) -> Start {
        crate::markers::warn_operation("run.start");
        Start::new(self.clone())
    }

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the Connection runs with (ADR 0083), as `config explain` shows
    /// it (`config.explain`), which awaiting asks: each setting, or those
    /// [`Query::keys`] names, with the origin that set it, the
    /// Connection's own settings over the project file it carries, which
    /// each Run it starts inherits. Once the Connection is closed it fails
    /// with `closed`.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        crate::markers::warn_operation(CONFIG_EXPLAIN.name);
        Query::new(
            self.sender(),
            CONFIG_EXPLAIN.name,
            Ok(ConfigExplainParams {
                project: self.project().cloned(),
                ..ConfigExplainParams::default()
            }),
        )
    }
}

/// C-SEND-SYNC: a Run's handles and its start move between threads, are
/// shared between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Run>();
    send_sync_static::<crate::generated::handles::Page>();
    send_static::<Start>();
    send_static::<<Start as IntoFuture>::IntoFuture>();
};
