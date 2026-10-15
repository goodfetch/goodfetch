//! Entry points and lifecycle (ADR 0078, decided in #465).
//!
//! Logs are `tracing` events under `goodfetch::…` targets; the crate never installs a subscriber.
//! The Rust crate links the engine (`EngineChoice::Linked`) and borrows a browser through
//! `session.source = cdp_url`, never through a relay. The Run's start verbs, its ID, its end and its
//! report are ADR 0102's, in `run`; Sessions, logins and secrets are ADR 0092's, in `sessions`; the page API is ADR 0085's,
//! in `page`. The error type is ADR 0079's, in `results`.
//! Timeouts, waits and Limits are #467's, in `time`; events, user code and sharing are #468's, in
//! `concurrency`.

use std::future::IntoFuture;
use std::path::PathBuf;
use std::time::Duration;

use crate::page::PageClose;
use crate::results::Error;
use crate::settings::{ExplainedSetting, SettingKey, Settings};
use crate::time::{Call, CallFuture, Limits, Timeout};

/// Which engine each Product's `connect` reaches.
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
        /// The token, a value from the program's own store: a Secret reference resolves in the
        /// engine, which the token is needed to reach (ADR 0092). It never appears anywhere.
        token: String,
    },
}

/// Options of each Product's `connect`: its own, then every other setting in
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
    pub fn settings(self, settings: impl Into<Settings>) -> Self {
        let _ = settings.into();
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
    /// The engine's Product (ADR 0095): `foxhound` serves a Collie connection too; a newer one is
    /// kept as received (G3).
    pub product: String,
    /// The engine's version.
    pub version: String,
    /// The protocol's `major.minor`.
    pub protocol_version: String,
    /// The project configuration file read, if any (ADR 0083).
    pub project_file: Option<PathBuf>,
    /// The Goodfetch home the engine uses (`GOODFETCH_HOME`); `None` for an Exposed Daemon or
    /// Goodfetch Cloud.
    pub home: Option<PathBuf>,
    /// The engine executable run for the Script sandbox and `plugin-shim` (ADR 0024), from the
    /// Product's own engine-path variable, `GOODFETCH_COLLIE_ENGINE_PATH` or
    /// `GOODFETCH_FOXHOUND_ENGINE_PATH` (ADR 0090); `None` when none was run.
    pub engine_path: Option<PathBuf>,
    /// The address reached (`GOODFETCH_URL` or `engine`); `None` for an engine on this machine.
    /// `GOODFETCH_TOKEN` never appears.
    pub url: Option<String>,
}

/// Starting a Run, not sent until awaited.
#[derive(Debug)]
#[must_use = "a Run does not start until this is awaited"]
pub struct Start {
    _private: (),
}

impl Start {
    /// The Run's ID, which names it for good in the Run index (ADR 0031); left out, the engine
    /// generates one. A start that names an existing Run resumes it or returns it ended, so a program
    /// run again under its ID carries on where it stopped. One the engine refuses fails with
    /// `run_id_invalid`.
    pub fn id(self, id: impl Into<String>) -> Self {
        let _ = id.into();
        unimplemented!()
    }

    /// How long the whole Run may last. A setting like the others (ADR 0028), whose built-in default
    /// is none. On expiry the Run stops with a
    /// Checkpoint, `stopped` and `incomplete`, and can be resumed; a resume's clock starts afresh.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout for this Run, where no lower layer locked or capped the setting (ADR 0028).
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// The Run's Limits. On a resume, compared with what the Run began with: equal leaves each Limit
    /// as it stands, a person's raise included; lower lowers it; higher fails with
    /// `limit_raise_refused` (ADR 0041).
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }

    /// The Run's Plan file, relative to the working directory: an input of the Run (ADR 0091), read
    /// here and written back here when a new version is promoted. Left out, the Plan lives only in
    /// the Run trace.
    pub fn plan(self, path: impl AsRef<std::path::Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// The Run's Response recording (ADR 0105), an input like its Plan file, read and written
    /// here: replayed, every request is answered from it and none is sent; recorded, it receives
    /// what the Run fetched when the Run ends.
    pub fn recording(self, recording: crate::recording::Recording) -> Self {
        let _ = recording;
        unimplemented!()
    }

    /// Settings for this Run and everything opened beneath it (ADR 0083). Its own timeout is
    /// [`Start::timeout`] or [`Start::no_timeout`]; `timeout::RUN` here fails with
    /// `setting_not_applicable` (S2).
    pub fn settings(self, settings: impl Into<Settings>) -> Self {
        let _ = settings.into();
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

/// A Run, always opened explicitly. [`Run::finish`] ends it `finished`; closing an unfinished Run
/// stops it with a Checkpoint, resumable by starting its ID again (ADR 0102). A cheap clone of one
/// Run (ADR 0081).
#[derive(Debug, Clone)]
pub struct Run {
    _private: (),
}

impl Run {
    /// The Run's ID, chosen at the start or generated: it names this Run for good.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// Opens a page in the Run's default Session, which the first call opens with the Run's
    /// settings and every later call shares; `page.session()` reaches it. Once it is closed, this
    /// fails with `closed`. A call like any other: its future can be spawned.
    pub fn new_page(&self) -> Call<Page> {
        unimplemented!()
    }

    /// Stops the Run if unfinished, after closing its children. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }

    /// Opens a Session of its own (ADR 0092): its source with `.source(..)`, its settings with
    /// `.settings(..)`.
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
    pub fn settings(self, settings: impl Into<Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }
}

/// A Session of a Run: Playwright's context, under the glossary's name (ADR 0085). Its page members
/// are in `page`, its cookies and login in `sessions` (ADR 0092). Closing it never closes a Borrowed browser. There is no
/// active page. A cheap clone of one Session (ADR 0081).
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
    pub fn settings(self, settings: impl Into<Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }
}

/// A page Goodfetch drives: a tab of a Session, opened by `session.new_page()` or by the page
/// itself (a popup). Its API is ADR 0085's, in `page`. Closing it closes only a tab Goodfetch
/// opened; on a Borrowed browser it only releases it. A cheap clone of one page (ADR 0081).
#[derive(Debug, Clone)]
pub struct Page {
    _private: (),
}

impl Page {
    /// Closes the tab. Idempotent.
    pub fn close(&self) -> PageClose {
        unimplemented!()
    }

    /// The page's effective settings, with the layer that set each; all of them when `keys` is empty.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }
}
