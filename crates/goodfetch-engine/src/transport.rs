//! Every connection the engine serves, taken from a network value (ADR
//! 0060): the OS's, or the in-memory network through which each Product's
//! SDK crate embeds its engine and its tests drive it, a public product
//! adapter. An Attached engine's one Connection is the OS's stdio, or the
//! one an in-memory dialer opens. A Daemon's Connections come from its
//! listener: a `0600` Unix socket on macOS and Linux, loopback TCP on
//! Windows (ADR 0024), or every Connection an in-memory dialer opens; a
//! Client of the Daemon connects to the endpoint its runtime file names. A
//! Daemon a service manager started at the first connection takes the
//! listening socket it was handed instead (launchd's, systemd's).
//! This is the only module that opens stdin, stdout, the protocol's sockets,
//! the Stub gateway's listener, the ops listener (ADR 0050) and the Politeness
//! file's wake endpoints (a Unix socket on macOS and Linux, a named pipe on
//! Windows, ADR 0053); the other listeners join here with the tickets that
//! add them.
//! The engine's own outbound connections, which the Simulation never
//! carries, open in `http_client`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use goodfetch_protocol::daemon::DaemonEndpoint;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

#[cfg(unix)]
mod activation;
mod socket_folder;
mod wake;

pub use socket_folder::{
    LONGEST_SOCKET_PATH, socket_folder, socket_folder_in, temporary_folder_for_sockets,
};
pub use wake::{WakeListener, is_gone, listen_wakes, wake};

/// The name of the socket a launch agent's `Sockets` defines for the
/// Daemon, which it asks launchd for.
pub const LAUNCHD_SOCKETS: &str = "Listeners";

/// How many bytes an in-memory Connection buffers in each direction before
/// its writer waits.
const IN_MEMORY_BUFFER: usize = 64 * 1024;

/// A Connection's bytes, both ways.
pub struct Connection {
    reader: ConnectionReader,
    writer: ConnectionWriter,
}

impl Connection {
    fn new(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        Self {
            reader: ConnectionReader(Box::new(reader)),
            writer: ConnectionWriter(Box::new(writer)),
        }
    }

    /// Its two directions, which separate tasks may own.
    pub fn split(self) -> (ConnectionReader, ConnectionWriter) {
        (self.reader, self.writer)
    }
}

/// What a Connection receives.
pub struct ConnectionReader(Box<dyn AsyncRead + Send + Unpin>);

impl AsyncRead for ConnectionReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

/// What a Connection sends.
pub struct ConnectionWriter(Box<dyn AsyncWrite + Send + Unpin>);

impl AsyncWrite for ConnectionWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

/// The network an engine takes its Connections from.
pub enum Network {
    /// The operating system's: stdio, and later sockets and pipes.
    Os,
    /// The in-memory network of a Product's SDK crate or a test.
    InMemory(InMemoryNetwork),
}

/// Why the engine got no Connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The in-memory network's dialer went away without connecting.
    #[error("the in-memory network's dialer closed before connecting")]
    DialerClosed,
    /// The in-memory network's engine went away.
    #[error("the in-memory network's engine is gone")]
    EngineGone,
    /// One attempt to accept a Connection failed, such as `EMFILE` while
    /// the process has no descriptor left, or a stream the OS refused to
    /// set up: the listener still listens, and the next attempt may pass.
    #[error("the listener failed to accept a Connection: {0}")]
    Accept(io::Error),
    /// The listener is closed for good: the OS says its descriptor is no
    /// listening socket any more, so no later attempt can pass.
    #[error("the listener is closed: {0}")]
    ListenerClosed(io::Error),
}

/// What a failed `accept` on a listener is: the listener's end when the OS
/// says its descriptor is closed or no listening socket, else that one
/// attempt's (`EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM`, `ECONNABORTED` and
/// whatever else leaves the listener listening).
fn accept_failure(error: io::Error) -> TransportError {
    if error.raw_os_error().is_some_and(names_a_closed_listener) {
        TransportError::ListenerClosed(error)
    } else {
        TransportError::Accept(error)
    }
}

#[cfg(unix)]
fn names_a_closed_listener(code: i32) -> bool {
    [libc::EBADF, libc::EINVAL, libc::ENOTSOCK].contains(&code)
}

