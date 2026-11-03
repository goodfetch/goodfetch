//! The Kennel's Client of the Daemon (ADR 0141), over `goodfetch-protocol`'s
//! types alone: it reads the running Daemon's runtime file in the Goodfetch
//! home, opens its socket, says `hello` as the `kennel` surface needing
//! Collie, which both Products contain, and reads the Daemon's Product from
//! the answer. It frames JSON-RPC by line. Its interface is the stream of
//! connection changes it reports; later tickets add the events and the
//! typed calls. It is the one module that could later move into a shared
//! client crate.
//!
//! It follows the Daemon's life without polling and without holding it
//! awake: the platform's file notifications on the runtime files make it
//! look again when a Daemon starts, stops or hands over to its major's
//! pick (`runtime_files`), and its `hello` asks the Daemon to stay alive
//! only when the keep-alive setting is on. Its one poll is `daemon.health`,
//! on a slow timer, while connected.
//!
//! The Daemon's token is read from the runtime file at each connection,
//! held in memory only, sent only in `hello`, and never logged: the
//! protocol's `DaemonToken` prints `[REDACTED]`, and no `hello` is logged.

use std::io;
use std::pin::Pin;
use std::time::Duration;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use goodfetch_protocol::connection::{
    DaemonEndpoint, HELLO, HelloParams, HelloResult, PROTOCOL_VERSION_UNSUPPORTED, ProductVersion,
    ProtocolVersionUnsupportedDetails, Surface,
};
use goodfetch_protocol::daemon::{self, DaemonHealth, DaemonParams, DaemonRuntimeFile};
use goodfetch_protocol::jsonrpc::{ErrorObject, Message, Request, RequestId, Version};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::home::Home;
use crate::tasks::Tasks;
use runtime_files::RuntimeFiles;

mod runtime_files;

/// The id of the Kennel's `hello`, its first request on each Connection.
const HELLO_ID: i64 = 1;

/// How often the Kennel asks the reached Daemon `daemon.health`, the one
/// poll ADR 0141 allows: slow, so that an idle Kennel costs nothing.
pub const HEALTH_EVERY: Duration = Duration::from_secs(30);

/// Who the reached Daemon is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonIdentity {
    /// Its Product, by which later tickets hide Foxhound's views from a
    /// Collie Daemon.
    pub product: Product,
    /// Its Product version.
    pub product_version: ProductVersion,
}

/// Why a running Daemon refused the Kennel's `hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// It speaks another protocol version.
    ProtocolVersion {
        /// The version it speaks.
        engine: ProtocolVersion,
        /// The version this Kennel needs.
        needs: ProtocolVersion,
    },
    /// It is handing over to its major's newer version (ADR 0098).
    HandOver,
    /// Another refusal, by its Error code.
    Other {
        /// The Error code.
        code: String,
    },
}

/// A change of the Kennel's Connection to the Daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionChange {
    /// No Daemon runs, or the one that ran is gone.
    NoDaemon,
    /// A Daemon answered `hello`.
    Connected(DaemonIdentity),
    /// A Daemon runs and refused the Kennel.
    Refused(Refusal),
    /// The reached Daemon's answer to the health poll, `daemon.health`.
    Health(DaemonHealth),
}

/// What the Client follows the Daemon with.
#[derive(Debug)]
pub struct Follow {
    /// The Goodfetch home, whose per-major folders hold the Daemons'
    /// runtime files.
    pub home: Home,
    /// Asks the Client to look for the Daemon again, as after a "Start
    /// Daemon", where the platform cannot watch the home.
    pub look_again: mpsc::Receiver<()>,
    /// The keep-alive setting: when on, `hello` asks the Daemon to count
    /// the Connection as a Client that keeps it alive; a change reconnects.
    pub keep_daemon_alive: watch::Receiver<bool>,
    /// How often the Client asks the reached Daemon `daemon.health`.
    pub health_every: Duration,
}

/// Follows the Daemon until the Kennel stops: looks for it in the home,
/// connects, and reports each change. It looks again whenever the
/// platform's file notifications report a change to a runtime file, which
/// a Daemon that starts or hands over to its major's pick makes, never by
/// polling; and when `look_again` asks. A lost Connection makes it look
/// again at once, and so does a runtime file that now names another
/// Daemon, or a change of the keep-alive setting, which `hello` carries.
/// While connected, it asks the Daemon `daemon.health` every
/// `health_every`, and reports each answer that differs from the last.
///
/// # Cancel safety
/// Dropping the future closes the Connection; nothing else is lost.
pub async fn follow(
    tasks: Tasks,
    follow: Follow,
    report: impl Fn(ConnectionChange) + Send + Sync + 'static,
) {
    let mut follower = Follower {
        tasks,
        follow,
        runtime_files: RuntimeFiles::new(),
        report,
    };
    follower.run().await;
}

/// The Client while it follows the Daemon.
struct Follower<R> {
    tasks: Tasks,
    follow: Follow,
    runtime_files: RuntimeFiles,
    report: R,
}

