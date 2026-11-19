//! The linked engine (ADRs 0078, 0081 and 0087): the engine a Product's
//! Rust SDK composes, which each connection starts in the program's own
//! process and reaches over area 3's in-memory network. It serves its one
//! Connection as an Attached engine does, and stops at the end of its
//! input, which closing the Connection gives it.
//!
//! [`LinkedConnection`] is the one way both Products' SDKs open such a
//! connection: [`Collie`](crate::Collie) with [`Composed::COLLIE`], and
//! Foxhound's crate with its own composition, which starts from Collie's
//! (ADR 0087). It names no Product's entries, so Foxhound's stay out of
//! this crate's API (ADR 0095).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, mpsc};

use goodfetch_collie_client::transport::{Dial, Endpoint, Link, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, Limits};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::Keychain;
use goodfetch_engine::settings::{SettingsFiles, deployment_variable};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, InMemoryDialer};
use goodfetch_engine::{AttachedFailure, Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_protocol::settings::{BROWSERS_PATH_VARIABLE, HOME_VARIABLE};
use goodfetch_protocol::usage::{self, LimitUsage, USAGE_SET, UsageSetParams};
use tokio::runtime::{Handle, Runtime};
use tokio_util::task::TaskTracker;

use crate::runtime::{self, Threads};

/// What a connection opens with, in either Product's SDK.
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    environment: Option<BTreeMap<OsString, OsString>>,
    limits: Limits,
}

impl ConnectOptions {
    /// The environment variables the linked engine reads, such as
    /// `GOODFETCH_HOME`, in place of the process's own.
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

    /// The environment the linked engine reads, the one set or the
    /// process's, and the connection's Limits.
    #[expect(
        clippy::disallowed_methods,
        reason = "the SDK's connect(), given no environment map, reads the process environment once, the one reader besides main (ADR 0083)"
    )]
    fn into_parts(self) -> (BTreeMap<OsString, OsString>, Limits) {
        let environment = self
            .environment
            .unwrap_or_else(|| std::env::vars_os().collect());
        (environment, self.limits)
    }
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

/// A connection to a linked engine of its own, which a Product's
/// connection wraps. Cloning it is cheap and every clone is the same
/// connection.
///
/// ```
/// use goodfetch_collie::ConnectOptions;
/// use goodfetch_collie::linked::{Composed, LinkedConnection};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// let linked = LinkedConnection::connect_blocking(Composed::COLLIE, options)?;
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
pub struct LinkedConnection {
    connection: ClientConnection,
    engine: Arc<LinkedEngine>,
    handle: Handle,
}

impl LinkedConnection {
    /// Opens a connection to a linked `engine` of its own, whose `hello`
    /// needs that engine's Product. The engine runs on a runtime the
    /// connection owns, so the connection works from any executor, and
    /// reads `options`' environment, or the process's, for its Goodfetch
    /// home and the machine's settings. It sets `options`' Limits on the
    /// connection before it returns.
    ///
    /// # Errors
    ///
    /// [`Error::Engine`] when the linked engine cannot start, with no
    /// Goodfetch home among others, what `hello` fails with, and what
    /// `usage.set` fails with for a Limit the engine refuses, such as
    /// `not_in_product` for a Usage kind its Product does not count; the
    /// connection is closed then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes stops the linked engine it
    /// started.
    pub async fn connect(engine: Composed, options: ConnectOptions) -> Result<Self, Error> {
        Self::connect_needing(engine, engine.product, options).await
    }

    /// Opens a connection as [`LinkedConnection::connect`] does, whose
    /// `hello` needs `needs`: it fails with `not_in_product` when `engine`
    /// does not contain `needs`.
    async fn connect_needing(
        engine: Composed,
        needs: Product,
        options: ConnectOptions,
    ) -> Result<Self, Error> {
        let (environment, limits) = options.into_parts();
        let (runtime, dependencies) = production(engine.product, environment)?;
        let handle = runtime.handle().clone();
        Self::open(
            ConnectionRuntime::owned(runtime),
            handle,
            engine,
            needs,
            dependencies,
            limits,
        )
        .await
    }

