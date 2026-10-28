//! The egress relay's connections in process, on every OS (spec #564's
//! Testing Decisions): a browser end and the network beyond the relay are
//! in-memory streams, names resolve through the Scripted resolver, and the
//! oracle is the list of connections the relay opened, which stands for
//! the Proving ground's connection record.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::unused_async_trait_impl,
    reason = "the test network answers at once, through the async interface the relay calls"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "each far end of the test network runs as a local task beside the relay, on the test's own runtime (ADR 0060)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the test network's records are shared with the relay's task under a std lock (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;

use data_encoding::BASE64;
use goodfetch_engine::egress::{Policy, Refused};
use goodfetch_engine::secrets::ExceptionHosts;
use goodfetch_engine::tls::{CertificateExceptions, ExceptionUsed, Trust};
use goodfetch_protocol::egress::EgressRule;
use goodfetch_test_support::{
    Outcome, Refusal, ResolverHandle, Route, Runner, ScriptedResolver, SsrfCase, SsrfTable,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, duplex};

use super::authority::Authority;
use super::client_hello::{self, ClientHello};
use super::serve::{
    Met, Network, Observe, ProxyRefused, Relaying, Resolved, Served, Unreached, Upstream, serve,
};
use super::terminate::{Stop, Terminating};

/// The address that stands for a public one.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 31, 196, 1));

/// The Session's credentials, as the browser sends them.
fn authorization() -> String {
    format!("Basic {}", BASE64.encode(b"goodfetch:session-secret"))
}

/// A relaying of the Run's `policy` to `upstream`.
fn relaying(policy: Policy, upstream: Upstream) -> Relaying {
    Relaying {
        policy,
        authorization: authorization(),
        upstream,
        bypasses: Box::new(|_| false),
        terminating: None,
        clock: goodfetch_engine::clock::Clock::system(),
        connect_deadline: CONNECT_DEADLINE,
    }
}

/// The connect deadline of the relays these tests build.
const CONNECT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

fn every_host() -> Policy {
    Policy::new(&["*"], &[] as &[&str]).unwrap()
}

/// What the far end of a connection answers its first bytes with.
#[derive(Clone)]
enum Far {
    /// A site that answers any request with an empty 200, and records its
    /// head.
    Site,
    /// An upstream proxy that answers its first head with `answer`, then
    /// acts as the site.
    Proxy { answer: &'static str },
    /// An upstream proxy whose own address the egress policy refuses.
    RefusedProxy(Refused),
    /// An upstream proxy whose connection never opens: a black-holed
    /// address, which drops every SYN.
    Silent,
}

/// The network beyond the relay: names through the Scripted resolver or
/// the mapping, and every connection opened recorded.
struct Scripted {
    resolver: Option<ResolverHandle>,
    mapping: BTreeMap<String, Vec<IpAddr>>,
    far: Far,
    /// Each connection's address, `None` for the upstream proxy's.
    connections: Mutex<Vec<Option<SocketAddr>>>,
    /// The first bytes each far end read.
    heads: std::sync::Arc<Mutex<Vec<String>>>,
}

impl Scripted {
    fn new(resolver: Option<&ScriptedResolver>, far: Far) -> Self {
        Self {
            resolver: resolver.map(ScriptedResolver::handle),
            mapping: BTreeMap::new(),
            far,
            connections: Mutex::default(),
            heads: std::sync::Arc::default(),
        }
    }

    fn connections(&self) -> Vec<Option<SocketAddr>> {
        self.connections.lock().unwrap().clone()
    }

    fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }

    fn open(&self, to: Option<SocketAddr>) -> DuplexStream {
        self.connections.lock().unwrap().push(to);
        let (near, mut far_end) = duplex(1 << 16);
        let (far, heads) = (self.far.clone(), std::sync::Arc::clone(&self.heads));
        // The far end runs beside the relay on the test's runtime.
        drop(tokio::task::spawn_local(async move {
            let head = read_head(&mut far_end).await;
            heads.lock().unwrap().push(head);
            if let Far::Proxy { answer } = far {
                far_end.write_all(answer.as_bytes()).await.unwrap();
                if !answer.starts_with("HTTP/1.1 200") {
                    return;
                }
                let head = read_head(&mut far_end).await;
                heads.lock().unwrap().push(head);
            }
            let _ = far_end
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .await;
        }));
        near
    }
}

async fn read_head(stream: &mut DuplexStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

impl Network for Scripted {
    type Stream = DuplexStream;

    async fn resolve(&self, host: &str, port: u16) -> Option<Resolved> {
        let on_port = |addresses: &[IpAddr]| {
            addresses
                .iter()
                .map(|address| SocketAddr::new(*address, port))
                .collect()
        };
        if let Some(mapped) = self.mapping.get(host) {
            return Some(Resolved {
                addresses: on_port(mapped),
            });
        }
        let addresses = match host.parse::<IpAddr>() {
            Ok(address) => vec![address],
            Err(_) => self.resolver.as_ref()?.lookup(host),
        };
        Some(Resolved {
            addresses: on_port(&addresses),
        })
    }

    async fn connect(&self, address: SocketAddr) -> io::Result<DuplexStream> {
        Ok(self.open(Some(address)))
    }

    async fn connect_upstream(&self) -> io::Result<DuplexStream> {
        if let Far::RefusedProxy(refused) = &self.far {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                ProxyRefused(refused.clone()),
            ));
        }
        if let Far::Silent = &self.far {
            return std::future::pending().await;
        }
        Ok(self.open(None))
    }
}

