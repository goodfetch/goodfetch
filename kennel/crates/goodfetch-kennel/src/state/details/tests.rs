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

use super::{
    Details, Field, Filter, GovernorPanel, Input, Page, RecordsPanel, Repaint, RunLine,
    SettingLine, View,
};
use crate::client::{
    Call, CallFailure, ConnectionChange, DaemonEvent, DaemonIdentity, Listing, SeedsAdded,
    UsageRead, Wanted,
};
use crate::settings::{Settings, Views};

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

/// The Records query of `run` from `offset`, as the window asks it.
fn records_sql(run: &str, offset: u64) -> String {
    let from = format!(
        "SELECT dataset, number, url, committed_at, value FROM records('{run}') ORDER BY dataset, number"
    );
    if offset == 0 {
        from
    } else {
        format!("{from} OFFSET {offset}")
    }
}

/// `dataset.query`'s answer of `rows` Records of the `products` Dataset
/// numbered from `first`, of `total` from there.
fn queried(details: &Details, first: u64, rows: u64, total: u64) -> Input {
    let generation = details
        .wanted()
        .records
        .expect("the Records view reads the chosen Run's Records")
        .generation;
    let rows = (first..first + rows)
        .map(|number| {
            vec![
                json!("products"),
                json!(number),
                json!(format!("https://shop.example/{number}")),
                json!("2026-11-03T09:10:00Z"),
                json!({"name": format!("Product {number}")})
                    .to_string()
                    .into(),
            ]
        })
        .collect::<Vec<Vec<Value>>>();
    Input::Queried {
        generation,
        result: Ok(serde_json::from_value(json!({
            "columns": [
                {"name": "dataset", "type": "VARCHAR"},
                {"name": "number", "type": "BIGINT"},
                {"name": "url", "type": "VARCHAR"},
                {"name": "committed_at", "type": "VARCHAR"},
                {"name": "value", "type": "VARCHAR"},
            ],
            "rows": rows,
            "total": total,
        }))
        .unwrap()),
    }
}

/// A Foxhound window whose settings were read, with `crawl-a` (Recipe
/// `shop`, running) and `crawl-c` (finished) listed, showing `view` with
/// `run` chosen.
fn showing(view: View, run: &str) -> Details {
    let mut details = listed(table());
    details.apply(Input::Settings(Ok(Settings::default())));
    details.apply(Input::Chose(Some(run_id(run))));
    details.apply(Input::Show(view));
    details
}

#[test]
fn the_records_view_reads_the_chosen_runs_records_a_page_at_a_time() {
    let mut details = showing(View::Records, "crawl-c");

    let first = details.wanted().records.map(|query| query.params);
    details.apply(queried(&details, 1, 2, 52));
    let page = details.view().records;
    details.apply(Input::RecordsPage(Page::Next));
    let next = details.wanted().records.map(|query| query.params.sql);
    details.apply(Input::RecordsPage(Page::Previous));
    let back = details.wanted().records.map(|query| query.params.sql);

    let first = first.expect("the first page is read");
    assert_eq!(first.sql, records_sql("crawl-c", 0));
    assert_eq!(first.runs, vec![run_id("crawl-c")]);
    assert_eq!(first.limit, Some(50));
    assert_eq!(
        page,
        Some(RecordsPanel {
            title: "Records of crawl-c".to_owned(),
            note: None,
            columns: ["dataset", "number", "url", "committed_at", "value"]
                .map(str::to_owned)
                .to_vec(),
            rows: vec![
                [
                    "products",
                    "1",
                    "https://shop.example/1",
                    "2026-11-03T09:10:00Z",
                    r#"{"name":"Product 1"}"#,
                ]
                .map(str::to_owned)
                .to_vec(),
                [
                    "products",
                    "2",
                    "https://shop.example/2",
                    "2026-11-03T09:10:00Z",
                    r#"{"name":"Product 2"}"#,
                ]
                .map(str::to_owned)
                .to_vec(),
            ],
            range: "Records 1 to 2 of 52".to_owned(),
            previous: false,
            next: true,
        })
    );
    assert_eq!(next, Some(records_sql("crawl-c", 50)));
    assert_eq!(back, Some(records_sql("crawl-c", 0)));
}

