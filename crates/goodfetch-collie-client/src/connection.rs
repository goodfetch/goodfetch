//! The Connection: what a program holds to talk to one engine (ADR 0081).
//! It takes its runtime as a value, opens its transport and says `hello` on
//! that runtime, and hands out the call builders and the Runs it starts.
//! Its handles are cheap clones, `Send + Sync + 'static`; closing it closes
//! its Runs first (ADR 0078), and dropping its last handle shuts a runtime
//! it owns down in the background, never on the async worker that drops
//! it. It saves a file to the program's path (ADR 0116): it hands the
//! engine the path, or writes the bytes the engine streams.

use std::fmt::Debug;
use std::future::{Future, IntoFuture};
use std::io;
use std::marker::PhantomData;
use std::panic;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError, Weak};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult};
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::log::{CONNECTION_SET_LOG_LEVEL, LogLevel, LogLevelSet, SetLogLevelParams};
use goodfetch_protocol::page::{BodyHandle, CHUNK, Chunk};
use goodfetch_protocol::settings::TIMEOUT_CLOSE;
use serde::Serialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Value;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::call::{Call, Query, Reply};
use crate::connect_timeout::{self, ConnectTimeout};
use crate::error::Error;
use crate::file_saving::{self, FileSaving};
use crate::handler::{HandlerRequests, Streams};
use crate::logging;
use crate::published::Published;
use crate::raw_call::RawCall;
use crate::rpc::{self, Heard, Outgoing, Sender};
use crate::sync::{AtomicBool, Mutex, MutexGuard};
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
pub struct ClientConnection(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    sender: Sender,
    /// The streams of the engine's requests the program serves.
    streams: Streams,
    /// Its own Subscription, which the streams of the events the engine
    /// publishes share.
    published: Published,
    /// The runtime its tasks run on.
    handle: Handle,
    tasks: TaskTracker,
    /// The calls `blocking` hands to the runtime, which `close()` never
    /// waits for, as one of them may be that `close()`.
    handed: TaskTracker,
    /// The Runs it started, which `close()` closes first. A Run holds its
    /// Connection, so the Connection only refers to it.
    runs: Mutex<Vec<Weak<dyn Closes>>>,
    /// The level its engine was last told to send its logs at (ADR 0078):
    /// `hello`'s, then each one a start told it.
    log_level: Mutex<LogLevel>,
    /// Whether a close has returned, after which closing again does
    /// nothing, even when that close timed out with a task still running.
    closed: AtomicBool,
    /// Held only to be dropped with the Connection's last handle.
    _runtime: Owned,
}

impl ClientConnection {
    /// Opens a Connection to the engine at `endpoint` over `network`, on
    /// `runtime`, and says `hello`, waiting for its answer as long as it
    /// takes.
    ///
    /// # Errors
    ///
    /// `connection_lost` when the transport cannot reach the engine,
    /// and whatever `hello`'s call fails with.
    pub async fn open(
        runtime: ConnectionRuntime,
        network: &Network,
        endpoint: &Endpoint,
        hello: HelloParams,
    ) -> Result<(Self, HelloResult), Error> {
        Self::open_within(runtime, network, endpoint, hello, ConnectTimeout::Off).await
    }

