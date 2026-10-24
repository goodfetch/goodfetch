//! The SOCKS5 proxy (RFC 1928): it takes only the username and password
//! method (RFC 1929) with the authenticated proxies' credentials, then one
//! `CONNECT` per client connection, to a host it resolves as the DNS fixture
//! answers or to an address, and only to this host's loopback or the site's
//! address. Each relay is recorded as the forward proxies' are, its method
//! `CONNECT`, its target `host:port` and its username, never its password,
//! so a test sees which Session's credentials reached it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use super::{Context, Proxy, Username, accepted, bound_socket, carried};
use crate::connection_record::{Outcome, RelayRequest};

/// SOCKS's version 5.
const VERSION: u8 = 5;
/// The username and password method, the only one it takes.
const USERNAME_PASSWORD: u8 = 2;
/// The answer that no method a client offers is taken.
const NO_ACCEPTABLE_METHOD: u8 = 0xff;
/// RFC 1929's version.
const CREDENTIALS_VERSION: u8 = 1;
/// The one command it carries out.
const CONNECT: u8 = 1;
/// Its replies: success, refused by its rules, a host it cannot reach, a
/// refused connection, and an unsupported command or address type.
const SUCCEEDED: u8 = 0;
const NOT_ALLOWED: u8 = 2;
const HOST_UNREACHABLE: u8 = 4;
const CONNECTION_REFUSED: u8 = 5;
const COMMAND_NOT_SUPPORTED: u8 = 7;
const ADDRESS_NOT_SUPPORTED: u8 = 8;
/// The address types: IPv4, a domain name and IPv6.
const IPV4: u8 = 1;
const DOMAIN: u8 = 3;
const IPV6: u8 = 4;

/// Relays one client connection the SOCKS5 way.
pub(super) async fn relay(client: TcpStream, source: SocketAddr, proxy: &Proxy, context: &Context) {
    let Ok(listening) = client.local_addr() else {
        return;
    };
    let observations = &context.observations;
    let connection = observations.connection_record().accepted(
        observations.stamp(),
        proxy.name,
        source,
        listening,
    );
    let request = RelayRequest {
        proxy: proxy.name,
        kind: proxy.kind,
        connection,
        method: "CONNECT".to_owned(),
        target: String::new(),
        task: None,
        username: None,
        sticky: None,
        exit: None,
    };
    let mut exchange = Exchange {
        client,
        received: 0,
        sent: 0,
        context,
    };
    let Some((username, request)) = exchange.authenticate(request).await else {
        return;
    };
    exchange.connect(request, proxy, username).await;
}

/// One client connection, with the bytes it crossed so far.
struct Exchange<'a> {
    client: TcpStream,
    received: usize,
    sent: usize,
    context: &'a Context,
}

