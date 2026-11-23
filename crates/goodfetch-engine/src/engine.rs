//! The engine: an identity and a composition, checked once at start, which
//! serves Connections from the values it is given (ADR 0059): the Goodfetch
//! home, the `Clock`, the `Entropy`, the blocking policy and where run state
//! goes, with its event stream's buffers. An Attached engine serves the one Connection its network gives it
//! and, on the OS, its private endpoint (the `attached` module), and stops
//! when that Connection ends or its parent is gone, its live Runs stopped
//! as aborted by their Client (ADRs 0024 and 0078). A Daemon serves every Connection its
//! listener gives it until it stops: idle, asked to, or handing over to its
//! major's pick (the `daemon` module).
//!
//! Size: one concern, an engine's start: the values its Runs and calls are
//! given, built once from its dependencies, its Run registry and its
//! startup writes, then the Connections it serves as an Attached engine or
//! a Daemon.

use goodfetch_counted_work::CountedWork;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::connection::PrivateEndpoint;
use goodfetch_protocol::connection::{EngineInstance, PanicEnded, ProductVersion};
use goodfetch_protocol::daemon::DRAIN_GRACE;
use goodfetch_protocol::governor::{SYSTEM_INFO, SystemInfo};
use goodfetch_protocol::run::{RunKind, RunOwner};
use goodfetch_protocol::secret_reference::Provider;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::admission::Gates;
use crate::attached::{ParentWatch, PrivateFiles, relay};
use crate::audit::{AuditEvents, AuditLog};
use crate::clock::Clock;
use crate::composition::{Composition, CompositionError};
use crate::controls::Controls;
use crate::daemon::{self, DaemonEnd, DaemonOptions};
use crate::endpoint::Refusals;
use crate::engine_context::{EngineContext, Stores, os_user};
use crate::entropy::Entropy;
use crate::events::{Events, EventsOptions};
use crate::forward::ToOwner;
use crate::governor::{Governor, System};
use crate::home::{Home, RunStateLocation};
use crate::log::LogEvents;
use crate::mask::Masks;
use crate::metrics::Metrics;
use crate::needs_you::{NeedsYou, NeedsYouSource, SourceContext};
use crate::ops::{self, Health, OpsOptions};
use crate::politeness::{PolitenessOptions, PolitenessTables};
use crate::product::Identity;
use crate::rpc::{self, Access, Dispatch};
use crate::run_state::install::InstallDatabase;
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::runs::{
    CleanUpVersions, Driving, PanicStop, ProgressPart, ReportPart, Runs, RunsSetup, SendWindow,
};
use crate::sandbox_probe::{self, SandboxProbe};
use crate::secrets::{self, Carried, Keychain, ProviderFactory, Secrets};
use crate::settings::{HostDefaults, HostSearch, Settings, SettingsFiles, get_or_default};
use crate::tasks::{BlockingPolicy, OwnerKind, TaskOwner};
use crate::transition::{
    self,
    engine::{EngineEvent, EngineState},
};
use crate::transport::{Connection, DaemonListener, Network, TransportError};