    /// Opens a Connection as [`ClientConnection::open`] does, waiting for
    /// `hello`'s answer within `timeout` (ADR 0083); past it, the
    /// Connection is closed.
    ///
    /// # Errors
    ///
    /// As [`ClientConnection::open`], [`ConnectTimeout::check`]'s refusal,
    /// and `deadline_exceeded` when `hello`'s answer
    /// did not come within `timeout`, its `limit` naming `timeout.connect`
    /// and the layer that set it, and `waited_for` `hello`.
    pub async fn open_within(
        runtime: ConnectionRuntime,
        network: &Network,
        endpoint: &Endpoint,
        hello: HelloParams,
        timeout: ConnectTimeout,
    ) -> Result<(Self, HelloResult), Error> {
        timeout.check()?;
        let (handle, owned) = runtime.split();
        let tasks = TaskTracker::new();
        let (network, endpoint) = (network.clone(), endpoint.clone());
        let opening = tasks.spawn_on(
            async move { transport::open(&network, &endpoint).await },
            &handle,
        );
        let opened = joined(opening.await)?.map_err(Error::unreachable)?;
        if let Some(process) = opened.process {
            tasks.spawn_on(process.watch(), &handle);
        }
        let streams = Streams::new();
        let sender = rpc::start(opened.link, &handle, &tasks, streams.clone());
        let published = Published::new(sender.clone(), handle.clone(), tasks.clone());
        let connection = Self(Arc::new(Inner {
            sender,
            streams,
            published,
            handle,
            tasks,
            handed: TaskTracker::new(),
            runs: Mutex::new(Vec::new()),
            log_level: Mutex::new(hello.log_level.unwrap_or_default()),
            closed: AtomicBool::new(false),
            _runtime: owned,
        }));
        let deadline = timeout.deadline();
        let said = match &deadline {
            Some((after, _)) => connection
                .call::<HelloResult>(HELLO, &hello)
                .timeout(*after),
            None => connection.call::<HelloResult>(HELLO, &hello).no_timeout(),
        };
        match said.await {
            Ok(said) => Ok((connection, said)),
            Err(error) => {
                connection.close().await?;
                Err(match (error, deadline) {
                    (error, Some((after, limit)))
                        if !error.is_answer() && error.code() == ErrorCode::DEADLINE_EXCEEDED =>
                    {
                        connect_timeout::exceeded(after, limit)
                    }
                    (error, _) => error,
                })
            }
        }
    }

    /// Tells the engine `level`, which the program's subscriber records
    /// for [`logging::ENGINE`], with `connection.set_log_level` when it is
    /// not the level the engine was last told, and sends nothing when it
    /// is (ADR 0078). An engine that refuses it keeps sending its logs at
    /// the level it had, which the crate warns of under
    /// [`logging::CLIENT`]: the engine's logs are telemetry, so the
    /// refusal fails nothing, and the next start tells it again.
    ///
    /// # Errors
    ///
    /// `closed` when the Connection is closed.
    pub(crate) async fn follow_log_level(&self, level: LogLevel) -> Result<(), Error> {
        let told = std::mem::replace(&mut *self.log_level(), level);
        if told == level {
            return Ok(());
        }
        let params = SetLogLevelParams { log_level: level };
        match self
            .call::<LogLevelSet>(CONNECTION_SET_LOG_LEVEL.name, &params)
            .await
        {
            Ok(LogLevelSet {}) => Ok(()),
            Err(error) if error.code() == ErrorCode::CLOSED => Err(error),
            Err(error) => {
                let mut current = self.log_level();
                if *current == level {
                    *current = told;
                }
                drop(current);
                tracing::warn!(
                    target: logging::CLIENT,
                    log_level = level.as_str(),
                    %error,
                    "The engine did not take the log level `{}`; it keeps sending its logs at `{}`.",
                    level.as_str(),
                    told.as_str(),
                );
                Ok(())
            }
        }
    }

    /// The call of `method` with `params`, whose reply reads as `T`.
    pub fn call<T: DeserializeOwned>(&self, method: &str, params: &impl Serialize) -> Call<T> {
        Call::new(self.sender(), method, to_value(params))
    }

    /// The raw call of `operation` (ADR 0099), its params and its result
    /// the wire's JSON, which awaiting sends: the Products' `call`.
    pub fn raw_call(&self, operation: &str) -> RawCall {
        RawCall::new(self.sender(), operation, None)
    }

    /// The query of `method` with `params`, whose answer reads as `T`.
    pub fn query<T: DeserializeOwned>(&self, method: &str, params: &impl Serialize) -> Query<T> {
        Query::new(self.sender(), method, to_value(params))
    }

