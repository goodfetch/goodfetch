//! The Connection: what a program holds to talk to one engine (ADR 0081).
//! It takes its runtime as a value, opens its transport and says `hello` on
//! that runtime, and hands out the call builders and the Runs it starts.
//! Its handles are cheap clones, `Send + Sync + 'static`; closing it closes
//! its Runs first (ADR 0078), and dropping its last handle shuts a runtime
//! it owns down in the background, never on the async worker that drops
//! it. It saves a file to the program's path (ADR 0116): it hands the
//! engine the path, or writes the bytes the engine streams. A remote
//! Daemon's Connection reaches its engine again by itself once the link
//! drops, within `timeout.connect` (spec 606; see `relink`).

use std::fmt::Debug;
use std::future::{Future, IntoFuture};
use std::io;
use std::marker::PhantomData;
use std::panic;
use std::path::{Path, PathBuf};
use std::pin::{Pin, pin};
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult};
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::jsonrpc::META;
use goodfetch_protocol::log::{CONNECTION_SET_LOG_LEVEL, LogLevel, LogLevelSet, SetLogLevelParams};
use goodfetch_protocol::page::{BodyHandle, CHUNK, Chunk};
use goodfetch_protocol::settings::{ProjectSettings, TIMEOUT_CLOSE};
use serde::Serialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Value;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::call::{self, Call, Query, Reply};
use crate::call_bounds::CallBounds;
use crate::connect_timeout::{self, ConnectTimeout};
use crate::error::Error;
use crate::event_stream::EventStream;
use crate::file_saving::{self, FileSaving};
use crate::handler::{HandlerRequests, Streams};
use crate::listener_queue::ListenerQueue;
use crate::logging;
use crate::published::Published;
use crate::raw_call::RawCall;
use crate::relink::Relinking;
use crate::rpc::{self, Heard, Outgoing, Sender};
use crate::sync::{AtomicBool, Mutex, MutexGuard};
use crate::trace_context::{self, TraceContextSource};
use crate::transport::{self, Endpoint, Network};

/// How many `$/chunk` notifications a save takes ahead of those it has
/// written: the credits it grants when its call is sent, then one more for
/// each chunk written.
const CHUNKS_AHEAD: u32 = 16;

/// The runtime a Connection's tasks run on. Cloning it shares the runtime:
/// one the Connection owns shuts down once every clone, and every
/// Connection opened on it, is dropped, so an opening that failed can open
/// another Connection on the same runtime.
#[derive(Clone, Debug)]
pub struct ConnectionRuntime(Kind);

#[derive(Clone, Debug)]
enum Kind {
    /// One the Connection owns and shuts down with its last handle.
    Owned(Arc<Owned>),
    /// One the program drives, which outlives the Connection.
    Driven(Handle),
}

impl ConnectionRuntime {
    /// A runtime the Connection owns, which `goodfetch-runtime` builds. It
    /// runs its tasks on its own worker threads, so it must be a
    /// multi-thread one: nothing drives a current-thread runtime the
    /// Connection owns. Dropping the Connection's last handle, and this
    /// value's last clone, shuts it down in the background.
    pub fn owned(runtime: Runtime) -> Self {
        Self(Kind::Owned(Arc::new(Owned(Some(runtime)))))
    }

    /// The runtime behind `handle`, which the program drives and which
    /// outlives the Connection: an executable's own, or the test helper's
    /// paused current-thread runtime, whose `block_on` drives the
    /// Connection's tasks with the test's.
    pub fn driven(handle: Handle) -> Self {
        Self(Kind::Driven(handle))
    }

    /// Its handle, and the runtime itself when the Connection owns it.
    fn split(self) -> (Handle, Option<Arc<Owned>>) {
        match self.0 {
            Kind::Owned(owned) => (owned.handle(), Some(owned)),
            Kind::Driven(handle) => (handle, None),
        }
    }
}

/// The runtime a Connection owns, if any, shut down in the background
/// when it is dropped: dropping a runtime on an async worker would panic.
#[derive(Debug)]
struct Owned(Option<Runtime>);