#[test]
fn records_are_read_only_while_their_view_shows_a_chosen_run() {
    let mut details = showing(View::Records, "crawl-c");

    details.apply(Input::Show(View::Runs));
    let on_the_runs = details.wanted().records;
    details.apply(Input::Show(View::Records));
    details.apply(Input::Chose(None));
    let none_chosen = details.wanted().records;
    let note = details.view().records.and_then(|records| records.note);

    assert_eq!(on_the_runs, None);
    assert_eq!(none_chosen, None);
    assert_eq!(
        note.as_deref(),
        Some("Choose a Run in the Runs view to read its Records")
    );
}

#[test]
fn a_refused_records_query_says_why() {
    let mut details = showing(View::Records, "crawl-c");
    let generation = details.wanted().records.unwrap().generation;

    details.apply(Input::Queried {
        generation,
        result: Err(CallFailure {
            code: "query_memory_exceeded".to_owned(),
            message: "The query needed more memory than dataset.query.memory_limit".to_owned(),
        }),
    });

    let note = details.view().records.and_then(|records| records.note);
    assert_eq!(
        note.as_deref(),
        Some(
            "The Daemon refused the query: The query needed more memory than dataset.query.memory_limit (query_memory_exceeded)"
        )
    );
}

#[test]
fn foxhounds_records_and_add_seeds_are_hidden_on_a_collie_daemon() {
    let mut details = Details::new();
    details.apply(Input::Settings(Ok(Settings::default())));
    details.apply(connected(Product::Collie));
    details.apply(resync(&[]));
    let generation = listing(&details).generation;
    details.apply(Input::Listed {
        generation,
        rows: table(),
    });
    details.apply(Input::Chose(Some(run_id("crawl-a"))));
    let panel = details.view().chosen.unwrap();

    details.apply(Input::Show(View::Records));
    let view = details.view();

    assert_eq!(panel.add_seeds, None);
    assert_eq!(
        view.views,
        [
            View::Runs,
            View::DaemonSettings,
            View::Governor,
            View::KennelSettings
        ]
    );
    assert_eq!(view.shown, View::Runs);
    assert_eq!(details.wanted().records, None);
}

fn explained() -> Input {
    Input::Explained {
        generation: 0,
        result: Ok(serde_json::from_value(json!({
            "policy_version": "sha256:0123456789abcdef",
            "files": [
                {"layer": "operator_policy", "path": "/etc/goodfetch/policy.toml", "found": true},
                {"layer": "user_configuration", "path": "/home/someone/.config/goodfetch/config.toml", "found": false},
            ],
            "settings": [
                {"key": "timeout.navigation", "products": ["collie", "foxhound"], "applied": true,
                 "value": 30000, "layer": "default", "locked_by": "operator_policy", "weakens": false},
                {"key": "crawl.max_depth", "products": ["foxhound"], "applied": false,
                 "value": 3, "layer": "user_configuration", "weakens": false},
                {"key": "proxy.url", "products": ["collie", "foxhound"], "applied": true,
                 "value": "http://proxy.example:8080", "layer": "run", "origin": "flag", "weakens": false},
            ],
        }))
        .unwrap()),
    }
}

#[test]
fn the_daemon_settings_view_reads_the_settings_in_force_only_while_it_shows() {
    let mut details = showing(View::Runs, "crawl-a");

    let on_the_runs = details.wanted().explain;
    details.apply(Input::Show(View::DaemonSettings));
    let shown = details.wanted().explain;
    details.apply(Input::Refresh);
    let refreshed = details.wanted().explain;

    assert_eq!(on_the_runs, None);
    assert!(shown.is_some(), "the view reads the settings");
    assert!(
        refreshed.is_some() && refreshed != shown,
        "a refresh reads them again"
    );
}