    /// The handle the calls of the Connection's handles go through.
    pub(crate) fn sender(&self) -> Sender {
        self.0.sender.clone()
    }

    /// Its own Subscription, which the streams of the events the engine
    /// publishes share.
    pub(crate) fn published(&self) -> &Published {
        &self.0.published
    }

    /// The save of a file to the program's `path` through the call of
    /// `method` with `params`, whose reply reads as `T`, which awaiting it
    /// sends; `call` names the call the program made, which a failure's fix
    /// names. A relative path resolves against the working directory. Every
    /// engine a Connection reaches is on the program's machine, linked,
    /// Attached or a Daemon on a local socket, so the call carries the
    /// absolute path and the engine writes it under ADR 0116's rule; an
    /// engine that cannot reach the path streams the file in `$/chunk`
    /// notifications under `$/credit`, and answers `written: false`, and
    /// the Connection writes the chunks to the path, whole or not at all,
    /// on its tracker.
    pub fn save_file<T>(
        &self,
        method: &str,
        params: &impl Serialize,
        path: &Path,
        call: &str,
    ) -> SaveFile<T> {
        SaveFile {
            connection: self.clone(),
            method: method.to_owned(),
            params: to_value(params),
            path: path.to_owned(),
            call: call.to_owned(),
            timeout: None,
            reply: PhantomData,
        }
    }