impl Owned {
    /// The runtime's handle.
    fn handle(&self) -> Handle {
        self.0
            .as_ref()
            .expect("invariant: an owned runtime is taken only when it is dropped")
            .handle()
            .clone()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

/// A Connection to one engine. Cloning it is cheap and every clone is the
/// same Connection.
///
/// It belongs to the process that opened it: in another, such as the child
/// of a `fork`, each use of it or of a value under it fails with
/// `connection_forked` before anything reaches its engine, and that
/// process opens a Connection of its own (ADR 0078).
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
    /// `hello`'s, then each one a start told it, which a remote Daemon
    /// reached again is greeted with.
    log_level: Arc<Mutex<LogLevel>>,
    /// The project file its program read (ADR 0083), which each Run's
    /// start and its own `config.explain` carry, if it read one.
    project: Option<ProjectSettings>,
    /// Whether a close has returned, after which closing again does
    /// nothing, even when that close timed out with a task still running.
    closed: AtomicBool,
    /// Held only to be dropped with the Connection's last handle.
    _runtime: Option<Arc<Owned>>,
}

impl ClientConnection {
    /// Opens a Connection to the engine at `endpoint` over `network`, on
    /// `runtime`, and says `hello`, waiting for its answer as long as it
    /// takes. The Connection holds its calls within the bounds `hello`'s
    /// own settings give, `connection.max_pending_calls` and
    /// `connection.max_queued_write_bytes`, and sends at most the
    /// `max_calls_in_flight` its engine answers at once, queuing the rest
    /// (issue 3189).
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a bound that is no whole number of 1 or more,
    /// before the engine is reached, `connection_lost` when the transport
    /// cannot reach the engine, and whatever `hello`'s call fails with.
    pub async fn open(
        runtime: ConnectionRuntime,
        network: &Network,
        endpoint: &Endpoint,
        hello: HelloParams,
    ) -> Result<(Self, HelloResult), Error> {
        Self::open_within(
            runtime,
            network,
            endpoint,
            hello,
            ConnectTimeout::Off,
            ListenerQueue::DEFAULT,
            None,
            None,
        )
        .await
    }

