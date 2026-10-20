//! Every connection the engine serves, taken from a network value (ADR
//! 0060): the OS's, or the in-memory network through which each Product's
//! SDK crate embeds its engine and its tests drive it, a public product
//! adapter. An Attached engine's one Connection is the OS's stdio, or the
//! one an in-memory dialer opens. A Daemon's Connections come from its
//! listener: a `0600` Unix socket on macOS and Linux, loopback TCP on
//! Windows (ADR 0024), or every Connection an in-memory dialer opens; a
//! Client of the Daemon connects to the endpoint its runtime file names.
//! This is the only module that opens stdin, stdout and the protocol's
//! sockets; the other listeners join here with the tickets that add them.
//! The engine's own outbound connections, which the Simulation never
//! carries, open in `http_client`.

use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use goodfetch_protocol::daemon::DaemonEndpoint;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

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
    #[cfg(not(unix))]
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
            #[cfg(not(unix))]
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
