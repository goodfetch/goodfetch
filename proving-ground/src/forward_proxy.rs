//! The forward proxies (ADR 0059: proxies are tested against a real one,
//! never mocked). Each listens on its own address and relays one request per
//! client connection: plain HTTP in absolute form, its head rewritten to
//! origin form, or a `CONNECT` tunnel. It resolves a target's host the way
//! the DNS fixture answers it, so a Proving ground host name reaches its
//! site, and connects only to this host's loopback, so no test reaches a
//! public service. Each relay goes through an exit of the proxy's pool, the
//! same one for each sticky session its username names, and is recorded in
//! the connection record before the connection it opens is accepted.
//!
//! An authenticated proxy takes Basic credentials (`407` without them); one
//! may refuse every tunnel (`403`), and one relays one request at a time,
//! answering any other `429` as a vendor's plan does: it counts a request
//! until it read the site's whole answer, before relaying it, and counts
//! none for a site's robots.txt or icon. It holds each request it counts
//! [`LIMITED_HOLD`] before relaying it, or, while a harness turned its hold
//! on (`PUT /limited-proxy-hold`), until the harness turns it off. A proxy's own refusal names its cause in
//! `Proxy-Status` (RFC 9209). The SOCKS5 proxy ([`socks5`]) takes the same
//! credentials by RFC 1929 and relays one `CONNECT` per connection.
#![expect(
    clippy::disallowed_types,
    reason = "the forward proxies open their own sockets, outside the engine's transport (ADR 0060)"
)]

mod socks5;
mod username;

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::connection_record::{Carried, Outcome, RelayRequest};
use crate::dns::Zone;
use crate::generator;
use crate::observation::Observations;
use crate::sync::Mutex;

use self::username::Username;

/// The largest request head a proxy reads (STYLE.md: every buffer is
/// bounded).
const HEAD_LIMIT: usize = 64 * 1024;
/// The most headers a request head may carry.
const HEADER_LIMIT: usize = 128;
/// The most sticky sessions a proxy keeps the exit of; a later one takes an
/// exit each time.
const SESSION_LIMIT: usize = 100_000;
/// The generator place of the proxies' credentials.
const CREDENTIALS_PLACE: u64 = 0x7072_6f78;
/// How long the `limited` proxy holds each request before it relays it, so
/// two sent at once are in flight together.
const LIMITED_HOLD: Duration = Duration::from_millis(300);
/// The most of a site's answer the `limited` proxy reads before it relays
/// any of it (STYLE.md: every buffer is bounded); past it, the rest is
/// relayed as it comes.
const ANSWER_LIMIT: u64 = 1024 * 1024;
/// The paths the `limited` proxy counts no request for: a site's
/// robots.txt, which an engine reads of its own beside a page, and its
/// icon, which a browser asks for of its own once a page loaded (ADRs 0086
/// and 0088). It counts the pages a client loads.
const UNCOUNTED: [&str; 2] = ["/robots.txt", "/favicon.ico"];

/// Who a proxy relays for.
#[derive(Clone, Copy)]
pub(crate) enum Access {
    /// Anyone, without credentials.
    Open,
    /// Anyone, without credentials, but never a tunnel.
    RefusingTunnels,
    /// Anyone, without credentials, one request at a time: any other
    /// arriving meanwhile is answered `429`.
    Limited,
    /// Only a client with the reported credentials.
    Authenticated,
    /// Only a SOCKS5 client with the reported credentials (RFC 1929).
    Socks5,
}

/// The username and password the authenticated proxies take, derived from
/// the seed as login accounts are (ADR 0063).
#[derive(Serialize)]
pub(crate) struct Credentials {
    pub(crate) username: String,
    pub(crate) password: String,
}

impl Credentials {
    pub(crate) fn new(seed: u64) -> Self {
        let draw = |part| generator::draw(seed, &[CREDENTIALS_PLACE, part]);
        Self {
            username: format!("pg{:08x}", draw(1) >> 32),
            password: format!("{:016x}{:08x}", draw(2), draw(3) >> 32),
        }
    }
}

/// One proxy: its name, its access and its exits.
pub(crate) struct Proxy {
    pub(crate) name: &'static str,
    /// The proxy kind it serves, if it is one of a kind's.
    pub(crate) kind: Option<&'static str>,
    pub(crate) access: Access,
    exits: Mutex<Exits>,
    /// How many requests it relays now.
    in_flight: AtomicUsize,
}

/// A proxy's pool of exits and the one each sticky session has.
struct Exits {
    names: Vec<String>,
    sessions: BTreeMap<String, usize>,
    next: usize,
}

