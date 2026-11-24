//! The connection, which is the Product (ADRs 0078, 0090 and 0095): what a
//! Rust program opens to drive Foxhound, and the scope that opens and
//! closes it around a body. Each connection starts its own linked engine,
//! the Foxhound engine this crate composes, in the program's own process
//! (ADRs 0024 and 0087), through Collie's crate's linked connection, or
//! reaches a Daemon ([`crate::EngineChoice`]).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_collie::EngineInfo;
use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_collie_client::{
    ClientConnection, Connection, Error, HandlerlessStart, Query, RawCall, RunView, Start,
    UsageKind,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{CatalogTarget, RunId};
use goodfetch_protocol::settings::{ConfigExplainParams, SettingsExplanation};

use crate::crawl_requests::CrawlRequests;

/// Foxhound's linked engine: Foxhound, of [`crate::composition`].
pub(crate) const ENGINE: Composed = Composed {
    product: Product::Foxhound,
    composition: crate::composition,
};

/// Foxhound's paid fetches, the Usage kind a Limit caps besides Collie's
/// (ADR 0087): a Collie engine refuses it with `not_in_product`.
pub const PAID_FETCHES: UsageKind =
    UsageKind::named(goodfetch_protocol::foxhound::usage::PAID_FETCH);

/// What a Foxhound connection opens with: every option of Collie's, under
/// the same names, with Foxhound's settings and Limits, and the program's
/// own Fetch backends and Sinks, each registered by name, which settings
/// choose (ADR 0104). A key the engine reads only from its configuration,
/// such as `storage.low_free`, is no key of [`crate::settings::Settings`],
/// and an engine refuses it with `setting_not_applicable`, naming the
/// files that set it.
///
/// ```
/// use goodfetch_foxhound::settings::{Settings, crawl};
/// use goodfetch_foxhound::{ConnectOptions, Foxhound};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// let options = ConnectOptions::default()
///     .settings(Settings::new().set(crawl::MAX_DEPTH, 2))
///     .fetch_backend("in-house")
///     .sink("warehouse");
/// # let options = options
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
/// let foxhound = Foxhound::connect(options).await?;
/// foxhound.close().await?;
/// # Ok::<(), goodfetch_foxhound::Error>(())
/// # })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    collie: goodfetch_collie::ConnectOptions,
    /// The names of the program's own Fetch backends, which the engine
    /// calls once #1484 lands.
    fetch_backends: BTreeSet<String>,
    /// The names of the program's own Sinks, which the engine delivers to
    /// once #1491 lands.
    sinks: BTreeSet<String>,
}

impl ConnectOptions {
    /// The engine the connection reaches, in place of
    /// [`crate::EngineChoice::Auto`], whatever `GOODFETCH_URL` says.
    #[must_use]
    pub fn engine(self, engine: crate::EngineChoice) -> Self {
        self.collie(|collie| collie.engine(engine))
    }

    /// The environment variables the connection and its linked engine
    /// read, such as `GOODFETCH_HOME` and `GOODFETCH_URL`, in place of the
    /// process's own, as Collie's `ConnectOptions::environment` checks them.
    #[must_use]
    pub fn environment(self, environment: BTreeMap<OsString, OsString>) -> Self {
        self.collie(|collie| collie.environment(environment))
    }

    /// The connection's own `limits` (ADR 0080), in place of any given
    /// before: each only lowers what covers the connection's work,
    /// [`crate::PAID_FETCHES`] among them.
    #[must_use]
    pub fn limits(self, limits: crate::Limits) -> Self {
        self.collie(|collie| collie.limits(limits))
    }

    /// The connection's own settings, Collie's and Foxhound's (ADR 0083),
    /// in place of any given before: they travel in `hello`, over
    /// configuration and the Operator policy, and every Run the connection
    /// starts inherits what its own settings leave out.
    #[must_use]
    pub fn settings(self, settings: crate::settings::Settings) -> Self {
        self.collie(|collie| collie.settings(settings))
    }

    /// Fails the connect if the engine has not answered `hello` within
    /// `timeout`, with `deadline_exceeded` naming `timeout.connect`, as
    /// Collie's `ConnectOptions::timeout` does.
    #[must_use]
    pub fn timeout(self, timeout: Duration) -> Self {
        self.collie(|collie| collie.timeout(timeout))
    }

    /// Waits for the engine to answer `hello` as long as it takes.
    #[must_use]
    pub fn no_timeout(self) -> Self {
        self.collie(goodfetch_collie::ConnectOptions::no_timeout)
    }

    /// Reads the project file `path` (ADR 0083) in place of the
    /// `goodfetch.toml` found upward from the working directory, as
    /// Collie's `ConnectOptions::project` reads it.
    #[must_use]
    pub fn project(self, path: impl Into<PathBuf>) -> Self {
        self.collie(|collie| collie.project(path))
    }

    /// Reads no project file: the connection's Runs and its
    /// `explain_settings()` have no layer 3.
    #[must_use]
    pub fn no_project(self) -> Self {
        self.collie(goodfetch_collie::ConnectOptions::no_project)
    }

