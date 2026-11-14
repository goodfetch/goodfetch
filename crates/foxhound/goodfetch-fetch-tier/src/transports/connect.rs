//! The TCP connections of the HTTP tier's transports (ADR 0118; RFC 8305),
//! reqwest's and `wreq`'s, each library opening its connections through
//! [`Connect`], given to its client as its TCP connector: the engine's
//! connection race over the transport resolver's answer, judged by the
//! Session's egress policy, from the Session's local address of each
//! attempt's family and bound to its interface's device (ADR 0044).
//!
//! The resolver answers both families at once, so the race starts with
//! both ready: `resolver.resolution_delay` has no answer to wait for here,
//! as on the engine's paths whose answer is judged before the race (ADR
//! 0118). The next address starts after `resolver.attempt_delay`, or at
//! once when an attempt fails. A name whose every address failed is
//! forgotten, so the next connection asks again (RFC 8305 section 9).
//!
//! Each socket keeps the libraries' own defaults: TCP keepalive after 15 s
//! of quiet, every 15 s, 3 probes, and on Linux a 30 s user timeout; the
//! library sets `TCP_NODELAY` on the stream it is given.
#![expect(
    clippy::disallowed_types,
    reason = "the HTTP fetch tier's transports open their own sockets (ADR 0060)"
)]

use std::error::Error;
use std::future::{Future, ready};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::LookupError;
use goodfetch_engine::happy_eyeballs::{Delays, Families, Race, RaceError};
use goodfetch_protocol::dns::IpFamily;
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::mpsc;

use super::{LocalSource, ResolveFailure, Resolver, TransportConfig, family_of};

/// What a connection that could not open fails with.
type BoxError = Box<dyn Error + Send + Sync>;

/// TCP keepalive: the first probe after 15 s of quiet, then every 15 s,
/// 3 unanswered probes ending the connection, as reqwest and `wreq` set.
const KEEPALIVE: socket2::TcpKeepalive = socket2::TcpKeepalive::new()
    .with_time(Duration::from_secs(15))
    .with_interval(Duration::from_secs(15))
    .with_retries(3);

/// How long sent data may stay unacknowledged before Linux ends the
/// connection, as reqwest and `wreq` set.
#[cfg(target_os = "linux")]
const USER_TIMEOUT: Duration = Duration::from_secs(30);

/// A transport's TCP connector: a connection to a URI's host, the site's
/// or its proxy's, raced over the addresses the transport's resolver gives.
/// Clones share the resolver.
#[derive(Clone, Debug)]
pub(super) struct Connect {
    resolver: Arc<Resolver>,
    clock: Clock,
    delays: Delays,
    local: Option<LocalSource>,
    device: Device,
}

impl Connect {
    /// The connector of a transport under `config`, whose names `resolver`
    /// answers.
    pub(super) fn new(config: &TransportConfig, resolver: Arc<Resolver>) -> Self {
        Self {
            resolver,
            clock: config.clock,
            delays: config.delays,
            local: config.local.clone(),
            device: Device::of(
                config
                    .local
                    .as_ref()
                    .and_then(|local| local.interface.as_deref()),
            ),
        }
    }

    /// The families its connections may reach: the one the Session holds a
    /// local address of alone, else both.
    fn families(&self) -> Families {
        match self.local.as_ref().map(|local| (local.ipv4, local.ipv6)) {
            Some((Some(_), None)) => Families::Only(IpFamily::Ipv4),
            Some((None, Some(_))) => Families::Only(IpFamily::Ipv6),
            _ => Families::Both,
        }
    }

    /// A connection to `uri`'s host on its port: its address, or the race
    /// over those its name resolves to.
    async fn to(&self, uri: &http::Uri) -> Result<TcpStream, BoxError> {
        let host = uri.host().ok_or(ResolveFailure::NotResolved)?;
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        let port = uri.port_u16().unwrap_or(match uri.scheme_str() {
            Some("https") => 443,
            _ => 80,
        });
        let (name, addresses) = if let Ok(address) = host.parse::<IpAddr>() {
            (None, vec![address])
        } else {
            let addresses = self.resolver.addresses(host.to_owned()).await?;
            (Some(host), addresses.iter().map(SocketAddr::ip).collect())
        };
        let of = |family: IpFamily| -> Result<Vec<IpAddr>, LookupError> {
            Ok(addresses
                .iter()
                .copied()
                .filter(|address| family_of(*address) == family)
                .collect())
        };
        let race = Race {
            clock: self.clock,
            delays: self.delays,
            families: self.families(),
        };
        // The race keeps no attempt's error: each failed attempt leaves its
        // own here, so the connection fails with the last one's.
        let (failed, mut failures) = mpsc::unbounded_channel();
        let raced = race
            .connect(
                host,
                port,
                ready(of(IpFamily::Ipv6)),
                ready(of(IpFamily::Ipv4)),
                |address| {
                    let failed = failed.clone();
                    let local = self
                        .local
                        .as_ref()
                        .and_then(|local| local.address(family_of(address.ip())));
                    let device = &self.device;
                    async move {
                        attempt(address, local, device).await.map_err(|error| {
                            let kind = error.kind();
                            // The receiver lives until the race ends.
                            let _ = failed.send(error);
                            io::Error::from(kind)
                        })
                    }
                },
            )
            .await;
        match raced {
            Ok(won) => Ok(won.connection),
            Err(RaceError::Unreachable { tried }) => {
                if let Some(name) = name
                    && !tried.is_empty()
                {
                    self.resolver.forget(name).await;
                }
                let mut last = None;
                while let Ok(failure) = failures.try_recv() {
                    last = Some(failure);
                }
                Err(last.map_or_else(|| ResolveFailure::NotResolved.into(), Into::into))
            }
            Err(RaceError::FamilyUnavailable(unavailable)) => Err(ResolveFailure::Family {
                name: unavailable.name,
                held: unavailable.held,
            }
            .into()),
            Err(RaceError::Lookup(_)) => Err(ResolveFailure::NotResolved.into()),
        }
    }
}

