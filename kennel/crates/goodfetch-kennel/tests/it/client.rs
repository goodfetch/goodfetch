//! The Client against the Scripted engine as the Daemon, over a real
//! socket, and against the runtime files a Daemon leaves in the home.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write a home's files on the test thread, outside any runtime"
)]

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::Path;
use std::time::Duration;

use goodfetch_kennel::client::{
    self, CONNECTION_LOST, Call, CallFailure, ConnectionChange, DaemonEvent, DaemonIdentity,
    NO_DAEMON, Refusal, Report, Wanted,
};
use goodfetch_kennel::home::Home;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::daemon::{DaemonHealth, DaemonRuntimeFile, DaemonToken, Unready};
use goodfetch_protocol::needs_you::{
    NeedsYouAllowConsentDomainParams, NeedsYouDenyWallDomainParams, NeedsYouDismissParams,
    NeedsYouRemoveRefetchDomainParams,
};
use goodfetch_protocol::run::{Lifecycle, RunAbortParams, RunId, RunParams};
use goodfetch_protocol::usage::UsageRaiseParams;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};

use crate::support::{ScriptedDaemon, next, reports, runtime, script, scripted_engine};

const TOKEN: &str = "kennel-test-token";

/// A health poll period no test waits for.
pub(crate) const NEVER: Duration = Duration::from_secs(3600);

/// A Kennel's Client following the Daemon of a home; dropping it stops
/// the Client, which closes its Connection.
pub(crate) struct Following {
    runtime: Runtime,
    reports: mpsc::Receiver<Report>,
    keep_daemon_alive: watch::Sender<bool>,
    wanted: watch::Sender<Wanted>,
    calls: mpsc::Sender<Call>,
    _look_again: mpsc::Sender<()>,
}

impl Following {
    /// Follows the Daemon of `home`, with the keep-alive setting
    /// `keep_daemon_alive` and a health poll every `health_every`.
    pub(crate) fn start(home: &Home, keep_daemon_alive: bool, health_every: Duration) -> Self {
        Self::wanting(home, keep_daemon_alive, health_every, Wanted::default())
    }

    /// Follows the Daemon of `home`, asked first for `wanted`.
    fn wanting(
        home: &Home,
        keep_daemon_alive: bool,
        health_every: Duration,
        wanted: Wanted,
    ) -> Self {
        let runtime = runtime();
        let (tasks, report, reports) = reports(&runtime);
        let (look_again_sender, look_again) = mpsc::channel(1);
        let (keep_alive_sender, keep_alive) = watch::channel(keep_daemon_alive);
        let (wanted_sender, wanted) = watch::channel(wanted);
        let (calls_sender, calls) = mpsc::channel(16);
        tasks.spawn(client::follow(
            tasks.clone(),
            client::Follow {
                home: home.clone(),
                look_again,
                keep_daemon_alive: keep_alive,
                health_every,
                wanted,
                calls,
            },
            report,
        ));
        Self {
            runtime,
            reports,
            keep_daemon_alive: keep_alive_sender,
            wanted: wanted_sender,
            calls: calls_sender,
            _look_again: look_again_sender,
        }
    }

    /// Makes `call` through the Client.
    pub(crate) fn call(&self, call: Call) {
        self.calls.try_send(call).unwrap();
    }

    /// The next answer to a call, past the subscription's events.
    pub(crate) fn next_answer(&mut self) -> (Call, Result<(), CallFailure>) {
        loop {
            match self.next_report() {
                Report::Answered(call, answer) => return (call, answer),
                Report::Events(_) => {}
                other => panic!("a call's answer, not {other:?}"),
            }
        }
    }

    /// The next report.
    fn next_report(&mut self) -> Report {
        self.runtime.block_on(next(&mut self.reports))
    }

    /// The next report, which is a change of the Connection.
    fn next_change(&mut self) -> ConnectionChange {
        match self.next_report() {
            Report::Connection(change) => change,
            other => panic!("a change of the Connection, not {other:?}"),
        }
    }

    /// The next `count` reports, each a change of the Connection.
    pub(crate) fn next(&mut self, count: usize) -> Vec<ConnectionChange> {
        (0..count).map(|_| self.next_change()).collect()
    }

    /// The reports up to the first that is not `NoDaemon`, which a Daemon
    /// that stops may cause more than once, as each change of its files
    /// makes the Client look again.
    fn next_but_no_daemon(&mut self) -> ConnectionChange {
        loop {
            let seen = self.next_change();
            if seen != ConnectionChange::NoDaemon {
                return seen;
            }
        }
    }
}

