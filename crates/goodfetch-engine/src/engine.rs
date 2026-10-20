//! The engine: an identity and a composition, checked once at start, which
//! serves Connections from the values it is given (ADR 0059): the Goodfetch
//! home, the `Clock`, the `Entropy`, the blocking policy and where run state
//! goes, with its event stream's buffers. An Attached engine serves the one Connection its network gives it
//! and stops when that Connection ends, its live Runs stopped as aborted by
//! their Client (ADRs 0024 and 0078); the Daemon joins with the ticket that
//! adds it.

use std::sync::Arc;

use crate::clock::Clock;
use crate::composition::{Composition, CompositionError};
use crate::entropy::Entropy;
use crate::events::{Events, EventsOptions};
use crate::home::{Home, RunStateLocation};
use crate::product::Identity;
use crate::rpc::{self, Dispatch};
use crate::run_state::{RunStateOptions, RunStateTables};
use crate::runs::{Runs, RunsSetup};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::{Connection, Network, TransportError};

/// An engine of one Product, composed and ready to serve.
#[derive(Clone)]
pub struct Engine {
    dispatch: Arc<Dispatch>,
    tables: Vec<RunStateTables>,
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
        rpc::serve(Arc::clone(&self.dispatch), connection, owner, runs, events).await;
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
        let events = Events::start(self.events, &owner);
        let runs = Runs::start_registry(
            RunsSetup {
                home: dependencies.home,
                version: self.identity().product_version.clone(),
                location: dependencies.run_state_location,
                run_state: dependencies.run_state,
                tables: self.tables.clone(),
                clock: dependencies.clock,
                entropy: dependencies.entropy,
                events: events.clone(),
            },
            &owner,
        );
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
