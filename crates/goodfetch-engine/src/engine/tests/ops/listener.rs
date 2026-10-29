//! The ops listener's bounds through a Daemon's real ops listener on
//! loopback (ADR 0050; spec #213's Seam 3): a connection that sends no
//! request head is closed after the header-read timeout, five seconds, and
//! a connection past the 64 open at once is closed as it is accepted.
//!
//! The probes' sockets are real, on the tests' paused runtime: while every
//! task waits on a socket the runtime advances its clock to the next timer,
//! here the ops connections' header-read timeouts (TESTING.md, Time), and
//! the clock read when a connection closes says which timer closed it.
#![expect(
    clippy::disallowed_types,
    reason = "the probes connect to the ops listener as a kubelet would, from outside the engine (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the paused runtime's clock, which the Daemon's Clock follows (ADR 0059)"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpStream;
use tokio::time::Instant;

use super::super::daemon::the_pick;
use super::super::{dependencies, engine};
use crate::daemon::DaemonOptions;
use crate::ops::{Ops, OpsOptions, Scrape};
use crate::transport::{self, Network};

/// The header-read timeout ADR 0050's listener holds a connection to.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How many ops connections may be open at once.
const OPEN_AT_MOST: usize = 64;

struct NoMetrics;

impl Scrape for NoMetrics {
    fn text(&self) -> String {
        String::new()
    }
}

/// Runs a Daemon with an ops listener on a free loopback port while `body`
/// probes it, and answers what `body` answered.
fn probed<F, Fut, T>(body: F) -> T
where
    F: FnOnce(SocketAddr) -> Fut,
    Fut: Future<Output = T>,
{
    let env = TestEnv::new();
    goodfetch_test_support::runtime().block_on(async {
        let listener = transport::listen_ops("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let address = listener.local_address().unwrap();
        let options = DaemonOptions {
            ops: Ops {
                listener: Some(OpsOptions {
                    listener,
                    token: "the-ops-token".to_owned(),
                    scrape: Arc::new(NoMetrics),
                }),
                browsers: None,
            },
            ..the_pick()
        };
        let (network, _dialer) = transport::in_memory();
        let (daemon_listener, _) = Network::InMemory(network)
            .listen(std::path::Path::new("daemon.sock"))
            .await
            .unwrap();
        let engine = engine(Product::Collie);
        let (_, answered) = tokio::join!(
            engine.run_daemon(daemon_listener, dependencies(&env), options),
            body(address)
        );
        answered
    })
}

/// How long `connection` stayed open, sending nothing, until the listener
/// closed it.
async fn open_for(mut connection: TcpStream, since: Instant) -> Duration {
    let mut byte = [0_u8; 1];
    let read = connection.read(&mut byte).await;
    assert!(
        matches!(read, Ok(0) | Err(_)),
        "the listener answered a connection that sent nothing: {read:?}"
    );
    since.elapsed()
}

/// A connection that sends no request head holds a socket and a task only
/// until the header-read timeout, never until the Daemon stops.
/// Covers: story:3-102
#[test]
fn a_connection_that_sends_no_request_head_is_closed_after_the_header_read_timeout() {
    let open = probed(|address| async move {
        let connection = TcpStream::connect(address).await.unwrap();
        open_for(connection, Instant::now()).await
    });

    assert_eq!(open, HEADER_READ_TIMEOUT);
}

/// Past the connections open at once, the listener closes a new one as it
/// accepts it, while the others stay open until their timeout.
/// Covers: story:3-102
#[test]
fn a_connection_past_the_ones_open_at_once_is_closed_as_it_is_accepted() {
    let (past, first) = probed(|address| async move {
        let since = Instant::now();
        let mut open = Vec::new();
        for _ in 0..OPEN_AT_MOST {
            open.push(TcpStream::connect(address).await.unwrap());
        }
        let past = TcpStream::connect(address).await.unwrap();
        let past = open_for(past, since).await;
        let first = open_for(open.remove(0), since).await;
        (past, first)
    });

    assert_eq!((past, first), (Duration::ZERO, HEADER_READ_TIMEOUT));
}