    /// How many unread events each stream of a Session's events holds
    /// (ADR 0081), in place of 1,024, as Collie's
    /// `ConnectOptions::max_listener_queue` bounds it.
    #[must_use]
    pub fn max_listener_queue(self, bound: usize) -> Self {
        self.collie(|collie| collie.max_listener_queue(bound))
    }

    /// Registers the program's own Fetch backend under `name`, the name
    /// settings such as `fetch.backend` choose it by (ADR 0104). It runs
    /// with the program's full rights, never confined.
    #[must_use]
    pub fn fetch_backend(mut self, name: impl Into<String>) -> Self {
        self.fetch_backends.insert(name.into());
        self
    }

    /// Registers the program's own Sink under `name`, the name of the
    /// `sinks.<name>` entry it serves (ADR 0104). It runs with the
    /// program's full rights, never confined.
    #[must_use]
    pub fn sink(mut self, name: impl Into<String>) -> Self {
        self.sinks.insert(name.into());
        self
    }

    /// Collie's options, changed by `change`.
    fn collie(
        mut self,
        change: impl FnOnce(goodfetch_collie::ConnectOptions) -> goodfetch_collie::ConnectOptions,
    ) -> Self {
        self.collie = change(self.collie);
        self
    }

    /// Collie's options, which open the linked connection.
    pub(crate) fn into_collie(self) -> goodfetch_collie::ConnectOptions {
        self.collie
    }
}

/// A connection to Foxhound, which is the Product: it carries every member
/// of a Collie connection under the same names, so `foxhound.start()`
/// starts a Run. Cloning it is cheap and every clone is the same
/// connection; `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_foxhound::{ConnectOptions, Foxhound};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
/// let foxhound = Foxhound::connect(options).await?;
/// let run = foxhound.start().await?;
/// let report = run.finish().await?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// foxhound.close().await?;
/// # Ok::<(), goodfetch_foxhound::Error>(())
/// # })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Foxhound {
    linked: ProductConnection,
    /// The engine's Crawl requests to the connection, which the serve of
    /// each of its crawl Runs takes its own from.
    crawl_requests: Arc<CrawlRequests>,
}