/// An engine of one Product, composed and ready to serve.
#[derive(Clone)]
pub struct Engine {
    dispatch: Arc<Dispatch>,
    tables: Vec<RunStateTables>,
    driving: BTreeMap<RunKind, Driving>,
    politeness_tables: Vec<PolitenessTables>,
    report_parts: Vec<Arc<dyn ReportPart>>,
    progress_parts: Vec<Arc<dyn ProgressPart>>,
    clean_up: Option<Arc<dyn CleanUpVersions>>,
    send_window: Option<Arc<dyn SendWindow>>,
    gates: Option<Arc<dyn Gates>>,
    needs_you_sources: Vec<NeedsYouSource>,
    secret_providers: Vec<(Provider, ProviderFactory)>,
    controls: Controls,
    events: EventsOptions,
    metrics: Metrics,
    system: Option<System>,
    audit_events: Option<AuditEvents>,
    drain_grace: Duration,
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
        let metrics = Metrics::none();
        Ok(Self {
            dispatch: Arc::new(Dispatch {
                identity: Arc::new(identity),
                handlers: composition.handlers()?,
                drivers: Arc::new(composition.drivers()),
                log_events: None,
                masks: Masks::new(),
                metrics: metrics.clone(),
                runs_of_calls: composition.runs_of_calls(),
                attempt_stagings: composition.attempt_stagings(),
            }),
            tables: composition.run_state_tables(),
            driving: composition.driving(),
            politeness_tables: composition.politeness_tables(),
            report_parts: composition.report_parts(),
            progress_parts: composition.progress_parts(),
            clean_up: composition.clean_up_versions()?,
            send_window: composition.send_window(),
            gates: composition.admission_gates()?,
            needs_you_sources: composition.needs_you_sources(),
            secret_providers: composition.secret_providers(),
            controls: composition.controls(),
            events: EventsOptions::default(),
            metrics,
            system: None,
            audit_events: None,
            drain_grace: get_or_default(None, &DRAIN_GRACE),
        })
    }

    /// The same, recording the runtime's metrics on `metrics` (ADR 0050).
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        Arc::make_mut(&mut self.dispatch).metrics = metrics.clone();
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
        Arc::make_mut(&mut self.dispatch).log_events = Some(log_events);
        self
    }

    /// The same, masking every live Run's resolved values with `masks`,
    /// which the process's log writers also pass their lines through
    /// (spec #564); without it the engine masks with its own.
    #[must_use]
    pub fn with_masks(mut self, masks: Masks) -> Self {
        Arc::make_mut(&mut self.dispatch).masks = masks;
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

    /// The same, an Attached engine that stops on a panic in a part every
    /// Run shares waiting at most `grace` for its Runs: the value of
    /// `daemon.drain_grace`, which a Daemon drains within (ADR 0148).
    /// Without it, that setting's default.
    #[must_use]
    pub fn with_drain_grace(mut self, grace: Duration) -> Self {
        self.drain_grace = grace;
        self
    }

    /// What it runs as.
    pub fn identity(&self) -> &Identity {
        &self.dispatch.identity
    }

    /// Serves `connection` until it ends, its tasks those of `owner`, its
    /// calls reaching the Runs of `runs`, the event stream `events` and
    /// what `context` gives them.
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
        context: EngineContext,
    ) {
        rpc::serve(
            Arc::clone(&self.dispatch),
            connection,
            owner,
            runs,
            events,
            Access::Attached(None),
            context,
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
        let (runs, events, context) = self.start_runs(
            dependencies,
            &owner,
            Some(RunOwner::Daemon(options.endpoint.clone())),
            options.install.take(),
        );
        let ended = daemon::serve(
            daemon::Serving {
                dispatch: Arc::clone(&self.dispatch),
                runs: runs.clone(),
                events,
                engine: context,
                metrics: self.metrics.clone(),
            },
            listener,
            options,
            &owner,
        )
        .await;
        if matches!(
            ended,
            DaemonEnd::Drained | DaemonEnd::ListenerFailed | DaemonEnd::Panicked
        ) {
            // Its process exits now, within its drain grace: a Run whose
            // tasks did not stop in time stays as it is, resumable from its
            // run file, and nothing waits for them. One that panicked waits
            // for no task either, as one of them may hold what the panic
            // left half done (ADR 0148).
            drop(runs);
            owner.cancel();
        } else {
            runs.stop_all().await;
            drop(runs);
            owner.stop().await;
        }
        self.late_panic(&owner, ended)
    }

    /// `ended`, unless a part every Run shares panicked after the Daemon's
    /// loop stopped waiting for a panic, as it stopped: then
    /// [`DaemonEnd::Panicked`], which exits 1 like any other such panic
    /// (ADR 0148). A handover keeps its end, the next Daemon serving on.
    fn late_panic(&self, owner: &TaskOwner, ended: DaemonEnd) -> DaemonEnd {
        match ended {
            DaemonEnd::Idle
            | DaemonEnd::Stopped
            | DaemonEnd::Drained
            | DaemonEnd::ListenerFailed
                if owner.has_panicked() =>
            {
                tracing::error!(
                    part = owner.panicked_part(),
                    "a part every Run shares panicked as the Daemon stopped"
                );
                self.metrics.panicked(PanicEnded::Engine);
                DaemonEnd::Panicked
            }
            ended => ended,
        }
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

    /// The engine's event stream, Runs registry and what its Runs and
    /// calls are given, tasks of `owner`; the Run index names `reached_at`
    /// as the owner of its live Runs, whose run state is in `install` in
    /// the per-install layout.
    fn start_runs(
        &self,
        mut dependencies: Dependencies,
        owner: &TaskOwner,
        reached_at: Option<RunOwner>,
        install: Option<InstallDatabase>,
    ) -> (Runs, Events, EngineContext) {
        dependencies.run_state.work = self.metrics.counted_work().clone();
        dependencies.run_state.metrics = Some(self.metrics.clone());
        dependencies.politeness.work = self.metrics.counted_work().clone();
        let instance = EngineInstance::new(format!("{:016x}", dependencies.entropy.next_u64()));
        let events = Events::start(
            self.events,
            instance,
            owner,
            self.metrics.counted_work().clone(),
        );
        let sandbox_probe =
            SandboxProbe::of(sandbox_probe::this_host, dependencies.environment.clone());
        let settings = host_settings(
            SettingsFiles::of(&dependencies.home, &self.identity().product_version),
            self.identity().product,
            owner,
            (dependencies.keychain.clone(), sandbox_probe.clone()),
            self.metrics.counted_work().clone(),
        );
        let governor = self.governor(&settings, dependencies.clock, owner, &events);
        let stores = Stores::of(
            &dependencies.home,
            &self.identity().product_version,
            dependencies.run_state.vfs.clone(),
            Arc::new(owner.part("engine's stores")),
            part_stopped_with(owner, "engine's store connections"),
        );
        let needs_you = NeedsYou::start(
            &self.needs_you_sources,
            SourceContext {
                clock: dependencies.clock,
                storage: governor.storage(),
                run_index: stores.run_index().clone(),
                ledger: stores.ledger().clone(),
                settings: settings.clone(),
                known_certificates: stores.known_certificates().clone(),
                owner: Arc::new(owner.part("Needs-you list")),
            },
            events.clone(),
            owner,
        );
        let forwarder = Arc::new(ToOwner {
            home: dependencies.home.clone(),
            version: self.identity().product_version.clone(),
            clock: dependencies.clock,
            settings: settings.clone(),
        });
        let (context, secrets) = self.engine_context(
            &mut dependencies,
            settings,
            (needs_you, sandbox_probe, stores),
            owner,
        );
        let runs = Runs::start_registry(
            RunsSetup {
                engine: context.clone(),
                secrets,
                product: self.identity().product,
                reached_at,
                forwarder,
                version: self.identity().product_version.clone(),
                location: dependencies.run_state_location,
                run_state: match &install {
                    Some(install) => RunStateOptions {
                        vfs: Some(install.vfs().to_owned()),
                        ..dependencies.run_state
                    },
                    None => dependencies.run_state,
                },
                install: install.map(Arc::new),
                tables: self.tables.clone(),
                driving: self.driving.clone(),
                report_parts: self.report_parts.clone(),
                progress_parts: self.progress_parts.clone(),
                clean_up: self.clean_up.clone(),
                send_window: self.send_window.clone(),
                gates: self.gates.clone(),
                politeness: dependencies.politeness,
                politeness_tables: self.politeness_tables.clone(),
                entropy: dependencies.entropy,
                events: events.clone(),
                metrics: self.metrics.clone(),
                governor,
                controls: self.controls.clone(),
                masks: self.dispatch.masks.clone(),
            },
            Box::pin(write_managed(
                context.clone(),
                self.identity().product_version.clone(),
            )),
            owner,
        );
        (runs, events, context)
    }

    /// What the engine's Runs and calls are given, under `settings` with
    /// `needs_you` and `stores`, and the engine's
    /// Secret-provider driver, which records in the same Audit log: both
    /// over `dependencies`' home and clock, the driver over its environment
    /// map, its keychain and a fork of its entropy. Nothing here opens the
    /// keychain: it opens on the first reference that needs it. The Audit
    /// log starts pruning: now, then once a day (ADR 0140).
    fn engine_context(
        &self,
        dependencies: &mut Dependencies,
        settings: Settings,
        (needs_you, sandbox_probe, stores): (NeedsYou, SandboxProbe, Stores),
        owner: &TaskOwner,
    ) -> (EngineContext, Secrets) {
        let audit = Arc::new(
            AuditLog::of(
                &dependencies.home,
                dependencies.run_state.vfs.clone(),
                part_stopped_with(owner, "Audit log"),
            )
            .counting_on(dependencies.run_state.work.clone())
            .retained_under(settings.clone())
            .sending_to(self.audit_events.clone()),
        );
        audit.start_pruning(dependencies.clock, owner);
        let refusals = Refusals::new(
            (*audit).clone(),
            dependencies.clock,
            part_stopped_with(owner, "listener refusals"),
        )
        .limited_under(settings.clone());
        let context = EngineContext::new(
            dependencies.home.clone(),
            dependencies.clock,
            settings,
            (Arc::clone(&audit), refusals),
            stores,
            needs_you,
            sandbox_probe,
            os_user(&dependencies.environment),
        );
        let secrets = Secrets::new(
            Carried {
                environment: std::mem::take(&mut dependencies.environment),
                keychain: dependencies.keychain.clone(),
                home: dependencies.home.clone(),
                entropy: dependencies.entropy.fork(),
            },
            audit,
            Arc::clone(context.owner()),
            dependencies.clock,
        )
        .masked_by(self.dispatch.masks.clone());
        let secrets = self
            .secret_providers
            .iter()
            .fold(secrets, |secrets, (provider, factory)| {
                secrets.with_configured(*provider, Arc::clone(factory))
            });
        (context, secrets)
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
    /// and removes its private endpoint. A panic in a part every Run shares
    /// stops it too, with no drain: it stops its live Runs for the panic,
    /// unless the Run registry is what panicked, waiting for them at most
    /// its drain grace ([`Engine::with_drain_grace`]), each Run not stopped
    /// by then left resumable from its run file; then it cancels every task
    /// it started and fails with [`AttachedFailure::Panicked`] (ADR 0148).
    ///
    /// # Errors
    ///
    /// [`AttachedFailure::NoConnection`] when `network` gave it no
    /// Connection; [`AttachedFailure::Panicked`] when it stopped on such a
    /// panic.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_attached(
        &self,
        network: Network,
        dependencies: Dependencies,
    ) -> Result<(), AttachedFailure> {
        self.attached(network, dependencies, None).await
    }

    /// Runs as an Attached engine, as [`Engine::run_attached`] does, and
    /// serves `ops` beside its Connection while it runs: an ops listener
    /// (ADR 0050) whose `/metrics` gives the metrics of the meter this
    /// engine was made with ([`Engine::with_metrics`]), its Counted work
    /// among them, and whose probes answer from its Runs. It is the
    /// listener `ops.attached_address` asks for; without one, an Attached
    /// engine serves none.
    ///
    /// # Errors
    ///
    /// As [`Engine::run_attached`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_attached_with_ops(
        &self,
        network: Network,
        dependencies: Dependencies,
        ops: OpsOptions,
    ) -> Result<(), AttachedFailure> {
        self.attached(network, dependencies, Some(ops)).await
    }

    async fn attached(
        &self,
        mut network: Network,
        dependencies: Dependencies,
        ops: Option<OpsOptions>,
    ) -> Result<(), AttachedFailure> {
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
        let (runs, events, context) = self.start_runs(
            dependencies,
            &owner,
            private
                .as_ref()
                .map(|(_, endpoint, _)| RunOwner::Attached(endpoint.clone())),
            None,
        );
        let connection_owner = owner.child_for(OwnerKind::Connection);
        let reported = private.as_ref().map(|(_, endpoint, _)| endpoint.clone());
        // The private endpoint's Connections, which the stdio Connection
        // does not wait for.
        let accepting = owner.part("private endpoint");
        serve_attached_ops(ops, &runs, &owner, &context);
        let stdio = rpc::serve(
            Arc::clone(&self.dispatch),
            connection,
            &connection_owner,
            runs.clone(),
            events.clone(),
            Access::Attached(reported),
            context.clone(),
        );
        let gone = async {
            match parent {
                Some(parent) => parent.gone().await,
                None => std::future::pending().await,
            }
        };
        let mut private_files = None;
        if let Some((listener, _, files)) = private {
            serve_private_endpoint(
                &accepting,
                Arc::clone(&self.dispatch),
                listener,
                Arc::from(files.token()),
                runs.clone(),
                events,
                context,
            );
            private_files = Some(files);
        }
        let panicked = owner.panicked();
        let ended = tokio::select! {
            () = stdio => EngineEvent::StdioEnded,
            () = gone => {
                tracing::warn!("the Attached engine's parent is gone: its Runs stop");
                EngineEvent::ParentGone
            }
            () = panicked => EngineEvent::panicked(runs.registry_panicked()),
        };
        // Each end stops it, as the engine's table says (ADR 0128).
        if let Err(refused) = transition::engine::engine(EngineState::Attached, ended) {
            let _ = refused.logged();
        }
        accepting.stop().await;
        match ended {
            EngineEvent::PartPanicked | EngineEvent::RegistryPanicked => {
                // A panic in a part every Run shares stops the engine with
                // no drain, and its process exits 1 (ADR 0148). Its Runs
                // stop for the panic first, within the drain grace, unless
                // the registry that stops them is what panicked: then each
                // stays as it is, read as interrupted.
                let registry = matches!(ended, EngineEvent::RegistryPanicked);
                tracing::error!(
                    part = owner.panicked_part(),
                    registry,
                    "a part every Run shares panicked: the Attached engine stops"
                );
                self.metrics.panicked(PanicEnded::Engine);
                if !registry {
                    stop_runs_on_panic(&runs, clock.sleep(self.drain_grace)).await;
                }
                drop(runs);
                connection_owner.cancel();
                owner.cancel();
                drop(private_files);
                Err(AttachedFailure::Panicked)
            }
            _ => {
                runs.stop_all().await;
                drop(runs);
                connection_owner.stop().await;
                owner.stop().await;
                drop(private_files);
                if owner.has_panicked() {
                    // One panicked after the wait above stopped waiting, as
                    // the engine stopped: it fails all the same (ADR 0148).
                    tracing::error!(
                        part = owner.panicked_part(),
                        "a part every Run shares panicked as the Attached engine stopped"
                    );
                    self.metrics.panicked(PanicEnded::Engine);
                    return Err(AttachedFailure::Panicked);
                }
                Ok(())
            }
        }
    }
}