#[cfg(not(unix))]
fn names_a_closed_listener(code: i32) -> bool {
    /// Winsock's `WSAEBADF`, `WSAEINVAL` and `WSAENOTSOCK`.
    const CLOSED: [i32; 3] = [10009, 10022, 10038];
    CLOSED.contains(&code)
}

impl Network {
    /// The one Connection an Attached engine serves: stdio on the OS, the
    /// first dialed Connection in memory.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it resolves loses no Connection: an
    /// in-memory one waits for the next call.
    pub async fn attached_connection(&mut self) -> Result<Connection, TransportError> {
        match self {
            Self::Os => Ok(stdio()),
            Self::InMemory(network) => network.accept().await,
        }
    }
}

impl Network {
    /// A Daemon's listener: on the OS, a Unix socket at `socket`, readable
    /// and writable by its owner alone, on macOS and Linux, and loopback TCP
    /// on a port the OS chooses on Windows; in memory, every Connection the
    /// dialer opens. Answers the endpoint a runtime file names.
    ///
    /// # Errors
    ///
    /// When the OS refuses the socket, such as a path too long for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves at most a socket file that the next
    /// listener at its path replaces.
    pub async fn listen(self, socket: &Path) -> io::Result<(DaemonListener, DaemonEndpoint)> {
        match self {
            Self::InMemory(network) => Ok((
                DaemonListener(Listening::InMemory(network)),
                DaemonEndpoint::Unix(socket.to_owned()),
            )),
            Self::Os => listen_os(socket).await,
        }
    }

    /// A foreground Daemon's listener (ADR 0049): TCP at `address`, whose
    /// port 0 takes a free one; in memory, every Connection the dialer
    /// opens. Answers the endpoint it listens at, with the port it bound.
    ///
    /// # Errors
    ///
    /// When the OS refuses the address, such as a port another program
    /// holds.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the listener it was opening.
    pub async fn listen_at(
        self,
        address: std::net::SocketAddr,
    ) -> io::Result<(DaemonListener, DaemonEndpoint)> {
        match self {
            Self::InMemory(network) => Ok((
                DaemonListener(Listening::InMemory(network)),
                DaemonEndpoint::Tcp(address),
            )),
            Self::Os => {
                #[expect(
                    clippy::disallowed_types,
                    reason = "the transport module is the one place that opens sockets (ADR 0060)"
                )]
                let listener = tokio::net::TcpListener::bind(address).await?;
                let bound = listener.local_addr()?;
                Ok((
                    DaemonListener(Listening::Tcp(listener)),
                    DaemonEndpoint::Tcp(bound),
                ))
            }
        }
    }
}

impl Network {
    /// The listener the OS's service manager handed this process at its
    /// start, launchd's or systemd's, with the endpoint it answers on;
    /// `None` when none started it with one, and on the in-memory network.
    /// A process takes it once.
    ///
    /// # Errors
    ///
    /// When the service manager's socket cannot be taken, or is no Unix
    /// socket at a path.
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime, which the listener registers with.
    pub fn activated(
        self,
        environment: &BTreeMap<OsString, OsString>,
    ) -> io::Result<Option<(DaemonListener, DaemonEndpoint)>> {
        match self {
            Self::InMemory(_) => Ok(None),
            #[cfg(unix)]
            Self::Os => activated_os(environment),
            #[cfg(not(unix))]
            Self::Os => {
                let _ = environment;
                Ok(None)
            }
        }
    }
}

#[cfg(unix)]
fn activated_os(
    environment: &BTreeMap<OsString, OsString>,
) -> io::Result<Option<(DaemonListener, DaemonEndpoint)>> {
    let Some(descriptor) = activation::take(environment, LAUNCHD_SOCKETS)? else {
        return Ok(None);
    };
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = std::os::unix::net::UnixListener::from(descriptor);
    listener.set_nonblocking(true)?;
    let path = listener
        .local_addr()?
        .as_pathname()
        .map(Path::to_owned)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the service manager's socket is not at a path",
            )
        })?;
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::UnixListener::from_std(listener)?;
    Ok(Some((
        DaemonListener(Listening::Unix(listener)),
        DaemonEndpoint::Unix(path),
    )))
}