impl Exchange<'_> {
    /// Takes the client's credentials: the username they name, once the
    /// proxy took them, and `request` with it; a refusal is recorded.
    async fn authenticate(
        &mut self,
        mut request: RelayRequest,
    ) -> Option<(Username, RelayRequest)> {
        let methods = self.read_methods().await.ok()?;
        if !methods.contains(&USERNAME_PASSWORD) {
            self.refuse(
                &[VERSION, NO_ACCEPTABLE_METHOD],
                request,
                Outcome::ProxyAuthenticationRequired,
                None,
            )
            .await;
            return None;
        }
        self.write(&[VERSION, USERNAME_PASSWORD]).await;
        let given = self.read_credentials().await.ok()?;
        request.username = Some(given.0.clone());
        let Some(username) = accepted(Some(&given), &self.context.credentials) else {
            self.refuse(
                &[CREDENTIALS_VERSION, 1],
                request,
                Outcome::ProxyAuthenticationRequired,
                None,
            )
            .await;
            return None;
        };
        self.write(&[CREDENTIALS_VERSION, 0]).await;
        Some((username, request))
    }

    /// Reads the client's `CONNECT`, connects to where it names, and relays
    /// between them until either closes.
    async fn connect(mut self, mut request: RelayRequest, proxy: &Proxy, username: Username) {
        let Ok((command, host, port)) = self.read_request().await else {
            self.refuse(
                &reply(ADDRESS_NOT_SUPPORTED),
                request,
                Outcome::Malformed,
                None,
            )
            .await;
            return;
        };
        request.target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        if command != CONNECT {
            self.refuse(
                &reply(COMMAND_NOT_SUPPORTED),
                request,
                Outcome::Malformed,
                None,
            )
            .await;
            return;
        }
        let context = self.context;
        request.task = context.observations.task_of(&host);
        let sticky = match username {
            Username::Reported => None,
            Username::Sticky(sticky) => Some(sticky),
        };
        let session = sticky.as_ref().and_then(|sticky| sticky.session.as_deref());
        request.exit = Some(proxy.exits.lock().take(session));
        request.sticky = sticky;
        let resolved = host
            .parse::<IpAddr>()
            .ok()
            .or_else(|| context.zone.resolve(&host));
        let Some(ip) = resolved else {
            self.refuse(&reply(HOST_UNREACHABLE), request, Outcome::Unresolved, None)
                .await;
            return;
        };
        let address = SocketAddr::new(ip, port);
        if !(ip.is_loopback() || ip == context.site) {
            self.refuse(
                &reply(NOT_ALLOWED),
                request,
                Outcome::DestinationProhibited,
                Some(address),
            )
            .await;
            return;
        }
        let Ok((socket, side)) = bound_socket(ip) else {
            self.write(&reply(CONNECTION_REFUSED)).await;
            return;
        };
        let observations = &context.observations;
        let record = observations.connection_record();
        let relay = record.relay(
            observations.stamp(),
            request,
            Some(address),
            Some(side),
            Outcome::Relayed,
        );
        let Ok(mut upstream) = socket.connect(address).await else {
            self.write(&reply(CONNECTION_REFUSED)).await;
            let bytes = carried(self.received, self.sent, (0, 0));
            record.ended(relay, Outcome::ConnectionRefused, bytes);
            return;
        };
        self.write(&reply(SUCCEEDED)).await;
        let copied = tokio::io::copy_bidirectional(&mut self.client, &mut upstream)
            .await
            .unwrap_or_default();
        let bytes = carried(self.received, self.sent, copied);
        record.ended(relay, Outcome::Relayed, bytes);
    }

    /// Refuses the relay with `answer`, `destination` the address the target
    /// resolved to if it did. The relay is recorded before the answer goes,
    /// as the forward proxies record theirs, so a client that read the
    /// answer finds the refusal in the connection record.
    async fn refuse(
        &mut self,
        answer: &[u8],
        request: RelayRequest,
        outcome: Outcome,
        destination: Option<SocketAddr>,
    ) {
        let observations = &self.context.observations;
        let record = observations.connection_record();
        let relay = record.relay(observations.stamp(), request, destination, None, outcome);
        self.write(answer).await;
        record.ended(relay, outcome, carried(self.received, self.sent, (0, 0)));
    }

    /// Writes `bytes`, counting them once they went.
    async fn write(&mut self, bytes: &[u8]) {
        if self.client.write_all(bytes).await.is_ok() {
            self.sent += bytes.len();
        }
    }

    /// Reads exactly `count` bytes, counting them.
    async fn read(&mut self, count: usize) -> std::io::Result<Vec<u8>> {
        let mut bytes = vec![0; count];
        self.client.read_exact(&mut bytes).await?;
        self.received += count;
        Ok(bytes)
    }

    /// The methods the client's greeting offers.
    async fn read_methods(&mut self) -> std::io::Result<Vec<u8>> {
        let head = self.read(2).await?;
        if head[0] != VERSION {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        self.read(usize::from(head[1])).await
    }

    /// The client's username and password (RFC 1929).
    async fn read_credentials(&mut self) -> std::io::Result<(String, String)> {
        let head = self.read(2).await?;
        if head[0] != CREDENTIALS_VERSION {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let username = self.read(usize::from(head[1])).await?;
        let length = self.read(1).await?;
        let password = self.read(usize::from(length[0])).await?;
        Ok((text(username)?, text(password)?))
    }

    /// The client's request: its command, the host it names and the port.
    async fn read_request(&mut self) -> std::io::Result<(u8, String, u16)> {
        let head = self.read(4).await?;
        if head[0] != VERSION {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let host = match head[3] {
            IPV4 => {
                let bytes: [u8; 4] = self.read(4).await?.try_into().map_err(|_| invalid())?;
                Ipv4Addr::from(bytes).to_string()
            }
            IPV6 => {
                let bytes: [u8; 16] = self.read(16).await?.try_into().map_err(|_| invalid())?;
                Ipv6Addr::from(bytes).to_string()
            }
            DOMAIN => {
                let length = self.read(1).await?;
                text(self.read(usize::from(length[0])).await?)?
            }
            _ => return Err(invalid()),
        };
        let port = self.read(2).await?;
        Ok((head[1], host, u16::from_be_bytes([port[0], port[1]])))
    }
}

/// The error of bytes that are no SOCKS5 message.
fn invalid() -> std::io::Error {
    std::io::ErrorKind::InvalidData.into()
}

/// `bytes` as UTF-8 text.
fn text(bytes: Vec<u8>) -> std::io::Result<String> {
    String::from_utf8(bytes).map_err(|_| invalid())
}

/// A reply carrying `code` and no bound address.
fn reply(code: u8) -> [u8; 10] {
    [VERSION, code, 0, IPV4, 0, 0, 0, 0, 0, 0]
}
