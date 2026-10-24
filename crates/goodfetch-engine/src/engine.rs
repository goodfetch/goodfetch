//! The engine: an identity and a composition, checked once at start, which
//! serves Connections from the values it is given (ADR 0059): the Goodfetch
//! home, the `Clock`, the `Entropy`, the blocking policy and where run state
//! goes, with its event stream's buffers. An Attached engine serves the one Connection its network gives it
//! and, on the OS, its private endpoint (the `attached` module), and stops
//! when that Connection ends or its parent is gone, its live Runs stopped
//! as aborted by their Client (ADRs 0024 and 0078). A Daemon serves every Connection its
//! listener gives it until it stops: idle, asked to, or handing over to its
//! major's pick (the `daemon` module).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::connection::PrivateEndpoint;
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::governor::{SYSTEM_INFO, SystemInfo};
use goodfetch_protocol::run::{RunKind, RunOwner};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::admission::Gates;
use crate::attached::{ParentWatch, PrivateFiles, relay};
use crate::clock::Clock;
use crate::composition::{Composition, CompositionError, ConsoleBuild};
use crate::console;
use crate::daemon::{self, DaemonConsole, DaemonEnd, DaemonOptions};
use crate::entropy::Entropy;
use crate::events::{Events, EventsOptions};
use crate::forward::ToOwner;
use crate::governor::{Governor, System};
use crate::home::{Home, RunStateLocation, major_name};
use crate::log::LogEvents;
use crate::metrics::Metrics;
use crate::needs_you::{NeedsYou, NeedsYouSource, SourceContext};
use crate::people::{AccountsStore, ConsoleKey, SignInDriver, SignIns, SingleUserLink, os_user};
use crate::politeness::{PolitenessOptions, PolitenessTables};
use crate::product::Identity;
use crate::rpc::{self, Access, Dispatch};
use crate::run_state::install::InstallDatabase;
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::runs::{CleanUpVersions, Driver, ProgressPart, ReportPart, Runs, RunsSetup};
use crate::secrets::{self, Keychain};
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transition::{
    self,
    engine::{EngineEvent, EngineState},
};
use crate::transport::{Connection, DaemonListener, Network, TransportError};

/// How many Connections the Console's server opened wait for the Daemon to
/// serve them before it waits too.
const CONSOLE_CONNECTIONS: usize = 16;

/// An engine of one Product, composed and ready to serve.
#[derive(Clone)]
pub struct Engine {
    dispatch: Arc<Dispatch>,
    tables: Vec<RunStateTables>,
    politeness_tables: Vec<PolitenessTables>,
    drivers: BTreeMap<RunKind, Driver>,
    report_parts: Vec<Arc<dyn ReportPart>>,
    progress_parts: Vec<Arc<dyn ProgressPart>>,
    clean_up: Option<Arc<dyn CleanUpVersions>>,
    gates: Option<Arc<dyn Gates>>,
    needs_you_sources: Vec<NeedsYouSource>,
    events: EventsOptions,
    console: Option<&'static ConsoleBuild>,
    metrics: Metrics,
    system: Option<System>,
}

/// What an engine runs on, which `main` builds once (the system versions)
/// and a test builds for itself (ADR 0059).
pub struct Dependencies {
    /// One user's Goodfetch data folder.
    pub home: Home,
    /// Where time is read.
    pub clock: Clock,
    /// Where random choices are drawn.
    pub entropy: Entropy,
    /// Where blocking work runs.
    pub blocking: BlockingPolicy,
    /// Where the run folders are (`run_state.location`).
    pub run_state_location: RunStateLocation,
    /// How run files open.
    pub run_state: RunStateOptions,
    /// How the per-user Politeness file opens.
    pub politeness: PolitenessOptions,
    /// The engine's environment map, which `main` reads once (ADR 0083)
    /// and `env:` Secret references resolve in.
    pub environment: BTreeMap<OsString, OsString>,
    /// The OS keychain `keychain:` Secret references resolve in, whose
    /// absence makes the encrypted local file `run_state.key_reference`'s
    /// default: `Keychain::os()` in `main`, absent in most tests.
    pub keychain: Keychain,
}

