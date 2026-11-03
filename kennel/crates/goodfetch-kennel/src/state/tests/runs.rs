//! The live Runs in the menu, the verdict, the reads the view wants and
//! the redraws' pace.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{RunId, RunIndexRow, RunProgress};
use rstest::rstest;
use serde_json::{Value, json};

use super::super::{Frame, Input, MenuChange, Redraw, Row, State, Verdict};
use crate::client::{ConnectionChange, DaemonEvent, DaemonIdentity};

/// A State that reached a Daemon of `product`.
pub(super) fn reached(product: Product) -> State {
    let mut state = State::new();
    state.apply(Input::Connection(ConnectionChange::Connected(
        DaemonIdentity {
            product,
            product_version: ProductVersion::parse("0.4.1").unwrap(),
        },
    )));
    state
}

pub(super) fn axes(run: &str, lifecycle: &str) -> DaemonEvent {
    DaemonEvent::RunAxes(
        serde_json::from_value(json!({"run_id": run, "lifecycle": lifecycle})).unwrap(),
    )
}

fn needs_you(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "storage_level",
        "since": "2026-11-03T10:00:00Z",
        "summary": "The disk is nearly full",
        "actions": ["needs_you.dismiss"],
    })
}

fn listed(id: &str) -> DaemonEvent {
    DaemonEvent::NeedsYou(
        serde_json::from_value(json!({"change": "listed", "item": needs_you(id)})).unwrap(),
    )
}

fn removed(id: &str) -> DaemonEvent {
    DaemonEvent::NeedsYou(
        serde_json::from_value(json!({"change": "removed", "id": id, "reason": "resolved"}))
            .unwrap(),
    )
}

fn resync(runs: &[&str], items: &[&str]) -> DaemonEvent {
    let runs: Vec<Value> = runs
        .iter()
        .map(|run| json!({"run_id": run, "lifecycle": "running"}))
        .collect();
    let items: Vec<Value> = items.iter().map(|id| needs_you(id)).collect();
    DaemonEvent::Resync(serde_json::from_value(json!({"runs": runs, "needs_you": items})).unwrap())
}

/// The progress of `run`, with `parts` added to the fields every Run's
/// progress holds.
pub(super) fn progress(run: &str, parts: Value) -> RunProgress {
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
    if let (Value::Object(fields), Value::Object(parts)) = (&mut value, parts) {
        fields.extend(parts);
    }
    serde_json::from_value(value).unwrap()
}

fn row(run: &str, tags: &[&str], message: Option<&str>) -> RunIndexRow {
    serde_json::from_value(json!({
        "run_id": run,
        "location": format!("runs/{run}"),
        "kind": "crawl",
        "needs": "foxhound",
        "tags": tags,
        "lifecycle": "running",
        "started_at": "2026-11-03T09:00:00Z",
        "presence": "present",
        "progress_message": message,
        "replayed": false,
    }))
    .unwrap()
}

pub(super) fn run_id(run: &str) -> RunId {
    RunId::parse(run).unwrap()
}

fn runs(ids: &[&str]) -> BTreeSet<RunId> {
    ids.iter().map(|id| run_id(id)).collect()
}

/// The menu as a person reads it: a detail indented, a separator as
/// `---`.
pub(super) fn lines(state: &State) -> Vec<String> {
    state
        .view()
        .rows
        .into_iter()
        .map(|row| match row {
            Row::Text(line) => line,
            Row::Detail(line) => format!("  {line}"),
            Row::Choice { label, .. } => label,
            Row::Separator => "---".to_owned(),
            Row::Submenu { label, .. } => format!("{label} >"),
        })
        .collect()
}

/// A fixed instant the tests count from: the State reads no clock.
pub(super) fn origin() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test's origin for the instants it gives the State, which reads no clock itself"
    )]
    Instant::now()
}

