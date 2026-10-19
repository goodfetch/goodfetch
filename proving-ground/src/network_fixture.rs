//! The network fixtures `serve` starts beside its site (spec #152, story 82),
//! each on its own address of the site's host, which `serve`'s output line
//! reports: the forward proxies, the DNS fixture, and the target, a listener
//! that no test should reach, whose every connection the connection record
//! lists.
//!
//! The proxies are `open`, `refusing_tunnels` (it refuses `CONNECT`),
//! `down` (an address where nothing listens), and one authenticated proxy
//! per proxy kind (`datacenter`, `isp`, `residential`, `mobile`) with its
//! own pool of exits.
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
use tokio::net::{TcpListener, TcpStream};
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

/// The target's answer to whatever reached it.
const REACHED: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\nreached";

/// The bound fixtures.
pub(crate) struct NetworkFixtures {
    proxies: Vec<ForwardProxy>,
    /// The address where the `down` proxy would listen.
    down: SocketAddr,
    dns: DnsServer,
    target: TcpListener,
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
        ];
        for kind in KINDS {
            proxies.push(ForwardProxy::bind(host, Proxy::of_kind(kind, EXITS_PER_KIND)).await?);
        }
        let down = TcpListener::bind((host, 0))
            .await
            .context("binding the down proxy's address")?
            .local_addr()
            .context("reading the down proxy's address")?;
        let dns = DnsServer::bind(SocketAddr::new(host, 0), Arc::clone(&zone)).await?;
        let target = TcpListener::bind((host, 0))
            .await
            .context("binding the target")?;
        Ok(Self {
            proxies,
            down,
            dns,
            target,
            context: Arc::new(Context {
                observations,
                zone,
                credentials: Credentials::new(seed),
                site: host,
            }),
        })
    }

    /// What `serve`'s output line reports of them: `proxies` by name,
    /// `proxy_credentials`, `dns` and `target`.
    pub(crate) fn report(&self) -> Result<Map<String, Value>> {
        let mut proxies = BTreeMap::from([("down", self.down.to_string())]);
        for proxy in &self.proxies {
            proxies.insert(proxy.name(), proxy.address()?.to_string());
        }
        let target = self
            .target
            .local_addr()
            .context("reading the target's address")?;
        Ok(Map::from_iter([
            ("proxies".to_owned(), json!(proxies)),
            (
                "proxy_credentials".to_owned(),
                json!(self.context.credentials),
            ),
            ("dns".to_owned(), json!(self.dns.address()?.to_string())),
            ("target".to_owned(), json!(target.to_string())),
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
        tracker.spawn(watch(
            self.target,
            observations,
            tracker.clone(),
            stop.clone(),
        ));
        tokio::select! {
            () = self.dns.run() => {}
            () = stop.cancelled() => {}
        }
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
