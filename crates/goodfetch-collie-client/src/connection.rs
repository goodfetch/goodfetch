//! The Connection: what a program holds to talk to one engine (ADR 0081).
//! It takes its runtime as a value, opens its transport and says `hello` on
//! that runtime, and hands out the call builders and the Runs it starts.
//! Its handles are cheap clones, `Send + Sync + 'static`; closing it closes
//! its Runs first (ADR 0078), and dropping its last handle shuts a runtime
//! it owns down in the background, never on the async worker that drops
//! it.

use std::fmt::Debug;
use std::future::Future;
use std::panic;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, Weak};

use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::oneshot;
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::call::{Call, Query};
use crate::error::Error;
use crate::rpc::{self, Outgoing, Sender};
use crate::sync::{Mutex, MutexGuard};
use crate::transport::{self, Endpoint, Network};

/// The runtime a Connection's tasks run on.
#[derive(Debug)]
pub struct ConnectionRuntime(Kind);

#[derive(Debug)]
enum Kind {
    /// One the Connection owns and shuts down with its last handle.
    Owned(Runtime),
    /// One the program drives, which outlives the Connection.
    Driven(Handle),
}

impl ConnectionRuntime {
    /// A runtime the Connection owns, which `goodfetch-runtime` builds. It
    /// runs its tasks on its own worker threads, so it must be a
    /// multi-thread one: nothing drives a current-thread runtime the
    /// Connection owns. Dropping the Connection's last handle shuts it
    /// down in the background.
    pub fn owned(runtime: Runtime) -> Self {
        Self(Kind::Owned(runtime))
    }

    /// The runtime behind `handle`, which the program drives and which
    /// outlives the Connection: an executable's own, or the test helper's
    /// paused current-thread runtime, whose `block_on` drives the
    /// Connection's tasks with the test's.
    pub fn driven(handle: Handle) -> Self {
        Self(Kind::Driven(handle))
    }

    /// Its handle, and the runtime itself when the Connection owns it.
    fn split(self) -> (Handle, Owned) {
        match self.0 {
            Kind::Owned(runtime) => (runtime.handle().clone(), Owned(Some(runtime))),
            Kind::Driven(handle) => (handle, Owned(None)),
        }
    }
}

/// The runtime a Connection owns, if any, shut down in the background
/// when it is dropped: dropping a runtime on an async worker would panic.
#[derive(Debug)]
struct Owned(Option<Runtime>);

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

/// A Connection to one engine. Cloning it is cheap and every clone is the
/// same Connection.
#[derive(Clone, Debug)]
pub struct Connection(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    sender: Sender,
    /// The runtime its tasks run on.
    handle: Handle,
    tasks: TaskTracker,
    /// The calls `blocking` hands to the runtime, which `close()` never
    /// waits for, as one of them may be that `close()`.
    handed: TaskTracker,
    /// The Runs it started, which `close()` closes first. A Run holds its
    /// Connection, so the Connection only refers to it.
    runs: Mutex<Vec<Weak<dyn Closes>>>,
    /// Held only to be dropped with the Connection's last handle.
    _runtime: Owned,
}

impl Connection {
    /// Opens a Connection to the engine at `endpoint` over `network`, on
    /// `runtime`, and says `hello`.
    ///
    /// # Errors
    ///
    /// [`Error::Unreachable`] when the transport cannot reach the engine,
    /// and whatever `hello`'s call fails with.
    pub async fn open(
        runtime: ConnectionRuntime,
        network: &Network,
        endpoint: &Endpoint,
        hello: HelloParams,
    ) -> Result<(Self, HelloResult), Error> {
        let (handle, owned) = runtime.split();
        let tasks = TaskTracker::new();
        let (network, endpoint) = (network.clone(), endpoint.clone());
        let opening = tasks.spawn_on(
            async move { transport::open(&network, &endpoint).await },
            &handle,
        );
        let opened = joined(opening.await)?.map_err(Error::Unreachable)?;
        if let Some(process) = opened.process {
            tasks.spawn_on(process.watch(), &handle);
        }
        let sender = rpc::start(opened.link, &handle, &tasks);
        let connection = Self(Arc::new(Inner {
            sender,
            handle,
            tasks,
            handed: TaskTracker::new(),
            runs: Mutex::new(Vec::new()),
            _runtime: owned,
        }));
        match connection.call::<HelloResult>(HELLO, &hello).await {
            Ok(said) => Ok((connection, said)),
            Err(error) => {
                connection.close().await?;
                Err(error)
            }
        }
    }

