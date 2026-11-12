use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::daemon::{DaemonRuntimeFile, DaemonToken};
use goodfetch_test_support::{
    DAEMON_SERVING_LOCK, DAEMON_SOCKET, DaemonEndpoint as TestDaemonEndpoint, RuntimeFile, TestEnv,
};

use super::*;

#[test]
fn the_per_major_folder_holds_the_user_configuration_test_support_writes() {
    let env = TestEnv::new();

    let layout = HomeLayout::of(env.home());

    assert!(layout.user_configuration().is_file());
}

#[test]
fn the_runtime_file_is_where_test_support_reads_a_daemons() {
    let env = TestEnv::new();

    let layout = HomeLayout::of(env.home());

    assert_eq!(
        layout.daemon_file(DaemonFile::RuntimeFile),
        RuntimeFile::path(env.home(), product_version().as_str())
    );
}

#[test]
fn a_runs_file_and_trace_sit_in_its_run_folder_under_the_per_major_folder() {
    let layout = HomeLayout::of(Path::new("home"));

    let paths = (
        layout.run_file("first"),
        layout.run_trace("first", home::RUN_TRACE_PROVENANCE),
    );

    assert_eq!(
        paths,
        (
            layout.per_major().join("runs/first/run.sqlite"),
            layout.per_major().join("runs/first/trace/provenance.jsonl"),
        )
    );
}

#[test]
fn the_scripted_daemons_lock_and_socket_are_named_as_a_daemons() {
    let names = (DAEMON_SERVING_LOCK, DAEMON_SOCKET);

    assert_eq!(
        names,
        (DaemonFile::ServingLock.name(), DaemonFile::Socket.name())
    );
}

/// The runtime file a Daemon writes reads as
/// test support's copy of the format, which the Scripted engine writes for the
/// CLI and the SDKs. It holds this direction: a Client reading what the
/// Scripted engine writes is the scripted Daemon tests' to prove.
#[test]
fn a_daemons_runtime_file_reads_as_test_supports() {
    let written = serde_json::to_string(&DaemonRuntimeFile {
        endpoint: DaemonEndpoint::Unix("/home/daemon.sock".into()),
        token: DaemonToken::new("token"),
        version: ProductVersion::parse("0.0.1").unwrap(),
        pid: 42,
    })
    .unwrap();

    let read: RuntimeFile = serde_json::from_str(&written).unwrap();

    assert_eq!(
        read,
        RuntimeFile {
            endpoint: TestDaemonEndpoint::Unix("/home/daemon.sock".into()),
            token: "token".to_owned(),
            version: "0.0.1".to_owned(),
            pid: 42,
        }
    );
}
