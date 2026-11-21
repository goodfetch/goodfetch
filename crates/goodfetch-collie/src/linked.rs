//! The linked engine (ADRs 0078, 0081 and 0087): the engine a Product's
//! Rust SDK composes, which each connection starts in the program's own
//! process and reaches over area 3's in-memory network. It serves its one
//! Connection as an Attached engine does, and stops at the end of its
//! input, which closing the Connection gives it. A connection reaches a
//! Daemon in its place when [`EngineChoice`] says so
//! ([`crate::engine_choice`]).
//!
//! [`ProductConnection`] is the one way both Products' SDKs open a
//! connection: [`Collie`](crate::Collie) with [`Composed::COLLIE`], and
//! Foxhound's crate with its own composition, which starts from Collie's
//! (ADR 0087). It names no Product's entries, so Foxhound's stay out of
//! this crate's API (ADR 0095).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use goodfetch_collie_client::logging::{CLIENT, engine_log_level};
use goodfetch_collie_client::settings::Settings;
use goodfetch_collie_client::transport::{Dial, Endpoint, Link, Network};
use goodfetch_collie_client::{
    ClientConnection, ConnectTimeout, ConnectionRuntime, Error, Limits, ListenerQueue,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::Keychain;
use goodfetch_engine::settings::{
    SettingsError, SettingsFiles, check_environment, deployment_variable,
};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, InMemoryDialer};
use goodfetch_engine::{AttachedFailure, Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::{Product, SettingScope};
use goodfetch_protocol::connection::{
    EngineExitedDetails, HelloParams, HelloResult, ProductVersion, Surface,
};
use goodfetch_protocol::settings::{
    BROWSERS_PATH_VARIABLE, DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES, HOME_VARIABLE,
    SettingInvalidDetails, SettingUnknownDetails, TIMEOUT_CLOSE,
};
use goodfetch_protocol::usage::{self, LimitUsage, USAGE_SET, UsageSetParams};
use tokio::runtime::{Handle, Runtime};
use tokio_util::task::TaskTracker;

use crate::engine_choice::{self, DaemonReach, EngineChoice, EngineInfo, EngineKind, Reach};
use crate::runtime::{self, Threads};
use crate::trace_context;

/// What a connection opens with, in either Product's SDK. Its `Debug`
/// output names the environment's variables without their values, which
/// may hold a token such as `GOODFETCH_TOKEN`'s (ADR 0067).
#[derive(Clone, Default)]
pub struct ConnectOptions {
    environment: Option<BTreeMap<OsString, OsString>>,
    limits: Limits,
    timeout: ConnectTimeout,
    max_listener_queue: Option<usize>,
    engine: EngineChoice,
    settings: Settings,
}

impl fmt::Debug for ConnectOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectOptions")
            .field(
                "environment",
                &self.environment.as_ref().map(BTreeMap::keys),
            )
            .field("limits", &self.limits)
            .field("timeout", &self.timeout)
            .field("max_listener_queue", &self.max_listener_queue)
            .field("engine", &self.engine)
            .field("settings", &self.settings)
            .finish()
    }
}

impl ConnectOptions {
    /// The engine the connection reaches, in place of
    /// [`EngineChoice::Auto`], whatever `GOODFETCH_URL` says.
    #[must_use]
    pub fn engine(mut self, engine: EngineChoice) -> Self {
        self.engine = engine;
        self
    }

    /// The environment variables the connection and its linked engine
    /// read, such as `GOODFETCH_HOME` and `GOODFETCH_URL`, in place of the
    /// process's own. A `GOODFETCH_` variable off the closed list of
    /// deployment variables fails the connect with `setting_unknown`, naming
    /// the closest, and an empty one or one that is no Unicode with
    /// `setting_invalid`, before any engine is reached (ADR 0083).
    #[must_use]
    pub fn environment(mut self, environment: BTreeMap<OsString, OsString>) -> Self {
        self.environment = Some(environment);
        self
    }