/// What the relay told the Session.
#[derive(Default)]
struct Told {
    refused: Mutex<Vec<(Refused, String)>>,
    terminated: Mutex<Vec<(String, Option<ExceptionUsed>)>>,
    stopped: Mutex<Vec<(String, Stop)>>,
    met: Mutex<Vec<Met>>,
    unauthenticated: Mutex<Vec<bool>>,
    unreached: Mutex<Vec<(String, Unreached)>>,
    client_hellos: Mutex<Vec<(String, ClientHello)>>,
}

impl Observe for Told {
    fn unreached(&self, authority: &str, unreached: Unreached) {
        self.unreached
            .lock()
            .unwrap()
            .push((authority.to_owned(), unreached));
    }

    fn reached(&self, _: &str) {}

    fn refused(&self, refused: &Refused, method: &str) {
        self.refused
            .lock()
            .unwrap()
            .push((refused.clone(), method.to_owned()));
    }

    fn upstream(&self, _: &str, met: Met) {
        self.met.lock().unwrap().push(met);
    }

    fn unauthenticated(&self, given: bool) {
        self.unauthenticated.lock().unwrap().push(given);
    }

    fn terminated(&self, host: &str, used: Option<&ExceptionUsed>) {
        self.terminated
            .lock()
            .unwrap()
            .push((host.to_owned(), used.cloned()));
    }

    fn stopped(&self, url: &str, _: &str, stop: &Stop) {
        self.stopped
            .lock()
            .unwrap()
            .push((url.to_owned(), stop.clone()));
    }

    fn client_hello(&self, authority: &str, hello: ClientHello) {
        self.client_hellos
            .lock()
            .unwrap()
            .push((authority.to_owned(), hello));
    }
}

/// Sends `request` through the relay as the browser, then `then` once the
/// relay answered a tunnel with a 200: the relay's answer, every byte the
/// browser read, and how the relay ended the connection.
async fn through(
    request: &str,
    then: &[u8],
    relaying: &Relaying,
    network: &Scripted,
    told: &Told,
) -> (String, Served) {
    let (mut browser, relayed) = duplex(1 << 16);
    let driving = async {
        browser.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            match browser.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => answer.extend_from_slice(&chunk[..read]),
            }
            if answer.starts_with(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                && !then.is_empty()
            {
                browser.write_all(then).await.unwrap();
                break;
            }
        }
        let mut rest = Vec::new();
        let _ = browser.read_to_end(&mut rest).await;
        answer.extend_from_slice(&rest);
        drop(browser);
        String::from_utf8_lossy(&answer).into_owned()
    };
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async { tokio::join!(driving, serve(relayed, relaying, network, told)) })
        .await
}

fn connect(authority: &str) -> String {
    format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: {}\r\n\r\n",
        authorization()
    )
}

const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: site\r\n\r\n";