    /// The call of `method` with `params`, whose reply reads as `T`.
    pub fn call<T: DeserializeOwned>(&self, method: &str, params: &impl Serialize) -> Call<T> {
        Call::new(self.sender(), method, to_value(params))
    }

    /// The query of `method` with `params`, whose answer reads as `T`.
    pub fn query<T: DeserializeOwned>(&self, method: &str, params: &impl Serialize) -> Query<T> {
        Query::new(self.sender(), method, to_value(params))
    }

    /// The handle the calls of the Connection's handles go through.
    pub(crate) fn sender(&self) -> Sender {
        self.0.sender.clone()
    }

    /// Closes the Connection: first every Run it started that is still
    /// open, as [`crate::Run::close`] does, then every call in flight fails with
    /// [`Error::Closed`], as does every later one; the engine reads the end
    /// of its input, and an Attached engine exits. Returns once the
    /// Connection's tasks have ended. Closing again does nothing.
    ///
    /// # Errors
    ///
    /// The first error a Run's close failed with; the Connection closes
    /// all the same.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes leaves the Runs it had not
    /// closed yet open and the Connection open.
    pub async fn close(&self) -> Result<(), Error> {
        let runs: Vec<Arc<dyn Closes>> = std::mem::take(&mut *self.runs())
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let mut closed = Ok(());
        for run in runs {
            let result = run.close().await;
            closed = closed.and(result);
        }
        self.0.sender.close();
        self.0.tasks.close();
        self.0.tasks.wait().await;
        closed
    }

    /// Notes `run` as one `close()` closes, forgetting those already gone.
    pub(crate) fn register<R: Closes + 'static>(&self, run: &Arc<R>) {
        let run: Weak<R> = Arc::downgrade(run);
        let mut runs = self.runs();
        runs.retain(|known| known.strong_count() > 0);
        runs.push(run);
    }

    /// Sends the call of `method` with `params` without waiting for its
    /// reply, which is dropped: a `Drop` that must never block sends it.
    pub(crate) fn send_and_forget(&self, method: &str, params: &impl Serialize) {
        let Ok(params) = serde_json::to_value(params) else {
            return;
        };
        let (reply, _dropped) = oneshot::channel();
        self.0.sender.call(Outgoing {
            method: method.to_owned(),
            params,
            timeout: None,
            reply,
        });
    }

    /// Runs `future` on the Connection's runtime and blocks the calling
    /// thread, and only it, until it ends: `blocking`'s every call. It
    /// never enters a runtime, so it works inside one, where it blocks that
    /// worker; the runtime must be one the Connection owns, or one some
    /// other thread drives.
    pub(crate) fn wait<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T, Error>> + Send + 'static,
    ) -> Result<T, Error> {
        let (done, ended) = std::sync::mpsc::sync_channel(1);
        self.0.handed.spawn_on(
            async move {
                let _gone = done.send(future.await);
            },
            &self.0.handle,
        );
        // The task ends without sending only when the runtime shut down.
        ended.recv().unwrap_or(Err(Error::Closed))
    }

    /// The Runs it started, which a panic elsewhere leaves as they were.
    fn runs(&self) -> MutexGuard<'_, Vec<Weak<dyn Closes>>> {
        self.0.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a Connection closes before itself: a Run it started. The `host`
/// module, above the Connection, implements it (ADR 0126).
pub(crate) trait Closes: Debug + Send + Sync {
    /// Closes it, as [`crate::Run::close`] does.
    fn close(self: Arc<Self>) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.tasks.is_closed() {
            tracing::warn!(
                target: "goodfetch::client",
                "a Connection was dropped without close(): it closes without waiting"
            );
            self.sender.close();
            self.tasks.close();
        }
    }
}

/// `params` as the call sends them.
fn to_value(params: &impl Serialize) -> Result<serde_json::Value, Error> {
    serde_json::to_value(params).map_err(Error::Shape)
}

/// What a task on the Connection's runtime returned. A panic in it carries
/// on in the caller; a task cancelled because its runtime shut down means
/// the Connection is closed.
fn joined<T>(result: Result<T, JoinError>) -> Result<T, Error> {
    result.map_err(|error| {
        if error.is_panic() {
            panic::resume_unwind(error.into_panic());
        }
        Error::Closed
    })
}

/// C-SEND-SYNC: a Connection's handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Connection>();
    send_sync_static::<ConnectionRuntime>();
};