impl Engine {
    /// The engine `composition` makes for the Product `identity` names.
    pub fn new(identity: Identity, composition: &Composition) -> Result<Self, CompositionError> {
        Ok(Self {
            dispatch: Arc::new(Dispatch {
                identity: Arc::new(identity),
                handlers: composition.handlers()?,
                log_events: None,
            }),
            tables: composition.run_state_tables(),
            politeness_tables: composition.politeness_tables(),
            drivers: composition.drivers(),
            report_parts: composition.report_parts(),
            progress_parts: composition.progress_parts(),
            clean_up: composition.clean_up_versions()?,
            gates: composition.admission_gates()?,
            needs_you_sources: composition.needs_you_sources(),
            events: EventsOptions::default(),
            console: composition.console(),
            metrics: Metrics::none(),
            system: None,
        })
    }

    /// The same, recording the runtime's metrics on `metrics` (ADR 0050).
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// The same, its Resource governor sampling `system` (ADR 0040), as
    /// each Product's executable reads the OS it runs on; without it, the
    /// governor holds each kind of work at its default ceiling.
    #[must_use]
    pub fn with_governor(mut self, system: System) -> Self {
        self.system = Some(system);
        self
    }

    /// The same, each of its Connections receiving the log events of
    /// `log_events`, whose layer the process's `tracing` subscriber holds,
    /// at the level the Connection asks for.
    #[must_use]
    pub fn with_log_events(mut self, log_events: LogEvents) -> Self {
        self.dispatch = Arc::new(Dispatch {
            identity: Arc::clone(&self.dispatch.identity),
            handlers: self.dispatch.handlers.clone(),
            log_events: Some(log_events),
        });
        self
    }

    /// The same, its event stream's buffers sized by `options`.
    #[must_use]
    pub fn with_events(mut self, options: EventsOptions) -> Self {
        self.events = options;
        self
    }

    /// What it runs as.
    pub fn identity(&self) -> &Identity {
        &self.dispatch.identity
    }

    /// Whether its composition carries a Console build, which its Daemon
    /// serves; an SDK crate's linked engine carries none (ADR 0031).
    pub fn serves_console(&self) -> bool {
        self.console.is_some()
    }

    /// Serves `connection` until it ends, its tasks those of `owner`, its
    /// calls reaching the Runs of `runs` and the event stream `events`, on
    /// `clock`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves its calls to `owner`, which stops them.
    pub async fn serve(
        &self,
        connection: Connection,
        owner: &TaskOwner,
        runs: Runs,
        events: Events,
        clock: Clock,
    ) {
        rpc::serve(
            Arc::clone(&self.dispatch),
            connection,
            owner,
            runs,
            events,
            Access::Attached(None),
            clock,
        )
        .await;
    }

