//! The details window's model through its interface: the Client's reports
//! and the person's inputs in; the view, the repaints and the reads wanted
//! out, with no toolkit and no socket.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{
    Lifecycle, RunId, RunIndexRow, RunListParams, RunProgress, RunReport, RunUsage,
};
use serde_json::{Value, json};

use super::{Details, Field, Filter, Input, Repaint, RunLine};
use crate::client::{ConnectionChange, DaemonEvent, DaemonIdentity, Listing, UsageRead, Wanted};

fn connected(product: Product) -> Input {
    Input::Connection(ConnectionChange::Connected(DaemonIdentity {
        product,
        product_version: ProductVersion::parse("0.4.1").unwrap(),
    }))
}

fn resync(runs: &[&str]) -> Input {
    let runs: Vec<Value> = runs
        .iter()
        .map(|run| json!({"run_id": run, "lifecycle": "running"}))
        .collect();
    Input::Events(vec![DaemonEvent::Resync(
        serde_json::from_value(json!({"runs": runs, "needs_you": []})).unwrap(),
    )])
}

fn axes(run: &str, lifecycle: &str) -> Input {
    Input::Events(vec![DaemonEvent::RunAxes(
        serde_json::from_value(json!({"run_id": run, "lifecycle": lifecycle})).unwrap(),
    )])
}

fn row(run: &str, lifecycle: &str, tags: &[&str]) -> RunIndexRow {
    let ended = lifecycle != "running";
    serde_json::from_value(json!({
        "run_id": run,
        "location": format!("runs/{run}"),
        "kind": "crawl",
        "needs": "foxhound",
        "tags": tags,
        "lifecycle": lifecycle,
        "verdict": if ended { Some("verified") } else { None },
        "completeness": if ended { Some("incomplete") } else { None },
        "started_at": "2026-11-03T09:00:00Z",
        "ended_at": if ended { Some("2026-11-03T09:30:00Z") } else { None },
        "presence": "present",
        "progress_message": null,
        "replayed": false,
    }))
    .unwrap()
}

/// The progress of `run`, with `parts` added to the fields every Run's
/// progress holds.
fn progress(run: &str, position: u64, parts: Value) -> RunProgress {
    let mut value = json!({
        "run_id": run,
        "position": position,
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

fn usage(requests: u64) -> RunUsage {
    serde_json::from_value(json!({
        "model": {"calls": 3, "tokens": 1200},
        "browser_time": 65_000,
        "proxy_bytes": 2048,
        "requests": requests,
    }))
    .unwrap()
}

fn report(run: &str, parts: Value) -> RunReport {
    let mut value = json!({
        "run_id": run,
        "lifecycle": "failed",
        "verdict": "rejected",
        "verdict_reasons": [{"code": "failed", "message": "The Run failed"}],
        "completeness": "incomplete",
        "started_at": "2026-11-03T09:00:00Z",
        "ended_at": "2026-11-03T09:30:00Z",
        "usage": usage(40),
        "error": {"code": "navigation_failed", "message": "The page did not load"},
    });
    if let (Value::Object(fields), Value::Object(parts)) = (&mut value, parts) {
        fields.extend(parts);
    }
    serde_json::from_value(value).unwrap()
}

fn run_id(run: &str) -> RunId {
    RunId::parse(run).unwrap()
}

fn runs(ids: &[&str]) -> BTreeSet<RunId> {
    ids.iter().map(|id| run_id(id)).collect()
}

/// The listing the window wants now.
fn listing(details: &Details) -> Listing {
    details
        .wanted()
        .listing
        .expect("an open window that reached a Daemon lists the Runs")
}

/// A window that reached a Foxhound Daemon and listed `rows`.
fn listed(rows: Vec<RunIndexRow>) -> Details {
    let mut details = Details::new();
    details.apply(connected(Product::Foxhound));
    details.apply(resync(&[]));
    let generation = listing(&details).generation;
    details.apply(Input::Listed { generation, rows });
    details
}

fn table() -> Vec<RunIndexRow> {
    vec![
        row("crawl-a", "running", &["goodfetch.recipe:shop"]),
        row("crawl-b", "running", &[]),
        row("crawl-c", "finished", &[]),
    ]
}

fn on_screen(ids: &[&str]) -> Input {
    Input::OnScreen(ids.iter().map(|id| run_id(id)).collect())
}

/// A fixed instant the tests count from: the model reads no clock.
fn origin() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test's origin for the instants it gives the model, which reads no clock itself"
    )]
    Instant::now()
}

