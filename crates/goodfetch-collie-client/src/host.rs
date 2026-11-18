//! What every handle a program holds reaches its engine through (ADR
//! 0099): the Run it belongs to, with that Run's Connection, and the ID
//! that names the handle's value. The members the catalog gives a handle,
//! which `xtask codegen` writes, build their calls here, so a call on any
//! handle of a closed Run fails with [`Error::Closed`] before it is sent,
//! and a reply that says the Run ended notes it, which `close()` and the
//! handle's drop read.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};

use goodfetch_protocol::run::{Lifecycle, RUN_ABORT, RunAbortParams, RunId, RunReport, StartedAs};
use serde::de::DeserializeOwned;
use serde::{Deserialize as _, Serialize};
use serde_json::Value;

use crate::call::{Call, Query};
use crate::connection::{Closes, Connection};
use crate::error::Error;
use crate::sync::{Mutex, MutexGuard};

/// What a handle reaches its engine through: the Run it belongs to and its
/// own ID. Cloning it is cheap and every clone reaches the same value.
#[derive(Clone, Debug)]
pub(crate) struct Host<Id> {
    id: Id,
    run: Arc<HeldRun>,
    /// Whether the handle is the Run's own, whose replies say whether the
    /// Run ended.
    of_run: bool,
}

impl Host<RunId> {
    /// The host of the Run `run`, which its Connection closes first.
    pub(crate) fn run(run: HeldRun) -> Self {
        let run = Arc::new(run);
        run.connection.register(&run);
        Self {
            id: run.run_id.clone(),
            run,
            of_run: true,
        }
    }
}

impl<Id> Host<Id> {
    /// The ID that names the handle's value.
    pub(crate) fn id(&self) -> &Id {
        &self.id
    }

    /// The Run the handle belongs to.
    pub(crate) fn held_run(&self) -> &HeldRun {
        &self.run
    }

    /// The Connection the handle's calls go through.
    pub(crate) fn connection(&self) -> &Connection {
        &self.run.connection
    }

    /// The host of a value this handle opened or built, named `id`, in the
    /// same Run.
    pub(crate) fn child<C>(&self, id: C) -> Host<C> {
        Host {
            id,
            run: Arc::clone(&self.run),
            of_run: false,
        }
    }

    /// The call of `method` with `params`, whose reply reads as `T`.
    pub(crate) fn call<T, P>(&self, method: &'static str, params: P) -> Call<T, P>
    where
        T: DeserializeOwned + Send + 'static,
        P: Serialize + Send + 'static,
    {
        let call = Call::new(self.connection().sender(), method, self.open(params));
        if self.of_run {
            let run = Arc::clone(&self.run);
            call.note(move |reply| run.note(reply))
        } else {
            call
        }
    }

    /// The query of `method` with `params`, whose answer reads as `T`.
    pub(crate) fn query<T, P>(&self, method: &'static str, params: P) -> Query<T, P>
    where
        T: DeserializeOwned + Send + 'static,
        P: Serialize + Send + 'static,
    {
        Query::new(self.connection().sender(), method, self.open(params))
    }

    /// The call of `method` that closes the handle's value, whose reply
    /// says nothing more.
    pub(crate) fn close<P>(&self, method: &'static str, params: P) -> Call<(), P>
    where
        P: Serialize + Send + 'static,
    {
        self.call::<Value, P>(method, params).map(drop)
    }

    /// `params`, or [`Error::Closed`] once the Run is closed.
    fn open<P>(&self, params: P) -> Result<P, Error> {
        if self.run.state() == State::Closed {
            Err(Error::Closed)
        } else {
            Ok(params)
        }
    }
}

/// A Run a program holds, which every handle of it shares.
#[derive(Debug)]
pub(crate) struct HeldRun {
    connection: Connection,
    run_id: RunId,
    started_as: StartedAs,
    state: Mutex<State>,
}

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

impl HeldRun {
    /// The Run `run.start` answered, on `connection`.
    pub(crate) fn new(
        connection: Connection,
        run_id: RunId,
        started_as: StartedAs,
        lifecycle: Lifecycle,
    ) -> Self {
        let state = if lifecycle == Lifecycle::Running {
            State::Running
        } else {
            State::Ended
        };
        Self {
            connection,
            run_id,
            started_as,
            state: Mutex::new(state),
        }
    }

    /// Whether the start began the Run, found it, or went on with it.
    pub(crate) fn started_as(&self) -> StartedAs {
        self.started_as
    }

    /// Closes the Run for every handle, as [`crate::Run::close`] says.
    pub(crate) async fn close(&self) -> Result<(), Error> {
        let was = std::mem::replace(&mut *self.lock(), State::Closed);
        if was != State::Running {
            return Ok(());
        }
        let params = RunAbortParams {
            run_id: self.run_id.clone(),
            forwarded: false,
        };
        let aborted: Result<RunReport, Error> = self
            .connection
            .call(RUN_ABORT.name, &params)
            .no_timeout()
            .await;
        match aborted {
            Ok(_) | Err(Error::Closed) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Notes that the Run ended when `reply` says so: a Run report or a
    /// progress whose `lifecycle` is no longer `running`.
    fn note(&self, reply: &Value) {
        let lifecycle = reply
            .get("lifecycle")
            .and_then(|lifecycle| Lifecycle::deserialize(lifecycle).ok());
        if lifecycle.is_some_and(|lifecycle| lifecycle != Lifecycle::Running) {
            let mut state = self.lock();
            if *state == State::Running {
                *state = State::Ended;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn state(&self) -> State {
        *self.lock()
    }
}

impl Closes for HeldRun {
    fn close(self: Arc<Self>) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        Box::pin(async move { HeldRun::close(&self).await })
    }
}

impl Drop for HeldRun {
    fn drop(&mut self) {
        let state = *self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        if state == State::Running {
            tracing::warn!(
                target: "goodfetch::client",
                run_id = %self.run_id,
                "a running Run was dropped without close(): it is aborted without waiting"
            );
            self.connection.send_and_forget(
                RUN_ABORT.name,
                &RunAbortParams {
                    run_id: self.run_id.clone(),
                    forwarded: false,
                },
            );
        }
    }
}