    /// Runs as a Daemon on `dependencies`: serves every Connection
    /// `listener` gives it, each Client's Runs its own, until it stops as
    /// [`DaemonEnd`] says; then stops its live Runs and every task it
    /// started.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_daemon(
        &self,
        listener: DaemonListener,
        dependencies: Dependencies,
        mut options: DaemonOptions,
    ) -> DaemonEnd {
        let owner = TaskOwner::root(dependencies.blocking);
        let clock = dependencies.clock;
        let home = dependencies.home.clone();
        let person = os_user(&dependencies.environment);
        let console_url = match (&self.console, &options.console) {
            (Some(_), DaemonConsole::Listening { url, .. }) => Some(url.clone()),
            _ => None,
        };
        let (runs, events) = self.start_runs(
            dependencies,
            &owner,
            Some(RunOwner::Daemon(options.endpoint.clone())),
            options.install.take(),
            console_url,
        );
        let (opened_connections, console_connections) =
            tokio::sync::mpsc::channel(CONSOLE_CONNECTIONS);
        let (console, sign_ins) = match (
            self.console,
            std::mem::replace(&mut options.console, DaemonConsole::Absent),
        ) {
            (
                Some(build),
                DaemonConsole::Listening {
                    listener,
                    url,
                    sign_in,
                    content,
                },
            ) => {
                let sign_ins = SignInDriver::new(SignIns {
                    link: SingleUserLink::new(options.token.clone()),
                    person,
                    store: AccountsStore::at(home.accounts_store(&self.identity().product_version)),
                    key: ConsoleKey::at(home.console_key()),
                    audit: runs.audit().clone(),
                    policy: sign_in,
                    clock,
                    owner: Arc::new(owner.child()),
                });
                owner.spawn(console::serve(
                    console::Served {
                        build,
                        listener,
                        sign_ins: sign_ins.clone(),
                        cookie: format!(
                            "goodfetch_console_{}",
                            major_name(&self.identity().product_version)
                        ),
                        connections: opened_connections,
                        runs: runs.clone(),
                        clock,
                        content,
                    },
                    owner.child(),
                ));
                (ConsoleState::Serving(url), Some(sign_ins))
            }
            (_, DaemonConsole::Failed(failure)) => (ConsoleState::Failed(failure), None),
            (None, DaemonConsole::Listening { .. }) | (_, DaemonConsole::Absent) => {
                (ConsoleState::Absent, None)
            }
        };
        let ended = daemon::serve(
            daemon::Serving {
                dispatch: Arc::clone(&self.dispatch),
                runs: runs.clone(),
                events,
                clock,
                console,
                console_connections,
                metrics: self.metrics.clone(),
                sign_ins,
            },
            listener,
            options,
            &owner,
        )
        .await;
        if matches!(ended, DaemonEnd::Drained) {
            // Its process exits now, within its drain grace: a Run whose
            // tasks did not stop in time stays as it is, resumable from its
            // run file, and nothing waits for them.
            drop(runs);
            owner.cancel();
        } else {
            runs.stop_all().await;
            drop(runs);
            owner.stop().await;
        }
        ended
    }

    /// The engine's event stream and Runs registry, tasks of `owner`; the
    /// Run index names `reached_at` as the owner of its live Runs, whose
    /// run state is in `install` in the per-install layout.
    fn start_runs(
        &self,
        mut dependencies: Dependencies,
        owner: &TaskOwner,
        reached_at: Option<RunOwner>,
        install: Option<InstallDatabase>,
        console: Option<String>,
    ) -> (Runs, Events) {
        let instance = EngineInstance::new(format!("{:016x}", dependencies.entropy.next_u64()));
        let events = Events::start(self.events, instance, owner);
        let keychain = dependencies.keychain.clone();
        let settings = Settings::with_host(
            SettingsFiles::of(&dependencies.home, &self.identity().product_version),
            self.identity().product,
            owner.child(),
            move || {
                let keychain = keychain.clone();
                Box::pin(async move { secrets::host_defaults(&keychain).await })
            },
        );
        let governor = match &self.system {
            Some(system) => {
                let governor =
                    Governor::start(system.clone(), settings.clone(), dependencies.clock, owner);
                owner.spawn(report_samples(
                    governor.system_info(),
                    events.clone(),
                    self.metrics.clone(),
                    owner.token().clone(),
                ));
                governor
            }
            None => Governor::unsampled(),
        };
        let needs_you = NeedsYou::start(
            &self.needs_you_sources,
            SourceContext {
                clock: dependencies.clock,
                storage: governor.storage(),
                run_index: dependencies
                    .home
                    .run_index(&self.identity().product_version),
                version: self.identity().product_version.clone(),
                settings: settings.clone(),
                owner: Arc::new(owner.child()),
            },
            events.clone(),
            owner,
        );
        let forwarder = Arc::new(ToOwner {
            home: dependencies.home.clone(),
            version: self.identity().product_version.clone(),
            clock: dependencies.clock,
        });
        let runs = Runs::start_registry(
            RunsSetup {
                keychain: dependencies.keychain,
                settings,
                product: self.identity().product,
                reached_at,
                forwarder,
                home: dependencies.home,
                version: self.identity().product_version.clone(),
                location: dependencies.run_state_location,
                ledger_vfs: dependencies.run_state.vfs.clone(),
                run_state: match &install {
                    Some(install) => RunStateOptions {
                        vfs: Some(install.vfs().to_owned()),
                        ..dependencies.run_state
                    },
                    None => dependencies.run_state,
                },
                install: install.map(Arc::new),
                tables: self.tables.clone(),
                drivers: self.drivers.clone(),
                report_parts: self.report_parts.clone(),
                progress_parts: self.progress_parts.clone(),
                clean_up: self.clean_up.clone(),
                console,
                gates: self.gates.clone(),
                politeness: dependencies.politeness,
                politeness_tables: self.politeness_tables.clone(),
                clock: dependencies.clock,
                entropy: dependencies.entropy,
                events: events.clone(),
                metrics: self.metrics.clone(),
                governor,
                environment: dependencies.environment,
                needs_you,
            },
            owner,
        );
        (runs, events)
    }

    /// Runs as an Attached engine in the per-install layout, which hands
    /// its Runs to the Daemon (`run_state.layout`, ADR 0002): relays the
    /// one Connection `network` gives it to `daemon`, the Daemon of its
    /// major reached with `token`, which it adds to the Client's `hello`,
    /// until either ends or, on the OS, its parent is gone.
    ///
    /// # Errors
    ///
    /// When `network` gives no Connection.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes both Connections.
    pub async fn relay_attached(
        mut network: Network,
        daemon: Connection,
        token: String,
    ) -> Result<(), TransportError> {
        let parent = matches!(network, Network::Os).then(ParentWatch::start);
        let client = network.attached_connection().await?;
        let gone = async {
            match parent {
                Some(parent) => parent.gone().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = relay(client, daemon, &token) => {}
            () = gone => tracing::warn!("the Attached engine's parent is gone: its Runs on the Daemon stop"),
        }
        Ok(())
    }

    /// Runs as an Attached engine on `dependencies`: serves the one
    /// Connection `network` gives it and, on the OS, its private endpoint,
    /// whose every Connection must say `hello` with its token (ADRs 0024
    /// and 0031); stops its live Runs once that one Connection ends or, on
    /// the OS, once its parent is gone, then stops every task it started
    /// and removes its private endpoint.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_attached(
        &self,
        mut network: Network,
        dependencies: Dependencies,
    ) -> Result<(), TransportError> {
        let on_the_os = matches!(network, Network::Os);
        let parent = on_the_os.then(ParentWatch::start);
        let connection = network.attached_connection().await?;
        let owner = TaskOwner::root(dependencies.blocking);
        let clock = dependencies.clock;
        let private = if on_the_os {
            private_endpoint(network, &owner).await
        } else {
            None
        };
        let (runs, events) = self.start_runs(
            dependencies,
            &owner,
            private
                .as_ref()
                .map(|(_, endpoint, _)| RunOwner::Attached(endpoint.clone())),
            None,
            None,
        );
        let connection_owner = owner.child();
        let reported = private.as_ref().map(|(_, endpoint, _)| endpoint.clone());
        let stdio = rpc::serve(
            Arc::clone(&self.dispatch),
            connection,
            &connection_owner,
            runs.clone(),
            events.clone(),
            Access::Attached(reported),
            clock,
        );
        let gone = async {
            match parent {
                Some(parent) => parent.gone().await,
                None => std::future::pending().await,
            }
        };
        let accepting = owner.child();
        let mut private_files = None;
        if let Some((listener, _, files)) = private {
            accepting.spawn(serve_private_endpoint(
                Arc::clone(&self.dispatch),
                listener,
                Arc::from(files.token()),
                accepting.child(),
                runs.clone(),
                events,
                clock,
            ));
            private_files = Some(files);
        }
        let ended = tokio::select! {
            () = stdio => EngineEvent::StdioEnded,
            () = gone => {
                tracing::warn!("the Attached engine's parent is gone: its Runs stop");
                EngineEvent::ParentGone
            }
        };
        // Either end stops it, as the engine's table says (ADR 0128).
        if let Err(refused) = transition::engine::engine(EngineState::Attached, ended) {
            let _ = refused.logged();
        }
        accepting.stop().await;
        runs.stop_all().await;
        drop(runs);
        connection_owner.stop().await;
        owner.stop().await;
        drop(private_files);
        Ok(())
    }
}