/// Why an Attached engine failed ([`Engine::run_attached`]).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AttachedFailure {
    /// It got no Connection.
    #[error("it got no Connection: {0}")]
    NoConnection(#[from] TransportError),
    /// A part every Run shares panicked, so it stopped (ADR 0148).
    #[error("it stopped on a panic in a part every Run shares")]
    Panicked,
}

/// Writes the install Limits the managed Operator policy of the engine's
/// major, `version`'s, names as that major's managed rows in the Usage
/// ledger of `context`, as each engine does when it starts (ADR 0041); a
/// policy that names none, or none at all, leaves that major no managed
/// row. The Run registry answers no command before it ends.
/// A part of `owner` named `part` for the actors of a store the engine's
/// Runs, Connections and calls share, such as the Audit log's writer: its
/// actors end once every sender is gone, which `owner`'s stop would not
/// see, since whoever waits for the engine's readers, such as the Run
/// registry, holds a sender. A task of `owner` stops the part once that
/// stop began, which the engine starts after its Runs and Connections have
/// stopped, and `owner`'s stop waits for that task (ADR 0060).
fn part_stopped_with(owner: &TaskOwner, part: &'static str) -> Arc<TaskOwner> {
    let part = Arc::new(owner.part(part));
    let (stopping, stopped) = (owner.token().clone(), Arc::clone(&part));
    owner.spawn(async move {
        stopping.cancelled().await;
        stopped.stop().await;
    });
    part
}

