//! No log holds `hello`'s token (ADR 0067), though tungstenite logs each
//! WebSocket frame's payload at `trace` through the `log` crate: an
//! Exposed Daemon and this crate's Connection to it, in one process whose
//! subscriber records every target at `trace` and whose `log` records a
//! bridge hands to it, which carries both ends' frames. The subscriber caps
//! the frames' targets as the engine's and the CLI's do, and as the crate's
//! `logging` docs tell a program that bridges `log` to. The test installs
//! the process's global logger and subscriber, which the Daemon's tasks log
//! to, so it runs in a process of its own, as `just test` runs every test.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_collie_client::transport::{Endpoint, ExposedDaemon, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime};
use goodfetch_engine::daemon::{DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::endpoint::Authorities;
use goodfetch_engine::ops::Ops;
use goodfetch_protocol::connection::HelloParams;
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_test_support::{Canaries, CapturedLog, Output, SecretKind};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::connection::{dependencies, engine, hello};

/// The Daemon's major's pick, which never hands over.
struct Pick;

impl Succession for Pick {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

#[test]
fn neither_end_of_an_exposed_daemons_websocket_logs_hellos_token_at_trace() {
    let log = CapturedLog::default();
    let writer = log.clone();
    tracing_subscriber::registry()
        .with(goodfetch_engine::log::frame_payloads_unlogged())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(move || writer.clone())
                .with_ansi(false),
        )
        .init();
    tracing_log::LogTracer::init().unwrap();
    let env = goodfetch_test_support::TestEnv::new();
    // The Daemon's token, which `hello` carries in its first frame.
    let mut canaries = Canaries::new(env.seed());
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let home = tempfile::tempdir().unwrap();

    goodfetch_test_support::real_time_runtime().block_on(async {
        let (listener, endpoint) = goodfetch_engine::transport::Network::Os
            .listen_at(([127, 0, 0, 1], 0).into())
            .await
            .unwrap();
        let DaemonEndpoint::Tcp(address) = endpoint else {
            panic!("the Daemon listens on TCP");
        };
        let asked = CancellationToken::new();
        let options = DaemonOptions {
            token: token.clone(),
            endpoint,
            pid: 4242,
            idle_timeout: Duration::from_secs(600),
            keep_alive: true,
            holds: Holds::default(),
            succession: Arc::new(Pick),
            drain: Drain {
                asked: Some(asked.clone()),
                grace: Duration::from_secs(5),
            },
            install: None,
            ops: Ops::default(),
            websocket: Some(Authorities::loopback(address)),
            live_view: None,
        };
        let client = async {
            let exposed = Endpoint::Exposed(ExposedDaemon {
                address,
                authority: address.to_string(),
            });
            let (connection, _said) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &Network::Os,
                &exposed,
                HelloParams {
                    token: Some(token.clone()),
                    ..hello()
                },
            )
            .await
            .unwrap();
            connection.close().await.unwrap();
            asked.cancel();
        };
        let engine = engine();
        tokio::join!(
            engine.run_daemon(listener, dependencies(&home), options),
            client
        );
    });

    let written = log.written();
    let text = String::from_utf8_lossy(&written);
    let bridged =
        ["Server handshake done.", "Client handshake done."].map(|line| text.contains(line));
    let leaks = canaries
        .leaks(&[Output::Bytes {
            name: "log",
            bytes: &written,
        }])
        .unwrap();
    assert_eq!((bridged, leaks), ([true, true], Vec::new()), "{text}");
}