/// Why the Client left a Connection.
#[derive(Debug)]
enum Left {
    /// The Daemon closed it.
    Lost,
    /// The runtime files name another Daemon now.
    Replaced,
    /// The keep-alive setting changed, which only a new `hello` carries.
    KeepAliveChanged,
}

impl<R: Fn(ConnectionChange)> Follower<R> {
    /// Follows until the Kennel stops.
    async fn run(&mut self) {
        loop {
            // The watch covers every per-major folder before the look, so
            // no change after the look goes unreported.
            self.runtime_files
                .rewatch(&self.tasks, &self.follow.home)
                .await;
            let keep_daemon_alive = *self.follow.keep_daemon_alive.borrow_and_update();
            let opened = match self.running_daemon().await {
                None => Err(Opening::Unreachable),
                Some(runtime) => Connection::open(&runtime, keep_daemon_alive)
                    .await
                    .map(|connection| (connection, runtime)),
            };
            match opened {
                Ok((connection, runtime)) => {
                    tracing::info!(
                        product = connection.identity.product.as_str(),
                        version = %connection.identity.product_version,
                        keep_daemon_alive,
                        "reached the Daemon"
                    );
                    (self.report)(ConnectionChange::Connected(connection.identity.clone()));
                    let left = self.serve(connection, &runtime, keep_daemon_alive).await;
                    tracing::info!(?left, "left the Daemon");
                    continue;
                }
                Err(Opening::Refused(refusal)) => {
                    tracing::warn!(?refusal, "the Daemon refused the Kennel");
                    (self.report)(ConnectionChange::Refused(refusal));
                }
                Err(Opening::Unreachable) => (self.report)(ConnectionChange::NoDaemon),
            }
            tokio::select! {
                asked = self.follow.look_again.recv() => {
                    if asked.is_none() {
                        return;
                    }
                }
                () = self.runtime_files.changed() => {}
            }
        }
    }

    /// The running Daemon's runtime file.
    async fn running_daemon(&self) -> Option<DaemonRuntimeFile> {
        let home = self.follow.home.clone();
        self.tasks.blocking(move || home.running_daemon()).await
    }

    /// Serves `connection`, opened with the runtime file `reached` and
    /// the keep-alive setting `keep_daemon_alive`, until the Client
    /// leaves it.
    async fn serve(
        &mut self,
        mut connection: Connection,
        reached: &DaemonRuntimeFile,
        keep_daemon_alive: bool,
    ) -> Left {
        // The first poll comes one period after `hello`, not at once.
        let mut polls = tokio::time::interval(self.follow.health_every);
        polls.reset();
        polls.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut health = Health::default();
        let mut setting_followed = true;
        loop {
            tokio::select! {
                line = connection.lines.next_line() => {
                    let Ok(Some(line)) = line else {
                        return Left::Lost;
                    };
                    if let Some(reading) = health.answered(&line) {
                        (self.report)(ConnectionChange::Health(reading));
                    }
                }
                _ = polls.tick() => {
                    if send(&mut connection.writer, &health.ask()).await.is_err() {
                        return Left::Lost;
                    }
                }
                () = self.runtime_files.changed() => {
                    self.runtime_files.rewatch(&self.tasks, &self.follow.home).await;
                    let found = self.running_daemon().await;
                    if found.is_some_and(|found| !same_daemon(&found, reached)) {
                        return Left::Replaced;
                    }
                }
                changed = self.follow.keep_daemon_alive.changed(), if setting_followed => {
                    // A closed channel keeps the setting the Connection has.
                    setting_followed = changed.is_ok();
                    if setting_followed
                        && *self.follow.keep_daemon_alive.borrow_and_update() != keep_daemon_alive
                    {
                        return Left::KeepAliveChanged;
                    }
                }
            }
        }
    }
}

/// Whether two runtime files name the same Daemon: a Daemon's token may
/// change while it runs, its process and endpoint do not.
fn same_daemon(one: &DaemonRuntimeFile, other: &DaemonRuntimeFile) -> bool {
    one.pid == other.pid && one.endpoint == other.endpoint
}

/// The health poll of one Connection.
#[derive(Debug, Default)]
struct Health {
    /// The id of the last request sent, which no other request reuses.
    asked: Option<i64>,
    /// The last answer reported.
    last: Option<DaemonHealth>,
}

impl Health {
    /// The next `daemon.health` request.
    fn ask(&mut self) -> Request {
        let id = self.asked.map_or(HELLO_ID + 1, |id| id + 1);
        self.asked = Some(id);
        Request {
            jsonrpc: Version,
            id: RequestId::Number(id),
            method: daemon::HEALTH.name.to_owned(),
            params: Some(
                serde_json::to_value(DaemonParams {}).expect("invariant: daemon.health encodes"),
            ),
        }
    }