    /// The connection's own `limits` (ADR 0080), in place of any given
    /// before: each only lowers what covers the connection's work. The
    /// connection sets each with `usage.set` once `hello` is answered, and
    /// one the engine refuses closes it and fails the connect.
    #[must_use]
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The connection's own settings (ADR 0083), in place of any given
    /// before: they travel in `hello`, over configuration and the Operator
    /// policy, and every Run the connection starts inherits what its own
    /// settings leave out. A key the engine refuses, such as one a lower
    /// layer locked, fails the connect with the engine's error.
    #[must_use]
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Fails the connect if the engine has not answered `hello` within
    /// `timeout`, with `deadline_exceeded` naming `timeout.connect` at the
    /// run layer, given as an argument (ADR 0083); left out, the connect
    /// waits `timeout.connect`'s 30 s. A zero `timeout` is refused with
    /// `setting_invalid` before the engine starts.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = ConnectTimeout::After(timeout);
        self
    }

    /// Waits for the engine to answer `hello` as long as it takes.
    #[must_use]
    pub fn no_timeout(mut self) -> Self {
        self.timeout = ConnectTimeout::Off;
        self
    }

    /// How many unread events each stream of a Session's events holds
    /// (ADR 0081), in place of 1,024: a stream left unread past it loses
    /// only its own events, hands those it held, then ends with
    /// `listener_events_lost`, while the other streams go on. A bound below
    /// 16 can end even a stream read at once, as events arrive up to 16 at
    /// a time. 0 is refused with `setting_invalid` before the engine
    /// starts.
    #[must_use]
    pub fn max_listener_queue(mut self, bound: usize) -> Self {
        self.max_listener_queue = Some(bound);
        self
    }

    /// The environment the connection reads, the one set or the
    /// process's, the connection's Limits, its `timeout` and its listener
    /// queue, once checked, its engine, and its settings as `hello` carries
    /// them, once checked.
    #[expect(
        clippy::disallowed_methods,
        reason = "the SDK's connect(), given no environment map, reads the process environment once, the one reader besides main (ADR 0083)"
    )]
    fn into_parts(self) -> Result<Parts, Error> {
        self.timeout.check()?;
        let listener_queue = self
            .max_listener_queue
            .map_or(Ok(ListenerQueue::DEFAULT), ListenerQueue::new)?;
        let settings = self.settings.within(SettingScope::Connection)?;
        let environment = self
            .environment
            .unwrap_or_else(|| std::env::vars_os().collect());
        check_environment(&environment).map_err(environment_refused)?;
        Ok(Parts {
            environment,
            asked: Asked {
                limits: self.limits,
                timeout: self.timeout,
                listener_queue,
            },
            engine: self.engine,
            settings,
        })
    }
}

/// The connect's error for a deployment variable `check_environment`
/// refused, as the TypeScript and Python SDKs raise it.
fn environment_refused(error: SettingsError) -> Error {
    let message = error.to_string();
    match error {
        SettingsError::Unknown { key, closest, .. } => {
            let fix = match &closest {
                Some(closest) => format!("Rename {key} to {closest}, or unset it."),
                None => format!(
                    "Unset {key}: the {DEPLOYMENT_PREFIX} prefix is reserved for the deployment variables, {}.",
                    DEPLOYMENT_VARIABLES.join(", ")
                ),
            };
            Error::setting_unknown(
                message,
                &SettingUnknownDetails {
                    key,
                    layer: None,
                    closest,
                },
                fix,
            )
        }
        SettingsError::Invalid { key, reason, .. } => Error::setting_invalid(
            message,
            &SettingInvalidDetails {
                fix: Some(format!(
                    "Set {key} to a non-empty Unicode value, or unset it."
                )),
                key,
                layer: None,
                reason,
                limit: None,
            },
        ),
        other => unreachable!("invariant: check_environment refuses no variable with {other:?}"),
    }
}

/// What a connection opens with, once checked.
struct Parts {
    environment: BTreeMap<OsString, OsString>,
    asked: Asked,
    engine: EngineChoice,
    /// The connection's own settings, nested by key.
    settings: serde_json::Map<String, serde_json::Value>,
}

/// What the program asked of a connection, whatever engine it reaches: its
/// Limits, how long it waits for `hello`'s answer, and how many unread
/// events each stream of a Session's events holds.
#[derive(Debug, Default)]
pub(crate) struct Asked {
    pub(crate) limits: Limits,
    pub(crate) timeout: ConnectTimeout,
    pub(crate) listener_queue: ListenerQueue,
}

/// The engine a connection links: its Product and that Product's
/// composition (ADR 0087).
#[derive(Clone, Copy, Debug)]
pub struct Composed {
    /// The Product the engine is.
    pub product: Product,
    /// What its engine is composed of.
    pub composition: fn() -> Composition,
}

impl Composed {
    /// Collie's engine, of [`crate::composition`].
    pub const COLLIE: Self = Self {
        product: Product::Collie,
        composition: crate::composition,
    };
}