    /// Opens a connection as [`LinkedConnection::connect`] does, blocking
    /// the calling thread until it is open. It never enters a runtime, so
    /// it works inside one too.
    ///
    /// # Errors
    ///
    /// As [`LinkedConnection::connect`].
    pub fn connect_blocking(engine: Composed, options: ConnectOptions) -> Result<Self, Error> {
        let (environment, limits) = options.into_parts();
        let (runtime, dependencies) = production(engine.product, environment)?;
        let handle = runtime.handle().clone();
        let on = handle.clone();
        wait(&handle, async move {
            Self::open(
                ConnectionRuntime::owned(runtime),
                on,
                engine,
                engine.product,
                dependencies,
                limits,
            )
            .await
        })
    }

    /// The client crate's connection, which each Product's connection
    /// drives.
    pub fn connection(&self) -> &ClientConnection {
        &self.connection
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and
    /// stops; returns once it has stopped. Any later call fails with
    /// [`Error::Closed`]. Closing again does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, and [`Error::Engine`] when the linked
    /// engine failed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes leaves the engine stopping
    /// on its own.
    pub async fn close(&self) -> Result<(), Error> {
        let closed = self.connection.close().await;
        let stopped = self.engine.stopped().await;
        closed.and(stopped)
    }

    /// Closes the connection as [`LinkedConnection::close`] does, blocking
    /// the calling thread until its linked engine has stopped.
    ///
    /// # Errors
    ///
    /// As [`LinkedConnection::close`].
    pub fn close_blocking(&self) -> Result<(), Error> {
        let linked = self.clone();
        wait(&self.handle, async move { linked.close().await })
    }

    /// A connection on `runtime` whose `hello` needs `needs`, to a linked
    /// `engine` that runs on `handle`, that runtime's handle, with
    /// `dependencies`, and with `limits` set on it. When `hello` fails, the
    /// engine is left to stop at the end of its input, unwaited; when a
    /// Limit is refused, the connection is closed.
    pub(crate) async fn open(
        runtime: ConnectionRuntime,
        handle: Handle,
        engine: Composed,
        needs: Product,
        dependencies: Dependencies,
        limits: Limits,
    ) -> Result<Self, Error> {
        let (linked, network) = LinkedEngine::start(&handle, engine, dependencies)?;
        let hello = HelloParams {
            needs: Some(needs),
            ..HelloParams::new(Surface::Rust, version())
        };
        let (connection, _said) =
            ClientConnection::open(runtime, &network, &Endpoint::InMemory, hello).await?;
        let opened = Self {
            connection,
            engine: Arc::new(linked),
            handle,
        };
        if let Err(refused) = opened.set(limits).await {
            if let Err(error) = opened.close().await {
                tracing::warn!(
                    target: "goodfetch::collie",
                    %error,
                    "closing a connection whose Limit was refused failed"
                );
            }
            return Err(refused);
        }
        Ok(opened)
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
    send_sync_static::<LinkedConnection>();
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

/// The runtime a production linked engine of `product` and its connection
/// run on, sized by the machine's settings as the executable's are, and
/// what the engine runs on, from `environment`.
fn production(
    product: Product,
    environment: BTreeMap<OsString, OsString>,
) -> Result<(Runtime, Dependencies), Error> {
    let home = home(&environment).ok_or_else(|| {
        Error::Engine("no Goodfetch home: set GOODFETCH_HOME, or a home folder the OS knows".into())
    })?;
    let entropy = Entropy::from_os().map_err(|error| Error::Engine(Box::new(error)))?;
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
    ended.recv().unwrap_or(Err(Error::Closed))
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
        let engine = Engine::new(identity, &(engine.composition)())
            .map_err(|error| Error::Engine(Box::new(error)))?;
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
    /// [`Error::Engine`] when the engine failed.
    async fn stopped(&self) -> Result<(), Error> {
        self.tasks.wait().await;
        match self.failure.get() {
            Some(failure) => Err(Error::Engine(Box::new(Arc::clone(failure)))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;

/// The engine's in-memory dialer as the client crate dials.
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
            Ok(Link::new(reader, writer))
        })
    }
}
