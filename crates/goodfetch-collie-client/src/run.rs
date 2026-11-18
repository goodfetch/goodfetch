//! The Run a program drives (ADRs 0078 and 0102): a Connection starts it,
//! the program finishes it for its Run report, and closing it stops it if
//! it still runs. Its members are the ones the catalog gives a Run, which
//! `xtask codegen` writes (ADR 0099); this module adds its start and its
//! close. Its handles are cheap clones of one Run, `Send + Sync +
//! 'static`, and closing any of them closes it for all.

use std::future::{Future, IntoFuture};
use std::pin::Pin;

use goodfetch_protocol::run::{RUN_START, RunId, RunStartParams, RunStarted, StartedAs};

use crate::connection::Connection;
use crate::error::Error;
use crate::generated::handles::Run;
use crate::host::{HeldRun, Host};

/// The start of a Run (`run.start`), which awaiting sends. Nothing is sent
/// until it is awaited.
#[must_use = "a start is sent only when it is awaited"]
#[derive(Debug)]
pub struct Start {
    connection: Connection,
    params: RunStartParams,
}

impl Start {
    /// The start of a Run on `connection`, with no Run ID: the engine
    /// names it.
    pub(crate) fn new(connection: Connection) -> Self {
        Self {
            connection,
            params: RunStartParams::default(),
        }
    }

    /// Names the Run `run_id`: a start with the ID of a Run started before
    /// with the same inputs goes on with it when it was interrupted, and
    /// returns it as it ended when it finished (ADR 0102).
    pub fn run_id(mut self, run_id: &RunId) -> Self {
        self.params.run_id = Some(run_id.as_str().to_owned());
        self
    }
}

impl IntoFuture for Start {
    type Output = Result<Run, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Run, Error>> + Send + 'static>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let started: RunStarted = self
                .connection
                .call(RUN_START.name, &self.params)
                .no_timeout()
                .await?;
            let held = HeldRun::new(
                self.connection,
                started.run_id,
                started.started_as,
                started.lifecycle,
            );
            Ok(Run {
                host: Host::run(held),
            })
        })
    }
}

impl Run {
    /// Whether the start began the Run, went on with it, or found it ended.
    pub fn started_as(&self) -> StartedAs {
        self.host.held_run().started_as()
    }

    /// Closes the Run for every clone: a Run that still runs stops at its
    /// next safe point, `stopped` and `incomplete`, as an abort by its
    /// program (`run.abort`), resumable by its Run ID; an ended one stays
    /// as it is. Any later call on it, or on a handle it opened, fails
    /// with [`Error::Closed`]. Closing again does nothing.
    ///
    /// # Errors
    ///
    /// What `run.abort` fails with, unless the Connection is closed: its
    /// engine then stops the Run itself.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after its first poll leaves the value closed
    /// and the abort sent.
    pub async fn close(&self) -> Result<(), Error> {
        self.host.held_run().close().await
    }
}

impl Connection {
    /// The start of a Run this Connection drives (`run.start`), which
    /// awaiting sends.
    pub fn start(&self) -> Start {
        Start::new(self.clone())
    }
}

/// C-SEND-SYNC: a Run's handles and its start move between threads, are
/// shared between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Run>();
    send_sync_static::<crate::generated::handles::Page>();
    send_static::<Start>();
    send_static::<<Start as IntoFuture>::IntoFuture>();
};