/// A connection to the engine [`ConnectOptions::engine`] chooses: a
/// linked engine of its own, or a Daemon. A Product's connection wraps it.
/// Cloning it is cheap and every clone is the same connection.
///
/// ```
/// use goodfetch_collie::ConnectOptions;
/// use goodfetch_collie::linked::{Composed, ProductConnection};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// let linked = ProductConnection::connect_blocking(Composed::COLLIE, options)?;
/// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
/// let report = linked.connection().start().await?.finish().await?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// linked.close().await?;
/// # Ok::<(), goodfetch_collie::Error>(())
/// # })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct ProductConnection {
    connection: ClientConnection,
    /// The linked engine it started; `None` for a Daemon.
    engine: Option<Arc<LinkedEngine>>,
    handle: Handle,
    info: Arc<EngineInfo>,
}

impl ProductConnection {
    /// Opens a connection to the engine `options` chooses, whose `hello`
    /// needs `engine`'s Product: a linked `engine` of its own, or a Daemon
    /// ([`EngineChoice`]). The connection runs on a runtime it owns, so it
    /// works from any executor, and reads `options`' environment, or the
    /// process's, for its Goodfetch home, a remote Daemon and the
    /// machine's settings. It logs the engine it reached at info, and sets
    /// `options`' Limits on the connection before it returns.
    ///
    /// # Errors
    ///
    /// `engine_exited` when the linked engine cannot start, with no
    /// Goodfetch home among others, `setting_invalid` for a zero `timeout`
    /// ([`ConnectOptions::timeout`]), a zero
    /// [`ConnectOptions::max_listener_queue`] or a remote Daemon's URL that names no
    /// loopback `ws://` address, or `GOODFETCH_URL` without
    /// `GOODFETCH_TOKEN`, `daemon_unreachable` when the Daemon chosen
    /// cannot be reached, `deadline_exceeded` naming
    /// `timeout.connect` when `hello` is not answered within it, what
    /// `hello` fails with, and what `usage.set` fails with for a Limit the
    /// engine refuses, such as
    /// `not_in_product` for a Usage kind its Product does not count; the
    /// connection is closed then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes stops the linked engine it
    /// started, or leaves the Daemon.
    pub async fn connect(engine: Composed, options: ConnectOptions) -> Result<Self, Error> {
        Self::connect_needing(engine, engine.product, options).await
    }

    /// Opens a connection as [`ProductConnection::connect`] does, whose
    /// `hello` needs `needs`: it fails with `not_in_product` when `engine`
    /// does not contain `needs`.
    async fn connect_needing(
        engine: Composed,
        needs: Product,
        options: ConnectOptions,
    ) -> Result<Self, Error> {
        Opening::of(engine, needs, options)?.open().await
    }

    /// Opens a connection as [`ProductConnection::connect`] does, blocking
    /// the calling thread until it is open. It never enters a runtime, so
    /// it works inside one too.
    ///
    /// # Errors
    ///
    /// As [`ProductConnection::connect`].
    pub fn connect_blocking(engine: Composed, options: ConnectOptions) -> Result<Self, Error> {
        let opening = Opening::of(engine, engine.product, options)?;
        let handle = opening.runtime.handle().clone();
        wait(&handle, opening.open())
    }

    /// The client crate's connection, which each Product's connection
    /// drives.
    pub fn connection(&self) -> &ClientConnection {
        &self.connection
    }

