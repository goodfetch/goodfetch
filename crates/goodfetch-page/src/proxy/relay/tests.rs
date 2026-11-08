//! The relay against fake proxies on loopback, driven as the browser
//! drives it, an HTTP proxy client: a client without the Session's relay
//! credentials gets a `407` and no relayed byte, a wrong one in the Audit
//! log; the relay passes its column of the endpoint table; it opens a
//! SOCKS5 tunnel for a `CONNECT` and for a plain request; whether the proxy
//! refused the Session's credentials is the answer of the latest connection
//! for the same authority, never a refusal another connection met; and, on
//! Tokio's paused clock, the accept loop backs off after an error that is
//! not one connection's own instead of spinning, and a client that sends
//! no head is dropped at the head deadline.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the accept loop's test runs on the test Clock and Tokio's paused clock (ADR 0059)"
)]

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::endpoint::{Refusals, new_token};
use goodfetch_engine::home::Home;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_test_support::{Endpoint, EndpointRunner, Listener, ListenerKind, Rule, TestEnv};
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::{ACCEPT_BACKOFF, Credentials, Mode, Upstream, accepting, basic, relay};
use crate::egress_relay::answers::USERNAME;
use crate::egress_relay::deadlines::HEAD_DEADLINE;
use crate::egress_relay::{ByAuthority, Outcomes};

/// What a fake SOCKS5 proxy met: the connections it accepted, the first
/// bytes each tunnel it opened carried, and a wake-up for each of those.
#[derive(Default)]
struct Met {
    connections: AtomicUsize,
    carried: Mutex<Vec<Vec<u8>>>,
    carried_more: Notify,
}

/// A fake SOCKS5 proxy on loopback that asks for credentials and answers
/// each connection's with the next status of `statuses`, 0 taking them,
/// then opens the tunnel to the IPv4 address the relay asks for.
async fn fake_proxy(owner: &TaskOwner, statuses: Vec<u8>) -> (SocketAddr, Arc<Met>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let statuses = Arc::new(Mutex::new(VecDeque::from(statuses)));
    let met = Arc::new(Met::default());
    let meeting = Arc::clone(&met);
    owner.spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            meeting.connections.fetch_add(1, Ordering::Relaxed);
            let status = statuses.lock().await.pop_front().unwrap_or(0);
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            stream.write_all(&[5, 2]).await.unwrap();
            let mut head = [0; 2];
            stream.read_exact(&mut head).await.unwrap();
            let mut username = vec![0; usize::from(head[1])];
            stream.read_exact(&mut username).await.unwrap();
            let mut length = [0; 1];
            stream.read_exact(&mut length).await.unwrap();
            let mut password = vec![0; usize::from(length[0])];
            stream.read_exact(&mut password).await.unwrap();
            stream.write_all(&[1, status]).await.unwrap();
            if status != 0 {
                continue;
            }
            let mut request = [0; 10];
            stream.read_exact(&mut request).await.unwrap();
            stream
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            let mut carried = vec![0; 4096];
            let read = stream.read(&mut carried).await.unwrap_or(0);
            carried.truncate(read);
            meeting.carried.lock().await.push(carried);
            meeting.carried_more.notify_one();
        }
    });
    (address, met)
}

/// The Audit log of `env`'s home, and the refusals a relay records in it.
fn audit(env: &TestEnv, owner: &TaskOwner) -> (AuditLog, Refusals) {
    let log = AuditLog::of(&Home::new(env.home().to_path_buf()), None);
    let refusals = Refusals::new(
        log.clone(),
        Arc::new(owner.child()),
        Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001)),
    );
    (log, refusals)
}

/// How many entries of `log` record the relay refusing a wrong token.
fn refused_tokens(log: &AuditLog) -> usize {
    log.export().map_or(0, |entries| {
        entries
            .iter()
            .filter(|exported| {
                exported.entry.kind == "endpoint_authentication_failed"
                    && exported.entry.details
                        == serde_json::json!({ "listener": "proxy_relay", "reason": "token_wrong" })
            })
            .count()
    })
}

