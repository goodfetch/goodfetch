//! The Client against the Scripted engine as the Daemon, over a real
//! socket, and against the runtime files a Daemon leaves in the home.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write a home's files on the test thread, outside any runtime"
)]

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::Path;

use goodfetch_kennel::client::{
    self, ConnectionChange, DaemonEvent, DaemonIdentity, Refusal, Report, Wanted,
};
use goodfetch_kennel::home::Home;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::daemon::{DaemonRuntimeFile, DaemonToken};
use goodfetch_protocol::run::{Lifecycle, RunId};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};

use crate::support::{ScriptedDaemon, next, reports, runtime, script, scripted_engine};

const TOKEN: &str = "kennel-test-token";

/// The Client following the Daemon of a home, with the reads it is asked
/// for.
struct Following {
    runtime: Runtime,
    reports: mpsc::Receiver<Report>,
    wanted: watch::Sender<Wanted>,
    _look_again: mpsc::Sender<()>,
}

impl Following {
    /// Follows the Daemon of `home`, asked first for `wanted`.
    fn start(home: &Home, wanted: Wanted) -> Self {
        let runtime = runtime();
        let (tasks, report, reports) = reports(&runtime);
        let (look_again, looks) = mpsc::channel(1);
        let (wanted, wanted_now) = watch::channel(wanted);
        tasks.spawn(client::follow(
            tasks.clone(),
            home.clone(),
            looks,
            wanted_now,
            report,
        ));
        Self {
            runtime,
            reports,
            wanted,
            _look_again: look_again,
        }
    }

    /// The next report.
    fn next(&mut self) -> Report {
        self.runtime.block_on(next(&mut self.reports))
    }

    /// The next report, which is a change of the Connection.
    fn next_change(&mut self) -> ConnectionChange {
        match self.next() {
            Report::Connection(change) => change,
            other => panic!("a change of the Connection, not {other:?}"),
        }
    }
}

/// Follows the Daemon of `home` and returns its first `count` changes of
/// the Connection.
fn first_changes(home: &Home, count: usize) -> Vec<ConnectionChange> {
    let mut following = Following::start(home, Wanted::default());
    (0..count).map(|_| following.next_change()).collect()
}

fn run_id(run: &str) -> RunId {
    RunId::parse(run).unwrap()
}

#[test]
fn a_kennel_says_hello_as_the_kennel_surface_reads_the_daemons_product_and_subscribes() {
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

    let seen = first_changes(&home, 1);

    assert_eq!(
        seen,
        [ConnectionChange::Connected(DaemonIdentity {
            product: Product::Foxhound,
            product_version: ProductVersion::parse("0.4.1").unwrap(),
        })]
    );
    assert!(
        daemon.ended().success(),
        "the Daemon saw the hello and the subscription it expected"
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

    let seen = first_changes(&home, 2);

    assert!(
        matches!(seen[0], ConnectionChange::Connected(_)),
        "{seen:?}"
    );
    assert_eq!(seen[1], ConnectionChange::NoDaemon);
}

#[test]
fn the_subscriptions_resync_and_axes_reach_the_kennel_as_typed_events() {
    let Some(engine) = scripted_engine("the_subscriptions_resync_and_axes_reach_the_kennel") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("events-resync-then-axes.json"),
        TOKEN,
    );
    let mut following = Following::start(&home, Wanted::default());
    following.next_change();

    let Report::Events(batch) = following.next() else {
        panic!("a batch of events");
    };

    let (events, _applied) = batch.into_parts();
    let [DaemonEvent::Resync(resync), DaemonEvent::RunAxes(axes)] = events.as_slice() else {
        panic!("a resync, then a Run's axes: {events:?}");
    };
    assert_eq!(resync.runs[0].run_id, run_id("shop-crawl"));
    assert_eq!(
        (&axes.run_id, axes.lifecycle),
        (&run_id("shop-crawl"), Lifecycle::Failed)
    );
    assert_eq!(following.next_change(), ConnectionChange::NoDaemon);
    assert!(daemon.ended().success());
}

#[test]
fn the_kennel_renews_half_its_credits_once_it_has_applied_the_events_they_took() {
    let Some(engine) = scripted_engine("the_kennel_renews_half_its_credits") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("credits-renewed-as-events-are-applied.json"),
        TOKEN,
    );
    let mut following = Following::start(&home, Wanted::default());
    following.next_change();
    let mut sizes = Vec::new();

    for _ in 0..2 {
        let Report::Events(batch) = following.next() else {
            panic!("a batch of events");
        };
        sizes.push(batch.events.len());
        // Dropping the batch once its events are applied gives their
        // credits back.
        drop(batch);
    }

    assert_eq!(sizes, [31, 1]);
    assert_eq!(following.next_change(), ConnectionChange::NoDaemon);
    assert!(
        daemon.ended().success(),
        "the Daemon saw one renewal of 32 credits"
    );
}

#[test]
fn asked_for_the_live_runs_rows_the_kennel_reads_the_running_runs_of_the_run_index() {
    let Some(engine) = scripted_engine("asked_for_the_live_runs_rows") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(&engine, &home, &script("live-runs-read.json"), TOKEN);
    let mut following = Following::start(&home, Wanted::default());
    following.next_change();

    following.wanted.send_modify(|wanted| wanted.live_runs += 1);

    let Report::LiveRuns(rows) = following.next() else {
        panic!("the live Runs' rows");
    };
    assert_eq!(rows[0].run_id, run_id("shop-crawl"));
    assert_eq!(following.next_change(), ConnectionChange::NoDaemon);
    assert!(daemon.ended().success());
}

#[test]
fn a_runs_progress_is_followed_while_the_view_wants_it_and_its_read_cancelled_after() {
    let Some(engine) = scripted_engine("a_runs_progress_is_followed_while_the_view_wants_it")
    else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("progress-followed-then-cancelled.json"),
        TOKEN,
    );
    let wanted = Wanted {
        live_runs: 0,
        progress: BTreeSet::from([run_id("shop-crawl")]),
    };
    let mut following = Following::start(&home, wanted);
    following.next_change();
    let Report::Progress(progress) = following.next() else {
        panic!("the Run's progress");
    };

    following
        .wanted
        .send_modify(|wanted| wanted.progress.clear());

    assert_eq!(
        (&progress.run_id, progress.position),
        (&run_id("shop-crawl"), 5)
    );
    assert_eq!(following.next_change(), ConnectionChange::NoDaemon);
    assert!(
        daemon.ended().success(),
        "the Daemon saw the read followed with after, then cancelled"
    );
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

    let seen = first_changes(&home, 1);

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

    let seen = first_changes(&Home::new(folder.path()), 1);

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

    let seen = first_changes(&Home::new(folder.path()), 1);

    assert_eq!(seen, [ConnectionChange::NoDaemon]);
}

#[test]
fn a_held_serving_lock_beside_a_socket_nothing_listens_on_is_no_daemon() {
    let folder = tempfile::tempdir().unwrap();
    let major = dead_daemons_runtime_file(folder.path(), "0.1");
    let lock = File::create(major.join("daemon.lock")).unwrap();
    lock.try_lock().unwrap();

    let seen = first_changes(&Home::new(folder.path()), 1);

    assert_eq!(seen, [ConnectionChange::NoDaemon]);
}