impl tower_service::Service<http::Uri> for Connect {
    type Response = TcpStream;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<TcpStream, BoxError>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: http::Uri) -> Self::Future {
        let connect = self.clone();
        Box::pin(async move { connect.to(&uri).await })
    }
}

/// One connection attempt to `address`, from `local` when the Session
/// holds a local address of its family, bound to `device`.
async fn attempt(
    address: SocketAddr,
    local: Option<IpAddr>,
    device: &Device,
) -> io::Result<TcpStream> {
    let socket = if address.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    let options = socket2::SockRef::from(&socket);
    if let Err(error) = options.set_tcp_keepalive(&KEEPALIVE) {
        tracing::warn!(%error, "the connection's TCP keepalive could not be set");
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = options.set_tcp_user_timeout(Some(USER_TIMEOUT)) {
        tracing::warn!(%error, "the connection's TCP user timeout could not be set");
    }
    device.bind(&options, address)?;
    if let Some(local) = local {
        socket.bind(SocketAddr::new(local, 0))?;
    }
    socket.connect(address).await
}

/// The device of the interface the Session's local addresses came from,
/// which each socket binds (`SO_BINDTODEVICE`), on Linux.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct Device(Option<Arc<str>>);

#[cfg(target_os = "linux")]
impl Device {
    fn of(interface: Option<&str>) -> Self {
        Self(interface.map(Arc::from))
    }

    fn bind(&self, socket: &socket2::SockRef<'_>, _address: SocketAddr) -> io::Result<()> {
        match &self.0 {
            Some(interface) => socket.bind_device(Some(interface.as_bytes())),
            None => Ok(()),
        }
    }
}

/// The interface the Session's local addresses came from, which each
/// socket binds by its index (`IP_BOUND_IF`, `IPV6_BOUND_IF`), on macOS and
/// illumos. The index is read once, when the transport is built, as the
/// Session's addresses are.
#[cfg(any(target_os = "macos", target_os = "illumos"))]
#[derive(Clone, Copy, Debug)]
enum Device {
    /// The Session names no interface.
    Any,
    /// The interface of this index.
    Index(std::num::NonZeroU32),
    /// An interface the host no longer has, which fails each attempt.
    Gone,
}

#[cfg(any(target_os = "macos", target_os = "illumos"))]
impl Device {
    fn of(interface: Option<&str>) -> Self {
        let Some(name) = interface else {
            return Self::Any;
        };
        netdev::get_interfaces()
            .into_iter()
            .find(|interface| interface.name == name)
            .and_then(|interface| std::num::NonZeroU32::new(interface.index))
            .map_or(Self::Gone, Self::Index)
    }

    fn bind(&self, socket: &socket2::SockRef<'_>, address: SocketAddr) -> io::Result<()> {
        match *self {
            Self::Any => Ok(()),
            Self::Gone => Err(io::Error::from(io::ErrorKind::NotFound)),
            Self::Index(index) => match address {
                SocketAddr::V4(_) => socket.bind_device_by_index_v4(Some(index)),
                SocketAddr::V6(_) => socket.bind_device_by_index_v6(Some(index)),
            },
        }
    }
}

/// Nothing: Windows binds the interface's address alone, which its strong
/// host model sends from that interface (ADR 0044).
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "illumos")))]
#[derive(Clone, Debug)]
struct Device;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "illumos")))]
impl Device {
    fn of(_interface: Option<&str>) -> Self {
        Self
    }

    fn bind(&self, _socket: &socket2::SockRef<'_>, _address: SocketAddr) -> io::Result<()> {
        Ok(())
    }
}
