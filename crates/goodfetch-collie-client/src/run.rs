//! The Run a program drives (ADRs 0078 and 0102): a Connection starts it,
//! the program finishes it for its Run report, and closing it stops it if
//! it still runs. Its handles are cheap clones of one Run, `Send + Sync +
//! 'static`, and closing any of them closes it for all.

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, PoisonError};

use goodfetch_protocol::run::{
    Lifecycle, RUN_ABORT, RUN_FINISH, RUN_START, RunAbortParams, RunId, RunReport, RunStartParams,
    RunStarted, RunWaitParams, StartedAs,
};

use crate::call::Call;
use crate::connection::{Closes, Connection};
use crate::error::Error;
use crate::sync::{Mutex, MutexGuard};

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
    /// with the same inputs returns that Run, and goes on with one that was
    /// interrupted (ADR 0102).
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
            let state = if started.lifecycle == Lifecycle::Running {
                State::Running
            } else {
                State::Ended
            };
            let inner = Arc::new(Inner {
                connection: self.connection,
                started,
                state: Mutex::new(state),
            });
            inner.connection.register(&inner);
            Ok(Run(inner))
        })
    }
}

/// A Run the program drives. Cloning it is cheap and every clone is the
/// same Run.
#[derive(Clone, Debug)]
pub struct Run(Arc<Inner>);

/// Where a Run's value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// The Run runs.
    Running,
    /// The Run ended; the value is open.
    Ended,
    /// The value is closed.
    Closed,
}

#[derive(Debug)]
struct Inner {
    connection: Connection,
    started: RunStarted,
    state: Mutex<State>,
}

impl Run {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        &self.0.started.run_id
    }

    /// Whether the start began the Run, found it, or went on with it.
    pub fn started_as(&self) -> StartedAs {
        self.0.started.started_as
    }

    /// Lets the Run finish (`run.finish`), and answers its Run report: a
    /// Run its program drives ends at once.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`] once the Run is closed, and what the call fails
    /// with, a Run that ended `failed` among them.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future ends only the wait, as `run.finish` says.
    pub fn finish(&self) -> Call<RunReport> {
        let params = if self.0.state() == State::Closed {
            Err(Error::Closed)
        } else {
            Ok(RunWaitParams {
                run_id: self.id().clone(),
                timeout: None,
            })
        };
        let inner = Arc::clone(&self.0);
        self.0
            .connection
            .call_or_fail(RUN_FINISH.name, params)
            .on_reply(move |report: &RunReport| {
                if report.lifecycle != Lifecycle::Running {
                    inner.ended();
                }
            })
    }

    /// Closes the Run for every clone: a Run that still runs stops at its
    /// next safe point, `stopped` and `incomplete`, as an abort by its
    /// program (`run.abort`), resumable by its Run ID; an ended one stays
    /// as it is. Any later call fails with [`Error::Closed`]. Closing again
    /// does nothing.
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
        let was = std::mem::replace(&mut *self.0.lock(), State::Closed);
        if was != State::Running {
            return Ok(());
        }
        let params = RunAbortParams {
            run_id: self.id().clone(),
            forwarded: false,
        };
        let aborted: Result<RunReport, Error> = self
            .0
            .connection
            .call(RUN_ABORT.name, &params)
            .no_timeout()
            .await;
        match aborted {
            Ok(_) | Err(Error::Closed) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Connection {
    /// The start of a Run this Connection drives (`run.start`), which
    /// awaiting sends.
    pub fn start(&self) -> Start {
        Start::new(self.clone())
    }
}

impl Closes for Inner {
    fn close(self: Arc<Self>) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        Box::pin(async move { Run(self).close().await })
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn state(&self) -> State {
        *self.lock()
    }

    /// Notes that the Run ended, unless the value is closed.
    fn ended(&self) {
        let mut state = self.lock();
        if *state == State::Running {
            *state = State::Ended;
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let state = *self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        if state == State::Running {
            tracing::warn!(
                target: "goodfetch::client",
                run_id = %self.started.run_id,
                "a running Run was dropped without close(): it is aborted without waiting"
            );
            self.connection.send_and_forget(
                RUN_ABORT.name,
                &RunAbortParams {
                    run_id: self.started.run_id.clone(),
                    forwarded: false,
                },
            );
        }
    }
}

/// C-SEND-SYNC: a Run's handles and its start move between threads, are
/// shared between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Run>();
    send_static::<Start>();
    send_static::<<Start as IntoFuture>::IntoFuture>();
};
