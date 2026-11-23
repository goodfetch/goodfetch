//! The handler-less starts (ADRs 0036 and 0102): a Recipe's run
//! (`recipe.start`) and a code-free catalog run (`catalog_run.start`),
//! Runs with no handler in the program. Their builder takes [`Start`]'s
//! options, and the Run's recording, its `timeout` and `detached`, with
//! which the Run outlives its program: closing its value, or the
//! Connection, then sends no `run.abort`. Python's `handlerless.py` and
//! TypeScript's `handlerless.ts` are its twins.
//!
//! [`Start`]: crate::Start

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::Duration;

use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::run::{
    CATALOG_RUN_START, CatalogRunStartParams, CatalogTarget, RECIPE_START, RecipeStartParams,
    Recording, RunId, RunLimit, Tags,
};
use goodfetch_protocol::settings::{
    RunOrigin, SettingInvalidDetails, SettingLimit, SettingsLayer, TIMEOUT_RUN,
};
use serde_json::Value;

use crate::call::wire_millis;
use crate::connection::ClientConnection;
use crate::error::Error;
use crate::generated::handles::Run;
use crate::limits::Limits;
use crate::settings::Settings;

/// What a handler-less start runs.
#[derive(Debug)]
enum Target {
    /// An installed Recipe's name, or the path to its manifest.
    Recipe(String),
    /// A code-free catalog run's target.
    Catalog(CatalogTarget),
}

/// How long a handler-less Run may last, as its start gives it.
#[derive(Debug, Clone, Copy)]
enum RunTimeout {
    /// As its `timeout.run` setting says: nothing sent.
    Setting,
    /// At most this long.
    After(Duration),
    /// As long as it takes: `null` sent.
    Unbounded,
}

/// The start of a Run with no handler in the program, a Recipe's run
/// (`recipe.start`) or a code-free catalog run (`catalog_run.start`),
/// which awaiting sends. Nothing is sent until it is awaited.
#[must_use = "a start is sent only when it is awaited"]
#[derive(Debug)]
pub struct HandlerlessStart {
    connection: ClientConnection,
    target: Target,
    run_id: Option<String>,
    tags: Tags,
    limits: Vec<RunLimit>,
    /// The Run's own settings, which its params carry once checked.
    settings: Settings,
    recording: Option<Recording>,
    timeout: RunTimeout,
    detached: bool,
}

impl HandlerlessStart {
    /// The start of a Recipe's run on `connection`.
    pub(crate) fn recipe(connection: ClientConnection, recipe: &str) -> Self {
        Self::new(connection, Target::Recipe(recipe.to_owned()))
    }

    /// The start of a code-free catalog run on `connection`.
    pub(crate) fn catalog(connection: ClientConnection, target: CatalogTarget) -> Self {
        Self::new(connection, Target::Catalog(target))
    }

    fn new(connection: ClientConnection, target: Target) -> Self {
        Self {
            connection,
            target,
            run_id: None,
            tags: Tags::default(),
            limits: Vec::new(),
            settings: Settings::new(),
            recording: None,
            timeout: RunTimeout::Setting,
            detached: false,
        }
    }

    /// Names the Run for good, as [`crate::Start::run_id`] does.
    pub fn run_id(mut self, run_id: &RunId) -> Self {
        self.run_id = Some(run_id.as_str().to_owned());
        self
    }

    /// Tags the Run, in place of any tags given before (ADR 0036).
    pub fn tags(mut self, tags: Tags) -> Self {
        self.tags = tags;
        self
    }

    /// Sets the Run's own `limits`, in place of any given before, as
    /// [`crate::Start::limits`] does.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits.into();
        self
    }

    /// Sets the Run's own settings, in place of any given before, as
    /// [`crate::Start::settings`] does.
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Sets the Response recording the Run replays or records (ADR 0105).
    pub fn recording(mut self, recording: Recording) -> Self {
        self.recording = Some(recording);
        self
    }

    /// Bounds how long the Run may last, as `timeout.run` for this Run,
    /// sent in whole milliseconds, rounded up; its setting's value bounds
    /// it when neither this nor [`HandlerlessStart::no_timeout`] is given.
    /// A zero `timeout` fails the start with `setting_invalid` before
    /// anything is sent.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = RunTimeout::After(timeout);
        self
    }

    /// Lets the Run last as long as it takes, whatever `timeout.run` says.
    pub fn no_timeout(mut self) -> Self {
        self.timeout = RunTimeout::Unbounded;
        self
    }

    /// Lets the Run outlive the program that started it (ADR 0102):
    /// closing its value, or the Connection, then sends no `run.abort` and
    /// frees only the value, `run.finish` waits for its end and `run.stop`
    /// stops it.
    pub fn detached(mut self) -> Self {
        self.detached = true;
        self
    }
}

