//! The runner's connections to the listener under test: loopback TCP, or a
//! Unix socket. They are blocking, with a deadline on every read, since the
//! runner tries one attempt at a time and reads no clock.
#![expect(
    clippy::disallowed_types,
    reason = "the endpoint table's runner is a test helper that connects to a product's listener from outside it, never inside the Simulation (ADRs 0060 and 0067)"
)]

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

/// How long the runner waits to connect, and for each read of an answer,
/// before it records that the listener stayed silent.
pub(super) const DEADLINE: Duration = Duration::from_secs(5);

/// Where a listener listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerAddress {
    /// A TCP address, loopback for every local listener.
    Tcp(SocketAddr),
    /// A Unix socket's path.
    #[cfg(unix)]
    Unix(PathBuf),
}

/// One open connection.
pub(super) enum Connection {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Connection {
    /// Connects to `address`, with `DEADLINE` on every read.
    pub(super) fn open(address: &ListenerAddress) -> io::Result<Self> {
        match address {
            ListenerAddress::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, DEADLINE)?;
                stream.set_read_timeout(Some(DEADLINE))?;
                Ok(Self::Tcp(stream))
            }
            #[cfg(unix)]
            ListenerAddress::Unix(path) => {
                let stream = UnixStream::connect(path)?;
                stream.set_read_timeout(Some(DEADLINE))?;
                Ok(Self::Unix(stream))
            }
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
        }
    }
}

/// Whether `error` means the listener closed or reset the connection.
pub(super) fn is_closed(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
    )
}

/// Whether `error` means the deadline passed with no data.
pub(super) fn is_silent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