    /// The reading `line` gives, when it answers the last request and
    /// differs from the last reading.
    fn answered(&mut self, line: &str) -> Option<DaemonHealth> {
        let Ok(Message::Response(response)) = serde_json::from_str::<Message>(line) else {
            return None;
        };
        if Some(response.id) != self.asked.map(RequestId::Number) {
            return None;
        }
        let reading = serde_json::from_value::<DaemonHealth>(response.result).ok()?;
        if self.last.as_ref() == Some(&reading) {
            return None;
        }
        tracing::info!(
            live = reading.live,
            ready = reading.ready,
            unready = ?reading.unready,
            "the Daemon's health"
        );
        self.last = Some(reading.clone());
        Some(reading)
    }
}

/// Why opening a Connection failed.
#[derive(Debug)]
enum Opening {
    /// Nothing answered at the endpoint, or it closed before answering.
    Unreachable,
    /// The Daemon refused the Kennel.
    Refused(Refusal),
}

type Reader = Pin<Box<dyn AsyncRead + Send>>;
type Writer = Pin<Box<dyn AsyncWrite + Send>>;

/// An open Connection, after its `hello`.
struct Connection {
    lines: Lines<BufReader<Reader>>,
    writer: Writer,
    identity: DaemonIdentity,
}

impl Connection {
    /// Opens the Connection to the Daemon of `runtime` with `hello`,
    /// asking it to stay alive when `keep_daemon_alive` is on.
    async fn open(runtime: &DaemonRuntimeFile, keep_daemon_alive: bool) -> Result<Self, Opening> {
        let (reader, mut writer) = connect(&runtime.endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut lines = BufReader::new(reader).lines();
        send(&mut writer, &hello_request(runtime, keep_daemon_alive))
            .await
            .map_err(|_| Opening::Unreachable)?;
        loop {
            let Ok(Some(line)) = lines.next_line().await else {
                return Err(Opening::Unreachable);
            };
            match serde_json::from_str::<Message>(&line) {
                Ok(Message::Response(response)) if response.id == RequestId::Number(HELLO_ID) => {
                    let result: HelloResult =
                        serde_json::from_value(response.result).map_err(|_| {
                            Opening::Refused(Refusal::Other {
                                code: "invalid_hello_result".to_owned(),
                            })
                        })?;
                    let identity = DaemonIdentity {
                        product: result.product,
                        product_version: result.product_version,
                    };
                    return Ok(Self {
                        lines,
                        writer,
                        identity,
                    });
                }
                // An error whose id is null answers a message the Daemon
                // could not read, which can only be `hello`.
                Ok(Message::Error(failure))
                    if failure.id.is_none() || failure.id == Some(RequestId::Number(HELLO_ID)) =>
                {
                    return Err(Opening::Refused(refusal(failure.error)));
                }
                Ok(_) | Err(_) => {}
            }
        }
    }
}

/// The Kennel's `hello` to the Daemon of `runtime`, which carries the
/// keep-alive flag only when `keep_daemon_alive` is on.
fn hello_request(runtime: &DaemonRuntimeFile, keep_daemon_alive: bool) -> Request {
    let hello = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Kennel,
        form: None,
        token: Some(runtime.token.expose().to_owned()),
        needs: Some(Product::Collie),
        product_version: kennel_version(),
        log_level: None,
        client_surface: None,
        keep_daemon_alive,
    };
    Request {
        jsonrpc: Version,
        id: RequestId::Number(HELLO_ID),
        method: HELLO.to_owned(),
        params: Some(serde_json::to_value(hello).expect("invariant: hello encodes")),
    }
}

/// The Kennel's own version, which its `hello` names (ADR 0141).
fn kennel_version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version")
}

/// What a refused `hello` says.
fn refusal(error: ErrorObject) -> Refusal {
    let details = Value::Object(error.data.details);
    if details.get("handover").and_then(Value::as_bool) == Some(true) {
        return Refusal::HandOver;
    }
    if error.data.code == PROTOCOL_VERSION_UNSUPPORTED.name
        && let Ok(details) = serde_json::from_value::<ProtocolVersionUnsupportedDetails>(details)
    {
        return Refusal::ProtocolVersion {
            engine: details.engine_protocol_version,
            needs: PROTOCOL_VERSION,
        };
    }
    Refusal::Other {
        code: error.data.code,
    }
}

/// Writes `request` as one line.
async fn send(writer: &mut Writer, request: &Request) -> io::Result<()> {
    let mut line = serde_json::to_vec(request).expect("invariant: a request encodes");
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Opens the Daemon's socket: a Unix socket on macOS and Linux, loopback
/// TCP on Windows (ADR 0024).
#[expect(
    clippy::disallowed_types,
    reason = "the Kennel's Client is the one module of the Kennel that opens a socket, its Connection to the local Daemon (ADR 0141)"
)]
async fn connect(endpoint: &DaemonEndpoint) -> io::Result<(Reader, Writer)> {
    match endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            let (reader, writer) = tokio::net::UnixStream::connect(path).await?.into_split();
            Ok((Box::pin(reader), Box::pin(writer)))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a Unix socket on a host that has none",
        )),
        DaemonEndpoint::Tcp(address) => {
            let stream = tokio::net::TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let (reader, writer) = stream.into_split();
            Ok((Box::pin(reader), Box::pin(writer)))
        }
    }
}