fn after(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

#[test]
fn the_window_follows_the_progress_of_the_running_runs_on_screen_only() {
    let mut details = listed(table());

    details.apply(on_screen(&["crawl-a", "crawl-c"]));
    let first = details.wanted().progress;
    details.apply(on_screen(&["crawl-b"]));
    let scrolled = details.wanted().progress;

    assert_eq!(first, runs(&["crawl-a"]));
    assert_eq!(scrolled, runs(&["crawl-b"]));
}

#[test]
fn the_chosen_run_is_followed_and_its_report_and_usage_read_even_off_screen() {
    let mut details = listed(table());
    details.apply(on_screen(&["crawl-a"]));

    details.apply(Input::Chose(Some(run_id("crawl-b"))));
    let wanted = details.wanted();

    assert_eq!(wanted.progress, runs(&["crawl-a", "crawl-b"]));
    assert_eq!(wanted.report, Some(run_id("crawl-b")));
    assert_eq!(
        wanted.usage.map(|read| read.run_id),
        Some(run_id("crawl-b"))
    );
}

#[test]
fn an_ended_chosen_run_reads_its_report_and_follows_nothing() {
    let mut details = listed(table());

    details.apply(Input::Chose(Some(run_id("crawl-c"))));
    let wanted = details.wanted();

    assert_eq!(wanted.progress, runs(&[]));
    assert_eq!(wanted.report, Some(run_id("crawl-c")));
    assert_eq!(wanted.usage, None);
}

#[test]
fn nothing_is_read_while_the_window_is_closed() {
    let mut details = listed(table());
    details.apply(on_screen(&["crawl-a", "crawl-b"]));
    details.apply(Input::Chose(Some(run_id("crawl-a"))));

    details.apply(Input::Closed);

    assert_eq!(details.wanted(), Wanted::default());
}

#[test]
fn nothing_is_read_before_a_daemon_is_reached() {
    let mut details = Details::new();
    details.apply(on_screen(&["crawl-a"]));
    details.apply(Input::Chose(Some(run_id("crawl-a"))));

    let looking = details.wanted();
    details.apply(Input::Connection(ConnectionChange::NoDaemon));
    let no_daemon = details.wanted();

    assert_eq!(looking, Wanted::default());
    assert_eq!(no_daemon, Wanted::default());
}

#[test]
fn the_runs_are_listed_again_after_each_resync_and_each_run_that_starts_or_ends() {
    let mut details = listed(table());
    let first = listing(&details).generation;

    details.apply(axes("crawl-d", "running"));
    let started = listing(&details).generation;
    details.apply(axes("crawl-a", "finished"));
    let ended = listing(&details).generation;
    details.apply(resync(&["crawl-b"]));
    let resynced = listing(&details).generation;

    assert!(first < started && started < ended && ended < resynced);
}

#[test]
fn the_filters_narrow_the_listing_the_engine_answers() {
    let mut details = listed(table());
    let before = listing(&details).generation;

    details.apply(Input::Filter(Filter {
        text: " shop ".to_owned(),
        lifecycle: Some(Lifecycle::Failed),
    }));
    let filtered = listing(&details);

    assert!(filtered.generation > before);
    assert_eq!(
        filtered.params,
        RunListParams {
            text: Some("shop".to_owned()),
            lifecycle: Some(Lifecycle::Failed),
            ..RunListParams::default()
        }
    );
}

#[test]
fn rows_of_a_listing_no_longer_wanted_are_not_shown() {
    let mut details = listed(table());
    let stale = listing(&details).generation;
    details.apply(Input::Filter(Filter {
        text: "shop".to_owned(),
        lifecycle: None,
    }));

    details.apply(Input::Listed {
        generation: stale,
        rows: vec![row("crawl-z", "running", &[])],
    });

    let shown: Vec<RunId> = details
        .view()
        .runs
        .into_iter()
        .map(|line| line.run_id)
        .collect();
    assert!(!shown.contains(&run_id("crawl-z")), "{shown:?}");
}

#[test]
fn the_runs_table_shows_each_runs_name_axes_times_and_progress() {
    let mut details = listed(table());
    details.apply(on_screen(&["crawl-a"]));

    details.apply(Input::Progress(progress(
        "crawl-a",
        3,
        json!({
            "message": "Crawling page 12",
            "requests": {"pending": 4, "fetching": 1, "handling": 0, "in_human_pause": 0,
                         "succeeded": 7, "failed": 2, "skipped": 0, "blocked": 0},
        }),
    )));

    assert_eq!(
        details.view().runs,
        [
            RunLine {
                run_id: run_id("crawl-a"),
                name: "shop".to_owned(),
                lifecycle: "Running".to_owned(),
                verdict: String::new(),
                completeness: String::new(),
                started: "2026-11-03T09:00:00Z".to_owned(),
                ended: String::new(),
                progress: "Crawling page 12 · 7 succeeded, 2 failed, 4 pending".to_owned(),
            },
            RunLine {
                run_id: run_id("crawl-b"),
                name: "crawl-b".to_owned(),
                lifecycle: "Running".to_owned(),
                verdict: String::new(),
                completeness: String::new(),
                started: "2026-11-03T09:00:00Z".to_owned(),
                ended: String::new(),
                progress: String::new(),
            },
            RunLine {
                run_id: run_id("crawl-c"),
                name: "crawl-c".to_owned(),
                lifecycle: "Finished".to_owned(),
                verdict: "verified".to_owned(),
                completeness: "incomplete".to_owned(),
                started: "2026-11-03T09:00:00Z".to_owned(),
                ended: "2026-11-03T09:30:00Z".to_owned(),
                progress: String::new(),
            },
        ]
    );
}

#[test]
fn a_crawls_counts_are_hidden_on_a_collie_daemon() {
    let mut details = Details::new();
    details.apply(connected(Product::Collie));
    details.apply(resync(&[]));
    let generation = listing(&details).generation;
    details.apply(Input::Listed {
        generation,
        rows: vec![row("crawl-a", "running", &[])],
    });
    details.apply(on_screen(&["crawl-a"]));

    details.apply(Input::Progress(progress(
        "crawl-a",
        3,
        json!({
            "message": "Working",
            "requests": {"pending": 4, "fetching": 1, "handling": 0, "in_human_pause": 0,
                         "succeeded": 7, "failed": 2, "skipped": 0, "blocked": 0,
                         "by_error_code": [{"name": "timeout", "count": 2}]},
        }),
    )));
    details.apply(Input::Chose(Some(run_id("crawl-a"))));

    let view = details.view();
    assert_eq!(view.runs[0].progress, "Working");
    let chosen = view.chosen.expect("a chosen Run");
    assert_eq!(chosen.failures, []);
}

#[test]
fn the_chosen_runs_progress_usage_and_failures_by_error_code_are_shown() {
    let mut details = listed(table());
    details.apply(Input::Chose(Some(run_id("crawl-a"))));

    details.apply(Input::Progress(progress(
        "crawl-a",
        3,
        json!({
            "message": "Crawling page 12",
            "requests": {"pending": 4, "fetching": 1, "handling": 2, "in_human_pause": 0,
                         "succeeded": 7, "failed": 3, "skipped": 1, "blocked": 0,
                         "by_error_code": [{"name": "timeout", "count": 2},
                                           {"name": "navigation_failed", "count": 1}]},
        }),
    )));
    details.apply(Input::Usage {
        run_id: run_id("crawl-a"),
        usage: usage(12),
    });

    let chosen = details.view().chosen.expect("a chosen Run");
    assert_eq!(chosen.title, "shop (crawl-a)");
    assert_eq!(
        chosen.progress,
        [
            field("Lifecycle", "Running"),
            field("Message", "Crawling page 12"),
            field(
                "Requests",
                "7 succeeded, 3 failed, 1 skipped, 0 blocked, 4 pending, 3 in flight"
            ),
        ]
    );
    assert_eq!(
        chosen.usage,
        [
            field("Requests", "12"),
            field("Model calls", "3"),
            field("Model tokens", "1200"),
            field("Browser time", "65 s"),
            field("Proxy bytes", "2048"),
        ]
    );
    assert_eq!(
        chosen.failures,
        [field("timeout", "2"), field("navigation_failed", "1")]
    );
    assert_eq!(
        chosen.report,
        [field("Run report", "Comes when the Run ends")]
    );
}

#[test]
fn the_chosen_runs_usage_is_read_again_at_each_new_progress() {
    let mut details = listed(table());
    details.apply(Input::Chose(Some(run_id("crawl-a"))));
    let first = details.wanted().usage;

    details.apply(Input::Progress(progress("crawl-a", 3, json!({}))));
    let second = details.wanted().usage;

    let first: UsageRead = first.expect("a usage read");
    let second: UsageRead = second.expect("a usage read");
    assert_eq!(second.run_id, first.run_id);
    assert!(second.generation > first.generation);
}

#[test]
fn an_ended_runs_report_gives_its_axes_usage_and_failures() {
    let mut details = listed(table());
    details.apply(Input::Chose(Some(run_id("crawl-c"))));

    details.apply(Input::Report {
        run_id: run_id("crawl-c"),
        report: Ok(Box::new(report(
            "crawl-c",
            json!({
                "stop_reason": {"kind": "pause"},
                "warnings": [{"code": "trap_links_off", "message": "Trap-link detection was off"}],
                "requests": {"succeeded": 9, "failed": 4, "skipped": 0, "blocked": 1, "pending": 0,
                             "by_error_code": [{"name": "timeout", "count": 4}]},
            }),
        ))),
    });

    let chosen = details.view().chosen.expect("a chosen Run");
    assert_eq!(
        chosen.report,
        [
            field("Lifecycle", "Failed"),
            field("Stop reason", "pause"),
            field("Verdict", "rejected"),
            field("Why", "failed: The Run failed"),
            field("Completeness", "incomplete"),
            field("Started", "2026-11-03T09:00:00Z"),
            field("Ended", "2026-11-03T09:30:00Z"),
            field("Error", "navigation_failed: The page did not load"),
            field("Warning", "trap_links_off: Trap-link detection was off"),
        ]
    );
    assert_eq!(chosen.usage[0], field("Requests", "40"));
    assert_eq!(chosen.failures, [field("timeout", "4")]);
}

#[test]
fn a_report_that_cannot_be_read_says_why() {
    let mut details = listed(table());
    details.apply(Input::Chose(Some(run_id("crawl-b"))));

    details.apply(Input::Report {
        run_id: run_id("crawl-b"),
        report: Err("run_interrupted".to_owned()),
    });

    let chosen = details.view().chosen.expect("a chosen Run");
    assert_eq!(
        chosen.report,
        [field(
            "Run report",
            "None: no engine works the Run, and a start with its Run ID resumes it (run_interrupted)"
        )]
    );
}

#[test]
fn text_from_a_run_is_made_safe_before_the_window_shows_it() {
    let mut details = listed(table());
    details.apply(on_screen(&["crawl-a"]));

    details.apply(Input::Progress(progress(
        "crawl-a",
        3,
        json!({"message": "GET https://user:hunter2@example.com/\u{202e}evil"}),
    )));

    let shown = &details.view().runs[0].progress;
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains('\u{202e}'), "{shown}");
}