impl IntoFuture for HandlerlessStart {
    type Output = Result<Run, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Run, Error>> + Send + 'static>>;

    /// Reads the level the program's subscriber records for the engine on
    /// the calling thread, as [`crate::Start`]'s does, and carries the
    /// project file the Connection carries (ADR 0083).
    fn into_future(self) -> Self::IntoFuture {
        let checked = self
            .settings
            .within(SettingScope::Run)
            .and_then(|settings| Ok((settings, wire_timeout(self.timeout)?)));
        let (settings, timeout) = match checked {
            Ok(checked) => checked,
            Err(error) => return Box::pin(std::future::ready(Err(error))),
        };
        let connection = self.connection;
        let project = connection.project().cloned();
        let detached = self.detached;
        match self.target {
            Target::Recipe(recipe) => Box::pin(connection.start_held(
                RECIPE_START.name,
                RecipeStartParams {
                    recipe,
                    run_id: self.run_id,
                    tags: self.tags,
                    settings,
                    project,
                    limits: self.limits,
                    recording: self.recording,
                    timeout,
                    detached,
                },
                detached,
            )),
            Target::Catalog(target) => Box::pin(connection.start_held(
                CATALOG_RUN_START.name,
                CatalogRunStartParams {
                    target,
                    run_id: self.run_id,
                    tags: self.tags,
                    settings,
                    project,
                    limits: self.limits,
                    recording: self.recording,
                    timeout,
                    detached,
                },
                detached,
            )),
        }
    }
}

/// A Run's own `timeout` as its params carry it: absent for the setting's
/// value, null for none, else whole milliseconds, a zero one refused as
/// the engine refuses it.
#[expect(
    clippy::option_option,
    reason = "the params' field is absent, null or a number on the wire (ADR 0080)"
)]
fn wire_timeout(timeout: RunTimeout) -> Result<Option<Option<u64>>, Error> {
    match timeout {
        RunTimeout::Setting => Ok(None),
        RunTimeout::Unbounded => Ok(Some(None)),
        RunTimeout::After(after) if after.is_zero() => {
            let key = TIMEOUT_RUN.name;
            let reason = "a timeout of 0 ms would end the Run at once";
            let fix = "give at least 1 ms, or `.no_timeout()` for no timeout";
            let details = SettingInvalidDetails {
                key: key.to_owned(),
                layer: Some(SettingsLayer::Run),
                reason: reason.to_owned(),
                limit: Some(SettingLimit {
                    key: key.to_owned(),
                    value: Value::from(0),
                    layer: SettingsLayer::Run,
                    origin: Some(RunOrigin::Argument),
                }),
                fix: Some(fix.to_owned()),
            };
            Err(Error::setting_invalid(
                format!("The start's `timeout`, its `{key}`: {reason}; {fix}."),
                &details,
            ))
        }
        RunTimeout::After(after) => Ok(Some(Some(wire_millis(after)))),
    }
}

impl ClientConnection {
    /// Starts a Recipe's run, a Run with no handler in the program, and
    /// answers once it is started. Its Run ID works as `run.start`'s. With
    /// `detached`, the Run outlives the program that started it: closing
    /// its value frees only the value, `run.finish` waits for its end and
    /// `run.stop` stops it.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a Recipe's run (`recipe.start`), which awaiting sends:
    /// `recipe` is an installed Recipe's name or the path to its manifest.
    pub fn start_recipe(&self, recipe: &str) -> HandlerlessStart {
        crate::markers::warn_operation(RECIPE_START.name);
        HandlerlessStart::recipe(self.clone(), recipe)
    }

    /// Starts a code-free catalog run, a Run with no handler in the program
    /// whose verbs come from the catalog (ADR 0036): an Extraction of a
    /// page by a schema, `extract`, an Agent task, `agent`, or a Plan
    /// replayed with no model call, `plan_replay`; and answers once it is
    /// started. Its Run ID works as `run.start`'s. With `detached`, the Run
    /// outlives the program that started it: closing its value frees only
    /// the value, `run.finish` waits for its end and `run.stop` stops it.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a code-free catalog run of `target`
    /// (`catalog_run.start`), which awaiting sends.
    pub fn start_catalog_run(&self, target: CatalogTarget) -> HandlerlessStart {
        crate::markers::warn_operation(CATALOG_RUN_START.name);
        HandlerlessStart::catalog(self.clone(), target)
    }
}

/// C-SEND-SYNC: a handler-less start moves between threads and outlives
/// the borrow that made it (ADR 0081).
const _: () = {
    const fn send_static<T: Send + 'static>() {}
    send_static::<HandlerlessStart>();
    send_static::<<HandlerlessStart as IntoFuture>::IntoFuture>();
};