fn after(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

#[rstest]
#[case::idle(true, false, false, false, Verdict::Idle)]
#[case::running(true, false, false, true, Verdict::Running)]
#[case::failed(true, false, true, false, Verdict::Failed)]
#[case::failed_and_running(true, false, true, true, Verdict::Failed)]
#[case::needs_you(true, true, false, false, Verdict::NeedsYou)]
#[case::needs_you_and_running(true, true, false, true, Verdict::NeedsYou)]
#[case::needs_you_and_failed(true, true, true, false, Verdict::NeedsYou)]
#[case::needs_you_failed_and_running(true, true, true, true, Verdict::NeedsYou)]
#[case::unreachable(false, false, false, false, Verdict::Unreachable)]
#[case::unreachable_after_running(false, false, false, true, Verdict::Unreachable)]
#[case::unreachable_after_failed(false, false, true, false, Verdict::Unreachable)]
#[case::unreachable_after_failed_and_running(false, false, true, true, Verdict::Unreachable)]
#[case::unreachable_after_needs_you(false, true, false, false, Verdict::Unreachable)]
#[case::unreachable_after_needs_you_and_running(false, true, false, true, Verdict::Unreachable)]
#[case::unreachable_after_needs_you_and_failed(false, true, true, false, Verdict::Unreachable)]
#[case::unreachable_after_everything(false, true, true, true, Verdict::Unreachable)]
fn the_icon_shows_the_worst_verdict_first(
    #[case] reachable: bool,
    #[case] needs: bool,
    #[case] failed: bool,
    #[case] running: bool,
    #[case] expected: Verdict,
) {
    let mut state = reached(Product::Foxhound);
    let mut events = Vec::new();
    if needs {
        events.push(listed("storage-1"));
    }
    if failed {
        events.push(axes("broken-crawl", "failed"));
    }
    if running {
        events.push(axes("live-crawl", "running"));
    }
    state.apply(Input::Events(events));

    if !reachable {
        state.apply(Input::Connection(ConnectionChange::NoDaemon));
    }

    assert_eq!(state.view().verdict, expected);
}

#[test]
fn a_run_stopped_by_a_limit_counts_as_failed_once_its_stop_reason_is_read() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![
        axes("capped-crawl", "running"),
        axes("capped-crawl", "stopped"),
    ]));
    let wanted_while_stopping = state.wanted().progress;

    state.apply(Input::Progress(progress(
        "capped-crawl",
        json!({
            "lifecycle": "stopped",
            "last": true,
            "stop_reason": {
                "kind": "limit",
                "scope": {"kind": "install", "id": ""},
                "usage_kind": "requests",
            },
        }),
    )));

    assert_eq!(wanted_while_stopping, runs(&["capped-crawl"]));
    assert_eq!(state.view().verdict, Verdict::Failed);
    assert_eq!(
        lines(&state),
        [
            "Foxhound Daemon 0.4.1",
            "---",
            "capped-crawl — Stopped by a Limit >",
            "---",
            "Quit"
        ]
    );
    assert_eq!(state.wanted().progress, runs(&[]));
}

#[test]
fn a_run_stopped_for_another_reason_leaves_the_menu() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![
        axes("stopped-crawl", "running"),
        axes("stopped-crawl", "stopped"),
    ]));

    state.apply(Input::Progress(progress(
        "stopped-crawl",
        json!({"lifecycle": "stopped", "last": true, "stop_reason": {"kind": "abort", "by": "client"}}),
    )));

    assert_eq!(state.view().verdict, Verdict::Idle);
    assert_eq!(lines(&state), ["Foxhound Daemon 0.4.1", "---", "Quit"]);
}

#[test]
fn a_finished_run_leaves_the_menu() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![axes("done-crawl", "running")]));

    state.apply(Input::Events(vec![axes("done-crawl", "finished")]));

    assert_eq!(state.view().verdict, Verdict::Idle);
    assert_eq!(lines(&state), ["Foxhound Daemon 0.4.1", "---", "Quit"]);
}

