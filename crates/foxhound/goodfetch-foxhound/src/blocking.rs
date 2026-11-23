//! Foxhound for a program that does not use async (ADR 0081), under the
//! same names as Collie's `blocking`: a call ends in `send()`. The
//! connection owns the runtime its linked engine runs on; each call is
//! handed to it, and the calling thread waits for its end on a channel.
//! Nothing here enters a runtime, so it works inside one too, where it
//! blocks only the thread that calls it. A crawl's handlers are plain
//! functions, each run on one of the connection's tracked blocking
//! threads ([`Crawl`]).

use std::sync::Arc;

use goodfetch_collie::linked::ProductConnection;
use goodfetch_collie::{ConnectOptions, EngineInfo};
use goodfetch_collie_client::{Error, RunView, blocking};
use goodfetch_protocol::run::{CatalogTarget, RunId};
use goodfetch_protocol::settings::{ConfigExplainParams, SettingsExplanation};

pub use crawl::{Crawl, CrawlContext, CrawlRun, StartCrawl};
pub use goodfetch_collie_client::blocking::{
    Call, Connection, EventIter, Exchange, HandlerlessStart, Locator, Page, Query, RawCall,
    Response, Run, Session, Start,
};

use crate::crawl_requests::CrawlRequests;
use crate::foxhound::ENGINE;

mod crawl;

/// A connection to Foxhound whose calls block: `foxhound.start().send()`
/// starts a Run. Cloning it is cheap and every clone is the same
/// connection; `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_foxhound::ConnectOptions;
/// use goodfetch_foxhound::blocking::Foxhound;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// let foxhound = Foxhound::connect(options)?;
/// let run = foxhound.start().send()?;
/// let report = run.finish().send()?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// foxhound.close()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Foxhound {
    linked: ProductConnection,
    connection: blocking::ClientConnection,
    /// The connection's stream of Crawl requests, which the reader of
    /// each of its crawl Runs takes its own from.
    crawl_requests: Arc<CrawlRequests>,
}

impl Foxhound {
    /// Opens a connection to a linked Foxhound engine of its own, as
    /// [`crate::Foxhound::connect`] does, and blocks the calling thread
    /// until `hello` is answered.
    ///
    /// # Errors
    ///
    /// As [`crate::Foxhound::connect`].
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let linked = ProductConnection::connect_blocking(ENGINE, options)?;
        Ok(Self {
            connection: blocking::ClientConnection::from(linked.connection().clone()),
            linked,
            crawl_requests: Arc::default(),
        })
    }

    /// The engine the connection reached, as [`crate::Foxhound::engine`]
    /// reports it.
    pub fn engine(&self) -> &EngineInfo {
        self.linked.engine()
    }

    /// The raw call of `operation`, as [`crate::Foxhound::call`]'s, which
    /// `send()` sends.
    pub fn call(&self, operation: &str) -> RawCall {
        self.connection.raw_call(operation)
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
    /// The start of a Run the program drives (`run.start`), which `send()`
    /// sends.
    pub fn start(&self) -> Start {
        self.connection.start()
    }

    /// Starts a Recipe's run, a Run with no handler in the program, and
    /// answers once it is started. Its Run ID works as `run.start`'s. With
    /// `detached`, the Run outlives the program that started it: closing
    /// its value frees only the value, `run.finish` waits for its end and
    /// `run.stop` stops it.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a Recipe's run, as [`crate::Foxhound::start_recipe`]'s,
    /// which `send()` sends.
    pub fn start_recipe(&self, recipe: &str) -> HandlerlessStart {
        self.connection.start_recipe(recipe)
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
    /// The start of a code-free catalog run, as
    /// [`crate::Foxhound::start_catalog_run`]'s, which `send()` sends.
    pub fn start_catalog_run(&self, target: CatalogTarget) -> HandlerlessStart {
        self.connection.start_catalog_run(target)
    }

    /// Opens a view of the Run `run_id` names, as
    /// [`crate::Foxhound::open_run`] does, with no round trip, so nothing
    /// blocks.
    ///
    /// # Errors
    ///
    /// As [`crate::Foxhound::open_run`].
    pub fn open_run(&self, run_id: &RunId) -> Result<RunView, Error> {
        self.connection.open_run(run_id)
    }

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the connection runs with, as
    /// [`crate::Foxhound::explain_settings`] says, which `send()` asks.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        self.connection.explain_settings()
    }

    /// Closes the connection for every clone, as
    /// [`crate::Foxhound::close`] does, and returns once its linked engine
    /// has stopped.
    ///
    /// # Errors
    ///
    /// As [`crate::Foxhound::close`].
    pub fn close(&self) -> Result<(), Error> {
        self.linked.close_blocking()
    }
}

/// The blocking Connection both Products' blocking connections implement
/// (ADRs 0095 and 0099), so a helper taking `&impl blocking::Connection`
/// takes a `Foxhound`; its members are the inherent ones above.
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

    fn close(&self) -> Result<(), Error> {
        Foxhound::close(self)
    }
}

/// The client crate's connection, the way down to its typed `call` of a
/// runtime method, which the raw `call` (ADR 0099) does not type.
impl AsRef<blocking::ClientConnection> for Foxhound {
    fn as_ref(&self) -> &blocking::ClientConnection {
        &self.connection
    }
}

/// C-SEND-SYNC: a blocking connection moves between threads, is shared
/// between them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Foxhound>();
};