#[test]
fn the_daemon_settings_view_shows_each_settings_value_and_where_it_comes_from() {
    let mut details = showing(View::DaemonSettings, "crawl-a");
    let Some(generation) = details.wanted().explain else {
        panic!("the view reads the settings");
    };
    let Input::Explained { result, .. } = explained() else {
        unreachable!();
    };

    details.apply(Input::Explained { generation, result });
    let panel = details.view().daemon_settings.unwrap();

    assert_eq!(panel.note, None);
    assert_eq!(
        panel.sources,
        [
            field("Operator policy", "sha256:0123456789abcdef"),
            field("Operator policy file", "/etc/goodfetch/policy.toml"),
            field(
                "User configuration file",
                "/home/someone/.config/goodfetch/config.toml (not found)"
            ),
        ]
    );
    assert_eq!(
        panel.settings,
        [
            SettingLine {
                key: "timeout.navigation".to_owned(),
                value: "30000".to_owned(),
                set_by: "the default".to_owned(),
                locked_by: "the Operator policy".to_owned(),
            },
            SettingLine {
                key: "crawl.max_depth".to_owned(),
                value: "3, not applied: only Foxhound honours it".to_owned(),
                set_by: "user configuration".to_owned(),
                locked_by: String::new(),
            },
            SettingLine {
                key: "proxy.url".to_owned(),
                value: "http://proxy.example:8080".to_owned(),
                set_by: "the Run (a CLI flag)".to_owned(),
                locked_by: String::new(),
            },
        ]
    );
}

fn reading() -> Input {
    Input::SystemInfo(Box::new(
        serde_json::from_value(json!({
            "level": "pressure",
            "pressure_source": "memory",
            "last_resort": false,
            "memory": {"allowance": 8_589_934_592_u64, "allowance_source": "physical",
                       "used": 6_442_450_944_u64, "browsers": 2_147_483_648_u64, "host": "normal"},
            "cpu": {"parallelism": 8, "busy": 35, "tokio_busy": 12},
            "concurrency": {
                "http": {"floor": 1, "ceiling": 64, "target": 16, "in_use": 9},
                "browser_page": {"floor": 1, "ceiling": 8, "target": 4, "in_use": 2},
                "model_call": {"floor": 1, "ceiling": 4, "target": 4, "in_use": 0},
            },
        }))
        .unwrap(),
    ))
}

#[test]
fn the_governors_readings_are_subscribed_to_only_while_its_view_shows() {
    let mut details = showing(View::Runs, "crawl-a");

    let on_the_runs = details.wanted().system_info;
    details.apply(Input::Show(View::Governor));
    let shown = details.wanted().system_info;
    details.apply(Input::Connection(ConnectionChange::NoDaemon));
    let without_a_daemon = details.wanted().system_info;
    details.apply(connected(Product::Foxhound));
    let reconnected = details.wanted().system_info;
    details.apply(Input::Closed);
    let closed = details.wanted().system_info;

    assert_eq!(
        (on_the_runs, shown, without_a_daemon, reconnected, closed),
        (false, true, false, true, false)
    );
}

#[test]
fn the_governor_view_shows_its_latest_reading() {
    let mut details = showing(View::Governor, "crawl-a");
    let waiting = details.view().governor;

    details.apply(reading());
    let read = details.view().governor;
    details.apply(Input::Show(View::Runs));
    details.apply(Input::Show(View::Governor));
    let shown_again = details.view().governor;

    assert_eq!(
        waiting,
        Some(GovernorPanel {
            note: Some("Waiting for the Resource governor's next reading".to_owned()),
            readings: Vec::new(),
        })
    );
    assert_eq!(
        read,
        Some(GovernorPanel {
            note: None,
            readings: vec![
                field("Level", "pressure, held by memory"),
                field("Memory", "6.0 GiB used of 8.0 GiB, half of physical memory"),
                field("Browsers' memory", "2.0 GiB"),
                field("Host memory", "normal"),
                field("CPU", "35% busy, 8 threads"),
                field("Tokio workers", "12% busy"),
                field("HTTP fetches", "9 in use of 16 (from 1 to 64)"),
                field("Browser pages", "2 in use of 4 (from 1 to 8)"),
                field("Model calls", "0 in use of 4 (from 1 to 4)"),
            ],
        })
    );
    assert_eq!(shown_again, waiting, "a reading is not kept once hidden");
}