    /// Sends the save's call of `method` with `params` and writes what it
    /// streams to `saving`'s path, until its reply.
    async fn saved(
        &self,
        method: String,
        params: Value,
        timeout: Option<Duration>,
        mut saving: Saving,
    ) -> Result<Value, Error> {
        // The call's id, then at most a chunk for each credit granted.
        let (notified, mut heard) = mpsc::channel(CHUNKS_AHEAD as usize + 1);
        let (reply, mut replied) = oneshot::channel();
        self.0.sender.call(Outgoing {
            method,
            params,
            timeout,
            reply,
            notified: Some(notified),
        });
        let result = loop {
            tokio::select! {
                biased;
                Some(heard) = heard.recv() => self.hear(heard, &mut saving).await?,
                result = &mut replied => break result.map_err(|_| Error::closed()),
            }
        };
        // A call the engine answered is over; one the Connection failed, as
        // timed out, is not, and the dropped save stops it.
        saving.answered = match &result {
            Ok(Ok(_)) => true,
            Ok(Err(error)) => error.is_answer(),
            Err(_) => false,
        };
        let result = result??;
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
                self.0.sender.credit(id, CHUNKS_AHEAD);
                Ok(())
            }
            Heard::Notified { method, params } if method == CHUNK => {
                let chunk: Chunk = serde_json::from_value(params).map_err(Error::shape)?;
                let bytes = BASE64.decode(chunk.data.as_bytes()).map_err(|error| {
                    Error::shape(serde_json::Error::custom(format!(
                        "a chunk's data is no Base64: {error}"
                    )))
                })?;
                let (given, path, call, file) = (
                    saving.given.clone(),
                    saving.path.clone(),
                    saving.call.clone(),
                    saving.file.take(),
                );
                let file = self
                    .blocking(move || {
                        let mut file = match file {
                            Some(file) => file,
                            None => FileSaving::create_at(&given, path, &call)?,
                        };
                        file.write(&bytes)?;
                        Ok(file)
                    })
                    .await?;
                saving.file = Some(file);
                if let Some(id) = saving.id {
                    self.0.sender.credit(id, 1);
                }
                Ok(())
            }
            Heard::Notified { .. } => Ok(()),
        }
    }

    /// Writes the rest of a save that streamed its bytes: an empty file
    /// when none came, then renamed over its path.
    async fn finish(&self, mut saving: Saving) -> Result<(), Error> {
        let (given, path, call, file) = (
            saving.given.clone(),
            saving.path.clone(),
            saving.call.clone(),
            saving.file.take(),
        );
        self.blocking(move || {
            let file = match file {
                Some(file) => file,
                None => FileSaving::create_at(&given, path, &call)?,
            };
            file.finish()
        })
        .await
    }

    /// The bytes of the page body `handle` names, read from the file the
    /// engine keeps it in, which a Client on the same machine reads (ADR
    /// 0118).
    pub(crate) async fn read_body(&self, handle: BodyHandle) -> Result<Vec<u8>, Error> {
        self.blocking(move || {
            let path = PathBuf::from(handle.path);
            let unreadable = |source| Error::body_unreadable(&path, source);
            #[expect(
                clippy::disallowed_methods,
                reason = "a blocking task on the Connection's tracker reads the file"
            )]
            let bytes = std::fs::read(&path).map_err(unreadable)?;
            if u64::try_from(bytes.len()).ok() != Some(handle.size) {
                return Err(unreadable(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "the file holds {} bytes where the body has {}",
                        bytes.len(),
                        handle.size
                    ),
                )));
            }
            Ok(bytes)
        })
        .await
    }

    /// Runs the blocking `work` on the Connection's tracker. Dropping the
    /// future lets it end, and drops what it returns.
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        joined(self.0.tasks.spawn_blocking_on(work, &self.0.handle).await)?
    }

    /// The engine's requests of `method`, such as the Crawl requests a
    /// crawl's handlers answer (ADR 0093), from now until the Connection
    /// ends or the stream is dropped. The engine's requests of a method no
    /// stream serves are answered with the protocol's method error.
    ///
    /// # Errors
    ///
    /// `internal` while another stream serves `method`, and
    /// `closed` once the Connection has ended.
    pub fn handler_requests(&self, method: &str) -> Result<HandlerRequests, Error> {
        self.0.streams.open(method)
    }

    /// Closes the Connection: first every Run it started that is still
    /// open, as [`crate::Run::close`] does, each closed at once even when
    /// `timeout.close` runs out before its abort, then every call in flight fails with
    /// `closed`, as does every later one; the engine reads the end
    /// of its input, and an Attached engine exits. Returns once the
    /// Connection's tasks have ended, or once `timeout.close` has run out.
    /// Closing again does nothing. A Connection to a Daemon only
    /// disconnects: the Daemon goes on.
    ///
    /// # Errors
    ///
    /// The first error a Run's close failed with, and `deadline_exceeded`
    /// when `timeout.close` ran out first; the Connection closes all the
    /// same.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after its first poll leaves every Run it
    /// started closed, the aborts it had not sent unsent, and the
    /// Connection open.
    pub async fn close(&self) -> Result<(), Error> {
        if self.0.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        // The timer is the Connection's runtime's, so the close runs on
        // any executor (ADR 0081).
        let bounded = {
            let _entered = self.0.handle.enter();
            tokio::time::timeout(close_timeout(), self.close_unbounded())
        };
        let closed = match bounded.await {
            Ok(closed) => closed,
            Err(_elapsed) => {
                self.0.sender.close();
                self.0.tasks.close();
                Err(Error::timed_out())
            }
        };
        self.0.closed.store(true, Ordering::Release);
        closed
    }

    /// Closes the Connection as [`ClientConnection::close`] says, however
    /// long it takes.
    async fn close_unbounded(&self) -> Result<(), Error> {
        // Every Run is closed before the first abort is sent, so one the
        // close's timeout cut short before its abort is closed all the same.
        let closing: Vec<_> = std::mem::take(&mut *self.runs())
            .iter()
            .filter_map(Weak::upgrade)
            .map(Closes::close)
            .collect();
        let mut closed = Ok(());
        for run in closing {
            let result = run.await;
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
        ended.recv().unwrap_or_else(|_| Err(Error::closed()))
    }

    /// The level its engine was last told, held.
    fn log_level(&self) -> MutexGuard<'_, LogLevel> {
        self.0
            .log_level
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The Runs it started, which a panic elsewhere leaves as they were.
    fn runs(&self) -> MutexGuard<'_, Vec<Weak<dyn Closes>>> {
        self.0.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The save of a file to the program's path, whose reply reads as `T`.
/// Nothing is sent until it is awaited.
///
/// Awaiting it fails with `path_not_writable` or
/// `file_write_failed` when the Connection cannot write the path,
/// and with whatever the call fails with; none leaves a file beside the
/// path or changes it.
///
/// # Cancel safety
///
/// Dropping the future removes the file it was writing and stops the
/// engine's call with `$/cancel`; the path holds what it held before.
#[must_use = "a save is sent only when it is awaited"]
pub struct SaveFile<T> {
    connection: ClientConnection,
    method: String,
    params: Result<Value, Error>,
    path: PathBuf,
    call: String,
    timeout: Option<Duration>,
    reply: PhantomData<fn() -> T>,
}

impl<T> SaveFile<T> {
    /// Fails the save with `deadline_exceeded` if its reply has not come
    /// within `timeout`, the file it was writing removed.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Waits for the reply as long as the Connection lasts.
    pub fn no_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for SaveFile<T> {
    type Output = Result<T, Error>;
    type IntoFuture = Reply<T>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            method,
            params,
            path: given,
            call,
            timeout,
            reply: _,
        } = self;
        Box::pin(async move {
            let path = file_saving::absolute(&given, &call)?;
            let mut params = params?;
            let Value::Object(fields) = &mut params else {
                return Err(Error::shape(serde_json::Error::custom(
                    "a save's params are an object",
                )));
            };
            fields.insert("path".to_owned(), to_value(&path)?);
            let saving = Saving {
                given,
                path,
                call,
                id: None,
                file: None,
                answered: false,
                sender: connection.0.sender.clone(),
            };
            let result = connection.saved(method, params, timeout, saving).await?;
            serde_json::from_value(result).map_err(Error::shape)
        })
    }
}

