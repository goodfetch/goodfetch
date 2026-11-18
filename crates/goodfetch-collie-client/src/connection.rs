//! The Connection: what a program holds to talk to one engine (ADR 0081).
//! It takes its runtime as a value, opens its transport and says `hello` on
//! that runtime, and hands out the call builders and the Runs it starts.
//! Its handles are cheap clones, `Send + Sync + 'static`; closing it closes
//! its Runs first (ADR 0078), and dropping its last handle shuts a runtime
//! it owns down in the background, never on the async worker that drops
//! it. It saves a file to the program's path (ADR 0116): it hands the
//! engine the path, or writes the bytes the engine streams.

use std::fmt::Debug;
use std::future::Future;
use std::panic;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, PoisonError, Weak};

use data_encoding::BASE64;
use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::{CHUNK, Chunk};
use serde::Serialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Value;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::call::{Call, Query, Reply};
use crate::error::Error;
use crate::file_saving::{self, FileSaving};
use crate::rpc::{self, Heard, Outgoing, Sender};
use crate::sync::{Mutex, MutexGuard};
use crate::transport::{self, Endpoint, Network};

/// How many `$/chunk` notifications a save takes ahead of those it has
/// written: the credits it grants when its call is sent, then one more for
/// each chunk written.
const CHUNKS_AHEAD: u32 = 16;

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
    pub fn call<T>(&self, method: &str, params: &impl Serialize) -> Call<T> {
        Call::new(self.0.sender.clone(), method, to_value(params))
    }

    /// The call of `method` with `params`, or one that fails with their
    /// error when it is awaited.
    pub(crate) fn call_or_fail<T>(
        &self,
        method: &str,
        params: Result<impl Serialize, Error>,
    ) -> Call<T> {
        let params = params.and_then(|params| to_value(&params));
        Call::new(self.0.sender.clone(), method, params)
    }

    /// The query of `method` with `params`, whose answer reads as `T`.
    pub fn query<T>(&self, method: &str, params: &impl Serialize) -> Query<T> {
        Query::new(self.0.sender.clone(), method, to_value(params))
    }

    /// Saves a file to the program's `path` through the call of `method`
    /// with `params`, whose reply reads as `T`; `call` names the call the
    /// program made, which a failure's fix names. A relative path resolves
    /// against the working directory. Every engine a Connection reaches is
    /// on the program's machine, linked, Attached or a Daemon on a local
    /// socket, so the call carries the absolute path and the engine writes
    /// it under ADR 0116's rule; an engine that cannot reach the path
    /// streams the file in `$/chunk` notifications under `$/credit`, and
    /// answers `written: false`, and the Connection writes the chunks to
    /// the path, whole or not at all, on its tracker.
    ///
    /// # Errors
    ///
    /// [`Error::PathNotWritable`] or [`Error::FileWriteFailed`] when the
    /// Connection cannot write the path, and whatever the call fails with;
    /// none leaves a file beside the path or changes it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future removes the file it was writing; the path holds
    /// what it held before.
    pub fn save_file<T: DeserializeOwned + Send + 'static>(
        &self,
        method: &str,
        params: &impl Serialize,
        path: &Path,
        call: &str,
    ) -> Reply<T> {
        let connection = self.clone();
        let method = method.to_owned();
        let params = to_value(params);
        let path = path.to_owned();
        let call = call.to_owned();
        Box::pin(async move {
            let path = file_saving::absolute(&path, &call)?;
            let mut params = params?;
            let Value::Object(fields) = &mut params else {
                return Err(Error::Shape(serde_json::Error::custom(
                    "a save's params are an object",
                )));
            };
            fields.insert("path".to_owned(), to_value(&path)?);
            let result = connection
                .saved(method, params, Saving::new(path, call))
                .await?;
            serde_json::from_value(result).map_err(Error::Shape)
        })
    }

    /// Sends the save's call of `method` with `params` and writes what it
    /// streams to `saving`'s path, until its reply.
    async fn saved(
        &self,
        method: String,
        params: Value,
        mut saving: Saving,
    ) -> Result<Value, Error> {
        let (notified, mut heard) = mpsc::unbounded_channel();
        let (reply, mut replied) = oneshot::channel();
        self.0.sender.call(Outgoing {
            method,
            params,
            timeout: None,
            reply,
            notified: Some(notified),
        });
        let result = loop {
            tokio::select! {
                biased;
                Some(heard) = heard.recv() => self.hear(heard, &mut saving).await?,
                result = &mut replied => break result.map_err(|_| Error::Closed)??,
            }
        };
        while let Ok(heard) = heard.try_recv() {
            self.hear(heard, &mut saving).await?;
        }
        if result.get("written").and_then(Value::as_bool) != Some(true) {
            self.finish(saving).await?;
        }
        Ok(result)
    }

    /// Acts on what a save's call heard: grants it its first credits once
    /// it is sent, and writes each chunk, then grants one more.
    async fn hear(&self, heard: Heard, saving: &mut Saving) -> Result<(), Error> {
        match heard {
            Heard::Sent(id) => {
                saving.id = Some(id);
                self.credit(id, CHUNKS_AHEAD)
            }
            Heard::Notified { method, params } if method == CHUNK => {
                let chunk: Chunk = serde_json::from_value(params).map_err(Error::Shape)?;
                let bytes = BASE64.decode(chunk.data.as_bytes()).map_err(|error| {
                    Error::Shape(serde_json::Error::custom(format!(
                        "a chunk's data is no Base64: {error}"
                    )))
                })?;
                let (path, call, file) =
                    (saving.path.clone(), saving.call.clone(), saving.file.take());
                let file = self
                    .blocking(move || {
                        let mut file = match file {
                            Some(file) => file,
                            None => FileSaving::create(&path, &call)?,
                        };
                        file.write(&bytes)?;
                        Ok(file)
                    })
                    .await?;
                saving.file = Some(file);
                if let Some(id) = saving.id {
                    self.credit(id, 1)?;
                }
                Ok(())
            }
            Heard::Notified { .. } => Ok(()),
        }
    }

    /// Writes the rest of a save that streamed its bytes: an empty file
    /// when none came, then renamed over its path.
    async fn finish(&self, mut saving: Saving) -> Result<(), Error> {
        let (path, call, file) = (saving.path.clone(), saving.call.clone(), saving.file.take());
        self.blocking(move || {
            let file = match file {
                Some(file) => file,
                None => FileSaving::create(&path, &call)?,
            };
            file.finish()
        })
        .await
    }

    /// Grants the streaming call `id` `credits` more chunks.
    fn credit(&self, id: i64, credits: u32) -> Result<(), Error> {
        let params = to_value(&CreditParams {
            id: RequestId::Number(id),
            credits,
        })?;
        self.0.sender.notify(CREDIT, params);
        Ok(())
    }

    /// Runs the blocking `work` on the Connection's tracker. Dropping the
    /// future lets it end, and drops what it returns.
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        joined(self.0.tasks.spawn_blocking_on(work, &self.0.handle).await)?
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
            notified: None,
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

/// A save under way: where it writes, the call that writes it, its call's
/// id once sent, and the file it has started, if any chunk came.
struct Saving {
    path: PathBuf,
    call: String,
    id: Option<i64>,
    file: Option<FileSaving>,
}

impl Saving {
    /// A save to `path` that `call` makes, nothing written yet.
    fn new(path: PathBuf, call: String) -> Self {
        Self {
            path,
            call,
            id: None,
            file: None,
        }
    }
}

/// What a Connection closes before itself: a Run it started. The `run`
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
