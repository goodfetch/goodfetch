//! The Kennel's Client of the Daemon (ADR 0141), over `goodfetch-protocol`'s
//! types alone: it reads the running Daemon's runtime file in the Goodfetch
//! home, opens its socket, says `hello` as the `kennel` surface needing
//! Collie, which both Products contain, and reads the Daemon's Product from
//! the answer. It frames JSON-RPC by line. Its interface is the stream of
//! connection changes it reports; later tickets add the events and the
//! typed calls. It is the one module that could later move into a shared
//! client crate.
//!
//! The Daemon's token is read from the runtime file at each connection,
//! held in memory only, sent only in `hello`, and never logged: the
//! protocol's `DaemonToken` prints `[REDACTED]`, and no `hello` is logged.

use std::io;
use std::pin::Pin;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use goodfetch_protocol::connection::{
    DaemonEndpoint, HELLO, HelloParams, HelloResult, PROTOCOL_VERSION_UNSUPPORTED, ProductVersion,
    ProtocolVersionUnsupportedDetails, Surface,
};
use goodfetch_protocol::daemon::DaemonRuntimeFile;
use goodfetch_protocol::jsonrpc::{ErrorObject, Message, Request, RequestId, Version};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::sync::mpsc;

use crate::home::Home;
use crate::tasks::Tasks;

/// The id of the Kennel's `hello`, its first request on each Connection.
const HELLO_ID: i64 = 1;

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
}

/// Follows the Daemon until the Kennel stops: looks for it in `home`,
/// connects, and reports each change. Once no Daemon answers, it looks
/// again only when `look_again` asks (after a "Start Daemon"); a lost
/// Connection makes it look again at once.
///
/// # Cancel safety
/// Dropping the future closes the Connection; nothing else is lost.
pub async fn follow(
    tasks: Tasks,
    home: Home,
    mut look_again: mpsc::Receiver<()>,
    report: impl Fn(ConnectionChange) + Send + Sync + 'static,
) {
    loop {
        let lookup = home.clone();
        let found = tasks.blocking(move || lookup.running_daemon()).await;
        let reached = match found {
            None => None,
            Some(runtime) => match Connection::open(&runtime).await {
                Ok(connection) => Some(connection),
                Err(Opening::Unreachable) => None,
                Err(Opening::Refused(refusal)) => {
                    tracing::warn!(?refusal, "the Daemon refused the Kennel");
                    report(ConnectionChange::Refused(refusal));
                    if look_again.recv().await.is_none() {
                        return;
                    }
                    continue;
                }
            },
        };
        if let Some(connection) = reached {
            tracing::info!(
                product = connection.identity.product.as_str(),
                version = %connection.identity.product_version,
                "reached the Daemon"
            );
            report(ConnectionChange::Connected(connection.identity.clone()));
            connection.closed().await;
            tracing::info!("lost the Daemon");
        } else {
            report(ConnectionChange::NoDaemon);
            if look_again.recv().await.is_none() {
                return;
            }
        }
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
    /// Kept so the Connection stays open until the Kennel drops it.
    _writer: Writer,
    identity: DaemonIdentity,
}

impl Connection {
    /// Opens the Connection to the Daemon of `runtime` with `hello`.
    async fn open(runtime: &DaemonRuntimeFile) -> Result<Self, Opening> {
        let (reader, mut writer) = connect(&runtime.endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut lines = BufReader::new(reader).lines();
        send(&mut writer, &hello_request(runtime))
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
                        _writer: writer,
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

    /// Waits until the Daemon closes the Connection.
    async fn closed(mut self) {
        while let Ok(Some(_)) = self.lines.next_line().await {}
    }
}

/// The Kennel's `hello` to the Daemon of `runtime`.
fn hello_request(runtime: &DaemonRuntimeFile) -> Request {
    let hello = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Kennel,
        form: None,
        token: Some(runtime.token.expose().to_owned()),
        needs: Some(Product::Collie),
        product_version: kennel_version(),
        log_level: None,
        client_surface: None,
        keep_daemon_alive: false,
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