#[test]
fn the_menu_lists_each_live_run_with_its_recipe_or_run_id_lifecycle_and_progress_message() {
    let mut state = reached(Product::Collie);
    state.apply(Input::Events(vec![
        axes("a-script-run", "running"),
        axes("nightly-1", "running"),
    ]));
    state.apply(Input::LiveRuns(vec![
        row(
            "nightly-1",
            &["team:prices", "goodfetch.recipe:nightly-prices"],
            Some("Signing in"),
        ),
        row("a-script-run", &[], None),
    ]));

    state.apply(Input::Progress(progress(
        "nightly-1",
        json!({"message": "Fetching page 3 of 40"}),
    )));

    assert_eq!(state.view().verdict, Verdict::Running);
    assert_eq!(
        lines(&state),
        [
            "Collie Daemon 0.4.1",
            "---",
            "a-script-run — Running >",
            "nightly-prices — Running >",
            "  Fetching page 3 of 40",
            "---",
            "Quit"
        ]
    );
}

#[test]
fn a_runs_row_shows_the_progress_message_of_the_run_index_until_its_progress_is_read() {
    let mut state = reached(Product::Collie);
    state.apply(Input::Events(vec![axes("nightly-1", "running")]));

    state.apply(Input::LiveRuns(vec![row(
        "nightly-1",
        &["goodfetch.recipe:nightly-prices"],
        Some("Signing in"),
    )]));

    assert_eq!(
        lines(&state),
        [
            "Collie Daemon 0.4.1",
            "---",
            "nightly-prices — Running >",
            "  Signing in",
            "---",
            "Quit"
        ]
    );
}

fn crawl_progress() -> RunProgress {
    progress(
        "shop-crawl",
        json!({
            "message": "Crawling shop.example",
            "requests": {
                "pending": 45, "fetching": 2, "handling": 1, "in_human_pause": 0,
                "succeeded": 120, "failed": 3, "skipped": 4, "blocked": 0,
            },
        }),
    )
}

#[test]
fn a_crawls_succeeded_failed_and_pending_counts_show_under_its_row() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![axes("shop-crawl", "running")]));

    state.apply(Input::Progress(crawl_progress()));

    assert_eq!(
        lines(&state),
        [
            "Foxhound Daemon 0.4.1",
            "---",
            "shop-crawl — Running >",
            "  Crawling shop.example",
            "  120 succeeded, 3 failed, 45 pending",
            "---",
            "Quit"
        ]
    );
}

#[test]
fn foxhounds_views_are_hidden_on_a_collie_daemon() {
    let mut state = reached(Product::Collie);
    state.apply(Input::Events(vec![axes("shop-crawl", "running")]));

    state.apply(Input::Progress(crawl_progress()));

    assert_eq!(
        lines(&state),
        [
            "Collie Daemon 0.4.1",
            "---",
            "shop-crawl — Running >",
            "  Crawling shop.example",
            "---",
            "Quit"
        ]
    );
}

#[test]
fn runs_that_need_a_look_are_listed_before_the_running_ones() {
    let mut state = reached(Product::Foxhound);

    state.apply(Input::Events(vec![
        axes("a-running", "running"),
        axes("z-broken", "failed"),
    ]));

    assert_eq!(
        lines(&state),
        [
            "Foxhound Daemon 0.4.1",
            "---",
            "z-broken — Failed",
            "a-running — Running >",
            "---",
            "Quit"
        ]
    );
}

#[test]
fn the_menu_lists_at_most_ten_runs_and_counts_the_others() {
    let mut state = reached(Product::Foxhound);
    let ids = [
        "run-01", "run-02", "run-03", "run-04", "run-05", "run-06", "run-07", "run-08", "run-09",
        "run-10", "run-11", "run-12",
    ];

    state.apply(Input::Events(
        ids.iter().map(|id| axes(id, "running")).collect(),
    ));

    assert_eq!(
        lines(&state),
        [
            "Foxhound Daemon 0.4.1",
            "---",
            "run-01 — Running >",
            "run-02 — Running >",
            "run-03 — Running >",
            "run-04 — Running >",
            "run-05 — Running >",
            "run-06 — Running >",
            "run-07 — Running >",
            "run-08 — Running >",
            "run-09 — Running >",
            "run-10 — Running >",
            "2 more Runs",
            "---",
            "Quit"
        ]
    );
}

