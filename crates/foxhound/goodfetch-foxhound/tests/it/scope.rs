//! A crawl's scope on Foxhound's executable over stdio (spec #581's seam:
//! the protocol against the Proving ground; ADR 0094): the Proving
//! ground's `scope` site, with a `www` host, a subdomain, an `https` page,
//! a sister domain and an off-site link, crawled under each
//! `crawl.scope.strategy`; `crawl.scope.include` never widening the
//! strategy and `crawl.scope.exclude` narrowing it; a `www` seed and its
//! bare host; and a pattern the engine cannot match refused at the start.
//! Each link outside the scope is answered `out_of_scope`, and the Proving
//! ground receives no request off the site it was given (pitfall U15).
//! The domain policy's rows (ADRs 0093 and 0120): a seed, an added seed and
//! an enqueue whose host `egress.allowed_domains` or
//! `egress.blocked_domains` refuses are answered `domain_policy`, after the
//! scope, and never fetched; the scope alone ignores a leading `www.`; and
//! an entry that is no host fails the start.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::{Inside, added, attributes, crawl_with, fetch, not_added};

/// The Proving ground's seed.
const SEED: u64 = 840;

/// Crawls the seed `path` on `host` under the `crawl` settings `settings`: its handler fetches
/// each `http` page it is given (it speaks no TLS) and, on the seed's
/// page, enqueues every link it holds. Answers each enqueue's answer, as
/// `added` or the reason it was not, and the hosts the Proving ground
/// received, each once, sorted.
fn crawl_scope(host: &str, path: &str, settings: &Value) -> (Vec<String>, Vec<String>) {
    crawl_scope_with(host, path, &json!({ "crawl": settings }))
}

/// [`crawl_scope`] under the Run's settings `settings`, every key and not
/// only `crawl`'s.
fn crawl_scope_with(host: &str, path: &str, settings: &Value) -> (Vec<String>, Vec<String>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url(host, path), "label": "seed" })];
    let mut engine = attached(&env);

    let mut answers = Vec::new();
    let report = crawl_with(
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
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");

    let mut hosts: Vec<String> = ReceivedRequests::of(&observations)
        .every()
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

/// An `egress.blocked_domains` entry that is no host fails the start with
/// `setting_invalid`, naming the key, before the Run begins, so a crawl
/// never runs under a domain policy it could not read.
///
/// Covers: story:4-96, setting:egress.blocked_domains, error:setting_invalid
#[test]
fn a_domain_policy_entry_that_is_no_host_fails_the_start() {
    let env = TestEnv::new();
    let mut engine = attached(&env);

    let answer = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "unread",
            "seeds": [{ "url": "https://shop.test/", "label": "page" }],
            "settings": { "egress": { "blocked_domains": ["shop.test/cart"] } },
        }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let begun = HomeLayout::of(env.home()).run_folder("unread").exists();
    assert_eq!(
        (
            &answer["error"]["data"]["code"],
            &answer["error"]["data"]["key"],
            begun
        ),
        (
            &json!("setting_invalid"),
            &json!("egress.blocked_domains"),
            false
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
    let mut engine = attached(&env);

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
    assert!(engine.finished());

    let begun = HomeLayout::of(env.home()).run_folder("unmatched").exists();
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

/// The domain-policy rows of the crawl and URL identity conformance cases
/// (ADRs 0093 and 0120), pitfall U15's case among them: under the scope
/// strategy that reaches every link, a host the domain policy refuses is
/// answered `domain_policy` and never fetched, an allowed entry admitting
/// its subdomains; a link off the site and refused by the policy as well
/// stays `out_of_scope`, the scope judging first; and a `www` seed's bare
/// host, which the scope admits, is refused by a policy allowing the `www`
/// host alone, while its subdomain stays `out_of_scope`.
///
/// Covers: story:4-96, operation:crawl_request.enqueue
#[rstest]
/// Covers: setting:egress.blocked_domains
#[case::blocked_off_site_host(
    "scope.test",
    "/",
    json!({ "crawl": { "scope": { "strategy": "all" } }, "egress": { "blocked_domains": ["elsewhere.test"] } }),
    &["added", "added", "added", "added", "added", "domain_policy"],
    &["scope-sister.test", "scope.test", "shop.scope.test", "www.scope.test"],
)]
/// Covers: setting:egress.allowed_domains
#[case::allowed_site_and_its_subdomains(
    "scope.test",
    "/",
    json!({ "crawl": { "scope": { "strategy": "all" } }, "egress": { "allowed_domains": ["scope.test"] } }),
    &["added", "added", "added", "added", "domain_policy", "domain_policy"],
    &["scope.test", "shop.scope.test", "www.scope.test"],
)]
/// Covers: setting:egress.allowed_domains
#[case::scope_before_policy(
    "scope.test",
    "/",
    json!({ "egress": { "allowed_domains": ["scope.test"] } }),
    &["added", "added", "out_of_scope", "added", "out_of_scope", "out_of_scope"],
    &["scope.test", "www.scope.test"],
)]
/// Covers: story:4-95, story:6-226, setting:egress.allowed_domains
#[case::www_ignored_by_the_scope_alone(
    "www.scope.test",
    "/news",
    json!({ "egress": { "allowed_domains": ["www.scope.test"] } }),
    &["domain_policy", "out_of_scope"],
    &["www.scope.test"],
)]
fn a_link_the_domain_policy_refuses_is_not_added_after_the_scope_and_never_fetched(
    #[case] host: &str,
    #[case] path: &str,
    #[case] settings: Value,
    #[case] answers: &[&str],
    #[case] hosts: &[&str],
) {
    let crawled = crawl_scope_with(host, path, &settings);

    assert_eq!(crawled, (strings(answers), strings(hosts)));
}

