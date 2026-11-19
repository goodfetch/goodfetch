//! What every handle a program holds reaches its engine through (ADR
//! 0099): the Run it belongs to, with that Run's Connection, and the ID
//! that names the handle's value. The members the catalog gives a handle,
//! which `xtask codegen` writes, build their calls here, so a call on any
//! handle of a closed Run, or of a value closed with its parent, fails
//! with [`Error::Closed`] before it is sent, and a reply that says the Run
//! ended notes it, which `close()` and the handle's drop read (ADR 0078).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use goodfetch_protocol::page::session::{SESSION_CLOSE, SessionParams};
use goodfetch_protocol::page::session_pages::{PAGE_CLOSE, PageCloseParams};
use goodfetch_protocol::page::{Locator, PageId, SessionId};
use goodfetch_protocol::run::{Lifecycle, RUN_ABORT, RunAbortParams, RunId, RunReport, StartedAs};
use serde::de::DeserializeOwned;
use serde::{Deserialize as _, Serialize};
use serde_json::Value;

use crate::call::{Call, Query};
use crate::connection::{ClientConnection, Closes, close_timeout};
use crate::error::Error;
use crate::sync::{AtomicBool, Mutex, MutexGuard};

/// What a handle reaches its engine through: the Run it belongs to and its
/// own ID. Cloning it is cheap and every clone reaches the same value.
#[derive(Clone, Debug)]
pub(crate) struct Host<Id> {
    id: Id,
    run: Arc<HeldRun>,
    /// The value the handle names, which its last clone's drop closes;
    /// none for the Run's own handle, whose replies say whether the Run
    /// ended.
    value: Option<Arc<HeldValue>>,
}

impl Host<RunId> {
    /// The host of the Run `run`, which its Connection closes first.
    pub(crate) fn run(run: HeldRun) -> Self {
        let run = Arc::new(run);
        run.connection.register(&run);
        Self {
            id: run.run_id.clone(),
            run,
            value: None,
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
    pub(crate) fn connection(&self) -> &ClientConnection {
        &self.run.connection
    }

    /// The host of a value this handle opened or built, named `id`, in the
    /// same Run: it closes with this handle's value.
    pub(crate) fn child<C: Held>(&self, id: C) -> Host<C> {
        let value = HeldValue {
            run: Arc::clone(&self.run),
            parent: self.value.clone(),
            closed: AtomicBool::new(false),
            dropped: id.dropped(),
        };
        Host {
            id,
            run: Arc::clone(&self.run),
            value: Some(Arc::new(value)),
        }
    }

    /// The call of `method` with `params`, whose reply reads as `T`.
    pub(crate) fn call<T, P>(&self, method: &'static str, params: P) -> Call<T, P>
    where
        T: DeserializeOwned + Send + 'static,
        P: Serialize + Send + 'static,
    {
        let call = Call::new(self.connection().sender(), method, self.open(params));
        if self.value.is_none() {
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
    /// says nothing more. Once the value is closed, by its own close or
    /// with its Run or a parent, it answers at once and sends nothing; a
    /// close that fails leaves the value open. It waits at most
    /// `timeout.close` unless the program sets its own timeout.
    pub(crate) fn close<P>(&self, method: &'static str, params: P) -> Call<(), P>
    where
        P: Serialize + Send + 'static,
    {
        let (run, value) = (Arc::clone(&self.run), self.value.clone());
        let marked = self.value.clone();
        Call::<Value, P>::new(self.connection().sender(), method, Ok(params))
            .settled(move || closed(&run, value.as_deref()).then_some(Value::Null))
            .note(move |_| {
                if let Some(value) = marked {
                    value.closed.store(true, Ordering::Release);
                }
            })
            .timeout(close_timeout())
            .map(drop)
    }

    /// `params`, or [`Error::Closed`] once the value is closed.
    fn open<P>(&self, params: P) -> Result<P, Error> {
        if closed(&self.run, self.value.as_deref()) {
            Err(Error::Closed)
        } else {
            Ok(params)
        }
    }
}

/// Whether the value `value` names in `run`, or the Run itself when it
/// names none, is closed: by its own close, a parent's, or the Run's.
fn closed(run: &HeldRun, value: Option<&HeldValue>) -> bool {
    run.state() == State::Closed || value.is_some_and(HeldValue::closed)
}

/// A value of a Run a program holds, which every handle of it shares: a
/// Session, a page or a locator, each in the value that opened or built
/// it. Dropping its last handle while it is open and its Run runs fires
/// its close without waiting, and warns (ADR 0078).
#[derive(Debug)]
struct HeldValue {
    run: Arc<HeldRun>,
    /// The value it closes with, beside its Run.
    parent: Option<Arc<HeldValue>>,
    /// Whether its own close succeeded.
    closed: AtomicBool,
    /// What its drop sends while it is open, if anything.
    dropped: Option<Dropped>,
}

impl HeldValue {
    /// Whether it, or a value it closes with, is closed.
    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.parent.as_deref().is_some_and(Self::closed)
    }
}

impl Drop for HeldValue {
    fn drop(&mut self) {
        let Some(dropped) = self.dropped.take() else {
            return;
        };
        if self.closed() || self.run.state() != State::Running {
            return;
        }
        tracing::warn!(
            target: "goodfetch::client",
            id = %dropped.id,
            "a {} was dropped without close(): it closes without waiting",
            dropped.value
        );
        self.run
            .connection
            .send_and_forget(dropped.method, &dropped.params);
    }
}

/// A value's ID, which says what dropping its last handle unclosed sends.
pub(crate) trait Held {
    /// The close that dropping the value's last handle sends while it is
    /// open, or none for a value the engine keeps nothing of.
    fn dropped(&self) -> Option<Dropped>;
}

/// The close a value's drop sends.
#[derive(Debug)]
pub(crate) struct Dropped {
    /// What the value is, as the warning names it.
    value: &'static str,
    /// Its ID, as the warning names it.
    id: String,
    method: &'static str,
    params: Value,
}

impl Dropped {
    fn new(
        value: &'static str,
        id: String,
        method: &'static str,
        params: &impl Serialize,
    ) -> Option<Self> {
        let params = serde_json::to_value(params).ok()?;
        Some(Self {
            value,
            id,
            method,
            params,
        })
    }
}

impl Held for SessionId {
    fn dropped(&self) -> Option<Dropped> {
        let params = SessionParams {
            session_id: self.clone(),
        };
        Dropped::new("Session", self.0.clone(), SESSION_CLOSE.name, &params)
    }
}

impl Held for PageId {
    fn dropped(&self) -> Option<Dropped> {
        let params = PageCloseParams {
            page_id: self.clone(),
            run_before_unload: None,
        };
        Dropped::new("page", self.0.clone(), PAGE_CLOSE.name, &params)
    }
}

impl Held for Locator {
    fn dropped(&self) -> Option<Dropped> {
        None
    }
}

/// A Run a program holds, which every handle of it shares.
#[derive(Debug)]
pub(crate) struct HeldRun {
    connection: ClientConnection,
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
        connection: ClientConnection,
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

    /// Whether the start began the Run, went on with it, or found it ended.
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
            .timeout(close_timeout())
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
