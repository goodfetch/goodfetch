//! A crawl's scope on Foxhound's executable over stdio (spec #581's seam:
//! the protocol against the Proving ground; ADR 0094): the Proving
//! ground's `scope` site, with a `www` host, a subdomain, an `https` page,
//! a sister domain and an off-site link, crawled under each
//! `crawl.scope.strategy`; `crawl.scope.include` never widening the
//! strategy and `crawl.scope.exclude` narrowing it; a `www` seed and its
//! bare host; and a pattern the engine cannot match refused at the start.
//! Each link outside the scope is answered `out_of_scope`, and the Proving
//! ground receives no request off the site it was given (pitfall U15).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, attributes, crawl_under, fetch};

/// The Proving ground's seed.
const SEED: u64 = 840;

/// Crawls the seed `path` on `host` under the `crawl` settings `settings`: its handler fetches
/// each `http` page it is given (it speaks no TLS) and, on the seed's
/// page, enqueues every link it holds. Answers each enqueue's answer, as
/// `added` or the reason it was not, and the hosts the Proving ground
/// received, each once, sorted.
fn crawl_scope(host: &str, path: &str, settings: &Value) -> (Vec<String>, Vec<String>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url(host, path), "label": "seed" })];
    let mut engine = Attached::start(&env);

    let mut answers = Vec::new();
    let report = crawl_under(
        &mut engine,
        "scoped",
        &seeds,
        settings,
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap();
            if !url.starts_with("http://") {
                return json!({ "result": { "records": [] } });
            }
            let page = fetch(&proving_ground, url);
            assert_eq!(page.status, 200, "{url}");
            if handle["crawl_request"]["label"] == "seed" {
                for href in attributes(&page.body, "a href") {
                    let answer = inside.enqueue(handle, &href, "page", None);
                    answers.push(
                        answer
                            .get("reason")
                            .unwrap_or(&answer["kind"])
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    );
                }
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");

    let mut hosts: Vec<String> = observations["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| request["host"].as_str().unwrap().to_owned())
        .collect();
    hosts.sort();
    hosts.dedup();
    (answers, hosts)
}

fn strings(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

/// The index links, in order, `/about` on its own host, the `www` host's
/// `/news`, the subdomain's `/cart`, `/account` over `https`, the sister
/// domain's `/sister` and the off-site `/offsite`.
///
/// Covers: story:6-224, story:6-226, setting:crawl.scope.strategy, operation:crawl_request.enqueue
#[rstest]
#[case::same_hostname(
    "same_hostname",
    &["added", "added", "out_of_scope", "added", "out_of_scope", "out_of_scope"],
    &["scope.test", "www.scope.test"],
)]
#[case::same_domain(
    "same_domain",
    &["added", "added", "added", "added", "out_of_scope", "out_of_scope"],
    &["scope.test", "shop.scope.test", "www.scope.test"],
)]
#[case::same_origin(
    "same_origin",
    &["added", "added", "out_of_scope", "added", "out_of_scope", "out_of_scope"],
    &["scope.test", "www.scope.test"],
)]
#[case::all(
    "all",
    &["added", "added", "added", "added", "added", "added"],
    &["elsewhere.test", "scope-sister.test", "scope.test", "shop.scope.test", "www.scope.test"],
)]
fn a_crawl_follows_only_the_links_its_scope_strategy_reaches(
    #[case] strategy: &str,
    #[case] answers: &[&str],
    #[case] hosts: &[&str],
) {
    let crawled = crawl_scope(
        "scope.test",
        "/",
        &json!({ "scope": { "strategy": strategy } }),
    );

    assert_eq!(crawled, (strings(answers), strings(hosts)));
}

/// `crawl.scope.include` narrows the default `same_hostname` strategy and
/// never widens it: the off-site link it matches stays out of scope;
/// `crawl.scope.exclude` takes out a link the strategy reaches.
///
/// Covers: story:6-225, setting:crawl.scope.include, setting:crawl.scope.exclude
#[rstest]
#[case::include(
    json!({ "include": ["**/about", "**/offsite"] }),
    &["added", "out_of_scope", "out_of_scope", "out_of_scope", "out_of_scope", "out_of_scope"],
    &["scope.test"],
)]
#[case::exclude(
    json!({ "exclude": ["**/about"] }),
    &["out_of_scope", "added", "out_of_scope", "added", "out_of_scope", "out_of_scope"],
    &["scope.test", "www.scope.test"],
)]
fn the_scopes_patterns_narrow_its_strategy_and_never_widen_it(
    #[case] scope: Value,
    #[case] answers: &[&str],
    #[case] hosts: &[&str],
) {
    let crawled = crawl_scope("scope.test", "/", &json!({ "scope": scope }));

    assert_eq!(crawled, (strings(answers), strings(hosts)));
}

/// A `www.scope.test` seed admits `scope.test`'s pages under the default
/// `same_hostname`, while its subdomain stays out (ADR 0120).
///
/// Covers: story:6-226, setting:crawl.scope.strategy
#[test]
fn a_www_seed_admits_its_bare_host_and_no_subdomain() {
    let crawled = crawl_scope("www.scope.test", "/news", &json!({}));

    assert_eq!(
        crawled,
        (
            strings(&["added", "out_of_scope"]),
            strings(&["scope.test", "www.scope.test"])
        )
    );
}

/// A `crawl.scope.include` pattern the engine cannot match fails the start
/// with `pattern_unsupported`, before the Run begins.
///
/// Covers: story:6-225, setting:crawl.scope.include, error:pattern_unsupported
#[test]
fn a_scope_pattern_the_engine_cannot_match_fails_the_start() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    let answer = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "unmatched",
            "seeds": [{ "url": "https://shop.test/", "label": "page" }],
            "settings": { "crawl": { "scope": { "include": ["https://shop.test/{a,{b,c}}"] } } },
        }),
        &mut |_| None,
    );
    assert!(engine.finish());

    let begun = env
        .home()
        .join("0.0")
        .join("runs")
        .join("unmatched")
        .exists();
    assert_eq!(
        (
            &answer["error"]["data"]["code"],
            &answer["error"]["data"]["construct"],
            begun
        ),
        (
            &json!("pattern_unsupported"),
            &json!("nested_braces"),
            false
        )
    );
}
