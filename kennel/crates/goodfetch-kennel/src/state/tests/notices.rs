//! The desktop notices the State sends: which transitions send one, each
//! type's switch, the coalescing of a burst over an injected clock, and
//! no notice at the Kennel's start or for a resync's replayed state.

use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::RunProgress;
use rstest::rstest;
use serde_json::{Value, json};

use super::super::{Input, Notice, Notices, State};
use crate::client::{ConnectionChange, DaemonEvent, DaemonIdentity};
use crate::settings::{DesktopNotices, Settings, SettingsError};

fn foxhound() -> ConnectionChange {
    ConnectionChange::Connected(DaemonIdentity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("0.4.1").unwrap(),
    })
}

fn settings(notices: DesktopNotices) -> Input {
    Input::Settings(Ok(Settings {
        desktop_notices: notices,
        ..Settings::default()
    }))
}

/// A State past its start: its settings read with `notifications`, a
/// Foxhound Daemon reached, and the subscription's opening resync applied
/// with `runs` running and no Needs you item.
fn watching_with(notices: DesktopNotices, runs: &[&str]) -> State {
    let mut state = State::new();
    state.apply(settings(notices));
    state.apply(Input::Connection(foxhound()));
    state.apply(Input::Events(vec![resync(runs, &[])]));
    state
}

fn watching(runs: &[&str]) -> State {
    watching_with(DesktopNotices::default(), runs)
}

fn axes(run: &str, lifecycle: &str) -> DaemonEvent {
    DaemonEvent::RunAxes(
        serde_json::from_value(json!({"run_id": run, "lifecycle": lifecycle})).unwrap(),
    )
}

fn item(id: &str, summary: &str) -> Value {
    json!({
        "id": id,
        "kind": "storage_level",
        "since": "2026-11-03T10:00:00Z",
        "summary": summary,
        "actions": ["needs_you.dismiss"],
    })
}

fn listed(item: Value) -> DaemonEvent {
    let mut change = json!({"change": "listed"});
    change["item"] = item;
    DaemonEvent::NeedsYou(serde_json::from_value(change).unwrap())
}

fn resync(runs: &[&str], items: &[Value]) -> DaemonEvent {
    let runs: Vec<(&str, &str)> = runs.iter().map(|run| (*run, "running")).collect();
    resync_of(&runs, items)
}

/// A resync listing each Run with its Lifecycle.
fn resync_of(runs: &[(&str, &str)], items: &[Value]) -> DaemonEvent {
    let runs: Vec<Value> = runs
        .iter()
        .map(|(run, lifecycle)| json!({"run_id": run, "lifecycle": lifecycle}))
        .collect();
    DaemonEvent::Resync(serde_json::from_value(json!({"runs": runs, "needs_you": items})).unwrap())
}

fn progress(run: &str, parts: &Value) -> RunProgress {
    let mut value = json!({
        "run_id": run,
        "position": 7,
        "at": "2026-11-03T10:00:00Z",
        "lifecycle": "running",
        "writer": "live",
        "epoch": 1,
        "pauses": [],
        "last": false,
    });
    if let (Value::Object(fields), Value::Object(parts)) = (&mut value, parts.clone()) {
        fields.extend(parts);
    }
    serde_json::from_value(value).unwrap()
}

fn stopped_by_a_limit(run: &str) -> Input {
    Input::Progress(progress(
        run,
        &json!({
            "lifecycle": "stopped",
            "last": true,
            "stop_reason": {
                "kind": "limit",
                "scope": {"kind": "install", "id": ""},
                "usage_kind": "requests",
            },
        }),
    ))
}

fn limit_item(run: &str) -> Value {
    json!({
        "id": format!("run_stopped_by_limit:{run}:2026-11-03T10:00:00Z"),
        "kind": "run_stopped_by_limit",
        "since": "2026-11-03T10:00:00Z",
        "run": run,
        "summary": format!("The Run {run} stopped: a Limit on requests has nothing left."),
        "actions": ["needs_you.dismiss"],
        "details": {
            "scope": {"kind": "install", "id": ""},
            "usage_kind": "requests",
            "period": "day",
            "amount": 100,
        },
    })
}