    /// The engine the connection reached, fixed when it opened. It never
    /// holds a Daemon's token.
    pub fn engine(&self) -> &EngineInfo {
        &self.info
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and
    /// stops; returns once it has stopped, or once `timeout.close` has run
    /// out. A Daemon is left running. Any later call fails with `closed`. Closing again
    /// does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, `engine_exited` when the linked
    /// engine failed, and `deadline_exceeded` when `timeout.close` ran out
    /// first, the engine then stopping on its own. `connection_forked` in
    /// a process other than the one that opened it, such as the child of
    /// a `fork`, where it closes nothing.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes leaves the engine stopping
    /// on its own.
    pub async fn close(&self) -> Result<(), Error> {
        // In another process, the engine and the timer are the opener's.
        self.connection.ensure_opener()?;
        let closing = async {
            let closed = self.connection.close().await;
            let stopped = match &self.engine {
                Some(engine) => engine.stopped().await,
                None => Ok(()),
            };
            closed.and(stopped)
        };
        // The timer is the engine's runtime's, so the close runs on any
        // executor (ADR 0081).
        let bounded = {
            let _entered = self.handle.enter();
            tokio::time::timeout(close_timeout(), closing)
        };
        bounded.await.unwrap_or_else(|_| Err(Error::timed_out()))
    }

    /// Closes the connection as [`ProductConnection::close`] does, blocking
    /// the calling thread until its linked engine, if any, has stopped.
    ///
    /// # Errors
    ///
    /// As [`ProductConnection::close`].
    pub fn close_blocking(&self) -> Result<(), Error> {
        self.connection.ensure_opener()?;
        let linked = self.clone();
        wait(&self.handle, async move { linked.close().await })
    }

    /// The `hello` of a connection that needs `needs`, carrying the level
    /// the program's subscriber records for the engine on the calling
    /// thread (ADR 0078). The linked engine's own events reach that
    /// subscriber directly, under their own targets, so it sends no
    /// `$/log` and the level changes nothing it does.
    pub(crate) fn hello(needs: Product) -> HelloParams {
        HelloParams {
            needs: Some(needs),
            log_level: Some(engine_log_level()),
            form: Some(goodfetch_protocol::connection::Form::Rust),
            ..HelloParams::new(Surface::Rust, version())
        }
    }

    /// A connection on `runtime` saying `hello`, to a linked `engine` that
    /// runs on `handle`, that runtime's handle, with `dependencies`, and
    /// with what is `asked`: its Limits set on it once `hello` is answered
    /// within its timeout. When `hello` fails, the engine is left to stop
    /// at the end of its input, unwaited; when a Limit is refused, the
    /// connection is closed.
    pub(crate) async fn open(
        runtime: ConnectionRuntime,
        handle: Handle,
        engine: Composed,
        hello: HelloParams,
        dependencies: Dependencies,
        asked: Asked,
    ) -> Result<Self, Error> {
        let home = dependencies.home.root().to_owned();
        let (linked, network) = LinkedEngine::start(&handle, engine, dependencies)?;
        let (connection, said) = ClientConnection::open_within(
            runtime,
            &network,
            &Endpoint::InMemory,
            hello,
            asked.timeout,
            asked.listener_queue,
            trace_context::source(),
        )
        .await?;
        let opened = Self {
            connection,
            engine: Some(Arc::new(linked)),
            handle,
            info: Arc::new(info(EngineKind::Linked, None, said, Some(home))),
        };
        opened.limited(asked.limits).await
    }

    /// A connection on `runtime`, whose handle is `handle`, saying `hello`
    /// with the token of the Daemon `daemon` names, the one of `home`
    /// unless it is remote, and with what is `asked`: its Limits set on it
    /// once `hello` is answered within its timeout.
    async fn open_daemon(
        runtime: ConnectionRuntime,
        handle: Handle,
        daemon: DaemonReach,
        home: Option<Home>,
        hello: HelloParams,
        asked: Asked,
    ) -> Result<Self, Error> {
        let hello = HelloParams {
            token: Some(daemon.token.expose().to_owned()),
            ..hello
        };
        let endpoint = Endpoint::Daemon(daemon.endpoint.clone());
        let (connection, said) = ClientConnection::open_within(
            runtime,
            &Network::Os,
            &endpoint,
            hello,
            asked.timeout,
            asked.listener_queue,
            trace_context::source(),
        )
        .await
        .map_err(|error| daemon.unreached(error, home.as_ref()))?;
        let opened = Self {
            connection,
            engine: None,
            handle,
            info: Arc::new(info(
                daemon.kind(),
                Some(daemon.url()),
                said,
                daemon.home(home.as_ref()),
            )),
        };
        opened.limited(asked.limits).await
    }

    /// The connection, opened, once it logged the engine it reached and
    /// set each of `limits`; when a Limit is refused, it is closed.
    async fn limited(self, limits: Limits) -> Result<Self, Error> {
        tracing::info!(
            target: CLIENT,
            kind = %self.info.kind,
            url = self.info.url,
            "connect() reached its engine"
        );
        if let Err(refused) = self.set(limits).await {
            if let Err(error) = self.close().await {
                tracing::warn!(
                    target: "goodfetch::collie",
                    %error,
                    "closing a connection whose Limit was refused failed"
                );
            }
            return Err(refused);
        }
        Ok(self)
    }

    /// Sets each of `limits` on the connection, one `usage.set` on its own
    /// scope with no period (ADR 0080), and stops at the first refused.
    async fn set(&self, limits: Limits) -> Result<(), Error> {
        for limit in Vec::from(limits) {
            let params = UsageSetParams {
                scope: usage::CONNECTION.to_owned(),
                scope_id: None,
                usage_kind: limit.usage_kind,
                currency: limit.currency,
                period: None,
                amount: limit.amount,
            };
            let _set: LimitUsage = self.connection.call(USAGE_SET.name, &params).await?;
        }
        Ok(())
    }
}

/// C-SEND-SYNC: a connection moves between threads, is shared between
/// them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<ProductConnection>();
    send_sync_static::<ConnectOptions>();
};