impl Exits {
    /// The exit for a relay: the session's own, or the next one.
    fn take(&mut self, session: Option<&str>) -> String {
        if let Some(index) = session.and_then(|session| self.sessions.get(session)) {
            return self.names[*index].clone();
        }
        let index = self.next % self.names.len();
        self.next += 1;
        if let Some(session) = session
            && self.sessions.len() < SESSION_LIMIT
        {
            self.sessions.insert(session.to_owned(), index);
        }
        self.names[index].clone()
    }
}

impl Proxy {
    /// A proxy whose one exit is named after it.
    pub(crate) fn single(name: &'static str, access: Access) -> Self {
        Self::with_exits(name, None, access, vec![name.to_owned()])
    }

    /// The authenticated proxy of a proxy kind, with `count` exits named
    /// `<kind>-1` onwards.
    pub(crate) fn of_kind(kind: &'static str, count: usize) -> Self {
        let exits = (1..=count).map(|index| format!("{kind}-{index}")).collect();
        Self::with_exits(kind, Some(kind), Access::Authenticated, exits)
    }

    fn with_exits(
        name: &'static str,
        kind: Option<&'static str>,
        access: Access,
        names: Vec<String>,
    ) -> Self {
        Self {
            name,
            kind,
            access,
            exits: Mutex::new(Exits {
                names,
                sessions: BTreeMap::new(),
                next: 0,
            }),
            in_flight: AtomicUsize::new(0),
        }
    }
}

/// What every proxy shares: where it records, how it resolves, the
/// credentials it takes, and this host's address.
pub(crate) struct Context {
    pub(crate) observations: Arc<Observations>,
    pub(crate) zone: Arc<Zone>,
    pub(crate) credentials: Credentials,
    /// The address the site is served on, which a proxy may connect to
    /// beside loopback.
    pub(crate) site: IpAddr,
}

/// A bound proxy.
pub(crate) struct ForwardProxy {
    listener: TcpListener,
    proxy: Arc<Proxy>,
}

impl ForwardProxy {
    /// Binds `proxy` on port 0 of `ip`.
    pub(crate) async fn bind(ip: IpAddr, proxy: Proxy) -> Result<Self> {
        let listener = TcpListener::bind((ip, 0))
            .await
            .with_context(|| format!("binding the {} proxy", proxy.name))?;
        Ok(Self {
            listener,
            proxy: Arc::new(proxy),
        })
    }

    pub(crate) fn name(&self) -> &'static str {
        self.proxy.name
    }

    /// The bound address.
    pub(crate) fn address(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .context("reading a proxy's address")
    }

    /// Relays every connection until `stop`, each on a task of `tracker`.
    pub(crate) async fn run(
        self,
        context: Arc<Context>,
        tracker: TaskTracker,
        stop: CancellationToken,
    ) {
        loop {
            let accepted = tokio::select! {
                () = stop.cancelled() => return,
                accepted = self.listener.accept() => accepted,
            };
            let Ok((stream, source)) = accepted else {
                // A connection that failed before it was accepted concerns
                // only its client.
                continue;
            };
            let proxy = Arc::clone(&self.proxy);
            let context = Arc::clone(&context);
            let stop = stop.clone();
            tracker.spawn(async move {
                let relayed = async {
                    match proxy.access {
                        Access::Socks5 => socks5::relay(stream, source, &proxy, &context).await,
                        Access::Open
                        | Access::RefusingTunnels
                        | Access::Limited
                        | Access::Authenticated => {
                            relay(stream, source, &proxy, &context).await;
                        }
                    }
                };
                tokio::select! {
                    () = relayed => {}
                    () = stop.cancelled() => {}
                }
            });
        }
    }
}

/// A request head as read, and the bytes read past it.
struct Head {
    method: String,
    target: String,
    headers: Vec<(String, Vec<u8>)>,
    read: usize,
    rest: Vec<u8>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_slice())
    }
}

/// Reads a request head; `None` if the client sends none the proxy reads.
async fn read_head(stream: &mut TcpStream) -> Option<Head> {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let mut headers = [httparse::EMPTY_HEADER; HEADER_LIMIT];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&buffer).ok()? {
            httparse::Status::Complete(end) => {
                return Some(Head {
                    method: request.method?.to_owned(),
                    target: request.path?.to_owned(),
                    headers: request
                        .headers
                        .iter()
                        .map(|header| (header.name.to_owned(), header.value.to_vec()))
                        .collect(),
                    read: buffer.len(),
                    rest: buffer[end..].to_vec(),
                });
            }
            httparse::Status::Partial if buffer.len() < HEAD_LIMIT => {}
            httparse::Status::Partial => return None,
        }
    }
}

