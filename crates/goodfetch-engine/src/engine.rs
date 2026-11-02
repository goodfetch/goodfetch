//! The engine: an identity and a composition, checked once at start, which
//! serves Connections from the values it is given (ADR 0059): the Goodfetch
//! home, the `Clock`, the `Entropy`, the blocking policy and where run state
//! goes, with its event stream's buffers. An Attached engine serves the one Connection its network gives it
//! and, on the OS, its private endpoint (the `attached` module), and stops
//! when that Connection ends or its parent is gone, its live Runs stopped
//! as aborted by their Client (ADRs 0024 and 0078). A Daemon serves every Connection its
//! listener gives it until it stops: idle, asked to, or handing over to its
//! major's pick (the `daemon` module).

use goodfetch_counted_work::CountedWork;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::connection::PrivateEndpoint;
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::governor::{SYSTEM_INFO, SystemInfo};
use goodfetch_protocol::run::RunOwner;
use goodfetch_protocol::secret_reference::Provider;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::admission::Gates;
use crate::attached::{ParentWatch, PrivateFiles, relay};
use crate::audit::AuditEvents;
use crate::clock::Clock;
use crate::composition::{Composition, CompositionError, ConsoleBuild};
use crate::console;
use crate::controls::Controls;
use crate::daemon::{self, DaemonConsole, DaemonEnd, DaemonOptions};
use crate::entropy::Entropy;
use crate::events::{Events, EventsOptions};
use crate::forward::ToOwner;
use crate::governor::{Governor, System};
use crate::home::{Home, RunStateLocation, major_name};
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::log::LogEvents;
use crate::mask::Masks;
use crate::metrics::Metrics;
use crate::needs_you::{NeedsYou, NeedsYouSource, SourceContext};
use crate::people::{AccountsStore, ConsoleKey, SignInDriver, SignIns, SingleUserLink, os_user};
use crate::politeness::{PolitenessOptions, PolitenessTables};
use crate::product::Identity;
use crate::rpc::{self, Access, Dispatch};
use crate::run_state::install::InstallDatabase;
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::runs::{CleanUpVersions, ProgressPart, ReportPart, Runs, RunsSetup};
use crate::sandbox_probe;
use crate::secrets::{self, Keychain, ProviderFactory};
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
    report_parts: Vec<Arc<dyn ReportPart>>,
    progress_parts: Vec<Arc<dyn ProgressPart>>,
    clean_up: Option<Arc<dyn CleanUpVersions>>,
    gates: Option<Arc<dyn Gates>>,
    needs_you_sources: Vec<NeedsYouSource>,
    secret_providers: Vec<(Provider, ProviderFactory)>,
    controls: Controls,
    events: EventsOptions,
    console: Option<&'static ConsoleBuild>,
    metrics: Metrics,
    system: Option<System>,
    audit_events: Option<AuditEvents>,
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
                drivers: Arc::new(composition.drivers()),
                log_events: None,
                masks: Masks::new(),
                work: CountedWork::default(),
            }),
            tables: composition.run_state_tables(),
            politeness_tables: composition.politeness_tables(),
            report_parts: composition.report_parts(),
            progress_parts: composition.progress_parts(),
            clean_up: composition.clean_up_versions()?,
            gates: composition.admission_gates()?,
            needs_you_sources: composition.needs_you_sources(),
            secret_providers: composition.secret_providers(),
            controls: composition.controls(),
            events: EventsOptions::default(),
            console: composition.console(),
            metrics: Metrics::none(),
            system: None,
            audit_events: None,
        })
    }

    /// The same, recording the runtime's metrics on `metrics` (ADR 0050).
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.dispatch = Arc::new(Dispatch {
            identity: Arc::clone(&self.dispatch.identity),
            handlers: self.dispatch.handlers.clone(),
            drivers: Arc::clone(&self.dispatch.drivers),
            log_events: self.dispatch.log_events.clone(),
            masks: self.dispatch.masks.clone(),
            work: metrics.counted_work().clone(),
        });
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
            drivers: Arc::clone(&self.dispatch.drivers),
            log_events: Some(log_events),
            masks: self.dispatch.masks.clone(),
            work: self.dispatch.work.clone(),
        });
        self
    }

    /// The same, masking every live Run's resolved values with `masks`,
    /// which the process's log writers also pass their lines through
    /// (spec #564); without it the engine masks with its own.
    #[must_use]
    pub fn with_masks(mut self, masks: Masks) -> Self {
        self.dispatch = Arc::new(Dispatch {
            identity: Arc::clone(&self.dispatch.identity),
            handlers: self.dispatch.handlers.clone(),
            drivers: Arc::clone(&self.dispatch.drivers),
            log_events: self.dispatch.log_events.clone(),
            masks,
            work: self.dispatch.work.clone(),
        });
        self
    }

    /// The same, its Audit log also sending each entry it appends to
    /// `events`, the OpenTelemetry events of `audit.otlp` (ADR 0016).
    #[must_use]
    pub fn with_audit_events(mut self, events: AuditEvents) -> Self {
        self.audit_events = Some(events);
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
            Access::Attached(None, None),
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
        let (runs, events) = self.start_runs(
            dependencies,
            &owner,
            Some(RunOwner::Daemon(options.endpoint.clone())),
            options.install.take(),
            self.console_links(&options.console),
        );
        let opening = ConsoleOpening {
            token: options.token.clone(),
            home: &home,
            person,
            runs: &runs,
            clock,
            owner: &owner,
        };
        let (console, sign_ins, console_connections) = self.open_console(
            std::mem::replace(&mut options.console, DaemonConsole::Absent),
            opening,
        );
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

    /// The engine's Resource governor, a task of `owner` that reads
    /// `settings` on `clock` and reports each sample to `events`, when its
    /// starter gave it the machine to read; else one that never samples.
    fn governor(
        &self,
        settings: &Settings,
        clock: Clock,
        owner: &TaskOwner,
        events: &Events,
    ) -> Governor {
        let Some(system) = &self.system else {
            return Governor::unsampled();
        };
        let governor = Governor::start(
            system.clone(),
            settings.clone(),
            clock,
            owner,
            self.metrics.counted_work().clone(),
        );
        owner.spawn(report_samples(
            governor.system_info(),
            events.clone(),
            self.metrics.clone(),
            owner.token().clone(),
        ));
        governor
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
        dependencies.run_state.work = self.metrics.counted_work().clone();
        let instance = EngineInstance::new(format!("{:016x}", dependencies.entropy.next_u64()));
        let events = Events::start(
            self.events,
            instance,
            owner,
            self.metrics.counted_work().clone(),
        );
        let settings = host_settings(
            SettingsFiles::of(&dependencies.home, &self.identity().product_version),
            self.identity().product,
            owner,
            &dependencies,
        );
        let governor = self.governor(&settings, dependencies.clock, owner, &events);
        let needs_you = NeedsYou::start(
            &self.needs_you_sources,
            SourceContext {
                clock: dependencies.clock,
                storage: governor.storage(),
                run_index: dependencies
                    .home
                    .run_index(&self.identity().product_version),
                version: self.identity().product_version.clone(),
                ledger: Ledger::at(
                    dependencies.home.usage_ledger(),
                    dependencies.run_state.vfs.clone(),
                ),
                settings: settings.clone(),
                known_certificates: KnownCertificates::of(
                    &dependencies.home,
                    dependencies.run_state.vfs.clone(),
                ),
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
                audit_events: self.audit_events.clone(),
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
                controls: self.controls.clone(),
                masks: self.dispatch.masks.clone(),
                secret_providers: self.secret_providers.clone(),
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
        network: Network,
        dependencies: Dependencies,
    ) -> Result<(), TransportError> {
        self.run_attached_with_console(network, dependencies, DaemonConsole::Absent)
            .await
    }

    /// Runs as an Attached engine, as [`Engine::run_attached`] does, that
    /// also serves `console`, the Console its script asked for with an
    /// address of its own (ADR 0031): its stdio's `hello` answers the
    /// Console's sign-in link, under a per-launch token, and the Console
    /// stops listening when the engine stops, so nothing listens once the
    /// script has ended. An engine whose composition carries no Console
    /// build serves none.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_attached_with_console(
        &self,
        mut network: Network,
        dependencies: Dependencies,
        console: DaemonConsole,
    ) -> Result<(), TransportError> {
        let on_the_os = matches!(network, Network::Os);
        let parent = on_the_os.then(ParentWatch::start);
        let connection = network.attached_connection().await?;
        let owner = TaskOwner::root(dependencies.blocking);
        let clock = dependencies.clock;
        let home = dependencies.home.clone();
        let person = os_user(&dependencies.environment);
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
            self.console_links(&console),
        );
        let connection_owner = owner.child();
        let reported = private.as_ref().map(|(_, endpoint, _)| endpoint.clone());
        // The private endpoint's and the Console's Connections, which the
        // stdio Connection does not wait for.
        let accepting = owner.child();
        let console_link = self.serve_scripts_console(
            console,
            ConsoleOpening {
                token: String::new(),
                home: &home,
                person,
                runs: &runs,
                clock,
                owner: &owner,
            },
            &accepting,
            &events,
        );
        let stdio = rpc::serve(
            Arc::clone(&self.dispatch),
            connection,
            &connection_owner,
            runs.clone(),
            events.clone(),
            Access::Attached(reported, console_link),
            clock,
        );
        let gone = async {
            match parent {
                Some(parent) => parent.gone().await,
                None => std::future::pending().await,
            }
        };
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

/// What opening a Console takes beside the Console its starter set up.
struct ConsoleOpening<'a> {
    /// The token its single-user sign-in link carries.
    token: String,
    home: &'a Home,
    /// The OS user its single-user sign-in names.
    person: String,
    runs: &'a Runs,
    clock: Clock,
    owner: &'a TaskOwner,
}

impl Engine {
    /// The address links to `console` name (ADR 0031): its
    /// `console.public_url` when set, else its own URL; none when the
    /// engine serves no Console.
    fn console_links(&self, console: &DaemonConsole) -> Option<String> {
        match (&self.console, console) {
            (
                Some(_),
                DaemonConsole::Listening {
                    url, public_url, ..
                },
            ) => Some(console::links::link_base(url, public_url.as_deref())),
            _ => None,
        }
    }

    /// Serves an Attached engine's own `console`, its pages' Connections
    /// tasks of `connections` reaching the engine's Runs, under a sign-in
    /// token of this launch: its sign-in link, `<url>#token=<token>`; none
    /// when it serves no Console.
    fn serve_scripts_console(
        &self,
        console: DaemonConsole,
        opening: ConsoleOpening<'_>,
        connections: &TaskOwner,
        events: &Events,
    ) -> Option<String> {
        if self.console.is_none() || !matches!(console, DaemonConsole::Listening { .. }) {
            return None;
        }
        let token = match crate::endpoint::new_token() {
            Ok(token) => token,
            Err(error) => {
                tracing::error!(%error, "the OS generator gave no sign-in token, so the script's Console does not start");
                return None;
            }
        };
        let runs = opening.runs.clone();
        let clock = opening.clock;
        let opening = ConsoleOpening {
            token: token.clone(),
            ..opening
        };
        match self.open_console(console, opening) {
            (ConsoleState::Serving(url), Some(sign_ins), opened) => {
                connections.spawn(serve_attached_console(
                    Arc::clone(&self.dispatch),
                    opened,
                    sign_ins,
                    connections.child(),
                    runs,
                    events.clone(),
                    clock,
                ));
                Some(format!("{url}#token={token}"))
            }
            _ => None,
        }
    }

    /// Serves `console` with this engine's Console build, its tasks those
    /// of `opening`'s owner: where it is, its sign-in driver, and the
    /// Connections its pages open; or why it serves none.
    fn open_console(
        &self,
        console: DaemonConsole,
        opening: ConsoleOpening<'_>,
    ) -> (
        ConsoleState,
        Option<SignInDriver>,
        tokio::sync::mpsc::Receiver<rpc::ConsoleConnection>,
    ) {
        let (opened_connections, console_connections) =
            tokio::sync::mpsc::channel(CONSOLE_CONNECTIONS);
        let ConsoleOpening {
            token,
            home,
            person,
            runs,
            clock,
            owner,
        } = opening;
        let (state, sign_ins) = match (self.console, console) {
            (
                Some(build),
                DaemonConsole::Listening {
                    listener,
                    url,
                    public_url: _,
                    sign_in,
                    content,
                },
            ) => {
                let sign_ins = SignInDriver::new(SignIns {
                    link: SingleUserLink::new(token),
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
        (state, sign_ins, console_connections)
    }
}

/// Serves each Connection the pages of an Attached engine's own Console
/// open, every one a task of `owner`, reaching the engine's Runs: no
/// Daemon answers them, and their Runs end with the engine's.
async fn serve_attached_console(
    dispatch: Arc<Dispatch>,
    mut connections: tokio::sync::mpsc::Receiver<rpc::ConsoleConnection>,
    sign_ins: SignInDriver,
    owner: TaskOwner,
    runs: Runs,
    events: Events,
    clock: Clock,
) {
    let stopped = owner.token().clone();
    loop {
        let connection = tokio::select! {
            biased;
            () = stopped.cancelled() => return,
            next = connections.recv() => match next {
                Some(connection) => connection,
                // The Console's server stopped.
                None => return,
            },
        };
        let (dispatch, sign_ins, runs, events) = (
            Arc::clone(&dispatch),
            sign_ins.clone(),
            runs.clone(),
            events.clone(),
        );
        let its_owner = owner.child();
        let closed = its_owner.token().clone();
        owner.spawn(async move {
            let side = rpc::ConsoleSide {
                daemon: None,
                sign_ins,
            };
            let served =
                rpc::serve_console(dispatch, connection, &its_owner, runs, events, side, clock);
            // The engine's stop closes the Console's Connections with it.
            tokio::select! {
                biased;
                () = closed.cancelled() => {}
                () = served => {}
            }
            its_owner.stop().await;
        });
    }
}

/// Sends each of the Resource governor's samples as `engine.system_info`
/// to the subscriptions that take it, and records it in the runtime's
/// metrics, until `token` is cancelled.
/// The engine's settings, whose host defaults are the keychain's and
/// Chromium's sandbox probe's. The probe starts now, at startup, once for
/// the process (ADR 0049); the host's defaults wait for its answer, which
/// each browser launch reads.
fn host_settings(
    files: SettingsFiles,
    product: goodfetch_protocol::catalog::Product,
    owner: &TaskOwner,
    dependencies: &Dependencies,
) -> Settings {
    let probing = Arc::new(owner.child());
    let environment = Arc::new(dependencies.environment.clone());
    {
        let (probing, environment) = (Arc::clone(&probing), Arc::clone(&environment));
        owner.spawn(async move {
            probing
                .blocking(move || {
                    sandbox_probe::probed_in(&environment);
                })
                .await;
        });
    }
    let keychain = dependencies.keychain.clone();
    let work = dependencies.run_state.work.clone();
    Settings::with_host(files, product, owner.child(), work, move || {
        let keychain = keychain.clone();
        let (probing, environment) = (Arc::clone(&probing), Arc::clone(&environment));
        Box::pin(async move {
            let probed = probing
                .blocking(move || sandbox_probe::probed_in(&environment))
                .await;
            sandbox_probe::host_defaults(secrets::host_defaults(&keychain).await, probed)
        })
    })
}

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