/// A save under way: the path as given and resolved, the call that writes
/// it, its call's id once sent, the file it has started, if any chunk
/// came, and whether its reply came. Dropped before its reply, it stops
/// the engine's call.
struct Saving {
    given: PathBuf,
    path: PathBuf,
    call: String,
    id: Option<i64>,
    file: Option<FileSaving>,
    answered: bool,
    sender: Sender,
}

impl Drop for Saving {
    fn drop(&mut self) {
        let Some(id) = self.id.filter(|_| !self.answered) else {
            return;
        };
        self.sender.cancel(id);
    }
}

/// What a Connection closes before itself: a Run it started. The `host`
/// module, above the Connection, implements it (ADR 0126).
pub(crate) trait Closes: Debug + Send + Sync {
    /// Closes it at once, as [`crate::Run::close`] does; the future sends
    /// what the engine is told.
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

/// How long a close waits: `timeout.close`'s default (ADR 0078).
pub(crate) fn close_timeout() -> Duration {
    let millis = (TIMEOUT_CLOSE.default)()
        .as_u64()
        .expect("invariant: `timeout.close` defaults to a duration in milliseconds");
    Duration::from_millis(millis)
}

/// `params` as the call sends them.
fn to_value(params: &impl Serialize) -> Result<serde_json::Value, Error> {
    serde_json::to_value(params).map_err(Error::shape)
}

/// What a task on the Connection's runtime returned. A panic in it carries
/// on in the caller; a task cancelled because its runtime shut down means
/// the Connection is closed.
fn joined<T>(result: Result<T, JoinError>) -> Result<T, Error> {
    result.map_err(|error| {
        if error.is_panic() {
            panic::resume_unwind(error.into_panic());
        }
        Error::closed()
    })
}

/// C-SEND-SYNC: a Connection's handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<ClientConnection>();
    send_sync_static::<ConnectionRuntime>();
};