    /// Opens a Connection as [`ClientConnection::open`] does, waiting for
    /// `hello`'s answer within `timeout` (ADR 0083); past it, the
    /// Connection is closed. Each stream of its own Subscription, such as a
    /// Session's, holds at most `listener_queue` unread events. Each Run it
    /// starts and its own `config.explain` carry `project`, the project
    /// file its program read, if any (ADR 0083). Each call, `hello`'s
    /// included, carries the Trace context `trace_context_source` reads on
    /// the thread that makes it, and none without it (ADR 0132).
    ///
    /// # Errors
    ///
    /// As [`ClientConnection::open`], [`ConnectTimeout::check`]'s refusal,
    /// and `deadline_exceeded` when the transport's opening, such as an
    /// Exposed Daemon's WebSocket handshake, and `hello`'s answer did not
    /// both come within `timeout`, its `limit` naming `timeout.connect`
    /// and the layer that set it, and `waited_for` `hello`.
    #[expect(
        clippy::too_many_arguments,
        reason = "what a Product's connect() opens a Connection with, each the program's own or the opening's: bundled, they would only travel together to this one call"
    )]
    pub async fn open_within(
        runtime: ConnectionRuntime,
        network: &Network,
        endpoint: &Endpoint,
        hello: HelloParams,
        timeout: ConnectTimeout,
        listener_queue: ListenerQueue,
        project: Option<ProjectSettings>,
        trace_context_source: Option<Arc<dyn TraceContextSource>>,
    ) -> Result<(Self, HelloResult), Error> {
        timeout.check()?;
        let bounds = CallBounds::of(&hello.settings)?;
        let (handle, owned) = runtime.split();
        let tasks = TaskTracker::new();
        let (network, endpoint) = (network.clone(), endpoint.clone());
        let (opened_network, opened_endpoint) = (network.clone(), endpoint.clone());
        let deadline = timeout.deadline();
        // One expiry for the transport's opening and `hello`'s answer: an
        // Exposed Daemon's WebSocket handshake can hang as `hello` can.
        let expiry = deadline
            .as_ref()
            .map(|(after, _)| tokio::time::sleep(*after).deadline());
        let mut opening = tasks.spawn_on(
            async move { transport::open(&opened_network, &opened_endpoint).await },
            &handle,
        );
        let opened = match (expiry, &deadline) {
            (Some(expiry), Some((after, limit))) => {
                let Ok(opened) = tokio::time::timeout_at(expiry, &mut opening).await else {
                    opening.abort();
                    return Err(connect_timeout::exceeded(*after, limit.clone()));
                };
                joined(opened)?
            }
            _ => joined(opening.await)?,
        }
        .map_err(Error::unreachable)?;
        if let Some(process) = opened.process {
            logging::spawn_under_opener(process.watch(), &handle, &tasks);
        }
        let streams = Streams::new();
        let sender = rpc::start(
            opened.link,
            &handle,
            &tasks,
            streams.clone(),
            bounds,
            trace_context_source,
        );
        let published = Published::new(
            sender.clone(),
            handle.clone(),
            tasks.clone(),
            listener_queue,
        );
        let log_level = Arc::new(Mutex::new(hello.log_level.unwrap_or_default()));
        let connection = Self(Arc::new(Inner {
            sender,
            streams,
            published,
            handle,
            tasks,
            handed: TaskTracker::new(),
            runs: Mutex::new(Vec::new()),
            log_level: Arc::clone(&log_level),
            project,
            closed: AtomicBool::new(false),
            _runtime: owned,
        }));
        let said = connection.call::<HelloResult>(HELLO, &hello).no_timeout();
        let said = match expiry {
            Some(expiry) => tokio::time::timeout_at(expiry, said)
                .await
                .unwrap_or_else(|_| Err(Error::timed_out())),
            None => said.await,
        };
        match said {
            Ok(said) => {
                let sender = &connection.0.sender;
                if let Some(cap) = said.max_calls_in_flight {
                    sender.cap_calls_in_flight(cap);
                }
                // A remote Daemon is reached again once its link drops
                // (spec 606).
                if matches!(endpoint, Endpoint::Exposed(_)) {
                    let within = deadline.map(|(after, _)| after);
                    let relinking = Relinking::new(
                        network,
                        endpoint,
                        hello,
                        log_level,
                        within,
                        connection.0.handle.clone(),
                        connection.0.tasks.clone(),
                    );
                    sender.relink_with(relinking, said.engine_instance.clone());
                }
                Ok((connection, said))
            }
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
    /// `closed` when the Connection is closed, and `call_cancelled` when the
    /// program closes it while the level is in flight.
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
            Err(error) if error.is_ended_by_close() => Err(error),
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
        Call::new(self.sender(), method, typed_params(method, params))
    }

    /// The raw call of `operation` (ADR 0099), its params and its result
    /// the wire's JSON, which awaiting sends: the Products' `call`.
    pub fn raw_call(&self, operation: &str) -> RawCall {
        RawCall::new(self.sender(), operation, None)
    }

    /// The query of `method` with `params`, whose answer reads as `T`.
    pub fn query<T: DeserializeOwned>(&self, method: &str, params: &impl Serialize) -> Query<T> {
        Query::new(self.sender(), method, typed_params(method, params))
    }

    /// Sends the call of `method` with `params`, a sequence that streams
    /// one item in each `item` notification, and returns the stream of
    /// those items, each notification's params read as `T`: the way down
    /// for a Product's sequence whose items are not event batches, such as
    /// Foxhound's `run.records` and its `$/record` (ADR 0103). The stream
    /// grants its credits and cancels its call on drop as an
    /// [`EventStream`] of events does, and ends when the call answers.
    pub fn stream<T: DeserializeOwned>(
        &self,
        method: &str,
        params: &impl Serialize,
        item: &'static str,
    ) -> EventStream<T> {
        EventStream::open(
            self.sender(),
            method,
            typed_params(method, params),
            item,
            Box::new(|data| serde_json::from_value(data).map_err(Error::shape)),
        )
    }

    /// Fails unless the calling process opened the Connection, as each
    /// use of it does: a Product crate's connection checks it before it
    /// waits on tasks of its own.
    ///
    /// # Errors
    ///
    /// `connection_forked` in another process, such as the child of a
    /// `fork`.
    pub fn ensure_opener(&self) -> Result<(), Error> {
        self.0.sender.ensure_opener()
    }

    /// Whether the calling process opened the Connection.
    pub(crate) fn in_opener(&self) -> bool {
        self.0.sender.in_opener()
    }

    /// Whether the Connection's `close()` has ended.
    pub(crate) fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
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
            params: typed_params(method, params),
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
            timeout: timeout.map(call::kept_by_client),
            reply,
            notified: Some(notified),
            streams_itself: true,
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
                    .run_blocking(move || {
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
        self.run_blocking(move || {
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
        self.run_blocking(move || {
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

    /// Runs the blocking `work` on one of the Connection's tracked blocking
    /// threads (ADR 0060), never on its runtime's workers, such as the
    /// handlers a Product's blocking API calls (ADR 0081), and returns what
    /// it returns.
    ///
    /// # Errors
    ///
    /// What `work` returns, `closed` once the Connection's runtime has
    /// shut down, and `connection_forked` in a process other than the one
    /// that opened it.
    ///
    /// # Panics
    ///
    /// A panic in `work` resumes in the awaiting future.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not stop `work`: it runs to its end on its
    /// thread, and what it returns is dropped.
    pub async fn run_blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        self.ensure_opener()?;
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
    /// `call_cancelled`, `by: program` and `closed: connection`, and every
    /// later one with `closed`; the engine reads the end
    /// of its input, and an Attached engine exits. Returns once the
    /// Connection's tasks have ended, or once `timeout.close` has run out.
    /// Closing again does nothing. A Connection to a Daemon only
    /// disconnects: the Daemon goes on.
    ///
    /// # Errors
    ///
    /// The first error a Run's close failed with, and `deadline_exceeded`
    /// when `timeout.close` ran out first; the Connection closes all the
    /// same. `connection_forked` in a process other than the one that
    /// opened it, where it closes nothing.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after its first poll leaves every Run it
    /// started closed, the aborts it had not sent unsent, and the
    /// Connection open.
    pub async fn close(&self) -> Result<(), Error> {
        self.ensure_opener()?;
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
        let Ok(mut params) = serde_json::to_value(params) else {
            return;
        };
        trace_context::carry(&mut params, self.0.sender.trace_context());
        let (reply, _dropped) = oneshot::channel();
        self.0.sender.call(Outgoing {
            method: method.to_owned(),
            params,
            timeout: None,
            reply,
            notified: None,
            streams_itself: false,
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
        if !self.in_opener() {
            // In another process, the runtime has no thread to run
            // `future`: one poll here reaches its call's own check, which
            // names the call's operation, and nothing waits.
            let _runtime = self.0.handle.enter();
            let mut context = Context::from_waker(Waker::noop());
            return match pin!(future).poll(&mut context) {
                Poll::Ready(answer) => answer,
                Poll::Pending => Err(Error::connection_forked()),
            };
        }
        let (done, ended) = std::sync::mpsc::sync_channel(1);
        // The runtime's worker has no Trace context of its own: every call
        // `future` makes carries this thread's (ADR 0132).
        let callers = self.0.sender.trace_context();
        self.0.handed.spawn_on(
            trace_context::as_callers(callers, async move {
                let _gone = done.send(future.await);
            }),
            &self.0.handle,
        );
        // The task ends without sending only when the runtime shut down.
        ended.recv().unwrap_or_else(|_| Err(Error::closed()))
    }

    /// The project file it carries (ADR 0083), the one its program read,
    /// as layer 3 of each Run it starts and of its own `config.explain`,
    /// if it read one.
    #[must_use]
    pub fn project(&self) -> Option<&ProjectSettings> {
        self.0.project.as_ref()
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
        let trace_context = connection.0.sender.trace_context();
        Box::pin(async move {
            let path = file_saving::absolute(&given, &call)?;
            let mut params = params?;
            let Value::Object(fields) = &mut params else {
                return Err(Error::shape(serde_json::Error::custom(
                    "a save's params are an object",
                )));
            };
            fields.insert("path".to_owned(), to_value(&path)?);
            trace_context::carry(&mut params, trace_context);
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
        // In another process, such as the child of a `fork`, the tasks are
        // the opener's, which closes them.
        if !self.tasks.is_closed() && self.sender.in_opener() {
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

/// The params of a typed call of `method` as the wire's JSON. A `_meta`
/// among them is refused: the SDK writes a request's envelope, so a typed
/// call is never marked raw (ADR 0099).
fn typed_params(method: &str, params: &impl Serialize) -> Result<serde_json::Value, Error> {
    let params = to_value(params)?;
    if params
        .as_object()
        .is_some_and(|fields| fields.contains_key(META))
    {
        return Err(Error::own_meta(method));
    }
    Ok(params)
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
