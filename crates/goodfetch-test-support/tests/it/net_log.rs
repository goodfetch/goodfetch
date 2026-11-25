//! `NetLog::read`: every socket a browser launch connected, from the NetLog
//! file Chromium writes under `--log-net-log`, for the quiet-browser test
//! (ADR 0074). The fixtures follow Chromium's own layout, the constants on
//! the first line and an event a line: `whole.json` as a browser that
//! exited writes it, and `cut-short.json` as a killed one leaves it.

use std::path::PathBuf;

use goodfetch_test_support::{NetLog, NetLogError, NetLogSocket, SocketTransport, TestEnv};

/// The fixture `name` of `fixtures/net-log/`.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/fixtures/net-log")
        .join(name)
}

/// Each TCP connection attempt and each connected UDP socket, failed ones
/// too, with the address it went to: the DNS job, the byte count and the
/// event of a source numbered below zero, as another process's are, in
/// the same file are no socket.
#[test]
fn a_whole_net_log_lists_every_socket_the_launch_connected() {
    let log = NetLog::read(&fixture("whole.json")).unwrap();

    let socket = |transport, address: &str, source| NetLogSocket {
        transport,
        address: address.parse().unwrap(),
        source,
    };
    assert_eq!(
        log.sockets(),
        [
            socket(SocketTransport::Udp, "[2001:4860:4860::8888]:443", 18),
            socket(SocketTransport::Tcp, "127.0.0.1:35997", 20),
            socket(SocketTransport::Tcp, "192.0.2.7:443", 22),
            socket(SocketTransport::Udp, "198.51.100.4:53", 23),
        ]
    );
}

/// A killed browser's file ends inside its events, so the sockets it
/// connected last are missing: it is refused, never read as a quiet launch.
#[test]
fn a_net_log_cut_short_is_refused() {
    let path = fixture("cut-short.json");

    let read = NetLog::read(&path);

    assert!(
        matches!(&read, Err(NetLogError::Truncated(cut)) if *cut == path),
        "{read:?}"
    );
}

/// A file whose constants name no socket event would list no socket, as a
/// quiet launch's does: it is refused.
#[test]
fn a_net_log_whose_constants_name_no_socket_event_is_refused() {
    let env = TestEnv::new();
    let path = env.folder().join("no-constants.json");
    std::fs::write(
        &path,
        r#"{"constants":{"logEventTypes":{},"logEventPhase":{"PHASE_BEGIN":1}},"events":[]}"#,
    )
    .unwrap();

    let read = NetLog::read(&path);

    assert!(
        matches!(&read, Err(NetLogError::Malformed { reason, .. }) if reason.contains("TCP_CONNECT_ATTEMPT")),
        "{read:?}"
    );
}