#[test]
fn the_window_repaints_only_when_its_view_changes_and_at_most_once_a_second() {
    let start = origin();
    let mut details = listed(table());
    details.apply(on_screen(&["crawl-a"]));

    let first = details.repaint(start);
    let unchanged = details.repaint(after(start, 100));
    details.apply(Input::Progress(progress(
        "crawl-a",
        3,
        json!({"message": "one"}),
    )));
    let too_soon = details.repaint(after(start, 300));
    let due = details.repaint(after(start, 1000));
    details.apply(Input::Progress(progress(
        "crawl-a",
        4,
        json!({"message": "two"}),
    )));
    let next_too_soon = details.repaint(after(start, 1500));

    assert!(matches!(first, Repaint::Now(_)), "{first:?}");
    assert_eq!(unchanged, Repaint::Nothing);
    assert_eq!(too_soon, Repaint::At(after(start, 1000)));
    let Repaint::Now(view) = due else {
        panic!("a repaint is due a second after the last: {due:?}");
    };
    assert_eq!(view.runs[0].progress, "one");
    assert_eq!(next_too_soon, Repaint::At(after(start, 2000)));
}

#[test]
fn a_closed_window_repaints_nothing() {
    let start = origin();
    let mut details = listed(table());
    details.repaint(start);

    details.apply(Input::Closed);
    details.apply(axes("crawl-d", "running"));

    assert_eq!(details.repaint(after(start, 5000)), Repaint::Nothing);
}

fn field(label: &str, value: &str) -> Field {
    Field {
        label: label.to_owned(),
        value: value.to_owned(),
    }
}