/// Covers: story:4-10
#[rstest::rstest]
#[case::no_credentials(None)]
#[case::wrong_credentials(Some("Basic Z29vZGZldGNoOmd1ZXNz"))]
#[case::another_sessions(Some("Basic Z29vZGZldGNoOnNlc3Npb24tc2VjcmV0Mg=="))]
fn a_client_without_the_sessions_credentials_gets_a_407_and_no_relayed_byte(
    #[case] given: Option<&str>,
) {
    let network = Scripted::new(None, Far::Site);
    let told = Told::default();
    let relaying = relaying(every_host(), Upstream::Direct);
    let field = given.map_or(String::new(), |given| {
        format!("Proxy-Authorization: {given}\r\n")
    });
    let request =
        format!("CONNECT 192.31.196.1:443 HTTP/1.1\r\nHost: 192.31.196.1:443\r\n{field}\r\n");

    let (refused, served) = goodfetch_test_support::runtime()
        .block_on(through(&request, GET, &relaying, &network, &told));
    // The Session itself, with its credentials, is relayed.
    let (opened, relayed) = goodfetch_test_support::runtime().block_on(through(
        &connect("192.31.196.1:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    assert_eq!(
        (refused.lines().next(), &served),
        (
            Some("HTTP/1.1 407 Proxy Authentication Required"),
            &Served::Unauthenticated
        )
    );
    assert!(refused.contains("Proxy-Authenticate: Basic"), "{refused}");
    // Only wrong credentials are a refusal: every client's first request
    // carries none until the relay asks.
    assert_eq!(*told.unauthenticated.lock().unwrap(), vec![given.is_some()]);
    assert!(opened.ends_with("ok"), "{opened}");
    assert!(matches!(relayed, Served::Relayed { .. }), "{relayed:?}");
    assert_eq!(
        network.connections(),
        vec![Some(SocketAddr::new(PUBLIC, 443))]
    );
}

/// The Scripted resolver's rebinding cases (ADR 0067): a name that answers
/// a public address and then a private one reaches its public answer on
/// the first connection, the paired positive control, and its second is
/// refused naming the private address, before any connection to it; so in
/// terminating mode, the name listed for a certificate exception.
///
/// Covers: story:4-10, story:4-8, story:4-89, error:egress_refused
#[rstest::rstest]
#[case::tunnelled(Sent::AsATunnel)]
#[case::terminated(Sent::Terminated)]
fn a_name_that_rebinds_to_a_private_address_is_refused_on_its_next_connection_and_never_reached(
    #[case] sent: Sent,
) {
    let runner = Runner::committed(
        sent.layer(),
        file!(),
        "a_name_that_rebinds_to_a_private_address_is_refused_on_its_next_connection_and_never_reached",
    )
    .unwrap();
    let cases = runner.cases();
    let stores = super::terminate_tests::stores();

    let outcomes: Vec<_> = cases
        .iter()
        .map(|case| {
            let url = url::Url::parse(&case.url).unwrap();
            let host = url.host_str().unwrap();
            let authority = format!("{host}:443");
            let mut relaying = relaying(every_host(), Upstream::Direct);
            if let Sent::Terminated = sent {
                relaying.terminating = Some(std::sync::Arc::new(terminating_of(host, &stores)));
            }
            let resolver = ScriptedResolver::new(case.script(PUBLIC));
            let network = Scripted::new(Some(&resolver), Far::Site);
            let told = Told::default();
            let runtime = goodfetch_test_support::runtime();
            let first = runtime.block_on(through(
                &connect(&authority),
                GET,
                &relaying,
                &network,
                &told,
            ));
            let second = runtime.block_on(through(
                &connect(&authority),
                GET,
                &relaying,
                &network,
                &told,
            ));
            let refusals: Vec<_> = told
                .refused
                .lock()
                .unwrap()
                .iter()
                .map(|(refused, method)| (refused.rule, refused.address, method.clone()))
                .collect();
            (
                first.0.starts_with("HTTP/1.1 200 Connection Established"),
                second.0.lines().next().map(str::to_owned),
                refusals,
                network.connections(),
            )
        })
        .collect();

    assert_eq!(cases.len(), 3);
    assert_eq!(
        outcomes,
        cases
            .iter()
            .map(|case| (
                true,
                Some("HTTP/1.1 403 Forbidden".to_owned()),
                vec![(
                    EgressRule::PrivateAddress,
                    case.target_address(PUBLIC),
                    "CONNECT".to_owned()
                )],
                vec![Some(SocketAddr::new(PUBLIC, 443))],
            ))
            .collect::<Vec<_>>()
    );
}

/// Covers: story:4-10, story:4-12, error:egress_refused
#[test]
fn through_an_upstream_proxy_a_private_answer_is_refused_before_the_proxy_sees_it() {
    let resolver = ScriptedResolver::new([
        goodfetch_test_support::Lookup::new(
            "intranet.ssrf.test",
            [IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))],
        ),
        goodfetch_test_support::Lookup::new("public.ssrf.test", [PUBLIC]),
    ]);
    let network = Scripted::new(
        Some(&resolver),
        Far::Proxy {
            answer: "HTTP/1.1 200 Connection established\r\n\r\n",
        },
    );
    let told = Told::default();
    let upstream_credentials = format!("Basic {}", BASE64.encode(b"user:pass"));
    let relaying = relaying(
        every_host(),
        Upstream::Http {
            authorization: Some(upstream_credentials.clone()),
        },
    );
    let runtime = goodfetch_test_support::runtime();

    let refused = runtime.block_on(through(
        &connect("intranet.ssrf.test:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));
    let refused_connections = network.connections();
    let chained = runtime.block_on(through(
        &connect("public.ssrf.test:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    assert_eq!(
        (refused.0.lines().next(), refused.1, refused_connections),
        (Some("HTTP/1.1 403 Forbidden"), Served::Refused, vec![])
    );
    assert!(chained.0.ends_with("ok"), "{}", chained.0);
    // The proxy is asked for the name, with the Session's credentials for
    // it, never the relay's.
    let heads = network.heads();
    assert!(
        heads[0].starts_with("CONNECT public.ssrf.test:443 HTTP/1.1\r\n"),
        "{heads:?}"
    );
    assert!(heads[0].contains(&format!("Proxy-Authorization: {upstream_credentials}")));
    assert!(!heads[0].contains(&authorization()));
    assert_eq!(heads[1], String::from_utf8_lossy(GET));
    assert_eq!(network.connections(), vec![None]);
    assert_eq!(*told.met.lock().unwrap(), vec![Met::Reached]);
}

/// A proxy's 407 reaches the browser as the proxy's, its body left out, and
/// the Session learns that the proxy refused its credentials.
///
/// Covers: story:4-10, error:proxy_auth_failed
#[test]
fn an_upstream_proxy_answering_407_reaches_the_browser_and_marks_the_credentials_refused() {
    let resolver = ScriptedResolver::new([goodfetch_test_support::Lookup::new(
        "public.ssrf.test",
        [PUBLIC],
    )]);
    let network = Scripted::new(
        Some(&resolver),
        Far::Proxy {
            answer: "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"up\"\r\nContent-Length: 4\r\n\r\nnope",
        },
    );
    let told = Told::default();
    let relaying = relaying(
        every_host(),
        Upstream::Http {
            authorization: None,
        },
    );

    let (answer, served) = goodfetch_test_support::runtime().block_on(through(
        &connect("public.ssrf.test:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    assert_eq!(
        (answer.as_str(), served),
        (
            "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"up\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            Served::Failed
        )
    );
    assert_eq!(*told.met.lock().unwrap(), vec![Met::CredentialsRefused]);
}

/// An upstream proxy whose own address the egress policy refuses carries
/// nothing (ADR 0017; step 7 finding 564-2): the tunnel gets the relay's
/// `403`, the Session is told the refusal, naming the proxy's address and
/// the setting, and no connection opens.
///
/// Covers: story:4-16, error:egress_refused, setting:egress.no_private_endpoints
#[test]
fn an_upstream_proxy_whose_own_address_is_refused_carries_nothing() {
    let resolver = ScriptedResolver::new([goodfetch_test_support::Lookup::new(
        "public.ssrf.test",
        [PUBLIC],
    )]);
    let refused = Refused {
        url: "http://127.0.0.1:3128".to_owned(),
        rule: EgressRule::PrivateAddress,
        address: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        setting: Some("egress.no_private_endpoints"),
    };
    let network = Scripted::new(Some(&resolver), Far::RefusedProxy(refused));
    let told = Told::default();
    let relaying = relaying(
        every_host(),
        Upstream::Http {
            authorization: None,
        },
    );

    let (answer, served) = goodfetch_test_support::runtime().block_on(through(
        &connect("public.ssrf.test:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    let told = told.refused.lock().unwrap();
    assert_eq!(
        (
            answer.lines().next(),
            served,
            network.connections(),
            told.iter()
                .map(|(refused, _)| (refused.address, refused.setting))
                .collect::<Vec<_>>(),
        ),
        (
            Some("HTTP/1.1 403 Forbidden"),
            Served::Refused,
            vec![],
            vec![(
                Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
                Some("egress.no_private_endpoints")
            )],
        )
    );
}

/// An upstream proxy whose connection never opens, a black-holed address,
/// is the proxy's failure at the relay's connect deadline (deep review
/// finding 577-4), before the navigation's own: on Tokio's paused clock,
/// the tunnel gets the relay's `502` naming `connection_timeout` once
/// [`CONNECT_DEADLINE`] passed, and the Session learns that the connection
/// reached no proxy, which it raises as `proxy_unavailable` naming the
/// proxy.
///
/// Covers: story:5-88, error:proxy_unavailable
#[test]
fn an_upstream_proxy_that_never_answers_is_unreachable_at_the_connect_deadline() {
    let resolver = ScriptedResolver::new([goodfetch_test_support::Lookup::new(
        "public.ssrf.test",
        [PUBLIC],
    )]);
    let network = Scripted::new(Some(&resolver), Far::Silent);
    let told = Told::default();
    let relaying = relaying(
        every_host(),
        Upstream::Http {
            authorization: None,
        },
    );
    let runtime = goodfetch_test_support::runtime();

    let ((answer, served), waited) = runtime.block_on(async {
        let started = relaying.clock.now();
        let ended = through(
            &connect("public.ssrf.test:443"),
            GET,
            &relaying,
            &network,
            &told,
        )
        .await;
        (ended, relaying.clock.now().duration_since(started))
    });

    assert_eq!(
        (answer.lines().next(), served, waited),
        (
            Some("HTTP/1.1 502 Bad Gateway"),
            Served::Failed,
            CONNECT_DEADLINE
        )
    );
    assert!(answer.contains("error=connection_timeout"), "{answer}");
    assert_eq!(*told.met.lock().unwrap(), vec![Met::Unreachable]);
}

/// A client that sends no request head is dropped at the relay's head
/// deadline (deep review finding 577-8): on Tokio's paused clock, the
/// connection ends with nothing relayed once [`super::deadlines::HEAD_DEADLINE`]
/// passed.
///
/// Covers: story:4-10
#[test]
fn a_client_that_sends_no_head_is_dropped_at_the_head_deadline() {
    let network = Scripted::new(None, Far::Site);
    let told = Told::default();
    let relaying = relaying(every_host(), Upstream::Direct);
    let runtime = goodfetch_test_support::runtime();

    let ((answer, served), waited) = runtime.block_on(async {
        let started = relaying.clock.now();
        let ended = through("", b"", &relaying, &network, &told).await;
        (ended, relaying.clock.now().duration_since(started))
    });

    assert_eq!(
        (answer.as_str(), served, waited, network.connections()),
        ("", Served::Failed, super::deadlines::HEAD_DEADLINE, vec![])
    );
}

/// The domain policy holds a connection no interception sees, such as a
/// WebSocket's or WebRTC's over TCP, at the relay too.
///
/// Covers: story:4-10, error:egress_refused
#[test]
fn a_host_the_domain_policy_refuses_is_refused_at_the_relay_too() {
    let network = Scripted::new(None, Far::Site);
    let told = Told::default();
    let relaying = relaying(
        Policy::new(&["*"], &["refused.test"]).unwrap(),
        Upstream::Direct,
    );

    let (answer, served) = goodfetch_test_support::runtime().block_on(through(
        &connect("turn.refused.test:3478"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    assert_eq!(
        (answer.lines().next(), served, network.connections()),
        (Some("HTTP/1.1 403 Forbidden"), Served::Refused, vec![])
    );
    assert_eq!(
        told.refused.lock().unwrap()[0].0.rule,
        EgressRule::DomainPolicy
    );
}

/// A plain request goes to the site in origin form, the relay's
/// credentials left out and the connection closed after its answer; one
/// to a private address is refused with no answer made up.
///
/// Covers: story:4-10, error:egress_refused
#[test]
fn a_plain_request_reaches_the_site_in_origin_form_and_a_private_one_reaches_nothing() {
    let network = Scripted::new(None, Far::Site);
    let told = Told::default();
    let relaying = relaying(every_host(), Upstream::Direct);
    let request = |host: &str| {
        format!(
            "GET http://{host}/page?x=1 HTTP/1.1\r\nHost: {host}\r\nProxy-Authorization: {}\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n",
            authorization()
        )
    };
    let runtime = goodfetch_test_support::runtime();

    let (answer, _) = runtime.block_on(through(
        &request("192.31.196.1"),
        b"",
        &relaying,
        &network,
        &told,
    ));
    let (refused, served) = runtime.block_on(through(
        &request("127.0.0.1"),
        b"",
        &relaying,
        &network,
        &told,
    ));

    assert!(answer.ends_with("ok"), "{answer}");
    assert_eq!(
        network.heads(),
        vec!["GET /page?x=1 HTTP/1.1\r\nHost: 192.31.196.1\r\nAccept: */*\r\nConnection: close\r\n\r\n".to_owned()]
    );
    assert_eq!((refused.as_str(), served), ("", Served::Refused));
    assert_eq!(
        network.connections(),
        vec![Some(SocketAddr::new(PUBLIC, 80))]
    );
}

/// A site the relay cannot reach itself is named to the Session by why, as
/// the browser would name it: a tunnel gets the relay's 502, a plain
/// request a closed connection, so the browser fails it.
///
/// Covers: story:4-10
#[test]
fn a_name_with_no_address_is_unreached_by_name_and_the_browser_fails_it() {
    let network = Scripted::new(None, Far::Site);
    let told = Told::default();
    let relaying = relaying(every_host(), Upstream::Direct);
    let plain = format!(
        "GET http://nowhere.test/ HTTP/1.1\r\nHost: nowhere.test\r\nProxy-Authorization: {}\r\n\r\n",
        authorization()
    );
    let runtime = goodfetch_test_support::runtime();

    let tunnel = runtime.block_on(through(
        &connect("nowhere.test:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));
    let request = runtime.block_on(through(&plain, b"", &relaying, &network, &told));

    assert_eq!(tunnel.0.lines().next(), Some("HTTP/1.1 502 Bad Gateway"));
    assert_eq!(request.0, "");
    assert_eq!(
        *told.unreached.lock().unwrap(),
        vec![
            ("nowhere.test:443".to_owned(), Unreached::NameNotResolved),
            ("nowhere.test:80".to_owned(), Unreached::NameNotResolved),
        ]
    );
    assert_eq!(network.connections(), vec![]);
}

/// The relay reads a ClientHello whole, from one record or several, and
/// refuses records of another protocol.
/// Covers: story:4-40
#[test]
fn a_client_hello_is_read_from_its_records_whole_or_split() {
    let read = |name: &str| {
        client_hello::parse(&seed(name)).map(|hello| {
            (
                hello.alpn,
                hello.server_name,
                hello.versions.contains(&0x0304),
            )
        })
    };

    assert_eq!(
        [
            read("h2 and http1.1"),
            read("split over two records"),
            read("not a handshake"),
        ],
        [
            Ok((
                vec![b"h2".to_vec(), b"http/1.1".to_vec()],
                Some("site.listed.test".to_owned()),
                true
            )),
            Ok((vec![b"h2".to_vec()], Some("split.test".to_owned()), true)),
            Err(client_hello::Unread::Malformed),
        ]
    );
}

/// The Run's `network.resolve` mapping applies before every lookup and its
/// answer is still judged (ADR 0059; spec #563's reading of #325): a name
/// mapped to loopback or to the metadata stand-in reaches it only where
/// `egress.private_ranges` lists it, the metadata address only by its
/// exact address, never through a range that covers it. The far end of
/// the Scripted network at 169.254.169.254 stands in for the metadata
/// service; the oracle is the connections the relay opened.
///
/// Covers: story:4-11, story:4-13, error:egress_refused, setting:egress.private_ranges, setting:network.resolve
#[rstest::rstest]
#[case::loopback_unlisted("127.0.0.1", &[], false)]
#[case::loopback_under_its_range("127.0.0.1", &["127.0.0.0/8"], true)]
#[case::loopback_by_the_hosts_name("127.0.0.1", &["mapped.test"], true)]
#[case::metadata_under_a_range_that_covers_it("169.254.169.254", &["169.254.0.0/16"], false)]
#[case::metadata_through_the_hosts_name("169.254.169.254", &["mapped.test"], false)]
#[case::metadata_by_its_exact_address("169.254.169.254", &["169.254.169.254"], true)]
fn a_mapped_answer_is_judged_and_reaches_only_what_the_private_ranges_list(
    #[case] mapped: &str,
    #[case] ranges: &[&str],
    #[case] reached: bool,
) {
    let mut network = Scripted::new(None, Far::Site);
    network
        .mapping
        .insert("mapped.test".to_owned(), vec![mapped.parse().unwrap()]);
    let told = Told::default();
    let relaying = relaying(
        every_host().with_private_ranges(ranges).unwrap(),
        Upstream::Direct,
    );

    let (answer, _) = goodfetch_test_support::runtime().block_on(through(
        &connect("mapped.test:443"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    let address: IpAddr = mapped.parse().unwrap();
    let refusals: Vec<_> = told
        .refused
        .lock()
        .unwrap()
        .iter()
        .map(|(refused, _)| (refused.rule, refused.address))
        .collect();
    if reached {
        assert_eq!(
            (network.connections(), refusals, answer.ends_with("ok")),
            (vec![Some(SocketAddr::new(address, 443))], vec![], true),
            "{answer}"
        );
    } else {
        assert_eq!(
            (network.connections(), refusals),
            (vec![], vec![(EgressRule::PrivateAddress, Some(address))]),
            "{answer}"
        );
    }
}

/// The seed ClientHello records named `name`, of the relay's fuzz target.
fn seed(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fuzz/relay_client_hello/seeds")
            .join(name),
    )
    .unwrap()
}

/// A tunnel the relay passes as it is tells the Session the ClientHello
/// the browser opened it with, which the relay's own look at a
/// certificate the browser refused sends again, so the site sees the
/// browser's fingerprint alone (ADR 0042); its bytes reach the site
/// unchanged, and a tunnel that carries no TLS tells nothing.
///
/// Covers: story:4-40
#[test]
fn a_tunnel_it_passes_tells_the_session_the_browsers_client_hello_and_changes_no_byte() {
    let network = Scripted::new(None, Far::Site);
    let told = Told::default();
    let relaying = relaying(every_host(), Upstream::Direct);
    let records = seed("split over two records");
    let sent = [records.as_slice(), GET].concat();
    let runtime = goodfetch_test_support::runtime();

    let (tls, _) = runtime.block_on(through(
        &connect("192.31.196.1:443"),
        &sent,
        &relaying,
        &network,
        &told,
    ));
    let (plain, _) = runtime.block_on(through(
        &connect("192.31.196.1:80"),
        GET,
        &relaying,
        &network,
        &told,
    ));

    assert!(
        tls.ends_with("ok") && plain.ends_with("ok"),
        "{tls} {plain}"
    );
    assert_eq!(
        network.heads(),
        vec![
            String::from_utf8_lossy(&sent).into_owned(),
            String::from_utf8_lossy(GET).into_owned()
        ]
    );
    assert_eq!(
        *told.client_hellos.lock().unwrap(),
        vec![(
            "192.31.196.1:443".to_owned(),
            client_hello::parse(&records).unwrap()
        )]
    );
}

/// How a relay runner sends a case's URL to the relay.
#[derive(Clone, Copy, Debug)]
enum Sent {
    /// As the browser sends it: a plain `http:` request in absolute form,
    /// a tunnel for any other scheme.
    AsTheBrowserSendsIt,
    /// As a tunnel to its host and port, as a WebSocket or WebRTC's TURN
    /// over TCP goes.
    AsATunnel,
    /// As a tunnel to a host the Run lists for a certificate exception,
    /// which the relay terminates (ADR 0042).
    Terminated,
}

impl Sent {
    /// The table's layer the runner applies the table to sent this way.
    fn layer(self) -> &'static str {
        match self {
            Self::AsTheBrowserSendsIt | Self::AsATunnel => "egress_relay",
            Self::Terminated => "tls_relay",
        }
    }
}

/// The table's form of the rule a refusal names.
fn table_rule(rule: EgressRule) -> goodfetch_test_support::EgressRule {
    match rule {
        EgressRule::DomainPolicy => goodfetch_test_support::EgressRule::DomainPolicy,
        EgressRule::PrivateAddress => goodfetch_test_support::EgressRule::PrivateAddress,
        EgressRule::Scheme => goodfetch_test_support::EgressRule::Scheme,
    }
}

/// The case's domain policy, every host allowed when it sets none.
fn case_policy(case: &SsrfCase) -> Policy {
    match &case.domains {
        Some(domains) => Policy::new(&domains.allowed, &domains.blocked).unwrap(),
        None => every_host(),
    }
}

/// The relay terminating `host`, under a trust-on-first-use exception
/// pinning in `stores`.
fn terminating_of(host: &str, stores: &super::terminate_tests::Stores) -> Terminating {
    Terminating {
        authority: std::sync::Arc::new(Authority::new(vec![host.to_owned()]).unwrap()),
        exceptions: CertificateExceptions::listed(
            &serde_json::json!({ host: null }),
            stores.book.clone(),
            ExceptionHosts::default(),
        )
        .unwrap(),
        examiner: Trust::default_roots().examiner(),
        trust: Trust::default_roots(),
        application_settings: std::sync::Arc::default(),
        sessions: std::sync::Arc::default(),
    }
}

/// The head that sends `url` to the relay `sent`'s way.
fn head_of(url: &url::Url, sent: Sent) -> String {
    let host = match url.host() {
        Some(url::Host::Ipv6(address)) => format!("[{address}]"),
        host => host.map(|host| host.to_string()).unwrap_or_default(),
    };
    let port = url.port_or_known_default().unwrap_or(80);
    match sent {
        Sent::AsTheBrowserSendsIt if url.scheme() == "http" => format!(
            "GET {url} HTTP/1.1\r\nHost: {host}\r\nProxy-Authorization: {}\r\n\r\n",
            authorization()
        ),
        _ => connect(&format!("{host}:{port}")),
    }
}

/// What the relay did with one case sent `sent`'s way: the refusal it told
/// the Session, the addresses its connections reached, a connection to the
/// upstream proxy standing for one to the case's target, and whether the
/// relay was set to terminate the case's host.
fn relay_case(case: &SsrfCase, url: &url::Url, sent: Sent) -> (Outcome, bool) {
    let resolver = ScriptedResolver::new(case.script(PUBLIC));
    let proxied = case.route == Route::UpstreamProxy;
    let network = Scripted::new(
        Some(&resolver),
        if proxied {
            Far::Proxy {
                answer: "HTTP/1.1 200 Connection established\r\n\r\n",
            }
        } else {
            Far::Site
        },
    );
    let told = Told::default();
    let stores = super::terminate_tests::stores();
    let mut relaying = relaying(
        case_policy(case),
        if proxied {
            Upstream::Http {
                authorization: None,
            }
        } else {
            Upstream::Direct
        },
    );
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if let Sent::Terminated = sent {
        relaying.terminating = Some(std::sync::Arc::new(terminating_of(host, &stores)));
    }
    let terminates = relaying
        .terminating
        .as_ref()
        .is_some_and(|terminating| terminating.terminates(host));

    let _ = goodfetch_test_support::runtime().block_on(through(
        &head_of(url, sent),
        GET,
        &relaying,
        &network,
        &told,
    ));

    let refusal = told
        .refused
        .lock()
        .unwrap()
        .last()
        .map(|(refused, _)| Refusal::EgressRefused {
            rule: table_rule(refused.rule),
        });
    let connections = network
        .connections()
        .into_iter()
        .filter_map(|connection| match connection {
            Some(address) => Some(address.ip()),
            None => case.target_address(PUBLIC),
        })
        .collect();
    (
        Outcome {
            refusal,
            connections,
        },
        terminates,
    )
}

/// The relay runners' positive controls: a public name, directly and
/// through the upstream proxy, each reaching the public stand-in.
const RELAY_CONTROLS: &str = r#"
[[cases]]
name = "public_only"
family = "mixed_answers"
url = "https://public.ssrf.test/"
lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
target = "public"
refusal = { code = "egress_refused", rule = "private_address" }

[[cases]]
name = "public_name_through_the_proxy"
family = "upstream_proxy"
url = "https://public.ssrf.test/"
route = { via = "upstream_proxy" }
lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
target = "public"
refusal = { code = "egress_refused", rule = "private_address" }
"#;

/// The SSRF table's thin runner on the egress relay (ADR 0067): every case
/// a browser's connection meets, sent as the browser sends it, as a tunnel,
/// and as a tunnel the relay terminates, is refused before any connection
/// reaches its target, its upstream proxy's cases before the proxy sees
/// them. The positive controls reach the public stand-in each way, and the
/// terminating runner's relay is set to terminate every case's host, which
/// it judges before it would. Only a URL the WHATWG parser
/// refuses never reaches the relay, as no browser sends it.
///
/// Covers: story:4-10, story:4-89, error:egress_refused
#[rstest::rstest]
#[case::as_the_browser_sends_it(Sent::AsTheBrowserSendsIt)]
#[case::as_a_tunnel(Sent::AsATunnel)]
#[case::terminated(Sent::Terminated)]
fn every_case_of_the_ssrf_table_a_browser_connection_meets_is_refused_at_the_relay(
    #[case] sent: Sent,
) {
    let runner = Runner::committed(
        sent.layer(),
        file!(),
        "every_case_of_the_ssrf_table_a_browser_connection_meets_is_refused_at_the_relay",
    )
    .unwrap();
    let controls = SsrfTable::parse(RELAY_CONTROLS).unwrap();
    let mut sent_cases = 0;
    let mut terminating = 0;

    let result = runner.run(&controls.cases, PUBLIC, |case| {
        let Ok(url) = url::Url::parse(&case.url) else {
            return Outcome::default();
        };
        let (outcome, terminates) = relay_case(case, &url, sent);
        sent_cases += 1;
        terminating += usize::from(terminates);
        outcome
    });

    assert!(
        runner.cases().len() > 90,
        "only {} cases",
        runner.cases().len()
    );
    let expected_terminating = match sent {
        Sent::Terminated => sent_cases,
        _ => 0,
    };
    assert_eq!(terminating, expected_terminating);
    // Only the URLs the WHATWG parser refuses, which no browser sends,
    // never reach the relay, and reach nothing.
    let unsent = |case: &str| (case.to_owned(), None, false);
    assert_eq!(
        result.map_err(|failed| {
            let let_through: Vec<_> = failed
                .let_through
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect();
            (failed.no_control, failed.held, let_through)
        }),
        Err((
            false,
            Vec::new(),
            vec![unsent("zone_id"), unsent("url_that_does_not_parse")]
        ))
    );
}

/// The relay's own TCP network for a Session whose local address is
/// `127.0.0.1`, `dns` its resolver, `case`'s lookups its mapping.
fn bound_network(case: &SsrfCase, dns: &goodfetch_engine::dns::Dns) -> super::Tcp {
    super::Tcp {
        mapping: case
            .lookups
            .iter()
            .map(|lookup| {
                let addresses = lookup
                    .answers
                    .iter()
                    .map(|answer| match answer {
                        goodfetch_test_support::Answer::Public => PUBLIC,
                        goodfetch_test_support::Answer::Address(address) => *address,
                    })
                    .collect();
                (lookup.name.clone(), addresses)
            })
            .collect(),
        upstream: None,
        judged: None,
        local: Some(goodfetch_engine::http_client::LocalBinding::new(
            goodfetch_engine::local_addresses::SessionAddresses {
                ipv4: Some(Ipv4Addr::LOCALHOST),
                ipv6: None,
            },
            dns,
        )),
    }
}

/// The SSRF table's thin runner on the relay of a Session with local
/// addresses (ADR 0044): on the relay's own TCP network, bound to
/// `127.0.0.1`, every address case and domain policy case, at a loopback
/// listener's port, is refused before any connection, the listener's
/// accepted connections the oracle; a case's lookups are the Run's
/// `network.resolve` mapping. The positive control, the listener's own
/// address once the Session lists it, reaches it.
///
/// Covers: story:4-48, story:4-89, error:egress_refused
#[test]
fn with_local_addresses_every_address_case_of_the_ssrf_table_is_refused_at_the_relay() {
    let runner = Runner::committed(
        "local_address_relay",
        file!(),
        "with_local_addresses_every_address_case_of_the_ssrf_table_is_refused_at_the_relay",
    )
    .unwrap();
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = std::net::TcpListener::bind(SocketAddr::new(loopback, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "listed_loopback"
        family = "encoding"
        url = "http://127.0.0.1/"
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();
    let dns = goodfetch_engine::dns::Dns::new(
        goodfetch_engine::dns::HostMapping::new([]),
        goodfetch_engine::dns::ResolverConfiguration::name_servers([]),
        goodfetch_engine::clock::Clock::system(),
    );

    let result = runner.run(&controls.cases, loopback, |case| {
        let Ok(mut url) = url::Url::parse(&case.url) else {
            return Outcome::default();
        };
        if url.set_port(Some(port)).is_err() {
            return Outcome::default();
        }
        let listed = case.name == "listed_loopback";
        let mut policy = case_policy(case);
        if listed {
            policy = policy.with_private_ranges(&["127.0.0.1"]).unwrap();
        }
        let network = bound_network(case, &dns);
        let told = Told::default();
        let relaying = relaying(policy, Upstream::Direct);
        // A connection the relay let through waits for an answer the
        // listener never gives: the wait ends it, the connection counted.
        let _ = goodfetch_test_support::runtime().block_on(async {
            let local = tokio::task::LocalSet::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                local.run_until(async {
                    let (mut browser, relayed) = duplex(1 << 16);
                    browser
                        .write_all(head_of(&url, Sent::AsTheBrowserSendsIt).as_bytes())
                        .await
                        .unwrap();
                    serve(relayed, &relaying, &network, &told).await
                }),
            )
            .await
        });
        let mut connections = Vec::new();
        while listener.accept().is_ok() {
            connections.push(loopback);
        }
        Outcome {
            refusal: told.refused.lock().unwrap().last().map(|(refused, _)| {
                Refusal::EgressRefused {
                    rule: table_rule(refused.rule),
                }
            }),
            connections,
        }
    });

    assert!(
        runner.cases().len() > 80,
        "only {} cases",
        runner.cases().len()
    );
    let unsent = |case: &str| (case.to_owned(), None, false);
    assert_eq!(
        result.map_err(|failed| {
            let let_through: Vec<_> = failed
                .let_through
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect();
            (failed.no_control, failed.held, let_through)
        }),
        Err((
            false,
            Vec::new(),
            vec![unsent("zone_id"), unsent("url_that_does_not_parse")]
        ))
    );
}