fn run_id(run: &str) -> RunId {
    RunId::parse(run).unwrap()
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

    let seen = first_reports(&home, 1);

    assert_eq!(seen, [foxhound_0_4_1()]);
    assert!(
        daemon.ended().success(),
        "the Daemon saw the hello and the subscription it expected"
    );
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
    let mut kennel = Following::start(&home, false, NEVER);
    kennel.next(1);

    let Report::Events(batch) = kennel.next_report() else {
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
    assert_eq!(kennel.next(1), [ConnectionChange::NoDaemon]);
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
    let mut kennel = Following::start(&home, false, NEVER);
    kennel.next(1);
    let mut sizes = Vec::new();

    for _ in 0..2 {
        let Report::Events(batch) = kennel.next_report() else {
            panic!("a batch of events");
        };
        sizes.push(batch.events.len());
        // Dropping the batch once its events are applied gives their
        // credits back.
        drop(batch);
    }

    assert_eq!(sizes, [31, 1]);
    assert_eq!(kennel.next(1), [ConnectionChange::NoDaemon]);
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
    let mut kennel = Following::start(&home, false, NEVER);
    kennel.next(1);

    kennel.wanted.send_modify(|wanted| wanted.live_runs += 1);

    let Report::LiveRuns(rows) = kennel.next_report() else {
        panic!("the live Runs' rows");
    };
    assert_eq!(rows[0].run_id, run_id("shop-crawl"));
    assert_eq!(kennel.next(1), [ConnectionChange::NoDaemon]);
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
    let mut kennel = Following::wanting(&home, false, NEVER, wanted);
    kennel.next(1);
    let Report::Progress(progress) = kennel.next_report() else {
        panic!("the Run's progress");
    };

    kennel.wanted.send_modify(|wanted| wanted.progress.clear());

    assert_eq!(
        (&progress.run_id, progress.position),
        (&run_id("shop-crawl"), 5)
    );
    assert_eq!(kennel.next(1), [ConnectionChange::NoDaemon]);
    assert!(
        daemon.ended().success(),
        "the Daemon saw the read followed with after, then cancelled"
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

fn dismiss() -> Call {
    Call::Dismiss(NeedsYouDismissParams {
        item: "consent_wall:news.example".to_owned(),
    })
}

#[test]
fn each_call_a_person_makes_is_sent_to_its_operation_and_its_answer_reported() {
    let Some(engine) = scripted_engine("each_call_a_person_makes_is_sent_to_its_operation") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(&engine, &home, &script("needs-you-actions.json"), TOKEN);
    let mut kennel = Following::start(&home, false, NEVER);
    let reached = kennel.next(1);
    let raise = Call::RaiseLimit(
        serde_json::from_value::<UsageRaiseParams>(serde_json::json!({
            "scope": "daemon",
            "scope_id": "0.4",
            "usage_kind": "page_loads",
            "period": {"every": "day", "time_zone": "Europe/Paris"},
            "amount": 1000,
        }))
        .unwrap(),
    );
    let allow = Call::AllowConsentDomain(NeedsYouAllowConsentDomainParams {
        domain: "news.example".to_owned(),
    });
    let deny = Call::DenyWallDomain(NeedsYouDenyWallDomainParams {
        domain: "shop.example".to_owned(),
        reason: "Denied by a person from the Kennel's menu".to_owned(),
    });
    let remove = Call::RemoveRefetchDomain(NeedsYouRemoveRefetchDomainParams {
        domain: "shop.example".to_owned(),
        instead_of_star: None,
    });
    let calls = [raise, allow, deny, remove, dismiss()];

    for call in &calls {
        kennel.call(call.clone());
    }
    let answers: Vec<(Call, Result<(), CallFailure>)> =
        (0..calls.len()).map(|_| kennel.next_answer()).collect();

    assert_eq!(reached, [foxhound_0_4_1()]);
    let [raise, allow, deny, remove, dismiss] = calls;
    assert_eq!(
        answers,
        [
            (raise, Ok(())),
            (
                allow,
                Err(CallFailure {
                    code: "setting_locked".to_owned(),
                    message: "An Operator policy locks consent.domains: ask its administrator."
                        .to_owned(),
                })
            ),
            (deny, Ok(())),
            (remove, Ok(())),
            (dismiss, Ok(())),
        ]
    );
    drop(kennel);
    assert!(
        daemon.ended().success(),
        "the Daemon saw each call, with its params, in order"
    );
}

#[test]
fn a_call_made_while_no_daemon_is_reached_is_not_sent_and_fails() {
    let folder = tempfile::tempdir().unwrap();
    let mut kennel = Following::start(&Home::new(folder.path()), false, NEVER);
    let before = kennel.next(1);

    kennel.call(dismiss());

    assert_eq!(before, [ConnectionChange::NoDaemon]);
    let (call, answer) = kennel.next_answer();
    assert_eq!(call, dismiss());
    assert_eq!(answer.unwrap_err().code, NO_DAEMON);
}

#[test]
fn a_call_whose_connection_closes_before_its_answer_fails() {
    let Some(engine) = scripted_engine("a_call_whose_connection_closes_before_its_answer_fails")
    else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("call-unanswered-then-closed.json"),
        TOKEN,
    );
    let mut kennel = Following::start(&home, false, NEVER);
    let reached = kennel.next(1);

    kennel.call(dismiss());

    assert_eq!(reached, [foxhound_0_4_1()]);
    let (call, answer) = kennel.next_answer();
    assert_eq!(call, dismiss());
    assert_eq!(answer.unwrap_err().code, CONNECTION_LOST);
    assert!(daemon.ended().success());
}

#[test]
fn a_runs_controls_are_sent_to_their_operations_and_their_answers_reported() {
    let Some(engine) = scripted_engine("a_runs_controls_are_sent_to_their_operations") else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let daemon = ScriptedDaemon::start(
        &engine,
        &home,
        &script("calls-answered-and-refused.json"),
        TOKEN,
    );
    let mut kennel = Following::start(&home, false, NEVER);
    kennel.next(1);
    let pause = Call::Pause(RunParams {
        run_id: run_id("shop-crawl"),
    });
    let abort = Call::Abort(RunAbortParams {
        run_id: run_id("shop-crawl"),
        forwarded: false,
    });

    kennel.call(pause.clone());
    let paused = kennel.next_answer();
    kennel.call(abort.clone());
    let aborted = kennel.next_answer();

    assert_eq!(paused, (pause, Ok(())));
    assert_eq!(
        aborted,
        (
            abort,
            Err(CallFailure {
                code: "run_busy".to_owned(),
                message: "Another engine holds the Run's writer lock.".to_owned(),
            })
        )
    );
    assert_eq!(kennel.next(1), [ConnectionChange::NoDaemon]);
    assert!(daemon.ended().success(), "the Daemon saw both calls");
}