/// The relay's upstream: the proxy at `address` reached as `mode`, taking
/// the client credentials of `password`.
fn upstream(address: SocketAddr, mode: Mode, password: &str, refusals: Refusals) -> Upstream {
    Upstream {
        addresses: vec![address],
        mode,
        client: basic(USERNAME, password),
        refusals,
        outcomes: Outcomes::default(),
        usage: None,
        refusal: None,
        clock: Clock::system(),
        connect_deadline: Duration::from_secs(10),
    }
}

/// The Session's SOCKS5 credentials, which the relay gives the proxy.
fn socks5() -> Mode {
    Mode::Socks5 {
        credentials: Some(Credentials {
            username: "user".to_owned(),
            password: SecretString::from("secret"),
        }),
    }
}

/// One client connection through the relay to `upstream`: `request`, then
/// `after` once the relay answered, until the answer head came back.
async fn through(upstream: &Upstream, request: &str, after: &[u8]) -> String {
    let clients = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut client = TcpStream::connect(clients.local_addr().unwrap())
        .await
        .unwrap();
    let (relayed, _) = clients.accept().await.unwrap();
    let driving = async {
        client.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        let mut byte = [0; 1];
        while !answer.ends_with(b"\r\n\r\n") {
            if client.read_exact(&mut byte).await.is_err() {
                break;
            }
            answer.push(byte[0]);
        }
        if !after.is_empty() {
            let _ = client.write_all(after).await;
        }
        drop(client);
        String::from_utf8(answer).unwrap()
    };
    let ((), answer) = tokio::join!(relay(relayed, upstream), driving);
    answer
}

/// A `CONNECT` to `127.0.0.1:80` with `authorization`, if any.
fn connect(authorization: Option<&str>) -> String {
    connect_to("127.0.0.1:80", authorization)
}

/// A `CONNECT` to `authority` with `authorization`, if any.
fn connect_to(authority: &str, authorization: Option<&str>) -> String {
    let authorization = authorization
        .map(|value| format!("Proxy-Authorization: {value}\r\n"))
        .unwrap_or_default();
    format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{authorization}\r\n")
}

/// Covers: story:5-88
#[rstest]
#[case::no_credentials(None, 0)]
#[case::another_password(Some("another"), 1)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_without_the_relays_credentials_gets_a_407_and_no_relayed_byte(
    #[case] password: Option<&str>,
    #[case] audited: usize,
) {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Threads);
    let (log, refusals) = audit(&env, &owner);
    let (address, met) = fake_proxy(&owner, vec![]).await;
    let upstream = upstream(address, socks5(), "relay-password", refusals);
    let given = password.map(|password| basic(USERNAME, password));

    let answer = through(
        &upstream,
        &connect(given.as_deref()),
        b"GET / HTTP/1.1\r\n\r\n",
    )
    .await;

    owner.cancel();
    assert_eq!(
        (
            answer.starts_with("HTTP/1.1 407"),
            met.connections.load(Ordering::Relaxed),
            refused_tokens(&log)
        ),
        (true, 0, audited)
    );
}

/// Covers: story:5-88
#[rstest]
#[case::a_tunnel(
    "CONNECT 127.0.0.1:80 HTTP/1.1\r\nHost: 127.0.0.1:80\r\n",
    "HTTP/1.1 200",
    "ping"
)]
#[case::a_plain_request(
    "GET http://127.0.0.1:80/page?q=1 HTTP/1.1\r\nHost: 127.0.0.1\r\n",
    "",
    "GET /page?q=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
)]
#[tokio::test]
async fn a_client_with_the_relays_credentials_reaches_the_site_through_the_socks5_proxy(
    #[case] head: &str,
    #[case] answered: &str,
    #[case] carried: &str,
) {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let (_, refusals) = audit(&env, &owner);
    let (address, met) = fake_proxy(&owner, vec![0]).await;
    let upstream = upstream(address, socks5(), "relay-password", refusals);
    let request = format!(
        "{head}Proxy-Authorization: {}\r\n\r\n",
        basic(USERNAME, "relay-password")
    );
    let after: &[u8] = if answered.is_empty() { b"" } else { b"ping" };

    let answer = through(&upstream, &request, after).await;
    // The fake proxy reads the tunnel's first bytes once the relay passed
    // them on; a relay that passes none fails at this deadline.
    tokio::time::timeout(Duration::from_secs(60), met.carried_more.notified())
        .await
        .expect("the tunnel carries the client's first bytes");
    let seen = met.carried.lock().await.clone();

    owner.cancel();
    assert_eq!(
        (
            answer.starts_with(answered),
            seen.iter()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .collect::<Vec<_>>()
        ),
        (true, vec![carried.to_owned()])
    );
}