#[test]
fn each_view_turned_off_is_neither_offered_nor_read() {
    let mut details = showing(View::Runs, "crawl-c");
    details.apply(Input::Settings(Ok(Settings {
        views: Views {
            records: false,
            daemon_settings: false,
            governor: false,
            kennel_settings: false,
        },
        ..Settings::default()
    })));

    let mut wanted = Vec::new();
    for view in [View::Records, View::DaemonSettings, View::Governor] {
        details.apply(Input::Show(view));
        wanted.push(details.wanted());
    }
    let view = details.view();

    assert_eq!(view.views, [View::Runs]);
    assert_eq!(view.shown, View::Runs);
    for wanted in wanted {
        assert_eq!(
            (wanted.records, wanted.explain, wanted.system_info),
            (None, None, false)
        );
    }
}

#[test]
fn the_chosen_runs_reads_stop_while_another_view_shows() {
    let mut details = showing(View::Governor, "crawl-a");
    details.apply(on_screen(&["crawl-a"]));

    let wanted = details.wanted();

    assert_eq!(wanted.progress, runs(&[]));
    assert_eq!((wanted.report, wanted.usage), (None, None));
}

#[test]
fn add_seeds_is_offered_for_a_live_crawl_and_asks_a_confirmation_naming_the_run_and_the_consequence()
 {
    let mut details = showing(View::Runs, "crawl-a");
    let offer = details
        .view()
        .chosen
        .and_then(|panel| panel.add_seeds)
        .expect("a live crawl on a Foxhound Daemon offers Add Seeds");

    let confirmation = offer
        .confirmation(
            " detail ",
            "https://shop.example/a\n\n  https://shop.example/b  \n",
        )
        .unwrap();
    let call = details.confirmed(confirmation.clone());

    assert_eq!(confirmation.title, "Add 2 seeds to shop?");
    assert_eq!(
        confirmation.consequence,
        "The crawl shop (crawl-a) fetches them with the Label detail, beside its other requests. A seed it already holds is not added again, and adding cannot be undone."
    );
    assert_eq!(confirmation.confirm, "Add seeds");
    let Some(Call::AddSeeds(seeds)) = call else {
        panic!("the confirmed seeds are added: {call:?}");
    };
    assert_eq!(seeds.run_id, run_id("crawl-a"));
    let seeds: Vec<Value> = seeds
        .seeds
        .iter()
        .map(|seed| serde_json::to_value(seed).unwrap())
        .collect();
    assert_eq!(
        seeds,
        [
            json!({"url": "https://shop.example/a", "label": "detail"}),
            json!({"url": "https://shop.example/b", "label": "detail"}),
        ]
    );
    let status = details.view().chosen.and_then(|panel| panel.seeds);
    assert_eq!(status.as_deref(), Some("Adding 2 seeds…"));
}

#[test]
fn add_seeds_shows_what_the_crawl_added_and_what_it_did_not() {
    let mut details = showing(View::Runs, "crawl-a");
    let offer = details
        .view()
        .chosen
        .and_then(|panel| panel.add_seeds)
        .unwrap();
    let confirmation = offer
        .confirmation("detail", "https://a.example\nhttps://b.example\nnot a url")
        .unwrap();
    let call = details.confirmed(confirmation).unwrap();

    details.apply(Input::SeedsAdded {
        run_id: run_id("crawl-a"),
        added: SeedsAdded {
            added: 1,
            not_added: [("duplicate".to_owned(), 1), ("invalid_url".to_owned(), 1)]
                .into_iter()
                .collect(),
        },
    });
    details.apply(Input::Answered(call.clone(), Ok(())));
    let added = details.view().chosen.and_then(|panel| panel.seeds);
    let again = details.confirmed(offer.confirmation("detail", "https://c.example").unwrap());
    details.apply(Input::Answered(
        again.unwrap(),
        Err(CallFailure {
            code: "run_not_found".to_owned(),
            message: "The crawl is already ending its Run".to_owned(),
        }),
    ));
    let failed = details.view().chosen.and_then(|panel| panel.seeds);

    assert_eq!(
        added.as_deref(),
        Some("1 seed added; 2 not added: duplicate 1, invalid_url 1")
    );
    assert_eq!(
        failed.as_deref(),
        Some("Adding seeds to shop failed: The crawl is already ending its Run (run_not_found)")
    );
}

