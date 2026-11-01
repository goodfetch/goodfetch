//! A port held for a service manager's listener (ADR 0031): no other
//! program can take it before the manager listens on it (#1821).
#![cfg(unix)]

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use super::hold_free_port;

/// The listener a service manager opens on `address` with port reuse, as a
/// socket unit's `ReusePort=yes` does: `Ok` once it listens.
#[expect(
    clippy::disallowed_types,
    reason = "the test stands in for the service manager's own listener"
)]
fn manager_listens(address: SocketAddr) -> std::io::Result<()> {
    goodfetch_test_support::runtime().block_on(async {
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_reuseport(true)?;
        socket.bind(address)?;
        socket.listen(1).map(drop)
    })
}

/// A held port is a port of its own, which another program binding it
/// between the hold and the manager's listen cannot take, and which the
/// manager then listens on. A port bound and let go instead loses to that
/// program, and the manager finds its address in use, as systemd did.
#[test]
fn another_program_cannot_take_a_held_port_before_its_service_manager_listens() {
    let held = hold_free_port(IpAddr::V4(Ipv4Addr::LOCALHOST)).unwrap();
    let address = held.address();

    #[expect(
        clippy::disallowed_types,
        reason = "the test is the other program that takes a port it sees free"
    )]
    let taken = std::net::TcpListener::bind(address).map_err(|error| error.kind());
    let listens = manager_listens(address).map_err(|error| error.kind());
    drop(held);

    assert_eq!(
        (address.port() != 0, taken.err(), listens),
        (true, Some(ErrorKind::AddrInUse), Ok(()))
    );
}
