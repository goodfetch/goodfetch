//! The Client against the Scripted engine as the Daemon, over a real
//! socket, and against the runtime files a Daemon leaves in the home.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write a home's files on the test thread, outside any runtime"
)]

use std::fs::{self, File};
use std::path::Path;

use goodfetch_kennel::client::{self, ConnectionChange, DaemonIdentity, Refusal};
use goodfetch_kennel::home::Home;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::daemon::{DaemonRuntimeFile, DaemonToken};
use tokio::sync::mpsc;

use crate::support::{ScriptedDaemon, next, reports, runtime, script, scripted_engine};

const TOKEN: &str = "kennel-test-token";

/// Follows the Daemon of `home` and returns its first `count` reports.
fn first_reports(home: &Home, count: usize) -> Vec<ConnectionChange> {
    let runtime = runtime();
    let (tasks, report, mut reports) = reports(&runtime);
    let (_look_again, look_again) = mpsc::channel(1);
    tasks.spawn(client::follow(
        tasks.clone(),
        home.clone(),
        look_again,
        report,
    ));
    runtime.block_on(async {
        let mut seen = Vec::new();
        for _ in 0..count {
            seen.push(next(&mut reports).await);
        }
        seen
    })
}

#[test]
fn a_kennel_says_hello_as_the_kennel_surface_and_reads_the_daemons_product() {
    let Some(engine) = scripted_engine("a_kennel_says_hello_as_the_kennel_surface") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-accepted-by-foxhound.json"),
        TOKEN,
    );

    let seen = first_reports(&home, 1);

    assert_eq!(
        seen,
        [ConnectionChange::Connected(DaemonIdentity {
            product: Product::Foxhound,
            product_version: ProductVersion::parse("0.4.1").unwrap(),
        })]
    );
    assert!(
        daemon.ended().success(),
        "the Daemon saw the hello it expected"
    );
}

#[test]
fn a_daemon_that_closes_the_connection_is_reported_gone() {
    let Some(engine) = scripted_engine("a_daemon_that_closes_the_connection_is_reported_gone")
    else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let _daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-accepted-then-closed.json"),
        TOKEN,
    );

    let seen = first_reports(&home, 2);

    assert!(
        matches!(seen[0], ConnectionChange::Connected(_)),
        "{seen:?}"
    );
    assert_eq!(seen[1], ConnectionChange::NoDaemon);
}

#[test]
fn a_daemon_of_another_protocol_version_is_reported_with_the_version_the_kennel_needs() {
    let Some(engine) = scripted_engine("a_daemon_of_another_protocol_version") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-refused-protocol-version-unsupported.json"),
        TOKEN,
    );

    let seen = first_reports(&home, 1);

    assert_eq!(
        seen,
        [ConnectionChange::Refused(Refusal::ProtocolVersion {
            engine: ProtocolVersion { major: 0, minor: 2 },
            needs: ProtocolVersion { major: 0, minor: 1 },
        })]
    );
    assert!(daemon.ended().success());
}

#[test]
fn an_empty_home_has_no_daemon() {
    let folder = tempfile::tempdir().unwrap();

    let seen = first_reports(&Home::new(folder.path()), 1);

    assert_eq!(seen, [ConnectionChange::NoDaemon]);
}

/// Writes a runtime file into `home`'s per-major folder `major`, naming a
/// socket nothing listens on, as a dead Daemon leaves it.
fn dead_daemons_runtime_file(home: &Path, major: &str) -> std::path::PathBuf {
    let folder = home.join(major);
    fs::create_dir_all(&folder).unwrap();
    let runtime = DaemonRuntimeFile {
        endpoint: DaemonEndpoint::Unix(folder.join("daemon.sock")),
        token: DaemonToken::new(TOKEN),
        version: ProductVersion::parse("0.1.0").unwrap(),
        pid: 1,
        console: ConsoleState::Absent,
    };
    fs::write(
        folder.join("daemon.json"),
        serde_json::to_vec(&runtime).unwrap(),
    )
    .unwrap();
    folder
}

#[test]
fn a_runtime_file_whose_serving_lock_nobody_holds_is_no_daemon() {
    let folder = tempfile::tempdir().unwrap();
    let major = dead_daemons_runtime_file(folder.path(), "0.1");
    File::create(major.join("daemon.lock")).unwrap();

    let seen = first_reports(&Home::new(folder.path()), 1);

    assert_eq!(seen, [ConnectionChange::NoDaemon]);
}

#[test]
fn a_held_serving_lock_beside_a_socket_nothing_listens_on_is_no_daemon() {
    let folder = tempfile::tempdir().unwrap();
    let major = dead_daemons_runtime_file(folder.path(), "0.1");
    let lock = File::create(major.join("daemon.lock")).unwrap();
    lock.try_lock().unwrap();

    let seen = first_reports(&Home::new(folder.path()), 1);

    assert_eq!(seen, [ConnectionChange::NoDaemon]);
}
