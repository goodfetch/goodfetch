//! A crawl's bounds and the caps on a URL's shape, on Foxhound's
//! executable over stdio (spec #581's seam: the protocol against the
//! Proving ground; ADR 0094): `crawl.max_depth` and `crawl.max_requests`
//! refuse further enqueues with their reasons and end the Run `incomplete`
//! (CO-07, CO-08; pitfall U01), and the three URL caps, on by default, end
//! the archive's relative-link loop of the Proving ground's `traps` site and
//! refuse an over-long URL, a deep path and a repeated segment, leaving the
//! Run `unverified` (ADR 0120; pitfall U10). Each refusal is counted in the
//! Run report by its setting, and the report read back later says the same.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::{Inside, attributes, crawl_under, fetch};

/// The Proving ground's seed.
const SEED: u64 = 843;

/// What a crawl showed: each enqueue's answer, `added` or the reason it was
/// not, the paths the Proving ground received, and the Run report's
/// Completeness and its reasons, as `run.finish` answered them and as an
/// engine started afterwards reads them from the Run's state.
#[derive(Debug, PartialEq)]
struct Crawled {
    answers: Vec<String>,
    received: Vec<String>,
    completeness: Value,
    reasons: Value,
    read_back: (Value, Value),
}

/// The reason the setting `name` gives a finished Run in its report,
/// having refused `count` URLs.
fn refused(name: &str, count: u64) -> Value {
    let urls = if count == 1 { "URL" } else { "URLs" };
    json!({
        "code": name,
        "message": format!("`{name}` refused {count} {urls} the crawl would otherwise have admitted."),
        "count": count,
    })
}

fn strings(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

/// An enqueue's answer: `added`, or the reason it was not.
fn answered(answer: &Value) -> String {
    answer
        .get("reason")
        .unwrap_or(&answer["kind"])
        .as_str()
        .unwrap()
        .to_owned()
}

/// Crawls the Proving ground's archive, `traps.test/archive/`, under the
/// `crawl` settings `settings`: each page's handler fetches it and enqueues
/// its "Older" link, the relative `more/`, which nests without end.
fn crawl_archive(settings: &Value) -> Crawled {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url("traps.test", "/archive/"), "label": "page" })];
    let mut engine = attached(&env);

    let mut answers = Vec::new();
    let report = crawl_under(
        &mut engine,
        "archive",
        &seeds,
        settings,
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap();
            let page = fetch(&proving_ground, url);
            assert_eq!(page.status, 200, "{url}");
            for href in attributes(&page.body, "a href") {
                if href == "more/" {
                    answers.push(answered(&inside.enqueue(handle, &href, "page", None)));
                }
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());
    let received = ReceivedRequests::of(&observations)
        .every()
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .collect();
    crawled(&env, "archive", answers, received, &report)
}

