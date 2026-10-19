//! A broken IPv6 path (ADR 0118's Happy Eyeballs page): on the port the
//! Proving ground serves on IPv4, its IPv6 loopback address takes no
//! connection. It listens with a backlog of one, which the Proving ground
//! fills with its own connections and never accepts. Linux then drops every
//! later SYN, so a client's attempt there waits until it times out, as on a
//! path that drops packets, while IPv4 answers. macOS resets such an attempt
//! or queues it unanswered instead, so only Linux, where the pre-merge
//! suite runs, gives the full connect timeout. Where IPv6 loopback is
//! missing, the address is unreachable.
#![expect(
    clippy::disallowed_types,
    reason = "the Proving ground opens its own sockets, outside the engine's transport (ADR 0060)"
)]

use std::net::{Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::net::{TcpListener, TcpSocket, TcpStream};

/// How long a connection to the full queue may take before it counts as
/// dropped; a loopback handshake the kernel completes takes far less.
const DROPPED_AFTER: Duration = Duration::from_millis(250);

/// The most connections tried while filling the queue.
const FILL_LIMIT: usize = 16;

/// The listener that takes nothing, with the connections that fill its
/// queue; both live as long as it does.
pub(crate) struct Blackhole {
    _listener: TcpListener,
    _held: Vec<TcpStream>,
}

impl Blackhole {
    /// Listens on `[::1]:port` and fills its queue; `None` where IPv6
    /// loopback or the port is not available.
    pub(crate) async fn bind(port: u16) -> Option<Self> {
        let address = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
        let socket = TcpSocket::new_v6().ok()?;
        socket.bind(address).ok()?;
        let listener = socket.listen(1).ok()?;
        let mut held = Vec::new();
        while held.len() < FILL_LIMIT {
            match tokio::time::timeout(DROPPED_AFTER, TcpStream::connect(address)).await {
                Ok(Ok(stream)) => held.push(stream),
                _ => break,
            }
        }
        Some(Self {
            _listener: listener,
            _held: held,
        })
    }
}
