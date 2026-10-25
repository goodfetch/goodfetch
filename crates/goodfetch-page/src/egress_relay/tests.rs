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
use goodfetch_engine::tls::ExceptionUsed;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_test_support::{Family, ResolverHandle, ScriptedResolver, SsrfTable};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, duplex};

use super::serve::{Met, Network, Observe, Relaying, Resolved, Served, Unreached, Upstream, serve};
use super::terminate::Stop;

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
    }
}

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
                mapped: true,
            });
        }
        let addresses = match host.parse::<IpAddr>() {
            Ok(address) => vec![address],
            Err(_) => self.resolver.as_ref()?.lookup(host),
        };
        Some(Resolved {
            addresses: on_port(&addresses),
            mapped: false,
        })
    }

    async fn connect(&self, address: SocketAddr) -> io::Result<DuplexStream> {
        Ok(self.open(Some(address)))
    }

    async fn connect_upstream(&self) -> io::Result<DuplexStream> {
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
    unauthenticated: Mutex<usize>,
    unreached: Mutex<Vec<(String, Unreached)>>,
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

    fn upstream(&self, met: Met) {
        self.met.lock().unwrap().push(met);
    }

    fn unauthenticated(&self) {
        *self.unauthenticated.lock().unwrap() += 1;
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
    assert_eq!(*told.unauthenticated.lock().unwrap(), 1);
    assert!(opened.ends_with("ok"), "{opened}");
    assert!(matches!(relayed, Served::Relayed { .. }), "{relayed:?}");
    assert_eq!(
        network.connections(),
        vec![Some(SocketAddr::new(PUBLIC, 443))]
    );
}

/// The Scripted resolver's rebinding cases (ADR 0067): a name that answers
/// a public address and then a private one reaches its public answer on
/// the first connection, and its second is refused naming the private
/// address, before any connection to it.
///
/// Covers: story:4-10, story:4-8, error:egress_refused
#[test]
fn a_name_that_rebinds_to_a_private_address_is_refused_on_its_next_connection_and_never_reached() {
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<_> = table.cases_of(Family::Rebinding).collect();
    let relaying = relaying(every_host(), Upstream::Direct);

    let outcomes: Vec<_> = cases
        .iter()
        .map(|case| {
            let url = url::Url::parse(&case.url).unwrap();
            let authority = format!("{}:443", url.host_str().unwrap());
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
                first.0.ends_with("ok"),
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
fn an_upstream_proxys_407_reaches_the_browser_and_marks_the_credentials_refused() {
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
    let seed = |name: &str| {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fuzz/relay_client_hello/seeds")
                .join(name),
        )
        .unwrap()
    };
    let read = |name: &str| {
        super::client_hello::parse(&seed(name)).map(|hello| {
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
            Err(super::client_hello::Unread::Malformed),
        ]
    );
}
