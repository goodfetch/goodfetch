//! Entry points and lifecycle (ADR 0078, decided in #465).
//!
//! Logs are `tracing` events under `goodfetch::…` targets; the crate never installs a subscriber.
//! The Rust crate links the engine (`EngineChoice::Linked`) and borrows a browser through
//! `session.source = cdp_url`, never through a relay. The Run's start verbs, Sessions and the
//! page API are placeholders owned by #478, #476 and #472. The error type is ADR 0079's, in `results`.
//! Timeouts, waits and Limits are #467's, in `time`; events, user code and sharing are #468's, in
//! `concurrency`.

use std::future::IntoFuture;
use std::path::PathBuf;
use std::time::Duration;

use crate::results::Error;
use crate::settings::{ExplainedSetting, SettingKey, Settings};
use crate::time::{Call, CallFuture, Limits, Timeout};

/// Which engine [`Goodfetch::connect`] reaches.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum EngineChoice {
    /// ADR 0024's rule: a Daemon when one is configured or running, otherwise the linked engine.
    #[default]
    Auto,
    /// The engine linked into this program.
    Linked,
    /// The user's Daemon on this machine.
    Daemon,
    /// An Exposed Daemon or Goodfetch Cloud, by address and token.
    Address {
        /// The endpoint.
        url: String,
        /// The token, which #476 turns into a Secret reference.
        token: String,
    },
}

/// Options of [`Goodfetch::connect`]: its own, then every other setting in
/// [`ConnectOptions::settings`] (ADR 0083).
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ConnectOptions {
    #[expect(dead_code, reason = "a signatures-only sketch: the runtime reads it")]
    engine: EngineChoice,
    #[expect(dead_code, reason = "a signatures-only sketch: the runtime reads it")]
    timeout: Timeout,
    #[expect(dead_code, reason = "a signatures-only sketch: the runtime reads it")]
    limits: Limits,
}

impl ConnectOptions {
    /// Which engine to reach; by default [`EngineChoice::Auto`].
    pub fn engine(self, engine: EngineChoice) -> Self {
        Self { engine, ..self }
    }

    /// How long starting or finding the engine and `hello` may take; by default, the connect
    /// phase's setting.
    pub fn timeout(self, timeout: Duration) -> Self {
        Self { timeout: Timeout::After(timeout), ..self }
    }

    /// No timeout for connecting.
    pub fn no_timeout(self) -> Self {
        Self { timeout: Timeout::Never, ..self }
    }

    /// Limits on this Client connection's work (ADR 0041), set or lowered by the program itself.
    pub fn limits(self, limits: Limits) -> Self {
        Self { limits, ..self }
    }

    /// Settings for this connection and everything opened beneath it.
    pub fn settings(self, settings: Settings) -> Self {
        let _ = settings;
        unimplemented!()
    }

    /// The environment map, read once by `connect` and nowhere else: the process's own unless
    /// replaced here (ADRs 0062, 0069). Only the deployment variables are read, and an unknown
    /// `GOODFETCH_` name fails with `setting_unknown`. Left at [`EngineChoice::Auto`], `engine` is
    /// filled by `GOODFETCH_URL` when set.
    pub fn env(self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        let _ = env.into_iter();
        unimplemented!()
    }

    /// The project configuration file; by default found upward from the working directory, as
    /// the CLI does.
    pub fn project(self, path: impl Into<PathBuf>) -> Self {
        let _ = path.into();
        unimplemented!()
    }

    /// Reads no project configuration file, for a service whose working directory means nothing.
    pub fn no_project(self) -> Self {
        unimplemented!()
    }
}

/// The kind of engine a connection reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineKind {
    /// The engine linked into this program.
    Linked,
    /// A Daemon on this machine.
    Daemon,
    /// An Exposed Daemon or Goodfetch Cloud.
    Remote,
}

/// The engine a connection reached, reported for diagnosis (G6).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EngineInfo {
    /// The kind reached.
    pub kind: EngineKind,
    /// The engine's version.
    pub version: String,
    /// The protocol's `major.minor`.
    pub protocol_version: String,
    /// The project configuration file read, if any (ADR 0083).
    pub project_file: Option<PathBuf>,
    /// The Goodfetch home the engine uses (`GOODFETCH_HOME`); `None` for an Exposed Daemon or
    /// Goodfetch Cloud.
    pub home: Option<PathBuf>,
    /// The engine binary run (`GOODFETCH_ENGINE_PATH`), for the Script sandbox and `plugin-shim`
    /// (ADR 0024); `None` when none was run.
    pub engine_path: Option<PathBuf>,
    /// The address reached (`GOODFETCH_URL` or `engine`); `None` for an engine on this machine.
    /// `GOODFETCH_TOKEN` never appears.
    pub url: Option<String>,
}