#[test]
fn with_controls_off_add_seeds_is_neither_offered_nor_made() {
    let mut details = showing(View::Runs, "crawl-a");
    let offer = details
        .view()
        .chosen
        .and_then(|panel| panel.add_seeds)
        .unwrap();
    let confirmation = offer
        .confirmation("detail", "https://shop.example/a")
        .unwrap();

    details.apply(Input::Settings(Ok(Settings {
        controls: false,
        ..Settings::default()
    })));
    let offered = details.view().chosen.and_then(|panel| panel.add_seeds);
    let call = details.confirmed(confirmation);

    assert_eq!(offered, None);
    assert_eq!(
        call, None,
        "a confirmation given before controls went off adds nothing"
    );
}

#[test]
fn add_seeds_is_offered_only_for_a_live_crawl_once_the_settings_are_read() {
    let mut before_the_settings = listed(table());
    before_the_settings.apply(Input::Chose(Some(run_id("crawl-a"))));
    let ended = showing(View::Runs, "crawl-c");
    let mut collie_run = listed(vec![{
        let mut row = row("script-run", "running", &[]);
        row.kind = goodfetch_protocol::run::RunKind::Collie;
        row
    }]);
    collie_run.apply(Input::Settings(Ok(Settings::default())));
    collie_run.apply(Input::Chose(Some(run_id("script-run"))));

    for details in [before_the_settings, ended, collie_run] {
        let offer = details.view().chosen.and_then(|panel| panel.add_seeds);
        assert_eq!(offer, None);
    }
}

#[test]
fn add_seeds_asks_for_a_label_and_at_least_one_url() {
    let details = showing(View::Runs, "crawl-a");
    let offer = details
        .view()
        .chosen
        .and_then(|panel| panel.add_seeds)
        .unwrap();

    let no_label = offer.confirmation("  ", "https://shop.example/a");
    let no_url = offer.confirmation("detail", " \n \n");

    assert_eq!(
        no_label.err().as_deref(),
        Some("Give the Label of the handler the seeds go to")
    );
    assert_eq!(
        no_url.err().as_deref(),
        Some("Give at least one URL, one a line")
    );
}

#[test]
fn text_from_records_and_settings_is_made_safe_before_the_window_shows_it() {
    let mut details = showing(View::Records, "crawl-c");
    let generation = details.wanted().records.unwrap().generation;
    details.apply(Input::Queried {
        generation,
        result: Ok(serde_json::from_value(json!({
            "columns": [{"name": "url\u{202e}", "type": "VARCHAR"}],
            "rows": [["https://user:hunter2@example.com/\u{202e}evil"]],
            "total": 1,
        }))
        .unwrap()),
    });

    let records = details.view().records.unwrap();

    assert_eq!(records.columns, ["url"]);
    assert!(
        !records.rows[0][0].contains("hunter2"),
        "{:?}",
        records.rows
    );
    assert!(
        !records.rows[0][0].contains('\u{202e}'),
        "{:?}",
        records.rows
    );
}

mod kennel_settings {
    //! The Kennel settings view: the settings as the file holds them, a
    //! change in the file shown, and the view a setting of its own.

    use super::super::{Details, Input, KennelSettingsView, View};
    use crate::settings::{DesktopNotices, Settings, SettingsError, Switch, Views};

    fn shown_settings(details: &Details) -> KennelSettingsView {
        let view = details.view();
        assert_eq!(view.shown, View::KennelSettings);
        view.kennel_settings
            .expect("the Kennel settings view shows the settings")
    }

    fn switches(view: &KennelSettingsView) -> Vec<(Switch, bool)> {
        view.sections
            .iter()
            .flat_map(|section| &section.switches)
            .map(|line| (line.switch, line.on))
            .collect()
    }