#[test]
fn text_from_a_run_is_made_safe_before_the_menu_shows_it() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![axes("shop-crawl", "running")]));
    state.apply(Input::LiveRuns(vec![row(
        "shop-crawl",
        &["goodfetch.recipe:shop\u{202e}gpj.exe"],
        None,
    )]));

    state.apply(Input::Progress(progress(
        "shop-crawl",
        json!({"message": "GET\nhttps://shop.example/?token=abc123 \u{1b}[2J"}),
    )));

    assert_eq!(
        lines(&state)[2..4],
        [
            "shopgpj.exe — Running >",
            "  GET https://shop.example/?token=•••• [2J"
        ]
    );
}

#[test]
fn a_failed_run_stays_listed_until_the_menu_that_showed_it_closes() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![axes("broken-crawl", "failed")]));
    state.apply(Input::Menu(MenuChange::Opened(origin())));
    let while_open = state.view().verdict;

    state.apply(Input::Menu(MenuChange::Closed));

    assert_eq!(while_open, Verdict::Failed);
    assert_eq!(state.view().verdict, Verdict::Idle);
    assert_eq!(lines(&state), ["Foxhound Daemon 0.4.1", "---", "Quit"]);
}

#[test]
fn a_run_that_fails_while_the_menu_is_hidden_stays_until_a_menu_shows_it() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Menu(MenuChange::Opened(origin())));
    state.apply(Input::Menu(MenuChange::Closed));

    state.apply(Input::Events(vec![axes("broken-crawl", "failed")]));

    assert_eq!(state.view().verdict, Verdict::Failed);
}

#[test]
fn a_needs_you_item_shows_in_the_verdict_until_it_is_removed() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![listed("storage-1")]));
    let listed_verdict = state.view().verdict;

    state.apply(Input::Events(vec![removed("storage-1")]));

    assert_eq!(listed_verdict, Verdict::NeedsYou);
    assert_eq!(state.view().verdict, Verdict::Idle);
}

#[test]
fn a_resync_replaces_the_live_runs_and_the_needs_you_items() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![
        axes("old-crawl", "running"),
        listed("storage-1"),
    ]));

    state.apply(Input::Events(vec![resync(&["new-crawl"], &[])]));

    assert_eq!(state.view().verdict, Verdict::Running);
    assert_eq!(
        lines(&state),
        [
            "Foxhound Daemon 0.4.1",
            "---",
            "new-crawl — Running >",
            "---",
            "Quit"
        ]
    );
    // The Run the resync left out ended: its ending is read.
    assert_eq!(state.wanted().progress, runs(&["old-crawl"]));
}

#[test]
fn a_resync_and_a_run_with_no_row_each_ask_for_the_live_runs_rows() {
    let mut state = reached(Product::Foxhound);
    let once_reached = state.wanted().live_runs;
    state.apply(Input::Events(vec![resync(&[], &[])]));
    let after_resync = state.wanted().live_runs;
    state.apply(Input::LiveRuns(vec![row("known-crawl", &[], None)]));
    state.apply(Input::Events(vec![axes("known-crawl", "running")]));
    let after_a_known_run = state.wanted().live_runs;

    state.apply(Input::Events(vec![axes("new-crawl", "running")]));

    assert_eq!(
        [
            once_reached,
            after_resync,
            after_a_known_run,
            state.wanted().live_runs
        ],
        [0, 1, 1, 2]
    );
}

#[test]
fn only_an_open_menu_follows_the_progress_of_the_runs_it_shows() {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![
        axes("crawl-a", "running"),
        axes("crawl-b", "running"),
        axes("broken-crawl", "failed"),
    ]));
    let while_hidden = state.wanted().progress;
    state.apply(Input::Menu(MenuChange::Opened(origin())));
    let while_open = state.wanted().progress;

    state.apply(Input::Menu(MenuChange::Closed));

    assert_eq!(while_hidden, runs(&[]));
    assert_eq!(while_open, runs(&["crawl-a", "crawl-b"]));
    assert_eq!(state.wanted().progress, runs(&[]));
}

