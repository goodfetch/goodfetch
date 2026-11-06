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
//! the Console's listener, the ops listener (ADR 0050) and the Politeness
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
#[cfg(test)]
mod tests;
mod wake;

pub use socket_folder::{
    LONGEST_SOCKET_PATH, socket_folder, socket_folder_in, temporary_folder_for_sockets,
};
pub use wake::{WakeListener, is_gone, listen_wakes, wake};

/// The name of the socket a launch agent's `Sockets` defines for the
/// Daemon, which it asks launchd for.
pub const LAUNCHD_SOCKETS: &str = "Listeners";

/// The name of the socket a launch agent's `Sockets` defines for the
/// Console's address, which the Daemon asks launchd for (ADR 0031).
pub const LAUNCHD_CONSOLE_SOCKET: &str = "Console";

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
    /// The OS failed to accept a Connection on the Daemon's listener.
    #[error("the Daemon's listener cannot accept: {0}")]
    Accept(io::Error),
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
            Self::InMemory(network) => network
                .incoming
                .recv()
                .await
                .ok_or(TransportError::DialerClosed),
        }
    }
}

impl Network {
    /// A Daemon's listener: on the OS, a Unix socket at `socket`, readable
    /// and writable by its owner alone, on macOS and Linux, and loopback TCP
    /// on a port the OS chooses on Windows; in memory, every Connection the
    /// dialer opens. Answers the endpoint a runtime file names.
    ///
    /// It blocks on the file system, so it runs as its owner's blocking
    /// work ([`crate::tasks::TaskOwner::blocking`]), in the runtime's
    /// context, which registers the listener: inline under
    /// [`crate::tasks::BlockingPolicy::Inline`], so a run replays from its
    /// seed whatever the host's load.
    ///
    /// # Errors
    ///
    /// When the OS refuses the socket, such as a path too long for it.
    pub fn listen(self, socket: &Path) -> io::Result<(DaemonListener, DaemonEndpoint)> {
        match self {
            Self::InMemory(network) => Ok((
                DaemonListener(Listening::InMemory(network)),
                DaemonEndpoint::Unix(socket.to_owned()),
            )),
            Self::Os => listen_os(socket),
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

/// A port the OS chose, held for the listener a service manager opens on it
/// later, its Console socket for the Daemon it starts (ADR 0031), until
/// dropped once that listener is open: a socket bound with port reuse that
/// never listens. The OS refuses the port to every other socket but one of
/// the same user that asks for port reuse on this very port, as a socket
/// unit's `ReusePort=yes` does; it picks no held port for another port 0, and
/// nothing connects here, since the socket never listens (#1821).
#[derive(Debug)]
#[expect(
    clippy::disallowed_types,
    reason = "the transport module is the one place that opens sockets (ADR 0060)"
)]
pub struct HeldPort {
    address: std::net::SocketAddr,
    _socket: tokio::net::TcpSocket,
}

impl HeldPort {
    /// The host the hold was asked for, on the port held.
    #[must_use]
    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }
}

/// Holds a port the OS chooses on `host`, for a listener that opens later:
/// never a port bound and let go, which another program could take first.
///
/// # Errors
///
/// When the OS refuses to bind on `host`, or has no port reuse to hold a port
/// with (Windows, whose service has no socket).
#[cfg(unix)]
#[expect(
    clippy::disallowed_types,
    reason = "the transport module is the one place that opens sockets (ADR 0060)"
)]
pub fn hold_free_port(host: std::net::IpAddr) -> io::Result<HeldPort> {
    let socket = if host.is_ipv4() {
        tokio::net::TcpSocket::new_v4()?
    } else {
        tokio::net::TcpSocket::new_v6()?
    };
    socket.set_reuseport(true)?;
    socket.bind(std::net::SocketAddr::new(host, 0))?;
    Ok(HeldPort {
        address: socket.local_addr()?,
        _socket: socket,
    })
}

/// Holds a port the OS chooses on `host`: not on this OS, which has no port
/// reuse to hold one with and no service socket that would listen on it.
///
/// # Errors
///
/// Always.
#[cfg(not(unix))]
pub fn hold_free_port(_host: std::net::IpAddr) -> io::Result<HeldPort> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this OS has no port reuse to hold a port with",
    ))
}

