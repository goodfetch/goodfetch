//! The Client against the Scripted engine as the Daemon, over a real
//! socket, and against the runtime files a Daemon leaves in the home.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write a home's files on the test thread, outside any runtime"
)]

use std::fs::{self, File};
use std::path::Path;
use std::time::Duration;

use goodfetch_kennel::client::{self, ConnectionChange, DaemonIdentity, Refusal};
use goodfetch_kennel::home::Home;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::daemon::{DaemonHealth, DaemonRuntimeFile, DaemonToken, Unready};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};

use crate::support::{ScriptedDaemon, next, reports, runtime, script, scripted_engine};

const TOKEN: &str = "kennel-test-token";

/// A health poll period no test waits for.
const NEVER: Duration = Duration::from_secs(3600);

/// A Kennel's Client following the Daemon of a home; dropping it stops
/// the Client, which closes its Connection.
struct Following {
    runtime: Runtime,
    reports: mpsc::Receiver<ConnectionChange>,
    keep_daemon_alive: watch::Sender<bool>,
    _look_again: mpsc::Sender<()>,
}

impl Following {
    /// Follows the Daemon of `home`, with the keep-alive setting
    /// `keep_daemon_alive` and a health poll every `health_every`.
    fn start(home: &Home, keep_daemon_alive: bool, health_every: Duration) -> Self {
        let runtime = runtime();
        let (tasks, report, reports) = reports(&runtime);
        let (look_again_sender, look_again) = mpsc::channel(1);
        let (keep_alive_sender, keep_alive) = watch::channel(keep_daemon_alive);
        tasks.spawn(client::follow(
            tasks.clone(),
            client::Follow {
                home: home.clone(),
                look_again,
                keep_daemon_alive: keep_alive,
                health_every,
            },
            report,
        ));
        Self {
            runtime,
            reports,
            keep_daemon_alive: keep_alive_sender,
            _look_again: look_again_sender,
        }
    }

    /// The next `count` reports.
    fn next(&mut self, count: usize) -> Vec<ConnectionChange> {
        self.runtime.block_on(async {
            let mut seen = Vec::new();
            for _ in 0..count {
                seen.push(next(&mut self.reports).await);
            }
            seen
        })
    }

    /// The reports up to the first that is not `NoDaemon`, which a Daemon
    /// that stops may cause more than once, as each change of its files
    /// makes the Client look again.
    fn next_but_no_daemon(&mut self) -> ConnectionChange {
        self.runtime.block_on(async {
            loop {
                let seen = next(&mut self.reports).await;
                if seen != ConnectionChange::NoDaemon {
                    return seen;
                }
            }
        })
    }
}

/// Follows the Daemon of `home` and returns its first `count` reports.
fn first_reports(home: &Home, count: usize) -> Vec<ConnectionChange> {
    Following::start(home, false, NEVER).next(count)
}

fn foxhound_0_4_1() -> ConnectionChange {
    ConnectionChange::Connected(DaemonIdentity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("0.4.1").unwrap(),
    })
}

fn collie_0_4_2() -> ConnectionChange {
    ConnectionChange::Connected(DaemonIdentity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.4.2").unwrap(),
    })
}

#[test]
fn with_the_keep_alive_setting_on_hello_asks_the_daemon_to_stay_alive() {
    let Some(engine) = scripted_engine("with_the_keep_alive_setting_on_hello_asks") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-keeping-the-daemon-alive.json"),
        TOKEN,
    );

    let mut kennel = Following::start(&home, true, NEVER);
    let seen = kennel.next(1);
    drop(kennel);

    assert_eq!(seen, [foxhound_0_4_1()]);
    assert!(
        daemon.ended().success(),
        "the Daemon saw the hello with the keep-alive flag"
    );
}