#[cfg(unix)]
async fn listen_os(socket: &Path) -> io::Result<(DaemonListener, DaemonEndpoint)> {
    use std::os::unix::fs::PermissionsExt as _;
    match tokio::fs::remove_file(socket).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::UnixListener::bind(socket)?;
    tokio::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).await?;
    Ok((
        DaemonListener(Listening::Unix(listener)),
        DaemonEndpoint::Unix(socket.to_owned()),
    ))
}

#[cfg(not(unix))]
async fn listen_os(_socket: &Path) -> io::Result<(DaemonListener, DaemonEndpoint)> {
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    Ok((
        DaemonListener(Listening::Tcp(listener)),
        DaemonEndpoint::Tcp(address),
    ))
}

/// Where a Daemon takes its Connections from.
pub struct DaemonListener(Listening);

enum Listening {
    InMemory(InMemoryNetwork),
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    Unix(tokio::net::UnixListener),
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    Tcp(tokio::net::TcpListener),
}

impl DaemonListener {
    /// The next Connection.
    ///
    /// # Errors
    ///
    /// `DialerClosed` once every in-memory dialer is gone; `Accept` when
    /// this attempt failed and the listener still listens, a stream whose
    /// `TCP_NODELAY` the OS refused among them, which is dropped;
    /// `ListenerClosed` when the listener is closed for good.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no Connection.
    pub async fn accept(&mut self) -> Result<Connection, TransportError> {
        match &mut self.0 {
            Listening::InMemory(network) => network.accept().await,
            #[cfg(unix)]
            Listening::Unix(listener) => {
                let (stream, _) = listener.accept().await.map_err(accept_failure)?;
                let (reader, writer) = stream.into_split();
                Ok(Connection::new(reader, writer))
            }
            Listening::Tcp(listener) => {
                let (stream, _) = listener.accept().await.map_err(accept_failure)?;
                // The stream's own failure, never the listener's.
                stream.set_nodelay(true).map_err(TransportError::Accept)?;
                let (reader, writer) = stream.into_split();
                Ok(Connection::new(reader, writer))
            }
        }
    }
}

/// A Client's Connection to the Daemon at `endpoint`, as its runtime file
/// names it.
///
/// # Errors
///
/// When nothing listens there.
///
/// # Cancel safety
///
/// Dropping the future closes any connection it was opening.
pub async fn connect_daemon(endpoint: &DaemonEndpoint) -> io::Result<Connection> {
    match endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            #[expect(
                clippy::disallowed_types,
                reason = "the transport module is the one place that opens sockets (ADR 0060)"
            )]
            let stream = tokio::net::UnixStream::connect(path).await?;
            let (reader, writer) = stream.into_split();
            Ok(Connection::new(reader, writer))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(path) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{} is a Unix socket, which this OS's Daemon never listens on",
                path.display()
            ),
        )),
        DaemonEndpoint::Tcp(address) => {
            #[expect(
                clippy::disallowed_types,
                reason = "the transport module is the one place that opens sockets (ADR 0060)"
            )]
            let stream = tokio::net::TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let (reader, writer) = stream.into_split();
            Ok(Connection::new(reader, writer))
        }
    }
}

/// The OS's stdin and stdout, the Attached engine's Connection.
fn stdio() -> Connection {
    #[expect(
        clippy::disallowed_methods,
        reason = "the transport module is the one place that opens stdio (ADR 0060)"
    )]
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    Connection::new(stdin, stdout)
}

/// The engine's side of an in-memory network.
pub struct InMemoryNetwork {
    incoming: mpsc::Receiver<io::Result<Connection>>,
}

impl InMemoryNetwork {
    /// The next Connection a dialer opened, or the failure one injected.
    async fn accept(&mut self) -> Result<Connection, TransportError> {
        match self.incoming.recv().await {
            Some(Ok(connection)) => Ok(connection),
            Some(Err(error)) => Err(accept_failure(error)),
            None => Err(TransportError::DialerClosed),
        }
    }
}

/// The Client's side of an in-memory network, which opens Connections to
/// its engine.
#[derive(Clone)]
pub struct InMemoryDialer {
    outgoing: mpsc::Sender<io::Result<Connection>>,
}