fn notice(title: &str, body: &str) -> Notice {
    Notice {
        title: title.to_owned(),
        body: body.to_owned(),
    }
}

/// A fixed instant the tests count from: the State reads no clock.
fn origin() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test's origin for the instants it gives the State, which reads no clock itself"
    )]
    Instant::now()
}

fn minutes(origin: Instant, minutes: u64) -> Instant {
    origin + Duration::from_secs(minutes * 60)
}

fn sent(state: &mut State, now: Instant) -> Vec<Notice> {
    state.notices(now).send
}

/// The transitions, each from a State watching `crawl` run.
#[derive(Clone, Copy, Debug)]
enum Transition {
    NeedsYou,
    RunFailed,
    RunStoppedByLimit,
    DaemonLost,
    RunFinished,
}

impl Transition {
    fn inputs(self) -> Vec<Input> {
        match self {
            Self::NeedsYou => vec![Input::Events(vec![listed(item(
                "storage-1",
                "The disk is nearly full",
            ))])],
            Self::RunFailed => vec![Input::Events(vec![axes("crawl", "failed")])],
            Self::RunStoppedByLimit => vec![
                Input::Events(vec![axes("crawl", "stopped")]),
                stopped_by_a_limit("crawl"),
            ],
            Self::DaemonLost => vec![Input::Connection(ConnectionChange::NoDaemon)],
            Self::RunFinished => vec![Input::Events(vec![axes("crawl", "finished")])],
        }
    }

    fn switched(self, notices: &mut DesktopNotices, on: bool) {
        let switch = match self {
            Self::NeedsYou => &mut notices.needs_you,
            Self::RunFailed => &mut notices.run_failed,
            Self::RunStoppedByLimit => &mut notices.run_stopped_by_limit,
            Self::DaemonLost => &mut notices.daemon_lost,
            Self::RunFinished => &mut notices.run_finished,
        };
        *switch = on;
    }

    fn notice(self) -> Notice {
        match self {
            Self::NeedsYou => notice("Needs you", "The disk is nearly full"),
            Self::RunFailed => notice("Run failed", "crawl"),
            Self::RunStoppedByLimit => notice("Run stopped by a Limit", "crawl"),
            Self::DaemonLost => notice(
                "Daemon lost",
                "The Foxhound Daemon is gone, and 1 Run was live on it",
            ),
            Self::RunFinished => notice("Run finished", "crawl"),
        }
    }
}

#[rstest]
#[case::a_new_needs_you_item(Transition::NeedsYou, true)]
#[case::a_run_failed(Transition::RunFailed, true)]
#[case::a_run_stopped_by_a_limit(Transition::RunStoppedByLimit, true)]
#[case::the_daemon_lost_while_a_run_was_live(Transition::DaemonLost, true)]
#[case::a_run_that_ended_well(Transition::RunFinished, false)]
fn each_transition_sends_its_notice_when_its_type_is_on_by_default(
    #[case] transition: Transition,
    #[case] on_by_default: bool,
) {
    let mut state = watching(&["crawl"]);
    let now = origin();

    for input in transition.inputs() {
        state.apply(input);
    }

    let expected = if on_by_default {
        vec![transition.notice()]
    } else {
        Vec::new()
    };
    assert_eq!(sent(&mut state, now), expected);
}

#[rstest]
#[case::a_new_needs_you_item(Transition::NeedsYou)]
#[case::a_run_failed(Transition::RunFailed)]
#[case::a_run_stopped_by_a_limit(Transition::RunStoppedByLimit)]
#[case::the_daemon_lost_while_a_run_was_live(Transition::DaemonLost)]
#[case::a_run_that_ended_well(Transition::RunFinished)]
fn each_notice_type_has_a_switch_of_its_own(#[case] transition: Transition) {
    let mut on = DesktopNotices::default();
    transition.switched(&mut on, true);
    let mut off = DesktopNotices::default();
    transition.switched(&mut off, false);
    let mut switched_on = watching_with(on, &["crawl"]);
    let mut switched_off = watching_with(off, &["crawl"]);
    let now = origin();

    for input in transition.inputs() {
        switched_on.apply(input.clone());
        switched_off.apply(input);
    }

    assert_eq!(sent(&mut switched_on, now), [transition.notice()]);
    assert_eq!(sent(&mut switched_off, now), []);
}