/// Whether the proxy refused the credentials is each connection's own
/// answer, kept for the authority it asked for (deep review finding 577-5):
/// a connection the proxy took after one it refused leaves the refused
/// one's request the proxy's credentials failure, and only a later
/// connection to the same authority changes its answer.
///
/// Covers: story:5-88, error:proxy_auth_failed
#[tokio::test]
async fn whether_the_proxy_refused_the_credentials_is_each_connections_own_answer() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let (_, refusals) = audit(&env, &owner);
    let (address, _) = fake_proxy(&owner, vec![1, 0, 0]).await;
    let upstream = upstream(address, socks5(), "relay-password", refusals);
    let credentials = basic(USERNAME, "relay-password");
    let refused = || upstream.outcomes.refused("https://127.0.0.1:80/");
    let other = || upstream.outcomes.refused("https://127.0.0.2:80/");

    through(
        &upstream,
        &connect_to("127.0.0.1:80", Some(&credentials)),
        b"",
    )
    .await;
    through(
        &upstream,
        &connect_to("127.0.0.2:80", Some(&credentials)),
        b"",
    )
    .await;
    let after_another_connection = (refused(), other());
    through(
        &upstream,
        &connect_to("127.0.0.1:80", Some(&credentials)),
        b"",
    )
    .await;
    let after_its_own_next = refused();

    owner.cancel();
    assert_eq!(
        (after_another_connection, after_its_own_next),
        ((true, false), false)
    );
}

/// A fake HTTP proxy on loopback that answers each connection's `CONNECT`
/// with the next answer head of `answers`, and keeps each request head.
async fn fake_http_proxy(
    owner: &TaskOwner,
    answers: Vec<&'static str>,
) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&requests);
    owner.spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let answer = answers
                .lock()
                .await
                .pop_front()
                .unwrap_or("HTTP/1.1 200 OK\r\n\r\n");
            let mut request = Vec::new();
            let mut byte = [0; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            kept.lock()
                .await
                .push(String::from_utf8_lossy(&request).into_owned());
            stream.write_all(answer.as_bytes()).await.unwrap();
        }
    });
    (address, requests)
}

/// Covers: story:8-89, story:8-108
#[tokio::test]
async fn the_relay_keeps_a_hosted_providers_refusal_of_the_latest_connect_for_its_adapter() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let (_, refusals) = audit(&env, &owner);
    let (address, requests) = fake_http_proxy(
        &owner,
        vec![
            "HTTP/1.1 502 Bad Gateway\r\nX-Brd-Err-Code: client_10064\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 Connection established\r\n\r\n",
        ],
    )
    .await;
    let mode = Mode::Http {
        authorization: Some(SecretString::from(basic("customer", "provider-secret"))),
    };
    let upstream = Upstream {
        refusal: Some(ByAuthority::default()),
        ..upstream(address, mode, "relay-password", refusals)
    };
    let latest = |upstream: &Upstream| {
        upstream
            .refusal
            .as_ref()
            .unwrap()
            .get("site.test:443")
            .map(|answer| {
                (
                    answer.status,
                    answer.header("x-brd-err-code").map(str::to_owned),
                )
            })
    };
    let request = format!(
        "CONNECT site.test:443 HTTP/1.1\r\nHost: site.test:443\r\nProxy-Authorization: {}\r\n\r\n",
        basic(USERNAME, "relay-password")
    );

    let refused = through(&upstream, &request, b"").await;
    let after_a_refusal = latest(&upstream);
    let taken = through(&upstream, &request, b"").await;
    let after_a_tunnel = latest(&upstream);
    // The provider gets its own credentials, never the relay's.
    let credentials: Vec<bool> = requests
        .lock()
        .await
        .iter()
        .map(|head| {
            head.contains(&basic("customer", "provider-secret"))
                && !head.contains(&basic(USERNAME, "relay-password"))
        })
        .collect();

    owner.cancel();
    assert_eq!(
        (
            refused.starts_with("HTTP/1.1 502"),
            after_a_refusal,
            taken.starts_with("HTTP/1.1 200"),
            after_a_tunnel,
            credentials,
        ),
        (
            true,
            Some((502, Some("client_10064".to_owned()))),
            true,
            None,
            vec![true, true],
        )
    );
}