/// A seed, a `run.add_seed` and a handler's enqueue whose host
/// `egress.blocked_domains` refuses are each answered "not added" with
/// `domain_policy`, and no handler is given them, so the Proving ground
/// receives no request for the refused host and its connection record
/// holds only the connections of the allowed pages' requests; an
/// allowed seed, an allowed added seed and an allowed link are added
/// (positive controls).
///
/// Covers: story:4-96, setting:egress.blocked_domains, operation:crawl.start, operation:run.seeds, operation:run.add_seed, operation:crawl_request.enqueue
#[test]
fn a_seed_an_added_seed_and_an_enqueue_the_domain_policy_refuses_are_not_added_and_never_fetched() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let site = proving_ground.url("scope.test", "/");
    let refused_seed = proving_ground.url("elsewhere.test", "/offsite");
    let refused_added = proving_ground.url("elsewhere.test", "/offsite?added");
    let allowed_added = proving_ground.url("scope.test", "/storage?mark=added");
    let seeds = [
        json!({ "url": site, "label": "seed" }),
        json!({ "url": refused_seed, "label": "page" }),
    ];
    let settings = json!({
        "crawl": { "scope": { "strategy": "all" } },
        "egress": { "blocked_domains": ["elsewhere.test"] },
    });
    let mut engine = attached(&env);

    let mut enqueued = Vec::new();
    let mut seeded = Vec::new();
    let report = crawl_with(
        &mut engine,
        "policed",
        &seeds,
        &settings,
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap();
            if !url.starts_with("http://") {
                return json!({ "result": { "records": [] } });
            }
            let page = fetch(&proving_ground, url);
            assert_eq!(page.status, 200, "{url}");
            if handle["crawl_request"]["label"] == "seed" {
                for href in attributes(&page.body, "a href") {
                    enqueued.push(inside.enqueue(handle, &href, "page", None));
                }
                for url in [&refused_added, &allowed_added] {
                    let reply = inside.call(
                        "run.add_seed",
                        &json!({ "run_id": "policed", "seed": { "url": url, "label": "page" } }),
                    );
                    seeded.push(reply["result"].clone());
                }
            }
            json!({ "result": { "records": [] } })
        },
    );
    let answers = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": "policed" }),
        &mut |_| None,
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let requests = ReceivedRequests::of(&observations);
    let refused_hosts = requests
        .every()
        .filter(|request| request["host"] == "elsewhere.test")
        .count();
    // Every connection but the observations' own read carried one request.
    let connections = observations["connections"].as_array().unwrap().len() - 1;
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &answers["result"]["seeds"],
            enqueued.last(),
            &seeded,
            refused_hosts,
            connections,
        ),
        (
            &json!("finished"),
            &json!([added("1", &site), not_added("domain_policy", &refused_seed)]),
            Some(&not_added("domain_policy", &refused_seed)),
            &vec![
                not_added("domain_policy", &refused_added),
                added("7", &allowed_added),
            ],
            0,
            requests.every().count(),
        )
    );
}
