//! The engine: an identity and a composition, checked once at start, which
//! serves Connections from the values it is given (ADR 0059): the Goodfetch
//! home, the `Clock`, the `Entropy`, the blocking policy and where run state
//! goes, with its event stream's buffers. An Attached engine serves the one Connection its network gives it
//! and stops when that Connection ends, its live Runs stopped as aborted by
//! their Client (ADRs 0024 and 0078). A Daemon serves every Connection its
//! listener gives it until it stops: idle, asked to, or handing over to its
//! major's pick (the `daemon` module).

use std::sync::Arc;

use goodfetch_protocol::daemon::DaemonEndpoint;

use crate::admission::Gates;
use crate::clock::Clock;
use crate::composition::{Composition, CompositionError};
use crate::daemon::{self, DaemonEnd, DaemonOptions};
use crate::entropy::Entropy;
use crate::events::{Events, EventsOptions};
use crate::forward::ToDaemon;
use crate::home::{Home, RunStateLocation};
use crate::product::Identity;
use crate::rpc::{self, Dispatch};
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::runs::{Runs, RunsSetup};
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::{Connection, DaemonListener, Network, TransportError};

/// An engine of one Product, composed and ready to serve.
#[derive(Clone)]
pub struct Engine {
    dispatch: Arc<Dispatch>,
    tables: Vec<RunStateTables>,
    gates: Option<Arc<dyn Gates>>,
    events: EventsOptions,
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
}

impl Engine {
    /// The engine `composition` makes for the Product `identity` names.
    pub fn new(identity: Identity, composition: &Composition) -> Result<Self, CompositionError> {
        Ok(Self {
            dispatch: Arc::new(Dispatch {
                identity: Arc::new(identity),
                handlers: composition.handlers()?,
            }),
            tables: composition.run_state_tables(),
            gates: composition.admission_gates()?,
            events: EventsOptions::default(),
        })
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

    /// Serves `connection` until it ends, its tasks those of `owner`, its
    /// calls reaching the Runs of `runs` and the event stream `events`.
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
    ) {
        rpc::serve(
            Arc::clone(&self.dispatch),
            connection,
            owner,
            runs,
            events,
            None,
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
        options: DaemonOptions,
    ) -> DaemonEnd {
        let owner = TaskOwner::root(dependencies.blocking);
        let clock = dependencies.clock;
        let (runs, events) = self.start_runs(dependencies, &owner, Some(options.endpoint.clone()));
        let ended = daemon::serve(
            daemon::Serving {
                dispatch: Arc::clone(&self.dispatch),
                runs: runs.clone(),
                events,
                clock,
            },
            listener,
            options,
            &owner,
        )
        .await;
        runs.stop_all().await;
        drop(runs);
        owner.stop().await;
        ended
    }

    /// The engine's event stream and Runs registry, tasks of `owner`; the
    /// Run index names `endpoint` as the owner of its live Runs.
    fn start_runs(
        &self,
        dependencies: Dependencies,
        owner: &TaskOwner,
        endpoint: Option<DaemonEndpoint>,
    ) -> (Runs, Events) {
        let events = Events::start(self.events, owner);
        let settings = Settings::new(
            SettingsFiles::of(&dependencies.home, &self.identity().product_version),
            self.identity().product,
            owner.child(),
        );
        let forwarder = Arc::new(ToDaemon {
            home: dependencies.home.clone(),
            version: self.identity().product_version.clone(),
            clock: dependencies.clock,
        });
        let runs = Runs::start_registry(
            RunsSetup {
                settings,
                product: self.identity().product,
                endpoint,
                forwarder,
                home: dependencies.home,
                version: self.identity().product_version.clone(),
                location: dependencies.run_state_location,
                run_state: dependencies.run_state,
                tables: self.tables.clone(),
                gates: self.gates.clone(),
                clock: dependencies.clock,
                entropy: dependencies.entropy,
                events: events.clone(),
            },
            owner,
        );
        (runs, events)
    }

    /// Runs as an Attached engine on `dependencies`: serves the one
    /// Connection `network` gives it, stops its live Runs once that
    /// Connection ends, then stops every task it started.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_attached(
        &self,
        mut network: Network,
        dependencies: Dependencies,
    ) -> Result<(), TransportError> {
        let connection = network.attached_connection().await?;
        let owner = TaskOwner::root(dependencies.blocking);
        let (runs, events) = self.start_runs(dependencies, &owner, None);
        let connection_owner = owner.child();
        self.serve(connection, &connection_owner, runs.clone(), events)
            .await;
        runs.stop_all().await;
        drop(runs);
        owner.stop().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
