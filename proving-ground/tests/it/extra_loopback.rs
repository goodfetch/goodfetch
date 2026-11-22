//! Tests that need the extra loopback addresses `127.0.0.2` to `127.0.0.9`
//! (TESTING.md, Storage, network and OS services): a host without them skips
//! this module.
#![expect(
    clippy::disallowed_methods,
    reason = "the test builds its own runtime to send requests from a chosen address"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the test opens its own sockets, each bound to an extra loopback address"
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use regex::Regex;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpSocket;

use crate::limits::start_url;
use crate::proving_ground::{ProvingGround, submission};

/// Sends `GET path` with `host` from the address `local`, and returns the
/// answer's status and body.
fn get_from(local: IpAddr, server: SocketAddr, host: &str, path: &str) -> (u16, String) {
    let runtime = goodfetch_runtime::current_thread()
        .enable_io()
        .build()
        .expect("a runtime");
    let response = runtime.block_on(async {
        let socket = TcpSocket::new_v4().expect("a socket");
        socket
            .bind(SocketAddr::new(local, 0))
            .expect("the extra loopback address binds");
        let mut stream = socket
            .connect(server)
            .await
            .expect("the Proving ground accepts");
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("the request is sent");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("the answer is read");
        String::from_utf8(response).expect("the answer is UTF-8")
    });
    let status = response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status line");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[test]
fn a_crawl_that_moves_to_another_address_once_flagged_is_verified() {
    let proving_ground = ProvingGround::start(164);
    let start = start_url(&proving_ground, "limits/2");
    let host = start
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .unwrap()
        .to_owned();
    let server: SocketAddr = proving_ground.address().parse().unwrap();
    let mut addresses = (2..=9).map(|last| IpAddr::V4(Ipv4Addr::new(127, 0, 0, last)));
    let mut address = addresses.next().unwrap();
    let mut fetch = |path: &str| loop {
        let (status, body) = get_from(address, server, &host, path);
        if status != 403 {
            return body;
        }
        address = addresses.next().expect("an address not flagged yet");
    };
    let listing = fetch("/");
    let item = Regex::new(r#"href="(/items/[^"]+)""#).unwrap();
    let id = Regex::new(r#"data-id="([^"]+)""#).unwrap();
    let name = Regex::new(r#"<h1 class="name">([^<]+)</h1>"#).unwrap();
    let paths: Vec<String> = item
        .captures_iter(&listing)
        .map(|link| link[1].to_owned())
        .collect();
    let records: Vec<Value> = paths
        .iter()
        .map(|path| {
            let page = fetch(path);
            json!({ "id": &id.captures(&page).unwrap()[1], "name": &name.captures(&page).unwrap()[1] })
        })
        .collect();

    let answer = proving_ground.judge(&submission("limits/2", "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_ne!(
        address,
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)),
        "the crawl was never flagged, so it never moved"
    );
}
