//! `run.list` through the engine's public interface over the in-memory
//! network (spec #213's Seam 3; ADRs 0031, 0036 and 0117): the engine
//! filters and sorts the Run index's rows, one named case per filter and
//! per sort key, keeps missing and erased rows, and leaves Watch runs out
//! unless the filter asks for them. The index is written beforehand
//! through its own interface, as the engines that ran those Runs left it.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{
    Completeness, Lifecycle, PointInTime, Presence, RunId, RunIndexRow, RunKind, Tags, Verdict,
};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::run::call;
use super::{attached_on, engine, hello};
use crate::run_index::IndexFile;
use crate::sqlite_store::StoreFile as _;

fn at(text: &str) -> PointInTime {
    PointInTime::from_rfc3339(text.to_owned())
}

fn tags(tags: &[&str]) -> Tags {
    serde_json::from_value(json!(tags)).unwrap()
}

/// A Collie Run that started at `started` and runs.
fn running(run_id: &str, started: &str) -> RunIndexRow {
    RunIndexRow {
        run_id: RunId::parse(run_id).unwrap(),
        location: format!("runs/{run_id}"),
        kind: RunKind::Collie,
        needs: Product::Collie,
        tags: Tags::default(),
        lifecycle: Lifecycle::Running,
        verdict: None,
        completeness: None,
        started_at: at(started),
        ended_at: None,
        owner: None,
        presence: Presence::Present,
        progress_message: None,
        stop_reason: None,
        replayed: false,
    }
}

/// The same Run, ended at `ended` with these axes.
fn ended(
    row: RunIndexRow,
    lifecycle: Lifecycle,
    verdict: Verdict,
    completeness: Completeness,
    ended: &str,
) -> RunIndexRow {
    RunIndexRow {
        lifecycle,
        verdict: Some(verdict),
        completeness: Some(completeness),
        ended_at: Some(at(ended)),
        ..row
    }
}

/// The Run index the cases read: a live Run, a Schedule's target and its
/// Watch, a crawl that failed, a Run whose state is gone, and one
/// discarded on purpose.
fn rows() -> Vec<RunIndexRow> {
    vec![
        RunIndexRow {
            tags: tags(&["team:shop", "goodfetch.recipe:shop-prices"]),
            ..running("a-live", "2026-10-01T08:00:00Z")
        },
        RunIndexRow {
            tags: tags(&[
                "goodfetch.schedule:nightly",
                "goodfetch.occurrence:nightly.20261002t080000z",
                "goodfetch.trigger:time",
                "goodfetch.role:target",
                "goodfetch.recipe:shop-prices",
            ]),
            ..ended(
                running("b-target", "2026-10-02T08:00:00Z"),
                Lifecycle::Finished,
                Verdict::Verified,
                Completeness::Complete,
                "2026-10-02T09:00:00Z",
            )
        },
        RunIndexRow {
            tags: tags(&[
                "goodfetch.schedule:nightly",
                "goodfetch.occurrence:nightly.20261002t080000z",
                "goodfetch.trigger:time",
                "goodfetch.role:watch",
            ]),
            ..ended(
                running("c-watch", "2026-10-02T07:59:00Z"),
                Lifecycle::Finished,
                Verdict::Verified,
                Completeness::Complete,
                "2026-10-02T07:59:30Z",
            )
        },
        RunIndexRow {
            kind: RunKind::Crawl,
            needs: Product::Foxhound,
            tags: tags(&["team:data", "goodfetch.recipe:catalog-crawl"]),
            ..ended(
                running("d-crawl", "2026-10-03T08:00:00Z"),
                Lifecycle::Failed,
                Verdict::NeedsReview,
                Completeness::Incomplete,
                "2026-10-03T10:00:00Z",
            )
        },
        RunIndexRow {
            tags: tags(&["goodfetch.trigger:run_now"]),
            presence: Presence::Missing,
            ..ended(
                running("e-missing", "2026-10-04T08:00:00Z"),
                Lifecycle::Stopped,
                Verdict::Rejected,
                Completeness::Incomplete,
                "2026-10-04T08:30:00Z",
            )
        },
        RunIndexRow {
            presence: Presence::Erased,
            ..ended(
                running("f-erased", "2026-10-05T08:00:00Z"),
                Lifecycle::Finished,
                Verdict::Verified,
                Completeness::Unverified,
                "2026-10-05T08:10:00Z",
            )
        },
    ]
}

/// The Run IDs `run.list` answers with `params`, in its order, on an engine
/// whose Run index holds [`rows`].
fn listed(params: Value) -> Vec<String> {
    let env = TestEnv::new();
    let index = env.home().join("1").join("run-index.sqlite");
    let version = ProductVersion::parse("1.4.0").unwrap();
    let file = IndexFile::of(index, &version);
    let connection = file.open().unwrap();
    for row in rows() {
        file.write(&connection, &row).unwrap();
    }
    let reply = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.list", params).await
    });
    let runs = reply["result"]["runs"]
        .as_array()
        .unwrap_or_else(|| panic!("run.list did not answer its rows: {reply}"));
    runs.iter()
        .map(|run| run["run_id"].as_str().unwrap().to_owned())
        .collect()
}