/// A new in-memory network and its dialer.
pub fn in_memory() -> (InMemoryNetwork, InMemoryDialer) {
    let (outgoing, incoming) = mpsc::channel(1);
    (InMemoryNetwork { incoming }, InMemoryDialer { outgoing })
}

impl InMemoryDialer {
    /// Opens a Connection to the engine and returns the Client's end.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it resolves opens no Connection.
    pub async fn connect(&self) -> Result<Connection, TransportError> {
        let (client, engine) = tokio::io::duplex(IN_MEMORY_BUFFER);
        let (client_reader, client_writer) = tokio::io::split(client);
        let (engine_reader, engine_writer) = tokio::io::split(engine);
        self.outgoing
            .send(Ok(Connection::new(engine_reader, engine_writer)))
            .await
            .map_err(|_| TransportError::EngineGone)?;
        Ok(Connection::new(client_reader, client_writer))
    }

    /// Fails the engine's next accept with `error`, as the OS fails a
    /// listener's: the fault this adapter injects (ADR 0060), read as the
    /// OS's own, so `EMFILE` ends one attempt and `EBADF` the listener.
    ///
    /// # Errors
    ///
    /// `EngineGone` when the engine no longer accepts.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it resolves fails no accept.
    pub async fn fail_accept(&self, error: io::Error) -> Result<(), TransportError> {
        self.outgoing
            .send(Err(error))
            .await
            .map_err(|_| TransportError::EngineGone)
    }
}

/// The Stub gateway's listener (ADR 0096): TCP on loopback, on which that
/// test-only executable serves each Client's WebSocket.
pub struct GatewayListener(
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    tokio::net::TcpListener,
);

/// One connection a Client opened to the Stub gateway.
pub struct GatewayStream(
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    tokio::net::TcpStream,
);

/// Listens for the Stub gateway at `address`; port 0 takes a free port,
/// which [`GatewayListener::local_address`] answers.
///
/// # Errors
///
/// When the OS refuses the address, such as a port another program holds
/// (`AddrInUse`).
///
/// # Cancel safety
///
/// Dropping the future before it resolves leaves no listener.
pub async fn listen_gateway(address: std::net::SocketAddr) -> io::Result<GatewayListener> {
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::TcpListener::bind(address).await?;
    Ok(GatewayListener(listener))
}

impl GatewayListener {
    /// The address it listens at, its port the one the OS chose for port 0.
    ///
    /// # Errors
    ///
    /// When the OS cannot say.
    pub fn local_address(&self) -> io::Result<std::net::SocketAddr> {
        self.0.local_addr()
    }

    /// The next connection.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no connection.
    pub async fn accept(&self) -> io::Result<GatewayStream> {
        let (stream, _) = self.0.accept().await?;
        stream.set_nodelay(true)?;
        Ok(GatewayStream(stream))
    }
}

impl AsyncRead for GatewayStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for GatewayStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

/// The ops listener (ADR 0050): TCP at `ops.address`, on which a Daemon
/// serves its probes and `/metrics` over HTTP.
pub struct OpsListener(
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    tokio::net::TcpListener,
);

/// One connection a probe or a scraper opened to the ops listener.
pub struct OpsStream(
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    tokio::net::TcpStream,
);

/// Listens for probes and scrapers at `address`; port 0 takes a free port,
/// which [`OpsListener::local_address`] answers.
///
/// # Errors
///
/// When the OS refuses the address, such as a port another program holds
/// (`AddrInUse`).
///
/// # Cancel safety
///
/// Dropping the future before it resolves leaves no listener.
pub async fn listen_ops(address: std::net::SocketAddr) -> io::Result<OpsListener> {
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::TcpListener::bind(address).await?;
    Ok(OpsListener(listener))
}

impl OpsListener {
    /// The address it listens at, its port the one the OS chose for port 0.
    ///
    /// # Errors
    ///
    /// When the OS cannot say.
    pub fn local_address(&self) -> io::Result<std::net::SocketAddr> {
        self.0.local_addr()
    }

    /// The next connection.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no connection.
    pub(crate) async fn accept(&self) -> io::Result<OpsStream> {
        let (stream, _) = self.0.accept().await?;
        stream.set_nodelay(true)?;
        Ok(OpsStream(stream))
    }
}

impl AsyncRead for OpsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for OpsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}