/// Covers: story:5-88
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_relay_passes_its_column_of_the_endpoint_table() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Threads);
    let (log, refusals) = audit(&env, &owner);
    let (address, _) = fake_proxy(&owner, vec![]).await;
    let password = new_token().unwrap();
    let upstream = Arc::new(upstream(address, socks5(), &password, refusals));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let relay_address = listener.local_addr().unwrap();
    let serving = owner.child();
    owner.spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let upstream = Arc::clone(&upstream);
            serving.spawn(async move { relay(client, &upstream).await });
        }
    });
    let described = Listener::new(
        ListenerKind::ProxyRelay,
        Endpoint::HttpProxy {
            address: relay_address,
            username: USERNAME.to_owned(),
            target: "127.0.0.1:80".to_owned(),
        },
        password,
    )
    // Another Session's relay's password.
    .with_other_listener_token(new_token().unwrap());

    let ran = owner
        .blocking(move || {
            let mut seen = 0;
            EndpointRunner::new(move |_, _: Rule| {
                let now = refused_tokens(&log);
                let audited = now > seen;
                seen = now;
                Ok(audited)
            })
            .run(&described)
        })
        .await;

    owner.cancel();
    assert_eq!(ran, Ok(()));
}

/// Covers: story:5-88
#[tokio::test(start_paused = true)]
async fn an_accept_error_that_is_not_one_connections_backs_off_before_the_next_accept() {
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let (stop, ending) = (CancellationToken::new(), CancellationToken::new());
    let accepts = AtomicU32::new(0);
    let seconds = 10;

    let looping = accepting(
        || {
            let accepted = accepts.fetch_add(1, Ordering::Relaxed);
            async move {
                // A loop that never yields stops being offered errors, so
                // the test ends either way.
                if accepted >= 1_000 {
                    std::future::pending::<()>().await;
                }
                // Too many open files: the process's, never one connection's.
                Err::<(), _>(std::io::Error::from_raw_os_error(24))
            }
        },
        &clock,
        &stop,
        &ending,
        |()| {},
    );
    let _ = tokio::time::timeout(Duration::from_secs(seconds), looping).await;

    let most = u32::try_from(Duration::from_secs(seconds).as_millis() / ACCEPT_BACKOFF.as_millis())
        .unwrap()
        + 1;
    assert!(accepts.load(Ordering::Relaxed) <= most);
}

/// A client that sends no request head is dropped at the head deadline,
/// holding no task past it (deep review finding 577-8): on Tokio's paused
/// clock, the relay ends the connection once [`HEAD_DEADLINE`] passed.
///
/// Covers: story:5-88
#[tokio::test(start_paused = true)]
async fn a_client_that_sends_no_head_is_dropped_at_the_head_deadline() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let (_, refusals) = audit(&env, &owner);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let upstream = Upstream {
        clock,
        ..upstream(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
            socks5(),
            "relay-password",
            refusals,
        )
    };
    let clients = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let silent = TcpStream::connect(clients.local_addr().unwrap())
        .await
        .unwrap();
    let (relayed, _) = clients.accept().await.unwrap();
    let started = clock.now();

    let dropped = tokio::time::timeout(Duration::from_mins(10), relay(relayed, &upstream)).await;

    owner.cancel();
    drop(silent);
    assert_eq!(
        (dropped.is_ok(), clock.now().duration_since(started)),
        (true, HEAD_DEADLINE)
    );
}