/// Each filter narrows the rows to those it names, newest first; missing
/// and erased rows are kept, and Watch runs are left out unless asked for.
///
/// Covers: story:12-12, story:12-13, operation:run.list
#[rstest]
#[case::no_filter_newest_first_without_watch_runs(json!({}), &["f-erased", "e-missing", "d-crawl", "b-target", "a-live"])]
#[case::watch_runs_asked_for(json!({ "watch_runs": true }), &["f-erased", "e-missing", "d-crawl", "b-target", "c-watch", "a-live"])]
#[case::tags_every_one(json!({ "tags": ["team:shop"] }), &["a-live"])]
#[case::tags_none_carries_both(json!({ "tags": ["team:shop", "team:data"] }), &[])]
#[case::recipe(json!({ "recipe": "shop-prices" }), &["b-target", "a-live"])]
#[case::schedule(json!({ "schedule": "nightly" }), &["b-target"])]
#[case::schedule_with_its_watch_runs(json!({ "schedule": "nightly", "watch_runs": true }), &["b-target", "c-watch"])]
#[case::occurrence(json!({ "occurrence": "nightly.20261002t080000z" }), &["b-target"])]
#[case::trigger(json!({ "trigger": "run_now" }), &["e-missing"])]
#[case::product(json!({ "product": "foxhound" }), &["d-crawl"])]
#[case::lifecycle(json!({ "lifecycle": "running" }), &["a-live"])]
#[case::verdict(json!({ "verdict": "verified" }), &["f-erased", "b-target"])]
#[case::completeness(json!({ "completeness": "incomplete" }), &["e-missing", "d-crawl"])]
#[case::started_from_until(json!({ "started": { "from": "2026-10-02T08:00:00Z", "until": "2026-10-04T08:00:00Z" } }), &["d-crawl", "b-target"])]
#[case::started_in_another_offset(json!({ "started": { "from": "2026-10-04T10:00:00+02:00" } }), &["f-erased", "e-missing"])]
#[case::ended_from(json!({ "ended": { "from": "2026-10-03T10:00:00Z" } }), &["f-erased", "e-missing", "d-crawl"])]
#[case::text_in_a_tag_or_recipe_ignoring_case(json!({ "text": "SHOP" }), &["b-target", "a-live"])]
#[case::text_in_a_run_id(json!({ "text": "crawl" }), &["d-crawl"])]
fn run_list_filters_the_run_index_in_the_engine(#[case] params: Value, #[case] expected: &[&str]) {
    assert_eq!(listed(params), expected);
}

/// Each sort key orders the rows, ties in Run ID order; a value a Run does
/// not have yet sorts after every other in ascending order.
///
/// Covers: story:12-12, operation:run.list
#[rstest]
#[case::run_id(json!({ "sort": "run_id", "order": "ascending" }), &["a-live", "b-target", "d-crawl", "e-missing", "f-erased"])]
#[case::started_at_ascending(json!({ "order": "ascending" }), &["a-live", "b-target", "d-crawl", "e-missing", "f-erased"])]
#[case::ended_at_ascending(json!({ "sort": "ended_at", "order": "ascending" }), &["b-target", "d-crawl", "e-missing", "f-erased", "a-live"])]
#[case::ended_at_descending(json!({ "sort": "ended_at" }), &["a-live", "f-erased", "e-missing", "d-crawl", "b-target"])]
#[case::recipe(json!({ "sort": "recipe", "order": "ascending" }), &["d-crawl", "a-live", "b-target", "e-missing", "f-erased"])]
#[case::schedule(json!({ "sort": "schedule", "order": "ascending" }), &["b-target", "a-live", "d-crawl", "e-missing", "f-erased"])]
#[case::occurrence(json!({ "sort": "occurrence", "order": "ascending" }), &["b-target", "a-live", "d-crawl", "e-missing", "f-erased"])]
#[case::trigger(json!({ "sort": "trigger", "order": "ascending" }), &["b-target", "e-missing", "a-live", "d-crawl", "f-erased"])]
#[case::product(json!({ "sort": "product", "order": "ascending" }), &["a-live", "b-target", "e-missing", "f-erased", "d-crawl"])]
#[case::lifecycle(json!({ "sort": "lifecycle", "order": "ascending" }), &["a-live", "e-missing", "b-target", "f-erased", "d-crawl"])]
#[case::verdict(json!({ "sort": "verdict", "order": "ascending" }), &["b-target", "f-erased", "e-missing", "d-crawl", "a-live"])]
#[case::completeness(json!({ "sort": "completeness", "order": "ascending" }), &["b-target", "d-crawl", "e-missing", "f-erased", "a-live"])]
fn run_list_sorts_the_run_index_in_the_engine(#[case] params: Value, #[case] expected: &[&str]) {
    assert_eq!(listed(params), expected);
}