/// The Product version this crate is, which its linked engine and its
/// connection both speak.
fn version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version")
}

/// The Goodfetch home: `GOODFETCH_HOME` when it is set, the `goodfetch`
/// folder of the OS's usual data folder otherwise.
pub(crate) fn home(environment: &BTreeMap<OsString, OsString>) -> Option<Home> {
    let home = match deployment_variable(environment, HOME_VARIABLE) {
        Some(home) => Home::new(home.into()),
        None => Home::new(dirs::data_dir()?.join(goodfetch_protocol::home::HOME_FOLDER)),
    };
    Some(home.with_browsers_path(
        deployment_variable(environment, BROWSERS_PATH_VARIABLE).map(Into::into),
    ))
}

/// What `engine()` reports of an engine of `kind`, reached at `url`, which
/// answered `hello` with `said` and runs in `home`.
fn info(
    kind: EngineKind,
    url: Option<String>,
    said: HelloResult,
    home: Option<PathBuf>,
) -> EngineInfo {
    EngineInfo {
        kind,
        product: said.product,
        version: said.product_version,
        protocol_version: said.protocol_version,
        project_file: None,
        home,
        engine_path: None,
        url,
        fallback: None,
    }
}

/// A connection whose engine is chosen, ready to open: the runtime it runs
/// on, its `hello`, how its engine is reached, and what the program asked
/// of it.
struct Opening {
    runtime: Runtime,
    engine: Composed,
    hello: HelloParams,
    target: Target,
    asked: Asked,
}

/// How an opening connection's engine is reached.
enum Target {
    /// The linked engine it starts, with what it runs on.
    Linked(Box<Dependencies>),
    /// A Daemon, the one of `home` unless it is remote.
    Daemon {
        daemon: DaemonReach,
        home: Option<Home>,
    },
}

impl Opening {
    /// The connection `options` open, whose `hello` needs `needs`, to the
    /// engine they choose, `engine` when it is the linked one.
    fn of(engine: Composed, needs: Product, options: ConnectOptions) -> Result<Self, Error> {
        let Parts {
            environment,
            asked,
            engine: choice,
            settings,
        } = options.into_parts()?;
        let hello = HelloParams {
            settings,
            ..ProductConnection::hello(needs)
        };
        let found = home(&environment);
        let reach = engine_choice::reach(
            choice,
            &environment,
            found.as_ref(),
            engine.product,
            &version(),
        )?;
        let (runtime, target) = match reach {
            Reach::Linked => {
                let (runtime, dependencies) = production(engine.product, environment)?;
                (runtime, Target::Linked(Box::new(dependencies)))
            }
            Reach::Daemon(daemon) => (
                runtime::multi_thread(Threads::of(None)),
                Target::Daemon {
                    daemon,
                    home: found,
                },
            ),
        };
        Ok(Self {
            runtime,
            engine,
            hello,
            target,
            asked,
        })
    }

    /// Opens the connection on its runtime.
    async fn open(self) -> Result<ProductConnection, Error> {
        let handle = self.runtime.handle().clone();
        let runtime = ConnectionRuntime::owned(self.runtime);
        match self.target {
            Target::Linked(dependencies) => {
                ProductConnection::open(
                    runtime,
                    handle,
                    self.engine,
                    self.hello,
                    *dependencies,
                    self.asked,
                )
                .await
            }
            Target::Daemon { daemon, home } => {
                ProductConnection::open_daemon(
                    runtime, handle, daemon, home, self.hello, self.asked,
                )
                .await
            }
        }
    }
}