/// The engine connection: the entry point. Each one is independent, and many may be open in one process.
///
/// A cheap clone of one connection (ADR 0081). Dropping the last clone without [`Goodfetch::close`]
/// fires the cancellation tree without waiting and emits a `tracing` warning; Runs resume from their
/// last Checkpoint.
#[derive(Debug, Clone)]
pub struct Goodfetch {
    _private: (),
}

impl Goodfetch {
    /// Starts or finds the engine and completes `hello` before returning.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// Opens a connection, lends it to `body`, and closes it on every return and error.
    pub async fn scope<T>(
        options: ConnectOptions,
        body: impl AsyncFnOnce(&Goodfetch) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let _ = (options, body);
        unimplemented!()
    }

    /// The engine this connection reached.
    pub fn engine(&self) -> &EngineInfo {
        unimplemented!()
    }

    /// Collie, reached without I/O.
    pub fn collie(&self) -> Collie {
        unimplemented!()
    }

    /// The connection's effective settings, with the layer that set each; all of them when `keys`
    /// is empty.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }

    /// Cancels in-flight calls, closes every Run it opened, then stops a linked engine or
    /// disconnects from a Daemon, for every clone. Idempotent. Its timeout is the close phase's.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Drop for Goodfetch {
    /// Never blocks and never fails (C-DTOR-FAIL, C-DTOR-BLOCK); acts when the last clone is dropped.
    fn drop(&mut self) {}
}

/// Collie, a cheap clone that holds its connection.
#[derive(Debug, Clone)]
pub struct Collie {
    _connection: Goodfetch,
}

impl Collie {
    /// Placeholder start verb: its spelling is #478's; its timeout and Limits are #467's.
    pub fn start(&self) -> Start {
        unimplemented!()
    }
}

/// Starting a Run, not sent until awaited.
#[derive(Debug)]
#[must_use = "a Run does not start until this is awaited"]
pub struct Start {
    _collie: Goodfetch,
}

impl Start {
    /// How long the whole Run may last. A setting like the others (ADR 0028), whose built-in default
    /// is none. On expiry the Run stops with a
    /// Checkpoint, `stopped` and `incomplete`, and can be resumed.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout for this Run, where no lower layer locked or capped the setting (ADR 0028).
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// The Run's Limits.
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }

    /// Settings for this Run and everything opened beneath it (ADR 0083). Its own timeout is
    /// [`Start::timeout`] or [`Start::no_timeout`]; `timeout::RUN` here fails with
    /// `setting_not_applicable` (S2).
    pub fn settings(self, settings: Settings) -> Self {
        let _ = settings;
        unimplemented!()
    }
}

impl IntoFuture for Start {
    type Output = Result<Run, Error>;
    type IntoFuture = CallFuture<Run>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint,
/// resumable by its ID. A cheap clone of one Run (ADR 0081).
#[derive(Debug, Clone)]
pub struct Run {
    _private: (),
}

impl Run {
    /// The Run's ID.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// Opens a page. Its Session is #476's. A call like any other: its future can be spawned.
    pub fn new_page(&self) -> Call<Page> {
        unimplemented!()
    }

    /// Stops the Run if unfinished, after closing its children. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }

    /// Placeholder: opening a Session is #476's; its settings are #470's.
    pub fn new_session(&self) -> Call<Session> {
        unimplemented!()
    }

    /// The Run's effective settings, with the layer that set each; all of them when `keys` is empty.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }
}

impl Call<Session> {
    /// Settings for the Session being opened and its pages. On a Borrowed browser, any
    /// `browser::*` value fails with `setting_not_applicable`.
    pub fn settings(self, settings: Settings) -> Self {
        let _ = settings;
        unimplemented!()
    }
}

/// A Session of a Run. Its API is #476's. Closing it never closes a Borrowed browser.
/// A cheap clone of one Session (ADR 0081).
#[derive(Debug, Clone)]
pub struct Session {
    _private: (),
}

impl Session {
    /// Opens a page in this Session.
    pub fn new_page(&self) -> Call<Page> {
        unimplemented!()
    }

    /// The Session's effective settings, with the layer that set each; all of them when `keys` is
    /// empty.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }

    /// Closes its pages, then the Session. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Call<Page> {
    /// Settings for the page being opened. A part of the Session's identity, such as
    /// `browser::VIEWPORT`, fails with `setting_not_applicable` (ADR 0021).
    pub fn settings(self, settings: Settings) -> Self {
        let _ = settings;
        unimplemented!()
    }
}

/// A page Goodfetch drives. Its API is #472's. Closing it closes only a tab Goodfetch opened.
/// A cheap clone of one page (ADR 0081).
#[derive(Debug, Clone)]
pub struct Page {
    _private: (),
}

impl Page {
    /// Closes the tab. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }

    /// The page's effective settings, with the layer that set each; all of them when `keys` is empty.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }
}