#[test]
fn with_notifications_off_no_notice_is_sent_and_no_notice_code_is_wanted() {
    let mut state = watching_with(
        DesktopNotices {
            enabled: false,
            ..DesktopNotices::default()
        },
        &["crawl"],
    );

    state.apply(Input::Events(vec![
        listed(item("storage-1", "The disk is nearly full")),
        axes("crawl", "failed"),
    ]));
    state.apply(Input::Connection(ConnectionChange::NoDaemon));

    assert!(!state.notifying());
    assert_eq!(
        state.notices(origin()),
        Notices {
            send: Vec::new(),
            due: None
        }
    );
}

#[test]
fn notices_are_wanted_only_once_the_settings_are_read_and_say_so() {
    let mut state = State::new();
    let at_launch = state.notifying();

    state.apply(Input::Settings(Err(SettingsError::Unreadable(
        "unknown field `contrls`".to_owned(),
    ))));
    let with_the_defaults = state.notifying();
    state.apply(settings(DesktopNotices {
        enabled: false,
        ..DesktopNotices::default()
    }));
    let turned_off = state.notifying();

    assert!(!at_launch);
    assert!(with_the_defaults);
    assert!(!turned_off);
}

#[test]
fn no_notice_is_sent_before_the_settings_are_read() {
    let mut state = State::new();
    state.apply(Input::Connection(foxhound()));
    state.apply(Input::Events(vec![resync(&["crawl"], &[])]));

    state.apply(Input::Events(vec![axes("crawl", "failed")]));
    state.apply(Input::Settings(Ok(Settings::default())));

    assert_eq!(sent(&mut state, origin()), []);
}

#[test]
fn nothing_fires_for_the_state_the_kennel_finds_at_its_start() {
    let mut state = State::new();
    state.apply(Input::Settings(Ok(Settings::default())));
    state.apply(Input::Connection(ConnectionChange::NoDaemon));
    state.apply(Input::Connection(foxhound()));

    state.apply(Input::Events(vec![resync(
        &["crawl"],
        &[item("storage-1", "The disk is nearly full")],
    )]));
    state.apply(Input::Events(vec![axes("old-crawl", "failed")]));

    assert_eq!(
        sent(&mut state, origin()),
        [notice("Run failed", "old-crawl")],
        "only the change after the resync fires"
    );
}

#[test]
fn nothing_fires_for_a_resyncs_replayed_state() {
    let mut state = watching(&["crawl"]);

    // A full queue: the resync lists an item and a Run the Kennel had not
    // seen, and a reconnection replays them again.
    state.apply(Input::Events(vec![resync(
        &["crawl", "other-crawl"],
        &[item("storage-1", "The disk is nearly full")],
    )]));
    state.apply(Input::Connection(ConnectionChange::NoDaemon));
    state.apply(Input::Connection(foxhound()));
    state.apply(Input::Events(vec![resync(
        &["crawl"],
        &[
            item("storage-1", "The disk is nearly full"),
            item("storage-2", "The disk is full"),
        ],
    )]));

    assert_eq!(
        sent(&mut state, origin()),
        [notice(
            "Daemon lost",
            "The Foxhound Daemon is gone, and 2 Runs were live on it"
        )],
        "only the lost Daemon fires, not what the resyncs list"
    );
}

#[test]
fn a_run_a_resync_lists_as_ended_sends_nothing_even_once_its_stop_reason_is_read() {
    let mut state = watching(&[]);

    state.apply(Input::Events(vec![resync_of(
        &[("broken-crawl", "failed"), ("capped-crawl", "stopped")],
        &[],
    )]));
    state.apply(stopped_by_a_limit("capped-crawl"));

    assert_eq!(sent(&mut state, origin()), []);
}

#[test]
fn an_item_listed_again_sends_nothing() {
    let mut state = watching(&[]);
    state.apply(Input::Events(vec![resync(
        &[],
        &[item("storage-1", "The disk is nearly full")],
    )]));

    state.apply(Input::Events(vec![listed(item(
        "storage-1",
        "The disk is nearly full",
    ))]));

    assert_eq!(sent(&mut state, origin()), []);
}