impl Foxhound {
    /// Opens a connection to the engine `options` choose
    /// ([`ConnectOptions::engine`]): at `Auto`, the remote Daemon
    /// `GOODFETCH_URL` names, else the Daemon running in the Goodfetch
    /// home, else a linked Foxhound engine of its own. The connection runs
    /// on a runtime it owns, so it works from any executor, and returns
    /// once `hello` is answered. It reads `options`' environment, or the
    /// process's, for its Goodfetch home, a remote Daemon and the machine's
    /// settings, and never an engine-path variable; it logs the engine it
    /// reached at info.
    /// It sets `options`' Limits on the connection before it returns
    /// ([`ConnectOptions::limits`]), and waits for `hello`'s answer within
    /// `options`' `timeout`, `timeout.connect`'s 30 s when left out
    /// ([`ConnectOptions::timeout`]).
    ///
    /// # Errors
    ///
    /// `engine_exited` when the linked engine cannot start, with no
    /// Goodfetch home among others, `setting_invalid` for a zero `timeout`,
    /// a zero `max_listener_queue` or a remote Daemon's URL that names no loopback `ws://` address, or
    /// `GOODFETCH_URL` without `GOODFETCH_TOKEN` or a deployment variable
    /// set but empty, not Unicode or unparsable by the engine,
    /// `setting_unknown` for a `GOODFETCH_`
    /// variable off the closed list, naming the closest, `setting_invalid`
    /// at `project_configuration` or `format_version_unsupported` for a
    /// project file that cannot be read (`ConnectOptions::project`),
    /// `daemon_unreachable` when
    /// the Daemon chosen cannot be reached, which `Daemon` never starts
    /// (ADR 0098), `deadline_exceeded` naming `timeout.connect` when `hello` is not
    /// answered within it, what `hello` fails with, and what `usage.set`
    /// fails with for a Limit the engine refuses, the connection closed
    /// then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes stops the linked engine it
    /// started.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let linked = ProductConnection::connect(ENGINE, options.into_collie()).await?;
        Ok(Self {
            linked,
            crawl_requests: Arc::default(),
        })
    }

    /// Opens a connection with `options`, lends it to `body`, and closes it
    /// whatever `body` returns. Its future is `Send` whenever `body`'s is
    /// (ADR 0112).
    ///
    /// ```
    /// use goodfetch_foxhound::{ConnectOptions, Foxhound};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let home = tempfile::tempdir()?;
    /// # let options = ConnectOptions::default()
    /// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
    /// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
    /// let report = Foxhound::scope(options, async |foxhound| {
    ///     foxhound.start().await?.finish().await
    /// })
    /// .await?;
    /// # Ok::<(), goodfetch_foxhound::Error>(())
    /// # })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// What [`Foxhound::connect`] fails with, then `body`'s error, then
    /// what [`Foxhound::close`] fails with.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes drops the connection
    /// without waiting, as dropping a [`Foxhound`] does.
    pub async fn scope<R>(
        options: ConnectOptions,
        body: impl AsyncFnOnce(&Foxhound) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let foxhound = Self::connect(options).await?;
        let result = body(&foxhound).await;
        let closed = foxhound.close().await;
        let value = result?;
        closed.map(|()| value)
    }

    /// The engine the connection reached, fixed when it opened: the linked
    /// engine or a Daemon ([`crate::EngineChoice`]). It never holds a
    /// Daemon's token.
    pub fn engine(&self) -> &EngineInfo {
        self.linked.engine()
    }

    /// The raw call of `operation` (ADR 0099, G8), the way down to an
    /// operation the SDK does not generate, such as one a newer engine
    /// added: its params and its result are the wire's JSON, in
    /// snake case, and awaiting sends it.
    ///
    /// ```
    /// use goodfetch_foxhound::{ConnectOptions, Foxhound};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let home = tempfile::tempdir()?;
    /// # let options = ConnectOptions::default()
    /// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
    /// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
    /// let connection = Foxhound::connect(options).await?;
    /// let listed = connection
    ///     .call("run.list")
    ///     .params(serde_json::json!({ "lifecycle": "running" }))
    ///     .await?;
    /// assert_eq!(listed["runs"], serde_json::json!([]));
    /// connection.close().await?;
    /// # Ok::<(), goodfetch_foxhound::Error>(())
    /// # })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Once the connection is closed it fails with `closed`; the engine's
    /// error, such as `invalid_params`, and `deadline_exceeded` past
    /// `.timeout(d)`, which is sent as the operation's own, fail it too.
    /// It is never retried.
    pub fn call(&self, operation: &str) -> RawCall {
        self.linked.connection().raw_call(operation)
    }

    /// Leaving the scope or `close()` stops the Run with a Checkpoint.
    /// `finish()` ends it.
    ///
    /// In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK
    /// installs no signal handler, and a signal the program does not handle
    /// ends it as a crash would (ADRs 0078, 0093).
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
    /// The start of a Run the program drives (`run.start`), which awaiting
    /// sends.
    pub fn start(&self) -> Start {
        self.linked.connection().start()
    }

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
        self.linked.connection().start_recipe(recipe)
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
        self.linked.connection().start_catalog_run(target)
    }

    /// Opens a view of the Run `run_id` names, one another program started
    /// such as a detached Run, with no round trip (ADR 0102). The view takes
    /// no writer role, and closing it frees only the view.
    ///
    /// # Errors
    ///
    /// `closed` once the connection is closed.
    pub fn open_run(&self, run_id: &RunId) -> Result<RunView, Error> {
        self.linked.connection().open_run(run_id)
    }

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the connection runs with (ADR 0083), as `config explain`
    /// shows it (`config.explain`), which awaiting asks: each setting, or
    /// those `.keys(…)` names, with the origin that set it, the
    /// connection's own settings ([`ConnectOptions::settings`]), which each
    /// Run it starts inherits. A Run's own
    /// explanation is
    /// [`Run::explain_settings`](crate::Run::explain_settings). Once the connection is
    /// closed it fails with `closed`.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        self.linked.connection().explain_settings()
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and
    /// stops; returns once it has stopped, or once `timeout.close` has run
    /// out. Any later call fails with `closed`. Closing again
    /// does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, `engine_exited` when the linked
    /// engine failed, and `deadline_exceeded` when `timeout.close` ran out
    /// first.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes leaves the engine stopping
    /// on its own.
    pub async fn close(&self) -> Result<(), Error> {
        self.linked.close().await
    }
}

/// The Connection both Products' connections implement (ADRs 0095 and
/// 0099), so a helper taking `&impl Connection` takes a `Foxhound`; its
/// members are the inherent ones above.
impl Connection for Foxhound {
    fn start(&self) -> Start {
        Foxhound::start(self)
    }

    fn start_recipe(&self, recipe: &str) -> HandlerlessStart {
        Foxhound::start_recipe(self, recipe)
    }

    fn start_catalog_run(&self, target: CatalogTarget) -> HandlerlessStart {
        Foxhound::start_catalog_run(self, target)
    }

    fn open_run(&self, run_id: &RunId) -> Result<RunView, Error> {
        Foxhound::open_run(self, run_id)
    }

    fn call(&self, operation: &str) -> RawCall {
        Foxhound::call(self, operation)
    }

    fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        Foxhound::explain_settings(self)
    }

    async fn close(&self) -> Result<(), Error> {
        Foxhound::close(self).await
    }
}

impl Foxhound {
    /// The engine's Crawl requests to the connection, shared by the serves
    /// of its crawl Runs.
    pub(crate) fn crawl_requests(&self) -> &Arc<CrawlRequests> {
        &self.crawl_requests
    }
}

/// The client crate's connection, the way down to its typed `call` of a
/// runtime method, which the raw `call` (ADR 0099) does not type.
impl AsRef<ClientConnection> for Foxhound {
    fn as_ref(&self) -> &ClientConnection {
        self.linked.connection()
    }
}

/// C-SEND-SYNC: a connection's handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Foxhound>();
};