#[test]
fn an_open_menu_follows_only_the_runs_it_lists() {
    let mut state = reached(Product::Foxhound);
    let ids = [
        "run-01", "run-02", "run-03", "run-04", "run-05", "run-06", "run-07", "run-08", "run-09",
        "run-10", "run-11",
    ];
    state.apply(Input::Events(
        ids.iter().map(|id| axes(id, "running")).collect(),
    ));

    state.apply(Input::Menu(MenuChange::Opened(origin())));

    assert!(!state.wanted().progress.contains(&run_id("run-11")));
    assert_eq!(state.wanted().progress.len(), 10);
}

/// A State whose menu was drawn at `at`, open or hidden, with one live
/// crawl.
fn drawn(at: Instant, open: bool) -> State {
    let mut state = reached(Product::Foxhound);
    state.apply(Input::Events(vec![axes("shop-crawl", "running")]));
    if open {
        state.apply(Input::Menu(MenuChange::Opened(at)));
    } else {
        state.redraw(at);
    }
    state
}

fn message(text: &str) -> Input {
    Input::Progress(progress("shop-crawl", json!({ "message": text })))
}

#[test]
fn an_open_menu_is_redrawn_at_most_once_a_second() {
    let start = origin();
    let mut state = drawn(start, true);
    state.apply(message("page 1"));
    let soon = state.redraw(after(start, 200));
    let at_the_second = state.redraw(after(start, 1_000));
    state.apply(message("page 2"));
    let just_after = state.redraw(after(start, 1_300));

    let a_second_later = state.redraw(after(start, 2_000));

    assert_eq!(soon, Redraw::At(after(start, 1_000)));
    assert!(
        matches!(
            &at_the_second,
            Redraw::Now(Frame {
                verdict: None,
                rows: Some(_)
            })
        ),
        "{at_the_second:?}"
    );
    assert_eq!(just_after, Redraw::At(after(start, 2_000)));
    assert!(
        matches!(
            &a_second_later,
            Redraw::Now(Frame {
                verdict: None,
                rows: Some(_)
            })
        ),
        "{a_second_later:?}"
    );
}

#[test]
fn a_view_unchanged_since_its_last_draw_is_not_redrawn() {
    let start = origin();
    let mut state = drawn(start, true);

    assert_eq!(state.redraw(after(start, 5_000)), Redraw::Nothing);
}

#[test]
fn a_hidden_menu_is_never_redrawn() {
    let start = origin();
    let mut state = drawn(start, false);

    state.apply(Input::LiveRuns(vec![row(
        "shop-crawl",
        &["goodfetch.recipe:shop"],
        Some("Crawling"),
    )]));

    assert_eq!(state.redraw(after(start, 5_000)), Redraw::Nothing);
}

#[test]
fn the_icon_is_redrawn_while_the_menu_is_hidden_at_most_once_a_second() {
    let start = origin();
    let mut state = drawn(start, false);
    state.apply(Input::Events(vec![axes("shop-crawl", "failed")]));
    let soon = state.redraw(after(start, 300));

    let at_the_second = state.redraw(after(start, 1_000));

    assert_eq!(soon, Redraw::At(after(start, 1_000)));
    assert_eq!(
        at_the_second,
        Redraw::Now(Frame {
            verdict: Some(Verdict::Failed),
            rows: None
        })
    );
}

#[test]
fn the_first_draw_draws_the_icon_and_the_menu() {
    let mut state = reached(Product::Foxhound);

    let first = state.redraw(origin());

    assert_eq!(
        first,
        Redraw::Now(Frame {
            verdict: Some(Verdict::Idle),
            rows: Some(state.view().rows)
        })
    );
}