#[test]
fn a_run_that_ended_while_the_queue_was_full_is_a_real_change() {
    let mut state = watching(&["crawl"]);

    state.apply(Input::Events(vec![resync(&[], &[])]));
    state.apply(Input::Progress(progress(
        "crawl",
        &json!({"lifecycle": "failed", "last": true}),
    )));

    assert_eq!(sent(&mut state, origin()), [notice("Run failed", "crawl")]);
}

#[test]
fn a_daemon_lost_with_no_live_run_sends_nothing() {
    let mut state = watching(&[]);

    state.apply(Input::Connection(ConnectionChange::NoDaemon));

    assert_eq!(sent(&mut state, origin()), []);
}

#[test]
fn a_run_stopped_by_a_limit_sends_one_notice_with_its_needs_you_item() {
    let mut item_first = watching(&["crawl"]);
    let mut run_first = watching(&["crawl"]);
    let now = origin();

    item_first.apply(Input::Events(vec![
        axes("crawl", "stopped"),
        listed(limit_item("crawl")),
    ]));
    item_first.apply(stopped_by_a_limit("crawl"));
    run_first.apply(Input::Events(vec![axes("crawl", "stopped")]));
    run_first.apply(stopped_by_a_limit("crawl"));
    run_first.apply(Input::Events(vec![listed(limit_item("crawl"))]));

    let item = notice(
        "Needs you",
        "The Run crawl stopped: a Limit on requests has nothing left.",
    );
    assert_eq!(item_first.notices(now).send, [item]);
    assert_eq!(item_first.notices(minutes(now, 30)).send, []);
    assert_eq!(
        run_first.notices(now).send,
        [notice("Run stopped by a Limit", "crawl")]
    );
    assert_eq!(run_first.notices(minutes(now, 30)).send, []);
}

#[test]
fn a_burst_sends_the_first_notice_at_once_then_one_summary_per_period() {
    let mut state = watching(&["crawl-1", "crawl-2", "crawl-3"]);
    let start = origin();

    state.apply(Input::Events(vec![axes("crawl-1", "failed")]));
    let first = state.notices(start);
    state.apply(Input::Events(vec![
        axes("crawl-2", "failed"),
        listed(item("storage-1", "The disk is nearly full")),
    ]));
    let during = state.notices(minutes(start, 1));
    state.apply(Input::Events(vec![axes("crawl-3", "failed")]));
    let later = state.notices(minutes(start, 19));
    let summary = state.notices(minutes(start, 20));
    let after = state.notices(minutes(start, 21));

    assert_eq!(
        first,
        Notices {
            send: vec![notice("Run failed", "crawl-1")],
            due: None,
        }
    );
    assert_eq!(
        during,
        Notices {
            send: Vec::new(),
            due: Some(minutes(start, 20)),
        }
    );
    assert_eq!(
        later,
        Notices {
            send: Vec::new(),
            due: Some(minutes(start, 20)),
        }
    );
    assert_eq!(
        summary,
        Notices {
            send: vec![notice(
                "3 more changes",
                "1 new Needs you item; 2 Runs failed"
            )],
            due: None,
        }
    );
    assert_eq!(after.send, []);
}

#[test]
fn a_summary_starts_a_new_period_and_a_quiet_period_ends_the_burst() {
    let mut state = watching(&[]);
    let start = origin();
    let at = |state: &mut State, minute: u64, id: &str| {
        state.apply(Input::Events(vec![listed(item(id, id))]));
        state.notices(minutes(start, minute)).send
    };

    let first = at(&mut state, 0, "one");
    let held = at(&mut state, 5, "two");
    let summary = state.notices(minutes(start, 20)).send;
    let held_again = at(&mut state, 30, "three");
    let lone_summary = state.notices(minutes(start, 40)).send;
    let quiet = state.notices(minutes(start, 60)).send;
    let at_once = at(&mut state, 61, "four");

    assert_eq!(first, [notice("Needs you", "one")]);
    assert_eq!(held, []);
    assert_eq!(
        summary,
        [notice("Needs you", "two")],
        "one held notice shows as itself"
    );
    assert_eq!(held_again, []);
    assert_eq!(lone_summary, [notice("Needs you", "three")]);
    assert_eq!(quiet, []);
    assert_eq!(at_once, [notice("Needs you", "four")]);
}