#[test]
fn turning_the_keep_alive_setting_on_reconnects_with_the_flag() {
    let Some(engine) = scripted_engine("turning_the_keep_alive_setting_on_reconnects") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let observed = ScriptedDaemon::start_as(
        &engine,
        &home,
        &script("hello-accepted-by-foxhound.json"),
        TOKEN,
        "0.4.1",
    );
    let mut kennel = Following::start(&home, false, NEVER);
    let first = kennel.next(1);
    // An older major's Daemon, which the Kennel reaches only once the
    // newer one is gone: the Kennel stays with the newest.
    let kept_alive = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-keeping-the-daemon-alive.json"),
        TOKEN,
    );

    kennel.keep_daemon_alive.send_replace(true);

    assert_eq!(first, [foxhound_0_4_1()]);
    assert!(
        observed.ended().success(),
        "the Kennel left the Daemon it reached without the flag"
    );
    assert_eq!(kennel.next_but_no_daemon(), foxhound_0_4_1());
    drop(kennel);
    assert!(
        kept_alive.ended().success(),
        "the Kennel said hello again with the flag"
    );
}

#[test]
fn a_daemon_that_starts_while_none_runs_is_reached() {
    let Some(engine) = scripted_engine("a_daemon_that_starts_while_none_runs_is_reached") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let mut kennel = Following::start(&home, false, NEVER);
    let before = kennel.next(1);

    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-accepted-by-foxhound.json"),
        TOKEN,
    );

    assert_eq!(before, [ConnectionChange::NoDaemon]);
    assert_eq!(kennel.next_but_no_daemon(), foxhound_0_4_1());
    drop(kennel);
    assert!(daemon.ended().success());
}

#[test]
fn a_daemon_of_a_newer_major_that_starts_is_reached_in_place_of_the_one_reached() {
    let Some(engine) = scripted_engine("a_daemon_of_a_newer_major_that_starts_is_reached") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let older = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-accepted-by-foxhound.json"),
        TOKEN,
    );
    let mut kennel = Following::start(&home, false, NEVER);
    let first = kennel.next(1);

    let newer = ScriptedDaemon::start_as(
        &engine,
        &home,
        &script("hello-accepted-by-collie.json"),
        TOKEN,
        "0.4.2",
    );

    assert_eq!(first, [foxhound_0_4_1()]);
    assert_eq!(kennel.next_but_no_daemon(), collie_0_4_2());
    assert!(
        older.ended().success(),
        "the Kennel closed its Connection to the older Daemon"
    );
    drop(kennel);
    assert!(newer.ended().success());
}

#[test]
fn after_a_handover_refusal_the_kennel_reaches_the_pick_once_it_runs() {
    let Some(engine) = scripted_engine("after_a_handover_refusal_the_kennel_reaches_the_pick")
    else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let handing_over = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-refused-handing-over.json"),
        TOKEN,
    );
    let mut kennel = Following::start(&home, false, NEVER);
    let refused = kennel.next(1);
    assert!(handing_over.ended().success());

    let pick = ScriptedDaemon::start(
        &engine,
        &home,
        &script("hello-accepted-by-collie.json"),
        TOKEN,
    );

    assert_eq!(refused, [ConnectionChange::Refused(Refusal::HandOver)]);
    assert_eq!(kennel.next_but_no_daemon(), collie_0_4_2());
    drop(kennel);
    assert!(pick.ended().success());
}

#[test]
fn the_daemons_health_is_polled_and_reported() {
    let Some(engine) = scripted_engine("the_daemons_health_is_polled_and_reported") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("daemon-health-draining-then-closed.json"),
        TOKEN,
    );

    let seen = Following::start(&home, false, Duration::from_millis(100)).next(3);

    assert_eq!(
        seen,
        [
            foxhound_0_4_1(),
            ConnectionChange::Health(DaemonHealth {
                live: true,
                ready: false,
                unready: vec![Unready::Draining],
            }),
            ConnectionChange::NoDaemon,
        ]
    );
    assert!(daemon.ended().success());
}

#[test]
fn a_runtime_file_written_while_no_daemon_runs_makes_the_client_look_again() {
    let folder = tempfile::tempdir().unwrap();
    let mut kennel = Following::start(&Home::new(folder.path()), false, NEVER);
    let before = kennel.next(1);

    // A runtime file in a per-major folder that did not exist when the
    // Client started, whose serving lock nobody holds.
    let major = dead_daemons_runtime_file(folder.path(), "0.1");
    File::create(major.join("daemon.lock")).unwrap();

    assert_eq!(before, [ConnectionChange::NoDaemon]);
    assert_eq!(kennel.next(1), [ConnectionChange::NoDaemon]);
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