/// The username and password of a `Proxy-Authorization: Basic` value.
fn basic(value: &[u8]) -> Option<(String, String)> {
    let encoded = std::str::from_utf8(value).ok()?.strip_prefix("Basic ")?;
    let decoded = data_encoding::BASE64
        .decode(encoded.trim().as_bytes())
        .ok()?;
    let (username, password) = String::from_utf8(decoded)
        .ok()?
        .split_once(':')
        .map(|(username, password)| (username.to_owned(), password.to_owned()))?;
    Some((username, password))
}

/// Where a request asks to go: its host and port, and for plain HTTP the
/// path its head is rewritten to.
struct Destination {
    host: String,
    port: u16,
    path: Option<String>,
}

/// Reads a request's target: `host:port` for `CONNECT`, an `http://`
/// absolute URL otherwise.
fn destination(method: &str, target: &str) -> Option<Destination> {
    let (authority, path, default_port) = if method.eq_ignore_ascii_case("CONNECT") {
        (target, None, 443)
    } else {
        let rest = target.strip_prefix("http://")?;
        let at = rest.find('/').unwrap_or(rest.len());
        let path = if at == rest.len() { "/" } else { &rest[at..] };
        (&rest[..at], Some(path.to_owned()), 80)
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.ends_with(':') && !port.contains(']') => {
            (host, port.parse().ok()?)
        }
        _ => (authority, default_port),
    };
    (!host.is_empty()).then(|| Destination {
        host: host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port,
        path,
    })
}

/// A proxy's own answer, as it writes it before closing.
fn answer(status: &str, extra: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n")
        .into_bytes()
}

/// A proxy's refusal naming its cause in `Proxy-Status` (RFC 9209).
fn failure(status: &str, error: &str) -> Vec<u8> {
    answer(
        status,
        &format!("Proxy-Status: proving-ground; error={error}\r\n"),
    )
}

/// Relays one client connection.
async fn relay(mut client: TcpStream, source: SocketAddr, proxy: &Proxy, context: &Context) {
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
    let Some(head) = read_head(&mut client).await else {
        let _ = client
            .write_all(&failure("400 Bad Request", "http_request_error"))
            .await;
        return;
    };
    let credentials = head.header("proxy-authorization").and_then(basic);
    let mut request = RelayRequest {
        proxy: proxy.name,
        kind: proxy.kind,
        connection,
        method: head.method.clone(),
        target: head.target.clone(),
        task: None,
        username: credentials.as_ref().map(|(username, _)| username.clone()),
        sticky: None,
        exit: None,
    };
    let admitted =
        admit(proxy, context, &head, credentials.as_ref(), &mut request).and_then(|admitted| {
            let in_flight = InFlight::take(proxy, &admitted.1)?;
            Ok((admitted, in_flight))
        });
    match admitted {
        Ok(((address, destination), in_flight)) => {
            if in_flight.is_some() {
                if context.observations.limited_held() {
                    context.observations.limited_released().await;
                } else {
                    tokio::time::sleep(LIMITED_HOLD).await;
                }
            }
            carry(
                client,
                context,
                &head,
                (request, address, &destination),
                in_flight,
            )
            .await;
        }
        Err(refused) => {
            let record = observations.connection_record();
            let outcome = refused.outcome;
            let relay = record.relay(
                observations.stamp(),
                request,
                refused.destination,
                None,
                outcome,
            );
            let answer = refused.answer;
            let sent = client.write_all(&answer).await.map_or(0, |()| answer.len());
            record.ended(relay, outcome, carried(head.read, sent, (0, 0)));
        }
    }
}

/// A relay the proxy answers itself, without connecting anywhere.
struct Refused {
    /// The address the target resolved to, if it did.
    destination: Option<SocketAddr>,
    outcome: Outcome,
    answer: Vec<u8>,
}

impl Refused {
    fn new(outcome: Outcome, answer: Vec<u8>) -> Self {
        Self {
            destination: None,
            outcome,
            answer,
        }
    }
}

/// A request the `limited` proxy relays, counted until dropped: once the
/// proxy read the site's whole answer, before its client receives any of
/// it, so a client that loaded the answer never finds the proxy still
/// counting it, whenever it closes its connection.
struct InFlight<'proxy>(&'proxy Proxy);

