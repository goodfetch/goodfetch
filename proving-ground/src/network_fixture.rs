//! The network fixtures `serve` starts beside its site (spec #152, story 82),
//! each on its own address of the site's host, which `serve`'s output line
//! reports: the forward proxies, the DNS fixture, the target, a listener
//! that no test should reach, whose every connection the connection record
//! lists, and the UDP fixture, a socket whose every datagram the record
//! lists, which a page's STUN and TURN servers over UDP name, so a test sees
//! that no UDP left around a Session's proxy (ADR 0044).
//!
//! The proxies are `open`, `refusing_tunnels` (it refuses `CONNECT`),
//! `limited` (one page request at a time, until it read the site's
//! answer, `429` to any other meanwhile; a robots.txt or an icon goes
//! uncounted),
//! `down` (an address where nothing listens, which no other program can
//! take), `socks5` (a SOCKS5 proxy
//! behind the authenticated proxies' credentials), and one authenticated
//! proxy per proxy kind (`datacenter`, `isp`, `residential`, `mobile`) with
//! its own pool of exits.
#![expect(
    clippy::disallowed_types,
    reason = "the network fixtures open their own sockets, outside the engine's transport (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpSocket, TcpStream, UdpSocket};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::dns::{DnsServer, Zone};
use crate::forward_proxy::{Access, Context, Credentials, ForwardProxy, Proxy};
use crate::observation::Observations;

/// The proxy kinds, cheapest first, each with its own authenticated proxy.
const KINDS: [&str; 4] = ["datacenter", "isp", "residential", "mobile"];

/// The exits of each proxy kind's pool.
const EXITS_PER_KIND: usize = 3;

/// The most bytes of a request head the target reads before it answers.
const TARGET_READ_LIMIT: usize = 64 * 1024;

/// The most bytes of a datagram the UDP fixture reads.
const DATAGRAM_LIMIT: usize = 64 * 1024;

/// The target's answer to whatever reached it.
const REACHED: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\nreached";

/// The bound fixtures.
pub(crate) struct NetworkFixtures {
    proxies: Vec<ForwardProxy>,
    /// The address where the `down` proxy would listen.
    down: Down,
    dns: DnsServer,
    target: TcpListener,
    udp: UdpSocket,
    context: Arc<Context>,
}

impl NetworkFixtures {
    /// Binds every fixture on port 0 of the site's address `site`, recording
    /// into `observations`.
    pub(crate) async fn bind(
        site: IpAddr,
        seed: u64,
        observations: Arc<Observations>,
    ) -> Result<Self> {
        let host = reachable(site);
        let zone = Arc::new(Zone::new(observations.hosts().keys(), host));
        let mut proxies = vec![
            ForwardProxy::bind(host, Proxy::single("open", Access::Open)).await?,
            ForwardProxy::bind(
                host,
                Proxy::single("refusing_tunnels", Access::RefusingTunnels),
            )
            .await?,
            ForwardProxy::bind(host, Proxy::single("limited", Access::Limited)).await?,
            ForwardProxy::bind(host, Proxy::single("socks5", Access::Socks5)).await?,
        ];
        for kind in KINDS {
            proxies.push(ForwardProxy::bind(host, Proxy::of_kind(kind, EXITS_PER_KIND)).await?);
        }
        let down = Down::hold(host).await?;
        let dns = DnsServer::bind(
            SocketAddr::new(host, 0),
            Arc::clone(&zone),
            Arc::clone(&observations),
        )
        .await?;
        let target = TcpListener::bind((host, 0))
            .await
            .context("binding the target")?;
        let udp = UdpSocket::bind((host, 0))
            .await
            .context("binding the UDP fixture")?;
        Ok(Self {
            proxies,
            down,
            dns,
            target,
            udp,
            context: Arc::new(Context {
                observations,
                zone,
                credentials: Credentials::new(seed),
                site: host,
            }),
        })
    }

    /// What `serve`'s output line reports of them: `proxies` by name,
    /// `proxy_credentials`, `dns`, `target` and `udp`.
    pub(crate) fn report(&self) -> Result<Map<String, Value>> {
        let mut proxies = BTreeMap::from([("down", self.down.address.to_string())]);
        for proxy in &self.proxies {
            proxies.insert(proxy.name(), proxy.address()?.to_string());
        }
        let target = self
            .target
            .local_addr()
            .context("reading the target's address")?;
        let udp = self
            .udp
            .local_addr()
            .context("reading the UDP fixture's address")?;
        Ok(Map::from_iter([
            ("proxies".to_owned(), json!(proxies)),
            (
                "proxy_credentials".to_owned(),
                json!(self.context.credentials),
            ),
            ("dns".to_owned(), json!(self.dns.address()?.to_string())),
            ("target".to_owned(), json!(target.to_string())),
            ("udp".to_owned(), json!(udp.to_string())),
        ]))
    }