#[test]
fn the_period_is_a_setting_and_zero_sends_each_notice_at_once() {
    let mut five = watching_with(
        DesktopNotices {
            period_minutes: 5,
            ..DesktopNotices::default()
        },
        &[],
    );
    let mut zero = watching_with(
        DesktopNotices {
            period_minutes: 0,
            ..DesktopNotices::default()
        },
        &[],
    );
    let start = origin();
    let burst = Input::Events(vec![listed(item("one", "one")), listed(item("two", "two"))]);

    five.apply(burst.clone());
    zero.apply(burst);

    assert_eq!(
        five.notices(start),
        Notices {
            send: vec![notice("Needs you", "one")],
            due: Some(minutes(start, 5)),
        }
    );
    assert_eq!(
        zero.notices(start),
        Notices {
            send: vec![notice("Needs you", "one"), notice("Needs you", "two")],
            due: None,
        }
    );
}

#[test]
fn turning_a_type_off_drops_its_held_notices() {
    let mut state = watching(&["crawl"]);
    let start = origin();
    state.apply(Input::Events(vec![
        listed(item("one", "one")),
        listed(item("two", "two")),
        axes("crawl", "failed"),
    ]));
    state.notices(start);

    state.apply(settings(DesktopNotices {
        needs_you: false,
        ..DesktopNotices::default()
    }));

    assert_eq!(
        state.notices(minutes(start, 20)).send,
        [notice("Run failed", "crawl")]
    );
}

#[test]
fn turning_notifications_off_drops_every_held_notice() {
    let mut state = watching(&[]);
    let start = origin();
    state.apply(Input::Events(vec![
        listed(item("one", "one")),
        listed(item("two", "two")),
    ]));
    state.notices(start);

    state.apply(settings(DesktopNotices {
        enabled: false,
        ..DesktopNotices::default()
    }));
    state.apply(settings(DesktopNotices::default()));

    assert_eq!(state.notices(minutes(start, 20)).send, []);
}

#[test]
fn a_notices_text_goes_through_the_text_function() {
    let mut state = watching(&[]);
    let long = "x".repeat(400);

    state.apply(Input::Events(vec![listed(item(
        "spoof",
        &format!("Pay\u{202e}here https://user:hunter2@example.com/ {long}"),
    ))]));
    let sent = sent(&mut state, origin());

    assert_eq!(sent.len(), 1);
    let body = &sent[0].body;
    assert!(
        body.starts_with("Payhere https://user:••••@example.com/ xxx"),
        "{body}"
    );
    assert!(body.ends_with('…'), "{body}");
    assert!(body.chars().count() <= 200, "{body}");
}

#[test]
fn a_run_is_named_by_its_recipe_in_its_notice() {
    let mut state = watching(&["crawl"]);
    state.apply(Input::LiveRuns(vec![
        serde_json::from_value(json!({
            "run_id": "crawl",
            "location": "runs/crawl",
            "kind": "crawl",
            "needs": "foxhound",
            "tags": ["goodfetch.recipe:shop"],
            "lifecycle": "running",
            "started_at": "2026-11-03T09:00:00Z",
            "presence": "present",
            "replayed": false,
        }))
        .unwrap(),
    ]));

    state.apply(Input::Events(vec![axes("crawl", "failed")]));

    assert_eq!(sent(&mut state, origin()), [notice("Run failed", "shop")]);
}

#[test]
fn a_run_paused_or_stopped_for_another_reason_sends_nothing() {
    let mut state = watching(&["paused-crawl", "aborted-crawl"]);

    state.apply(Input::Events(vec![
        axes("paused-crawl", "stopped"),
        axes("aborted-crawl", "stopped"),
    ]));
    state.apply(Input::Progress(progress(
        "paused-crawl",
        &json!({"lifecycle": "stopped", "last": true, "stop_reason": {"kind": "pause"}}),
    )));
    state.apply(Input::Progress(progress(
        "aborted-crawl",
        &json!({"lifecycle": "stopped", "last": true, "stop_reason": {"kind": "abort", "by": "client"}}),
    )));

    assert_eq!(sent(&mut state, origin()), []);
}