/// The runtime a production linked engine of `product` and its connection
/// run on, sized by the machine's settings as the executable's are, and
/// what the engine runs on, from `environment`.
fn production(
    product: Product,
    environment: BTreeMap<OsString, OsString>,
) -> Result<(Runtime, Dependencies), Error> {
    let home = home(&environment).ok_or_else(|| {
        Error::engine_failed("no Goodfetch home: set GOODFETCH_HOME, or a home folder the OS knows")
    })?;
    let entropy = Entropy::from_os().map_err(Error::engine_failed)?;
    let settings = SettingsFiles::of(&home, &version()).machine_settings(product);
    // Built last, as nothing after it fails: dropping a runtime inside
    // another one panics.
    let runtime = runtime::multi_thread(Threads::of(settings.as_ref().ok()));
    let dependencies = Dependencies {
        environment,
        keychain: Keychain::os(),
        home,
        clock: Clock::system(),
        entropy,
        blocking: BlockingPolicy::Threads,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: PolitenessOptions::default(),
    };
    Ok((runtime, dependencies))
}

/// Runs `future` on `handle`'s runtime and blocks the calling thread until
/// it ends.
fn wait<T: Send + 'static>(
    handle: &Handle,
    future: impl Future<Output = Result<T, Error>> + Send + 'static,
) -> Result<T, Error> {
    let tasks = TaskTracker::new();
    let (done, ended) = mpsc::sync_channel(1);
    tasks.spawn_on(
        async move {
            let _gone = done.send(future.await);
        },
        handle,
    );
    tasks.close();
    // The task ends without sending only when the runtime shut down.
    ended.recv().unwrap_or_else(|_| Err(Error::closed()))
}

/// A linked engine serving one Connection on its runtime.
#[derive(Debug)]
struct LinkedEngine {
    tasks: TaskTracker,
    failure: Arc<OnceLock<Arc<AttachedFailure>>>,
}

impl LinkedEngine {
    /// Starts `engine` on `handle`'s runtime with `dependencies`, and the
    /// network its Connection reaches it over.
    fn start(
        handle: &Handle,
        engine: Composed,
        dependencies: Dependencies,
    ) -> Result<(Self, Network), Error> {
        let identity = Identity {
            product: engine.product,
            product_version: version(),
            channel: Channel::CratesIo,
        };
        let engine =
            Engine::new(identity, &(engine.composition)()).map_err(Error::engine_failed)?;
        let (network, dialer) = transport::in_memory();
        let tasks = TaskTracker::new();
        let failure = Arc::new(OnceLock::new());
        let failed = Arc::clone(&failure);
        tasks.spawn_on(
            async move {
                let served = engine
                    .run_attached(transport::Network::InMemory(network), dependencies)
                    .await;
                if let Err(error) = served {
                    tracing::warn!(target: "goodfetch::collie", %error, "the linked engine failed");
                    let _first = failed.set(Arc::new(error));
                }
            },
            handle,
        );
        tasks.close();
        let linked = Self { tasks, failure };
        Ok((linked, Network::in_memory(InMemory(dialer))))
    }

    /// Waits for the engine to stop, which it does once its Connection is
    /// closed.
    ///
    /// # Errors
    ///
    /// `engine_exited` when the engine failed.
    async fn stopped(&self) -> Result<(), Error> {
        self.tasks.wait().await;
        match self.failure.get() {
            Some(failure) => Err(Error::engine_failed(Arc::clone(failure))),
            None => Ok(()),
        }
    }
}

/// How long a close waits: `timeout.close`'s default (ADR 0078).
fn close_timeout() -> Duration {
    let millis = (TIMEOUT_CLOSE.default)()
        .as_u64()
        .expect("invariant: `timeout.close` defaults to a duration in milliseconds");
    Duration::from_millis(millis)
}

#[cfg(test)]
mod tests;

/// The engine's in-memory dialer as the client crate dials. The linked
/// engine serves one Connection and stops when it ends, so the engine
/// ending the Connection is the engine ending: each call it leaves
/// unanswered fails with `engine_exited`, whose details hold no exit code,
/// signal or stderr, the engine being the program's own process (ADR
/// 0078).
struct InMemory(InMemoryDialer);

impl Dial for InMemory {
    fn dial(&self) -> Pin<Box<dyn Future<Output = std::io::Result<Link>> + Send + '_>> {
        Box::pin(async {
            let (reader, writer) = self
                .0
                .connect()
                .await
                .map_err(std::io::Error::other)?
                .split();
            let ended = std::future::ready(EngineExitedDetails::without_evidence());
            Ok(Link::new(reader, writer).ending_with(ended))
        })
    }
}
