//! The linked engine (ADRs 0078, 0081 and 0087): the Collie engine this
//! crate composes, which each connection starts in the program's own
//! process and reaches over area 3's in-memory network. It serves its one
//! Connection as an Attached engine does, and stops at the end of its
//! input, which closing the Connection gives it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use goodfetch_collie_client::transport::{Dial, Endpoint, Link, Network};
use goodfetch_collie_client::{Connection, ConnectionRuntime, Error};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::Keychain;
use goodfetch_engine::settings::{SettingsFiles, deployment_variable};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, InMemoryDialer};
use goodfetch_engine::{AttachedFailure, Channel, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_protocol::settings::{BROWSERS_PATH_VARIABLE, HOME_VARIABLE};
use tokio::runtime::{Handle, Runtime};
use tokio_util::task::TaskTracker;

use crate::runtime::{self, Threads};

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

/// The runtime a production linked engine and its connection run on,
/// sized by the machine's settings as the executable's are, and what the
/// engine runs on, from `environment`.
pub(crate) fn production(
    environment: BTreeMap<OsString, OsString>,
) -> Result<(Runtime, Dependencies), Error> {
    let home = home(&environment).ok_or_else(|| {
        Error::Engine("no Goodfetch home: set GOODFETCH_HOME, or a home folder the OS knows".into())
    })?;
    let entropy = Entropy::from_os().map_err(|error| Error::Engine(Box::new(error)))?;
    let settings = SettingsFiles::of(&home, &version()).machine_settings(Product::Collie);
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

/// A linked engine serving one Connection on its runtime.
#[derive(Debug)]
pub(crate) struct LinkedEngine {
    tasks: TaskTracker,
    failure: Arc<OnceLock<Arc<AttachedFailure>>>,
}

impl LinkedEngine {
    /// Starts a Collie engine on `handle`'s runtime with `dependencies`,
    /// and the network its Connection reaches it over.
    pub(crate) fn start(
        handle: &Handle,
        dependencies: Dependencies,
    ) -> Result<(Self, Network), Error> {
        let identity = Identity {
            product: Product::Collie,
            product_version: version(),
            channel: Channel::CratesIo,
        };
        let engine = Engine::new(identity, &crate::composition())
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
    pub(crate) async fn stopped(&self) -> Result<(), Error> {
        self.tasks.wait().await;
        match self.failure.get() {
            Some(failure) => Err(Error::Engine(Box::new(Arc::clone(failure)))),
            None => Ok(()),
        }
    }
}

/// Opens the Connection to the linked engine over `network`, on `runtime`.
pub(crate) async fn open(
    runtime: ConnectionRuntime,
    network: &Network,
) -> Result<Connection, Error> {
    let hello = HelloParams {
        needs: Some(Product::Collie),
        ..HelloParams::new(Surface::Rust, version())
    };
    let (connection, _said) =
        Connection::open(runtime, network, &Endpoint::InMemory, hello).await?;
    Ok(connection)
}

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