    #[test]
    fn the_view_shows_each_setting_as_the_file_holds_it() {
        let mut details = Details::new();
        details.apply(Input::Settings(Ok(Settings {
            start_at_login: true,
            desktop_notices: DesktopNotices {
                run_finished: true,
                period_minutes: 5,
                ..DesktopNotices::default()
            },
            ..Settings::default()
        })));

        details.apply(Input::Show(View::KennelSettings));

        let view = shown_settings(&details);
        assert_eq!(
            switches(&view),
            [
                (Switch::Controls, true),
                (Switch::ConfirmStop, true),
                (Switch::Window, true),
                (Switch::KeepDaemonAlive, false),
                (Switch::StartAtLogin, true),
                (Switch::HiddenIconCheck, true),
                (Switch::Notices, true),
                (Switch::NeedsYouNotice, true),
                (Switch::RunFailedNotice, true),
                (Switch::RunStoppedByLimitNotice, true),
                (Switch::DaemonLostNotice, true),
                (Switch::RunFinishedNotice, true),
                (Switch::RecordsView, true),
                (Switch::DaemonSettingsView, true),
                (Switch::GovernorView, true),
                (Switch::KennelSettingsView, true),
            ]
        );
        assert_eq!(
            view.sections
                .iter()
                .filter_map(|section| section.period.as_ref())
                .map(|period| period.minutes)
                .collect::<Vec<_>>(),
            [5]
        );
        assert_eq!(view.note, None);
    }

    #[test]
    fn a_change_in_the_file_shows_in_the_view() {
        let mut details = Details::new();
        details.apply(Input::Settings(Ok(Settings::default())));
        details.apply(Input::Show(View::KennelSettings));

        details.apply(Input::Settings(Ok(Settings {
            hidden_icon_check: false,
            ..Settings::default()
        })));

        assert!(switches(&shown_settings(&details)).contains(&(Switch::HiddenIconCheck, false)));
    }

    #[test]
    fn an_unreadable_file_shows_the_defaults_and_says_why() {
        let mut details = Details::new();
        details.apply(Input::Settings(Ok(Settings {
            controls: false,
            ..Settings::default()
        })));
        details.apply(Input::Show(View::KennelSettings));

        details.apply(Input::Settings(Err(SettingsError::Unreadable(
            "unknown field `contrls`".to_owned(),
        ))));

        let view = shown_settings(&details);
        assert!(switches(&view).contains(&(Switch::Controls, true)));
        assert!(
            view.note
                .as_deref()
                .is_some_and(|note| note.contains("unknown field `contrls`")),
            "{:?}",
            view.note
        );
    }

    #[test]
    fn a_failed_edit_says_why_until_the_file_is_read_again() {
        let mut details = Details::new();
        details.apply(Input::Settings(Ok(Settings::default())));
        details.apply(Input::Show(View::KennelSettings));

        details.apply(Input::EditFailed(SettingsError::Unwritable(
            "permission denied".to_owned(),
        )));
        let failed = shown_settings(&details).note;
        details.apply(Input::Settings(Ok(Settings::default())));
        let read_again = shown_settings(&details).note;

        assert!(
            failed
                .as_deref()
                .is_some_and(|note| note.contains("permission denied")),
            "{failed:?}"
        );
        assert_eq!(read_again, None);
    }

    #[test]
    fn with_its_switch_off_the_view_is_not_offered_and_the_runs_show() {
        let mut details = Details::new();
        details.apply(Input::Settings(Ok(Settings::default())));
        details.apply(Input::Show(View::KennelSettings));

        details.apply(Input::Settings(Ok(Settings {
            views: Views {
                kennel_settings: false,
                ..Views::default()
            },
            ..Settings::default()
        })));

        let view = details.view();
        assert!(
            !view.views.contains(&View::KennelSettings),
            "{:?}",
            view.views
        );
        assert_eq!(view.shown, View::Runs);
        assert_eq!(view.kennel_settings, None);
    }

    #[test]
    fn before_the_settings_are_read_only_the_runs_are_offered() {
        let mut details = Details::new();

        details.apply(Input::Show(View::KennelSettings));

        let view = details.view();
        assert!(
            !view.views.contains(&View::KennelSettings),
            "{:?}",
            view.views
        );
        assert_eq!(view.shown, View::Runs);
    }

    #[test]
    fn the_runs_show_until_the_person_chooses_another_view() {
        let mut details = Details::new();
        details.apply(Input::Settings(Ok(Settings::default())));

        let view = details.view();

        assert_eq!(view.views.last(), Some(&View::KennelSettings));
        assert_eq!(view.shown, View::Runs);
        assert_eq!(view.kennel_settings, None);
    }
}
