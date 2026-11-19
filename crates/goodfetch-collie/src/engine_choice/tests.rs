//! The remote Daemon addresses a connection takes; the ones it refuses are
//! read through `connect()` (`tests/it/sdk/engine_choice.rs`).

use std::net::SocketAddr;

use rstest::rstest;

use super::{RemoteDaemon, remote_address};

#[rstest]
#[case::ipv4("ws://127.0.0.1:7878", "127.0.0.1:7878")]
#[case::trailing_slash("ws://127.0.0.1:7878/", "127.0.0.1:7878")]
#[case::other_loopback("ws://127.1.2.3:7878", "127.1.2.3:7878")]
#[case::localhost("ws://localhost:7878", "127.0.0.1:7878")]
#[case::ipv6("ws://[::1]:7878", "[::1]:7878")]
fn a_loopback_ws_url_with_a_port_names_its_address(#[case] url: &str, #[case] address: &str) {
    assert_eq!(
        remote_address(url, "engine").unwrap(),
        address.parse::<SocketAddr>().unwrap()
    );
}

#[test]
fn a_remote_daemons_debug_output_shows_neither_its_url_nor_its_token() {
    let shown = format!(
        "{:?}",
        RemoteDaemon::new("ws://user:url-secret@127.0.0.1:1", "token-secret")
    );

    assert!(!shown.contains("secret"), "{shown}");
}
