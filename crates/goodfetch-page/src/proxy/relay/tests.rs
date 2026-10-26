//! The relay against a fake SOCKS5 proxy on loopback, driven as the browser
//! drives it: whether the proxy refused the Session's credentials is the
//! latest connection's answer, never a refusal the Session met once; and
//! the accept loop, on Tokio's paused clock, backs off after an error that
//! is not one connection's own instead of spinning.
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{ACCEPT_BACKOFF, Mode, Upstream, accepting, relay};

/// A fake SOCKS5 proxy on loopback that asks for credentials and answers
/// each connection's with the next status of `statuses`, 0 taking them.
async fn fake_proxy(owner: &TaskOwner, statuses: Vec<u8>) -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let statuses = Arc::new(Mutex::new(VecDeque::from(statuses)));
    owner.spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
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
            if status == 0 {
                let mut request = [0; 10];
                stream.read_exact(&mut request).await.unwrap();
                stream
                    .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
            }
        }
    });
    address
}

/// One browser connection through the relay to `upstream`: its greeting
/// and its `CONNECT`, until the relay answers or closes.
async fn through(upstream: &Upstream) {
    let browsers = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut browser = TcpStream::connect(browsers.local_addr().unwrap())
        .await
        .unwrap();
    let (relayed, _) = browsers.accept().await.unwrap();
    let driving = async {
        browser.write_all(&[5, 1, 0]).await.unwrap();
        let mut chosen = [0; 2];
        browser.read_exact(&mut chosen).await.unwrap();
        browser
            .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0, 80])
            .await
            .unwrap();
        let mut reply = [0; 10];
        let _ = browser.read_exact(&mut reply).await;
        drop(browser);
    };
    tokio::join!(relay(relayed, upstream), driving);
}

/// The relay's upstream: the SOCKS5 proxy at `address`, with credentials.
fn upstream(address: SocketAddr) -> Upstream {
    Upstream {
        address,
        mode: Mode::Socks5 {
            username: "user".to_owned(),
            password: SecretString::from("secret"),
        },
        refused: AtomicBool::new(false),
        unreachable: AtomicBool::new(false),
        usage: None,
        refusal: None,
    }
}

/// Covers: story:5-88, error:proxy_auth_failed
#[tokio::test]
async fn whether_the_proxy_refused_the_credentials_is_the_latest_connections_answer() {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let upstream = upstream(fake_proxy(&owner, vec![1, 0]).await);

    through(&upstream).await;
    let after_a_refusal = upstream.refused.load(Ordering::Relaxed);
    through(&upstream).await;
    let after_an_acceptance = upstream.refused.load(Ordering::Relaxed);

    owner.cancel();
    assert_eq!((after_a_refusal, after_an_acceptance), (true, false));
}

/// A fake HTTP proxy on loopback that answers each connection's `CONNECT`
/// with the next answer head of `answers`.
async fn fake_http_proxy(owner: &TaskOwner, answers: Vec<&'static str>) -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
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
            stream.write_all(answer.as_bytes()).await.unwrap();
        }
    });
    address
}

/// One browser `CONNECT` through the relay to `upstream`, until the proxy's
/// answer head came back.
async fn connect_through(upstream: &Upstream) -> String {
    let browsers = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut browser = TcpStream::connect(browsers.local_addr().unwrap())
        .await
        .unwrap();
    let (relayed, _) = browsers.accept().await.unwrap();
    let driving = async {
        browser
            .write_all(b"CONNECT site.test:443 HTTP/1.1\r\nHost: site.test:443\r\n\r\n")
            .await
            .unwrap();
        let mut answer = Vec::new();
        let mut byte = [0; 1];
        while !answer.ends_with(b"\r\n\r\n") {
            browser.read_exact(&mut byte).await.unwrap();
            answer.push(byte[0]);
        }
        drop(browser);
        String::from_utf8(answer).unwrap()
    };
    let ((), answer) = tokio::join!(relay(relayed, upstream), driving);
    answer
}

/// Covers: story:8-89, story:8-108
#[tokio::test]
async fn the_relay_keeps_a_hosted_providers_refusal_of_the_latest_connect_for_its_adapter() {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let address = fake_http_proxy(
        &owner,
        vec![
            "HTTP/1.1 502 Bad Gateway\r\nX-Brd-Err-Code: client_10064\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 Connection established\r\n\r\n",
        ],
    )
    .await;
    let upstream = Upstream {
        mode: Mode::AsItIs { http: true },
        refusal: Some(std::sync::Mutex::new(None)),
        ..upstream(address)
    };
    let latest = |upstream: &Upstream| {
        upstream
            .refusal
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .clone()
            .map(|answer| {
                (
                    answer.status,
                    answer.header("x-brd-err-code").map(str::to_owned),
                )
            })
    };

    let refused = connect_through(&upstream).await;
    let after_a_refusal = latest(&upstream);
    let taken = connect_through(&upstream).await;
    let after_a_tunnel = latest(&upstream);

    owner.cancel();
    assert_eq!(
        (
            refused.starts_with("HTTP/1.1 502"),
            after_a_refusal,
            taken.starts_with("HTTP/1.1 200"),
            after_a_tunnel
        ),
        (
            true,
            Some((502, Some("client_10064".to_owned()))),
            true,
            None
        )
    );
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
