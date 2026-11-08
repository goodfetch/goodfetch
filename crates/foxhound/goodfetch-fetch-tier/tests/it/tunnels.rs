//! An `https://` site through a proxy's tunnel on each transport, reqwest's
//! and `wreq`'s: the request reaches the site over TLS through `CONNECT`, and
//! the proxy's answer decides whether the tunnel opens.
#![expect(
    clippy::disallowed_types,
    reason = "a loopback proxy on the test's own threads, never an async worker (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test's proxy answers on plain threads, for as long as the test's process runs"
)]

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};

use goodfetch_fetch_tier::{Hop, Reuse, Transport, TransportFailure};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};
use url::Url;

use crate::{Stack, built, config, running_clock_runtime};

/// Any seed: the proxies and `scope.test` are served on every one.
const SEED: u64 = 759;

/// The host whose certificate the Proving ground's authority makes valid,
/// once that authority is trusted for it.
const SITE: &str = "scope.test";

/// `stack`'s transport through the proxy at `proxy`, trusting the Proving
/// ground's authority for [`SITE`].
fn through(stack: Stack, proving_ground: &ProvingGround, proxy: &str) -> Box<dyn Transport> {
    let proxy = Url::parse(&format!("http://{proxy}")).unwrap();
    built(stack, config(proving_ground, Some(proxy), &[SITE]))
}

/// The hop to [`SITE`]'s root over TLS.
fn to_the_site(proving_ground: &ProvingGround) -> Hop {
    Hop::get(
        Url::parse(&format!(
            "https://{SITE}:{}/",
            proving_ground.address().port()
        ))
        .unwrap(),
    )
}

/// A forward proxy on loopback that answers each connection's request with
/// `answer` and then relays it to `site`, and the first line of each request
/// it took.
fn answering(answer: &'static [u8], site: SocketAddr) -> (SocketAddr, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (taken, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for client in listener.incoming().map_while(Result::ok) {
            let taken = taken.clone();
            std::thread::spawn(move || {
                let _ = relayed(client, answer, site, &taken);
            });
        }
    });
    (address, lines)
}

/// Reads the head of `client`'s request, answers it, then copies each way
/// between `client` and `site` until they close.
fn relayed(
    mut client: TcpStream,
    answer: &[u8],
    site: SocketAddr,
    taken: &mpsc::Sender<String>,
) -> io::Result<()> {
    let mut head = Vec::new();
    let mut chunk = [0; 4096];
    while !head.windows(4).any(|end| end == b"\r\n\r\n") {
        let read = client.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let first_line = String::from_utf8_lossy(&head)
        .lines()
        .next()
        .unwrap_or("")
        .to_owned();
    let _ = taken.send(first_line);
    client.write_all(answer)?;
    let mut site = TcpStream::connect(site)?;
    let mut client_side = client.try_clone()?;
    let mut site_side = site.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut client_side, &mut site_side);
        let _ = site_side.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut site, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

/// The fields of each relay the Proving ground's proxies made that a test
/// reads.
fn relays(observations: &Value) -> Vec<Value> {
    observations["relays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|relay| {
            json!([
                relay["proxy"],
                relay["method"],
                relay["target"],
                relay["outcome"]
            ])
        })
        .collect()
}

/// An `https://` site answers through the Proving ground's open proxy: the
/// proxy relayed one `CONNECT` to the site, and the site read the request
/// over TLS under its own name, not the proxy's.
/// Covers: story:4-12
#[rstest]
fn an_https_site_answers_through_the_tunnel_of_a_proxy(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let open = &proving_ground.description().fixtures.proxies["open"];
    let transport = through(stack, &proving_ground, open);

    let sent = running_clock_runtime()
        .block_on(transport.send(&to_the_site(&proving_ground), Reuse::Allowed))
        .unwrap();

    let observations = proving_ground.observations().unwrap();
    assert_eq!(sent.status, 200);
    assert_eq!(
        relays(&observations),
        [json!([
            "open",
            "CONNECT",
            format!("{SITE}:{}", proving_ground.address().port()),
            "relayed"
        ])]
    );
    let received = proving_ground.received_requests().unwrap();
    let tls = received
        .of_the_case()
        .next()
        .and_then(|request| request.tls.as_ref());
    assert_eq!(tls.map(|tls| &tls["sni"]), Some(&json!(SITE)));
}

/// A proxy's answer to `CONNECT` decides the tunnel, as each transport's
/// library reads it: 200 opens it, whichever version of HTTP says so, and
/// only 200 does, so another 2xx is the proxy's refusal.
/// Covers: story:4-12
#[rstest]
#[case::established_over_http_1_1(b"HTTP/1.1 200 Connection Established\r\n\r\n", Ok(200))]
#[case::established_over_http_1_0(b"HTTP/1.0 200 OK\r\n\r\n", Ok(200))]
#[case::another_2xx(
    b"HTTP/1.0 204 No Content\r\n\r\n",
    Err(TransportFailure::TunnelRefused)
)]
fn the_answer_of_a_proxy_to_connect_decides_the_tunnel(
    #[case] answer: &'static [u8],
    #[case] expected: Result<u16, TransportFailure>,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (proxy, taken) = answering(answer, proving_ground.address());
    let transport = through(stack, &proving_ground, &proxy.to_string());

    let sent = running_clock_runtime()
        .block_on(transport.send(&to_the_site(&proving_ground), Reuse::Allowed))
        .map(|sent| sent.status);

    assert_eq!(sent, expected);
    assert_eq!(
        taken.try_recv(),
        Ok(format!(
            "CONNECT {SITE}:{} HTTP/1.1",
            proving_ground.address().port()
        ))
    );
}
