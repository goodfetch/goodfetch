//! Collie for a program that does not use async (ADR 0081), under the same
//! names: a call ends in `send()`. The connection owns the runtime it and
//! its linked engine, if any, run on; each call is handed to it, and the calling thread
//! waits for its end on a channel. Nothing here enters a runtime, so it
//! works inside one too, where it blocks only the thread that calls it.

use goodfetch_collie_client::blocking;
use goodfetch_protocol::settings::{ConfigExplainParams, SettingsExplanation};

pub use goodfetch_collie_client::blocking::{
    Call, Connection, EventIter, Exchange, Locator, Page, Query, RawCall, Response, Run, Session,
    Start,
};

use crate::linked::{Composed, ConnectOptions, ProductConnection};
use crate::{EngineInfo, Error};

/// A connection to Collie whose calls block: `collie.start().send()`
/// starts a Run. Cloning it is cheap and every clone is the same
/// connection; `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_collie::ConnectOptions;
/// use goodfetch_collie::blocking::Collie;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// let collie = Collie::connect(options)?;
/// let run = collie.start().send()?;
/// let report = run.finish().send()?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// collie.close()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Collie {
    linked: ProductConnection,
    connection: blocking::ClientConnection,
}

impl Collie {
    /// Opens a connection to the engine `options` choose, as
    /// [`crate::Collie::connect`] does, and blocks the calling thread until
    /// it is open.
    ///
    /// # Errors
    ///
    /// As [`crate::Collie::connect`].
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let linked = ProductConnection::connect_blocking(Composed::COLLIE, options)?;
        Ok(Self {
            connection: blocking::ClientConnection::from(linked.connection().clone()),
            linked,
        })
    }

    /// The engine the connection reached, as [`crate::Collie::engine`]
    /// reports it.
    pub fn engine(&self) -> &EngineInfo {
        self.linked.engine()
    }

    /// The raw call of `operation`, as [`crate::Collie::call`]'s, which
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

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the connection runs with, as
    /// [`crate::Collie::explain_settings`] says, which `send()` asks.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        self.connection.explain_settings()
    }

    /// Closes the connection for every clone, as [`crate::Collie::close`]
    /// does, and returns once its linked engine, if any, has stopped.
    ///
    /// # Errors
    ///
    /// As [`crate::Collie::close`].
    pub fn close(&self) -> Result<(), Error> {
        self.linked.close_blocking()
    }
}

/// The blocking Connection both Products' blocking connections implement
/// (ADRs 0095 and 0099), so a helper taking `&impl blocking::Connection`
/// takes a `Collie`; its members are the inherent ones above.
impl blocking::Connection for Collie {
    fn start(&self) -> Start {
        Collie::start(self)
    }

    fn call(&self, operation: &str) -> RawCall {
        Collie::call(self, operation)
    }

    fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        Collie::explain_settings(self)
    }

    fn close(&self) -> Result<(), Error> {
        Collie::close(self)
    }
}

/// The client crate's connection, the way down to its typed `call` of a
/// runtime method, which the raw `call` (ADR 0099) does not type.
impl AsRef<blocking::ClientConnection> for Collie {
    fn as_ref(&self) -> &blocking::ClientConnection {
        &self.connection
    }
}

/// C-SEND-SYNC: a blocking connection moves between threads, is shared
/// between them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Collie>();
};
