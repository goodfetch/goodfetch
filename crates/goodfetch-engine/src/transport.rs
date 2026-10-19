//! Every connection the engine serves, taken from a network value (ADR
//! 0060): the OS's, or the in-memory network through which each Product's
//! SDK crate embeds its engine and its tests drive it, a public product
//! adapter. An Attached engine's one Connection is the OS's stdio, or the
//! one an in-memory dialer opens. This is the only module that opens stdin
//! and stdout; the Daemon's socket, loopback TCP and the other listeners
//! join here with the tickets that add them. It also opens the outbound TCP
//! connections of the engine's own pinned downloads (ADR 0043).

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

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

/// An outbound TCP connection on the OS's network, which the engine opens
/// only for its own pinned downloads (ADR 0043): the Browser cache's
/// first-use download and `browsers install`.
#[expect(
    clippy::disallowed_types,
    reason = "the transport module is the one place that opens sockets (ADR 0060)"
)]
pub struct TcpConnection(tokio::net::TcpStream);

/// Connects to the first of `addresses` that accepts, in order.
///
/// # Cancel safety
///
/// Dropping the future closes any connection it was opening.
pub async fn connect_tcp(addresses: &[std::net::SocketAddr]) -> io::Result<TcpConnection> {
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "no address to connect to");
    for address in addresses {
        #[expect(
            clippy::disallowed_types,
            reason = "the transport module is the one place that opens sockets (ADR 0060)"
        )]
        let connected = tokio::net::TcpStream::connect(address).await;
        match connected {
            Ok(stream) => return Ok(TcpConnection(stream)),
            Err(error) => last = error,
        }
    }
    Err(last)
}

impl AsyncRead for TcpConnection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for TcpConnection {
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