/// What the crawl `run_id` showed, its report read back by a new engine.
fn crawled(
    env: &TestEnv,
    run_id: &str,
    answers: Vec<String>,
    received: Vec<String>,
    report: &Value,
) -> Crawled {
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let mut engine = attached(env);
    let again = engine.call(
        "again",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    assert!(engine.finished());
    Crawled {
        answers,
        received,
        completeness: report["result"]["completeness"].clone(),
        reasons: report["result"]["completeness_reasons"].clone(),
        read_back: (
            again["result"]["completeness"].clone(),
            again["result"]["completeness_reasons"].clone(),
        ),
    }
}

/// A depth cutoff and a request cutoff each refuse the archive's next
/// level with their reasons, the Proving ground receives no more than they
/// admitted, and the Run ends `incomplete`, the refusal counted.
///
/// Covers: story:6-227, setting:crawl.max_depth, setting:crawl.max_requests, operation:crawl_request.enqueue
#[rstest]
#[case::depth("max_depth", "crawl.max_depth")]
#[case::requests("max_requests", "crawl.max_requests")]
fn a_bound_refuses_further_enqueues_and_the_run_ends_incomplete(
    #[case] reason: &str,
    #[case] setting: &str,
) {
    let settings = match reason {
        "max_depth" => json!({ "max_depth": 1 }),
        _ => json!({ "max_requests": 2 }),
    };

    let crawled = crawl_archive(&settings);

    let reasons = json!([refused(setting, 1)]);
    assert_eq!(
        crawled,
        Crawled {
            answers: strings(&["added", reason]),
            received: strings(&["/archive/", "/archive/more/"]),
            completeness: json!("incomplete"),
            reasons: reasons.clone(),
            read_back: (json!("incomplete"), reasons),
        }
    );
}

/// With nothing set, `crawl.max_segment_repeats` ends the archive's
/// relative-link loop at its third `more`, and the finished Run is
/// `unverified`, naming the cap; set to `null`, the loop goes on until
/// `crawl.max_requests` cuts it, and the Run is `incomplete`. Unset,
/// neither bound cuts anything.
///
/// Covers: story:6-228, story:6-227, setting:crawl.max_segment_repeats, setting:crawl.max_requests
#[test]
fn the_default_repeat_cap_ends_a_relative_link_loop_and_null_turns_it_off() {
    let capped = crawl_archive(&json!({}));
    let uncapped = crawl_archive(&json!({ "max_segment_repeats": null, "max_requests": 5 }));

    let cap = json!([refused("crawl.max_segment_repeats", 1)]);
    let bound = json!([refused("crawl.max_requests", 1)]);
    assert_eq!(
        (capped, uncapped),
        (
            Crawled {
                answers: strings(&["added", "added", "max_segment_repeats"]),
                received: strings(&["/archive/", "/archive/more/", "/archive/more/more/"]),
                completeness: json!("unverified"),
                reasons: cap.clone(),
                read_back: (json!("unverified"), cap),
            },
            Crawled {
                answers: strings(&["added", "added", "added", "added", "max_requests"]),
                received: strings(&[
                    "/archive/",
                    "/archive/more/",
                    "/archive/more/more/",
                    "/archive/more/more/more/",
                    "/archive/more/more/more/more/",
                ]),
                completeness: json!("incomplete"),
                reasons: bound.clone(),
                read_back: (json!("incomplete"), bound),
            }
        )
    );
}

/// The built-in caps refuse a 3,000-character URL, a 25-segment path and a
/// path repeating a segment three times, enqueued or added as a seed to the
/// running crawl, each counted, and the Run is `unverified`; an over-long
/// link to another host is `out_of_scope` and no cap's; with the three caps
/// set to `null`, the Run admits them all and is `complete`.
///
/// Covers: story:6-228, setting:crawl.max_url_length, setting:crawl.max_path_segments, setting:crawl.max_segment_repeats, operation:run.add_seed
#[rstest]
#[case::built_in(
    json!({}),
    &["max_url_length", "max_path_segments", "max_segment_repeats", "out_of_scope", "added", "max_segment_repeats"],
    json!("unverified"),
    json!([
        refused("crawl.max_path_segments", 1),
        refused("crawl.max_segment_repeats", 2),
        refused("crawl.max_url_length", 1),
    ]),
)]
#[case::off(
    json!({ "max_url_length": null, "max_path_segments": null, "max_segment_repeats": null }),
    &["added", "added", "added", "out_of_scope", "added", "added"],
    json!("complete"),
    Value::Null,
)]
fn the_url_caps_refuse_a_long_url_a_deep_path_and_a_repeated_segment(
    #[case] settings: Value,
    #[case] answers: &[&str],
    #[case] completeness: Value,
    #[case] reasons: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url("traps.test", "/"), "label": "seed" })];
    let deep = (1..=25)
        .map(|n| format!("/{n}"))
        .collect::<Vec<_>>()
        .concat();
    let links = [
        format!("/{}", "a".repeat(3_000)),
        deep,
        "/a/a/a/".to_owned(),
        proving_ground.url("elsewhere.test", &format!("/{}", "a".repeat(3_000))),
        "/a/a/".to_owned(),
    ];
    let looping_seed = proving_ground.url("traps.test", "/b/b/b/");
    let mut engine = attached(&env);

    let mut given = Vec::new();
    let report = crawl_under(
        &mut engine,
        "caps",
        &seeds,
        &settings,
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "seed" {
                for link in &links {
                    given.push(answered(&inside.enqueue(handle, link, "page", None)));
                }
                let added = inside.call(
                    "run.add_seed",
                    &json!({ "run_id": "caps", "seed": { "url": looping_seed, "label": "page" } }),
                );
                given.push(answered(&added["result"]));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        crawled(&env, "caps", given, Vec::new(), &report),
        Crawled {
            answers: strings(answers),
            received: Vec::new(),
            completeness: completeness.clone(),
            reasons: reasons.clone(),
            read_back: (completeness, reasons),
        }
    );
}