/// Sends each of the Resource governor's samples as `engine.system_info`
/// to the subscriptions that take it, and records it in the runtime's
/// metrics, until `token` is cancelled.
async fn report_samples(
    mut samples: watch::Receiver<Option<SystemInfo>>,
    events: Events,
    metrics: Metrics,
    token: CancellationToken,
) {
    loop {
        tokio::select! {
            () = token.cancelled() => return,
            changed = samples.changed() => if changed.is_err() {
                return;
            },
        }
        let sample = samples.borrow_and_update().clone();
        if let Some(sample) = sample {
            metrics.governor(&sample);
            events.publish(&SYSTEM_INFO, None, &sample).await;
        }
    }
}

/// Opens an Attached engine's private endpoint on `network`: its listener,
/// what the Client is told of it, and its files. `None`, logged, when the
/// OS refuses it; the engine then serves its stdio alone.
async fn private_endpoint(
    network: Network,
    owner: &TaskOwner,
) -> Option<(DaemonListener, PrivateEndpoint, PrivateFiles)> {
    let files = match owner.blocking(PrivateFiles::create).await {
        Ok(files) => files,
        Err(error) => {
            tracing::warn!(%error, "the private endpoint's files cannot be made: no worker or Daemon reaches this engine");
            return None;
        }
    };
    match network.listen(&files.socket()).await {
        Ok((listener, address)) => {
            let endpoint = PrivateEndpoint {
                address,
                token_file: files.token_file(),
            };
            Some((listener, endpoint, files))
        }
        Err(error) => {
            tracing::warn!(%error, "the private endpoint cannot listen: no worker or Daemon reaches this engine");
            None
        }
    }
}

/// Serves every Connection of an Attached engine's private endpoint, each
/// opening with a `hello` that carries `token`, its tasks children of
/// `owner`, until the listener fails or `owner` stops.
async fn serve_private_endpoint(
    dispatch: Arc<Dispatch>,
    mut listener: DaemonListener,
    token: Arc<str>,
    owner: TaskOwner,
    runs: Runs,
    events: Events,
    clock: Clock,
) {
    loop {
        let connection = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok(connection) => connection,
                Err(TransportError::DialerClosed) => return,
                Err(error) => {
                    tracing::warn!(%error, "the private endpoint stopped accepting");
                    return;
                }
            },
        };
        let (dispatch, token, runs, events) = (
            Arc::clone(&dispatch),
            Arc::clone(&token),
            runs.clone(),
            events.clone(),
        );
        let its_owner = owner.child();
        owner.spawn(async move {
            tokio::select! {
                () = its_owner.token().cancelled() => {}
                () = rpc::serve(dispatch, connection, &its_owner, runs, events, Access::PrivateEndpoint(token), clock) => {}
            }
            its_owner.stop().await;
        });
    }
}

#[cfg(test)]
mod tests;