/// Writes the managed Operator policy's Limits into the Usage ledger, from
/// the engine's snapshot of the machine's settings, read as the engine
/// starts (ADR 0140), so no Run counts a read of its own for them.
async fn write_managed(context: EngineContext, version: ProductVersion) {
    let snapshot = context.settings().snapshot().await;
    let machine = match snapshot.layers() {
        Ok(machine) => machine,
        Err(error) => {
            tracing::warn!(%error, "the machine's settings files could not be read, so no managed Limit was written");
            return;
        }
    };
    let limits: Vec<goodfetch_protocol::usage::Limit> = machine
        .operator_policy
        .managed
        .iter()
        .flat_map(|managed| managed.install_limits.iter())
        .map(goodfetch_protocol::usage::ManagedLimit::limit)
        .collect();
    let ledger = context.ledger().clone();
    let major = version.line().to_string();
    let now = context.now();
    let written = context
        .owner()
        .blocking(move || ledger.write_managed(&major, &limits, now))
        .await;
    if let Err(error) = written {
        tracing::warn!(%error, "the Usage ledger could not be opened to write the managed Operator policy's Limits");
    }
}

/// The engine's settings, whose host defaults are those of `probe`,
/// Chromium's sandbox's, and `keychain`'s, their reads counted on `work`.
/// The probe starts now, at startup (ADR 0049); every opening of a Run's
/// settings waits for its answer, which each browser launch reads. The
/// keychain's are only asked for where they are needed, such as where a
/// Run first makes its data key, so that an opening opens no OS keychain
/// (issue 2204).
fn host_settings(
    files: SettingsFiles,
    product: goodfetch_protocol::catalog::Product,
    owner: &TaskOwner,
    (keychain, probe): (Keychain, SandboxProbe),
    work: CountedWork,
) -> Settings {
    let probing = Arc::new(owner.part("sandbox probe"));
    {
        let (probing, probe) = (Arc::clone(&probing), probe.clone());
        owner.spawn(async move {
            probing
                .blocking(move || {
                    probe.probed();
                })
                .await;
        });
    }
    let asking = keychain.clone();
    let search = HostSearch::opening(move || {
        let (probing, probe) = (Arc::clone(&probing), probe.clone());
        Box::pin(async move {
            let probed = probing.blocking(move || probe.probed().clone()).await;
            sandbox_probe::host_defaults(HostDefaults::default(), &probed)
        })
    })
    .asking(
        move || {
            let keychain = asking.clone();
            Box::pin(async move { secrets::host_defaults(&keychain).await })
        },
        move || secrets::known_host_defaults(&keychain),
    );
    Settings::with_host(files, product, owner.part("settings"), work, search)
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
    let socket = files.socket();
    match owner.blocking(move || network.listen(&socket)).await {
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

/// Stops an Attached engine's live Runs for a panic in a part every Run
/// shares, waiting for them until `grace`, its drain grace, ends; then logs
/// how many it left, or that the Runs' registry never answered, each such
/// Run resumable from its run file (ADR 0148).
async fn stop_runs_on_panic(runs: &Runs, grace: impl Future<Output = ()>) {
    match runs.stop_all_on_panic(grace).await {
        PanicStop::Ended => {}
        PanicStop::NotEnded(live_runs) => tracing::warn!(
            live_runs,
            "the drain grace passed: each run not yet stopped stays resumable from its run file"
        ),
        PanicStop::Unanswered => tracing::warn!(
            "the drain grace passed before the Runs' registry answered: each live run stays resumable from its run file"
        ),
    }
}

/// Serves `ops`, if any, under `owner` while an Attached engine runs (ADR
/// 0050): its probes answer from `runs` alone, since an Attached engine
/// launches each Run's browser as the Run asks, and `/metrics` from
/// `context`'s meter.
fn serve_attached_ops(
    ops: Option<OpsOptions>,
    runs: &Runs,
    owner: &TaskOwner,
    context: &EngineContext,
) {
    let Some(ops) = ops else {
        return;
    };
    let health = Health {
        runs: runs.clone(),
        browsers: None,
        blocking: Arc::new(owner.part("ops checks")),
    };
    owner.spawn(ops::serve(
        ops,
        health,
        owner.part("ops endpoint"),
        context.clone(),
    ));
}

/// Serves every Connection of an Attached engine's private endpoint on
/// `listener`, each opening with a `hello` that carries `token`, until the
/// listener fails or `accepting` stops. `accepting` runs the accept loop
/// alone: each Connection and its calls belong to an owner under it, so
/// `accepting`'s stop waits for no call, even one that cannot be cancelled,
/// and an engine stopping on a panic goes on to its Runs' stop at once (ADR
/// 0148).
fn serve_private_endpoint(
    accepting: &TaskOwner,
    dispatch: Arc<Dispatch>,
    listener: DaemonListener,
    token: Arc<str>,
    runs: Runs,
    events: Events,
    context: EngineContext,
) {
    accepting.spawn(accept_private_connections(
        dispatch,
        listener,
        token,
        accepting.child(),
        runs,
        events,
        context,
    ));
}

/// The private endpoint's accept loop: serves every Connection on
/// `listener`, its tasks children of `owner`, until the listener fails or
/// `owner` is cancelled, never waiting for those tasks.
async fn accept_private_connections(
    dispatch: Arc<Dispatch>,
    listener: DaemonListener,
    token: Arc<str>,
    owner: TaskOwner,
    runs: Runs,
    events: Events,
    context: EngineContext,
) {
    let mut accepts = daemon::Accepts::new(listener, context.clock());
    loop {
        let connection = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = accepts.next() => match accepted {
                Ok(connection) => connection,
                Err(daemon::NoMoreConnections::DialersGone) => return,
                Err(daemon::NoMoreConnections::ListenerClosed(error)) => {
                    tracing::warn!(%error, "the private endpoint stopped accepting");
                    return;
                }
            },
        };
        let (dispatch, token, runs, events, context) = (
            Arc::clone(&dispatch),
            Arc::clone(&token),
            runs.clone(),
            events.clone(),
            context.clone(),
        );
        let its_owner = owner.child_for(OwnerKind::Connection);
        owner.spawn(async move {
            tokio::select! {
                () = its_owner.token().cancelled() => {}
                () = rpc::serve(dispatch, connection, &its_owner, runs, events, Access::PrivateEndpoint(token), context) => {}
            }
            its_owner.stop().await;
        });
    }
}

#[cfg(test)]
mod tests;