    /// Serves every fixture until dropped, each connection on a task of the
    /// fixtures' tracker; dropping the future stops them all.
    pub(crate) async fn run(self) {
        let tracker = TaskTracker::new();
        let stop = CancellationToken::new();
        let _stopped_when_dropped = stop.clone().drop_guard();
        for proxy in self.proxies {
            tracker.spawn(proxy.run(Arc::clone(&self.context), tracker.clone(), stop.clone()));
        }
        let observations = Arc::clone(&self.context.observations);
        tracker.spawn(listen(self.udp, Arc::clone(&observations), stop.clone()));
        tracker.spawn(watch(
            self.target,
            observations,
            tracker.clone(),
            stop.clone(),
        ));
        let _held_until_stopped = self.down;
        tokio::select! {
            () = self.dns.run() => {}
            () = stop.cancelled() => {}
        }
    }
}

/// The `down` proxy's address: a port of the site's host that one end of a
/// connection the fixtures make to themselves holds. Nothing listens there,
/// so the kernel refuses every connection to it, and no other program can
/// listen on it while the fixtures run, as one could on a port bound and
/// released (fix-queue 31). A socket bound without listening would hold the
/// port too, but macOS drops a connection to it unanswered instead of
/// refusing it.
struct Down {
    address: SocketAddr,
    /// Both ends of the connection that holds the port.
    _ends: (TcpStream, TcpStream),
}

impl Down {
    /// Holds a port of `host`.
    async fn hold(host: IpAddr) -> Result<Self> {
        let peer = TcpListener::bind((host, 0))
            .await
            .context("binding the down proxy's peer")?;
        let peer_address = peer
            .local_addr()
            .context("reading the down proxy's peer's address")?;
        // A socket of its own rather than a listener's, so it never reuses
        // addresses: no other socket can bind its port.
        let socket = match host {
            IpAddr::V4(_) => TcpSocket::new_v4(),
            IpAddr::V6(_) => TcpSocket::new_v6(),
        }
        .context("opening the down proxy's socket")?;
        socket
            .bind(SocketAddr::new(host, 0))
            .context("binding the down proxy's address")?;
        let address = socket
            .local_addr()
            .context("reading the down proxy's address")?;
        let accepted = async {
            loop {
                let (stream, from) = peer.accept().await?;
                if from == address {
                    return Ok::<_, std::io::Error>(stream);
                }
            }
        };
        let ends = tokio::try_join!(socket.connect(peer_address), accepted)
            .context("connecting the down proxy's address to its peer")?;
        Ok(Self {
            address,
            _ends: ends,
        })
    }
}

/// The address a fixture binds and the site is reached on: the site's own,
/// or loopback when the site binds every address.
fn reachable(site: IpAddr) -> IpAddr {
    match site {
        IpAddr::V4(address) if address.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(address) if address.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        address => address,
    }
}

/// Accepts every connection to the target until `stop`, records it, and
/// answers it.
async fn watch(
    target: TcpListener,
    observations: Arc<Observations>,
    tracker: TaskTracker,
    stop: CancellationToken,
) {
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => return,
            accepted = target.accept() => accepted,
        };
        let Ok((stream, source)) = accepted else {
            continue;
        };
        let Ok(destination) = stream.local_addr() else {
            continue;
        };
        observations.connection_record().accepted(
            observations.stamp(),
            "target",
            source,
            destination,
        );
        let stop = stop.clone();
        tracker.spawn(async move {
            tokio::select! {
                () = reached(stream) => {}
                () = stop.cancelled() => {}
            }
        });
    }
}

/// Records every datagram the UDP fixture receives until `stop`, answering
/// none.
async fn listen(udp: UdpSocket, observations: Arc<Observations>, stop: CancellationToken) {
    let Ok(destination) = udp.local_addr() else {
        return;
    };
    let mut buffer = vec![0; DATAGRAM_LIMIT];
    loop {
        let received = tokio::select! {
            () = stop.cancelled() => return,
            received = udp.recv_from(&mut buffer) => received,
        };
        // A failed read concerns only the client that sent it.
        let Ok((length, source)) = received else {
            continue;
        };
        observations.connection_record().udp_datagram(
            observations.stamp(),
            source,
            destination,
            length,
        );
    }
}

/// Reads a request head, or what comes before the client stops, and answers
/// that the target was reached.
async fn reached(mut stream: TcpStream) {
    let mut head = Vec::new();
    let mut chunk = [0; 4096];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") && head.len() < TARGET_READ_LIMIT {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
    let _ = stream.write_all(REACHED).await;
    let _ = stream.shutdown().await;
}