impl<'proxy> InFlight<'proxy> {
    /// Counts one more request of `proxy` to `destination` in flight; for
    /// the `limited` proxy, a refusal when another is already, and nothing
    /// to count for any other proxy, nor for an [`UNCOUNTED`] path.
    fn take(proxy: &'proxy Proxy, destination: &Destination) -> Result<Option<Self>, Refused> {
        let uncounted = destination.path.as_deref().is_some_and(|path| {
            let path = path.split_once('?').map_or(path, |(path, _)| path);
            UNCOUNTED.contains(&path)
        });
        if !matches!(proxy.access, Access::Limited) || uncounted {
            return Ok(None);
        }
        if proxy.in_flight.fetch_add(1, Ordering::SeqCst) > 0 {
            proxy.in_flight.fetch_sub(1, Ordering::SeqCst);
            let answer = failure("429 Too Many Requests", "http_request_denied");
            return Err(Refused::new(Outcome::TooManyRequests, answer));
        }
        Ok(Some(Self(proxy)))
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Decides whether the proxy relays the request, filling in what the relay
/// records: its task, sticky session and exit. A relay it carries out goes
/// to the address returned.
fn admit(
    proxy: &Proxy,
    context: &Context,
    head: &Head,
    credentials: Option<&(String, String)>,
    request: &mut RelayRequest,
) -> Result<(SocketAddr, Destination), Refused> {
    let tunnel = head.method.eq_ignore_ascii_case("CONNECT");
    let sticky = match proxy.access {
        Access::Authenticated => {
            let Some(username) = accepted(credentials, &context.credentials) else {
                let answer = answer(
                    "407 Proxy Authentication Required",
                    "Proxy-Authenticate: Basic realm=\"proving-ground\"\r\n",
                );
                return Err(Refused::new(Outcome::ProxyAuthenticationRequired, answer));
            };
            match username {
                Username::Reported => None,
                Username::Sticky(sticky) => Some(sticky),
            }
        }
        Access::RefusingTunnels if tunnel => {
            let answer = failure("403 Forbidden", "http_request_denied");
            return Err(Refused::new(Outcome::TunnelRefused, answer));
        }
        Access::Open | Access::RefusingTunnels | Access::Limited | Access::Socks5 => None,
    };
    let Some(destination) = destination(&head.method, &head.target) else {
        let answer = failure("400 Bad Request", "http_request_error");
        return Err(Refused::new(Outcome::Malformed, answer));
    };
    request.task = context.observations.task_of(&destination.host);
    let session = sticky.as_ref().and_then(|sticky| sticky.session.as_deref());
    request.exit = Some(proxy.exits.lock().take(session));
    request.sticky = sticky;
    let Some(ip) = context.zone.resolve(&destination.host) else {
        let answer = failure("502 Bad Gateway", "dns_error");
        return Err(Refused::new(Outcome::Unresolved, answer));
    };
    let address = SocketAddr::new(ip, destination.port);
    if !(ip.is_loopback() || ip == context.site) {
        return Err(Refused {
            destination: Some(address),
            outcome: Outcome::DestinationProhibited,
            answer: failure("403 Forbidden", "destination_ip_prohibited"),
        });
    }
    Ok((address, destination))
}

/// Connects to `address` for the client and relays between them until
/// either closes. A plain request `in_flight` counts reads the site's
/// answer first, up to [`ANSWER_LIMIT`], and stops counting before it
/// relays it.
async fn carry(
    mut client: TcpStream,
    context: &Context,
    head: &Head,
    (request, address, destination): (RelayRequest, SocketAddr, &Destination),
    in_flight: Option<InFlight<'_>>,
) {
    let Ok((socket, side)) = bound_socket(address.ip()) else {
        let answer = failure("502 Bad Gateway", "proxy_internal_error");
        let _ = client.write_all(&answer).await;
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
        let answer = failure("502 Bad Gateway", "connection_refused");
        let sent = client.write_all(&answer).await.map_or(0, |()| answer.len());
        let bytes = carried(head.read, sent, (0, 0));
        record.ended(relay, Outcome::ConnectionRefused, bytes);
        return;
    };
    let tunnel = destination.path.is_none();
    let established: &[u8] = if tunnel {
        b"HTTP/1.1 200 Connection Established\r\n\r\n"
    } else {
        &[]
    };
    let opened = if tunnel {
        client.write_all(established).await
    } else {
        upstream.write_all(&origin_form(head, destination)).await
    };
    let copied = match opened {
        Ok(()) => match upstream.write_all(&head.rest).await {
            Ok(()) if !tunnel && in_flight.is_some() => {
                answered_first(&mut client, &mut upstream, in_flight).await
            }
            Ok(()) => tokio::io::copy_bidirectional(&mut client, &mut upstream)
                .await
                .unwrap_or_default(),
            Err(_) => (0, 0),
        },
        Err(_) => (0, 0),
    };
    let bytes = carried(head.read, established.len(), copied);
    record.ended(relay, Outcome::Relayed, bytes);
}

/// Reads the site's answer on `upstream`, up to [`ANSWER_LIMIT`], ends
/// `in_flight`, then relays the answer to `client`, and anything past the
/// limit either way: the bytes copied each way, as
/// `copy_bidirectional` counts them.
async fn answered_first(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    in_flight: Option<InFlight<'_>>,
) -> (u64, u64) {
    let mut answer = Vec::new();
    let whole = match (&mut *upstream)
        .take(ANSWER_LIMIT)
        .read_to_end(&mut answer)
        .await
    {
        Ok(read) => u64::try_from(read).unwrap_or(u64::MAX) < ANSWER_LIMIT,
        Err(_) => true,
    };
    drop(in_flight);
    let sent = u64::try_from(answer.len()).unwrap_or(u64::MAX);
    if client.write_all(&answer).await.is_err() {
        return (0, 0);
    }
    if whole {
        let _ = client.shutdown().await;
        return (0, sent);
    }
    let (out, back) = tokio::io::copy_bidirectional(client, upstream)
        .await
        .unwrap_or_default();
    (out, sent.saturating_add(back))
}

/// The bytes of a relay: the head received and the answer sent, then what
/// was copied each way.
fn carried(received: usize, sent: usize, copied: (u64, u64)) -> Carried {
    let count = |bytes: usize| u64::try_from(bytes).unwrap_or(u64::MAX);
    Carried {
        received: count(received).saturating_add(copied.0),
        sent: count(sent).saturating_add(copied.1),
    }
}

/// The username the credentials name, if the proxy takes them.
fn accepted(given: Option<&(String, String)>, credentials: &Credentials) -> Option<Username> {
    let (username, password) = given?;
    if *password != credentials.password {
        return None;
    }
    username::read(username, &credentials.username)
}

/// A socket bound on this host before it connects to `ip`, and its address,
/// so the relay is recorded with it before the connection is accepted.
fn bound_socket(ip: IpAddr) -> std::io::Result<(TcpSocket, SocketAddr)> {
    let socket = if ip.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.bind(SocketAddr::new(ip, 0))?;
    let side = socket.local_addr()?;
    Ok((socket, side))
}

/// A plain HTTP request's head in origin form, without the proxy's own
/// headers, closing after its answer.
fn origin_form(head: &Head, destination: &Destination) -> Vec<u8> {
    let path = destination.path.as_deref().unwrap_or("/");
    let mut rewritten = format!("{} {path} HTTP/1.1\r\n", head.method).into_bytes();
    if head.header("host").is_none() {
        rewritten.extend_from_slice(format!("Host: {}\r\n", destination.host).as_bytes());
    }
    for (name, value) in &head.headers {
        let hop = [
            "proxy-authorization",
            "proxy-connection",
            "connection",
            "keep-alive",
        ]
        .iter()
        .any(|hop| name.eq_ignore_ascii_case(hop));
        if !hop {
            rewritten.extend_from_slice(name.as_bytes());
            rewritten.extend_from_slice(b": ");
            rewritten.extend_from_slice(value);
            rewritten.extend_from_slice(b"\r\n");
        }
    }
    rewritten.extend_from_slice(b"Connection: close\r\n\r\n");
    rewritten
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::connect("CONNECT", "catalog.test:8443", Some(("catalog.test", 8443, None)))]
    #[case::connect_without_a_port("CONNECT", "catalog.test", Some(("catalog.test", 443, None)))]
    #[case::ipv6("CONNECT", "[::1]:8443", Some(("::1", 8443, None)))]
    #[case::absolute("GET", "http://catalog.test:8080/catalog/1?page=2", Some(("catalog.test", 8080, Some("/catalog/1?page=2"))))]
    #[case::absolute_without_a_path("GET", "http://catalog.test", Some(("catalog.test", 80, Some("/"))))]
    #[case::https("GET", "https://catalog.test/", None)]
    #[case::origin_form("GET", "/catalog/1", None)]
    fn a_request_target_names_where_it_goes(
        #[case] method: &str,
        #[case] target: &str,
        #[case] expected: Option<(&str, u16, Option<&str>)>,
    ) {
        let read = destination(method, target);

        assert_eq!(
            read.as_ref()
                .map(|read| (read.host.as_str(), read.port, read.path.as_deref())),
            expected
        );
    }
}
