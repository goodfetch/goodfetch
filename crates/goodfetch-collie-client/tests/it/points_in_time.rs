//! Points in time in results through the client crate (ADRs 0092 and
//! 0099): the RFC 3339 text the engine sends, with any offset, reaches the
//! program as a `SystemTime`, the same instant to the nanosecond. The
//! Scripted engine plays the corpus's script the TypeScript SDK plays too,
//! as the Daemon of the test's home.
//! Covers: story:11-91

use std::time::{Duration, SystemTime};

use goodfetch_collie_client::ClientConnection;
use goodfetch_collie_client::ConnectionRuntime;
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_test_support::TestEnv;

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// 2026-11-19T12:30:00Z, as seconds since the Unix epoch.
const HALF_PAST_NOON: Duration = Duration::from_mins(29_918_190);

#[test]
fn a_run_reports_points_in_time_as_system_times_whatever_offset_the_engine_sent() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/points-in-time.json");
    let runtime = goodfetch_test_support::real_time_runtime();
    let report = runtime.block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        )
        .await
        .unwrap();
        let report = connection.start().await.unwrap().finish().await.unwrap();
        connection.close().await.unwrap();
        report
    });

    assert_eq!(
        (report.started_at, report.ended_at),
        (
            SystemTime::UNIX_EPOCH + HALF_PAST_NOON + Duration::from_nanos(123_456_789),
            SystemTime::UNIX_EPOCH + HALF_PAST_NOON + Duration::from_millis(60_500),
        )
    );
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}
