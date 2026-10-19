//! The user-space latency proxy, the lab's ported: it listens where clients
//! connect and forwards each connection to the address behind it, both
//! directions shaped by their [`Link`]. It needs no privileges, so it runs
//! on any OS, for unpublished runs.
#![expect(
    clippy::disallowed_types,
    reason = "the latency proxy opens its own listening socket and upstream connections, outside the engine's transport (ADR 0060)"
)]

use std::io;
use std::net::SocketAddr;

use anyhow::{Context as _, Result};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::task::TaskTracker;

use super::Profile;
use super::link::{Direction, Link};

/// A bound latency proxy.
pub(crate) struct Proxy {
    listener: TcpListener,
    profile: Profile,
    seed: u64,
    tracker: TaskTracker,
}

impl Proxy {
    /// Binds the address clients connect to; port 0 binds a free port.
    pub(crate) async fn bind(address: SocketAddr, profile: Profile, seed: u64) -> Result<Self> {
        let listener = TcpListener::bind(address)
            .await
            .with_context(|| format!("binding the latency proxy to {address}"))?;
        Ok(Self {
            listener,
            profile,
            seed,
            tracker: TaskTracker::new(),
        })
    }

    /// The bound address.
    pub(crate) fn address(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .expect("invariant: a bound listener has an address")
    }

    /// Forwards every connection it accepts to `upstream`, each on a task of
    /// the proxy's tracker, until the future is dropped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops accepting; connections already accepted are
    /// forwarded until they close.
    pub(crate) async fn forward(&self, upstream: SocketAddr) {
        let mut connection = 0_u64;
        loop {
            let Ok((client, _)) = self.listener.accept().await else {
                // A connection that failed before it was accepted concerns
                // only its client.
                continue;
            };
            let up = Link::new(&self.profile, self.seed, connection, Direction::Up);
            let down = Link::new(&self.profile, self.seed, connection, Direction::Down);
            connection += 1;
            self.tracker.spawn(async move {
                // A failed connection concerns only its client.
                let _ = shape(client, upstream, &up, &down).await;
            });
        }
    }
}

/// Forwards one connection both ways until both directions end.
async fn shape(client: TcpStream, upstream: SocketAddr, up: &Link, down: &Link) -> io::Result<()> {
    let server = TcpStream::connect(upstream).await?;
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    tokio::try_join!(
        up.forward(client_read, server_write),
        down.forward(server_read, client_write)
    )
    .map(drop)
}