/// The Console's listener a service manager handed this process, which it
/// started at the first connection to the Console's address (ADR 0031): the
/// second socket systemd passes, or launchd's [`LAUNCHD_CONSOLE_SOCKET`];
/// `None` when it handed over none. Taken once.
///
/// # Errors
///
/// When the OS refuses the handed-over socket.
pub fn activated_console(
    environment: &BTreeMap<OsString, OsString>,
) -> io::Result<Option<ConsoleListener>> {
    #[cfg(unix)]
    {
        let Some(descriptor) = activation::take_console(environment, LAUNCHD_CONSOLE_SOCKET)?
        else {
            return Ok(None);
        };
        #[expect(
            clippy::disallowed_types,
            reason = "the transport module is the one place that opens sockets (ADR 0060)"
        )]
        let listener = std::net::TcpListener::from(descriptor);
        listener.set_nonblocking(true)?;
        #[expect(
            clippy::disallowed_types,
            reason = "the transport module is the one place that opens sockets (ADR 0060)"
        )]
        let listener = tokio::net::TcpListener::from_std(listener)?;
        Ok(Some(ConsoleListener(listener)))
    }
    #[cfg(not(unix))]
    {
        let _ = environment;
        Ok(None)
    }
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "it runs as its owner's blocking work (`Network::listen`)"
)]
fn listen_os(socket: &Path) -> io::Result<(DaemonListener, DaemonEndpoint)> {
    use std::os::unix::fs::PermissionsExt as _;
    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok((
        DaemonListener(Listening::Unix(listener)),
        DaemonEndpoint::Unix(socket.to_owned()),
    ))
}

#[cfg(not(unix))]
fn listen_os(_socket: &Path) -> io::Result<(DaemonListener, DaemonEndpoint)> {
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::TcpListener::from_std(listener)?;
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
    /// `DialerClosed` once every in-memory dialer is gone, or the OS's
    /// failure to accept.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no Connection.
    pub async fn accept(&mut self) -> Result<Connection, TransportError> {
        match &mut self.0 {
            Listening::InMemory(network) => network
                .incoming
                .recv()
                .await
                .ok_or(TransportError::DialerClosed),
            #[cfg(unix)]
            Listening::Unix(listener) => {
                let (stream, _) = listener.accept().await.map_err(TransportError::Accept)?;
                let (reader, writer) = stream.into_split();
                Ok(Connection::new(reader, writer))
            }
            Listening::Tcp(listener) => {
                let (stream, _) = listener.accept().await.map_err(TransportError::Accept)?;
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
    incoming: mpsc::Receiver<Connection>,
}

/// The Client's side of an in-memory network, which opens Connections to
/// its engine.
#[derive(Clone)]
pub struct InMemoryDialer {
    outgoing: mpsc::Sender<Connection>,
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
            .send(Connection::new(engine_reader, engine_writer))
            .await
            .map_err(|_| TransportError::EngineGone)?;
        Ok(Connection::new(client_reader, client_writer))
    }
}

/// The Console's listener (ADR 0031): TCP at `console.address`, on which
/// the Daemon serves the Console's HTTP.
pub struct ConsoleListener(
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    tokio::net::TcpListener,
);

/// One connection a browser opened to the Console.
pub struct ConsoleStream(
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    tokio::net::TcpStream,
);

/// Listens for the Console at `address`; port 0 takes a free port, which
/// [`ConsoleListener::local_address`] answers.
///
/// # Errors
///
/// When the OS refuses the address, such as a port another program holds
/// (`AddrInUse`).
///
/// # Cancel safety
///
/// Dropping the future before it resolves leaves no listener.
pub async fn listen_console(address: std::net::SocketAddr) -> io::Result<ConsoleListener> {
    #[expect(
        clippy::disallowed_types,
        reason = "the transport module is the one place that opens sockets (ADR 0060)"
    )]
    let listener = tokio::net::TcpListener::bind(address).await?;
    Ok(ConsoleListener(listener))
}

impl ConsoleListener {
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
    pub async fn accept(&self) -> io::Result<ConsoleStream> {
        let (stream, _) = self.0.accept().await?;
        stream.set_nodelay(true)?;
        Ok(ConsoleStream(stream))
    }
}

impl ConsoleStream {
    /// The address of the browser that opened it.
    ///
    /// # Errors
    ///
    /// When the OS cannot say.
    pub(crate) fn peer_address(&self) -> io::Result<std::net::SocketAddr> {
        self.0.peer_addr()
    }
}

impl AsyncRead for ConsoleStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for ConsoleStream {
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
