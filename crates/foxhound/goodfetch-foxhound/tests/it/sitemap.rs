//! Crawl conformance cases for the sitemap seed (spec #581's seam: the
//! protocol against the Proving ground; ADRs 0093 and 0120): crawls on
//! Foxhound's executable over stdio of the Proving ground's `sitemap-seed`
//! family. A sitemap index's two sitemaps seed their entries, each with the
//! Label of the first rule matching it, one matching none counted
//! `no_rule` and one outside the scope `out_of_scope` in its sitemap's
//! seed's counts in the Run's report; a robots.txt seed admits its own
//! host's entries from the sitemap it cross-submits on another host, and
//! no other host's (pitfall U20); a sitemap's own fetches go through the
//! domain policy, Politeness and robots.txt and run no handler; and gzip
//! and plain sitemaps seed alike, while one past sitemaps.org's limits, a
//! decompression bomb, broken XML, a 500 and a 404 are each reported with
//! its URL and why and never stop the rest (pitfall U19); and an entry
//! robots.txt disallows ends `skipped` with `robots_txt`, unsent and
//! unhandled (ADR 0086).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::before_their_turn;
use crate::frontier::{added, crawl_declaring, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 903;

/// The site whose sitemaps the seeds read.
const SITE: &str = "sitemap-seed.test";

/// The host the site's robots.txt cross-submits a sitemap on.
const PARTNER: &str = "sitemap-partner.test";

/// The host the Politeness case's domain policy refuses.
const BLOCKED: &str = "sitemap-blocked.test";

/// The `sitemaps` family's host, whose index lists 50,048 sitemaps.
const OVER_LIMIT: &str = "sitemaps.test";

/// The delay between starts of the Politeness case: long enough that two
/// requests of one domain sent without it would arrive closer.
const DELAY_MILLISECONDS: u64 = 1_000;

/// The settings of a crawl of the family's hosts on `proving_ground`, with
/// `more` merged in.
fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let ip = proving_ground.address().ip().to_string();
    let mut settings = json!({
        "network": {
            "resolve": { SITE: [&ip], PARTNER: [&ip], BLOCKED: [&ip], OVER_LIMIT: [&ip] },
        },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    // The Proving ground serves on loopback: its hosts are listed beside
    // any egress setting `more` gives.
    settings["egress"]["private_ranges"] = json!([SITE, PARTNER, BLOCKED, OVER_LIMIT]);
    settings
}

/// What a crawl of the sitemap seed `seed` under `more` settings gave: the
/// Run's report and its last progress, each seed's answer, each crawl
/// request its handler was given (its Label and URL, sorted), each
/// outcome's URL, kind and skip reason (sorted) and the Proving ground's
/// observations.
struct Crawled {
    report: Value,
    progress: Value,
    seeds: Value,
    handled: Vec<(String, String)>,
    outcomes: Vec<(String, Value, Value)>,
    observations: Value,
}

fn crawl_sitemap(proving_ground: &ProvingGround, seed: &Value, more: &Value) -> Crawled {
    let env = TestEnv::new();
    let mut engine = crate::site::engine_on(&env, proving_ground);
    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        "sitemap",
        std::slice::from_ref(seed),
        &settings(proving_ground, more),
        &mut |_, handle: &Value| {
            let request = &handle["crawl_request"];
            handled.push((
                request["label"].as_str().unwrap().to_owned(),
                request["url"].as_str().unwrap().to_owned(),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    let seeds = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": "sitemap" }),
        &mut |_| None,
    );
    let progress = engine.call(
        "progress",
        "run.progress",
        &json!({ "run_id": "sitemap" }),
        &mut |_| None,
    );
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "sitemap" }),
        &mut |_| None,
    );
    let mut outcomes: Vec<(String, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    handled.sort();
    Crawled {
        report: report["result"].clone(),
        progress: progress["result"].clone(),
        seeds: seeds["result"]["seeds"].clone(),
        handled,
        outcomes,
        observations,
    }
}

/// The targets `host` received, in arrival order, with each arrival in
/// microseconds.
fn received(observations: &Value, host: &str) -> Vec<(String, u64)> {
    let mut received: Vec<(String, u64)> = ReceivedRequests::of(observations)
        .every()
        .filter(|request| request.host == host)
        .map(|request| {
            (
                request.target.as_str().to_owned(),
                request.timing.arrived_microseconds,
            )
        })
        .collect();
    received.sort_by_key(|(_, at)| *at);
    received
}

fn targets(observations: &Value, host: &str) -> Vec<String> {
    let mut targets: Vec<String> = received(observations, host)
        .into_iter()
        .map(|(target, _)| target)
        .collect();
    targets.sort();
    targets
}

/// A sitemap seed's counts as the Run's report and progress give them:
/// the sitemaps it read, each it could not with its URL and why, and its
/// entries added and not added, by reason.
fn counts(
    seed: &str,
    read: u64,
    failures: &[Value],
    added: u64,
    not_added: &[(&str, u64)],
) -> Value {
    json!({
        "seed": seed,
        "read": read,
        "failed": failures.len(),
        "failures": failures,
        "added": added,
        "not_added": not_added.iter().map(|(_, count)| count).sum::<u64>(),
        "not_added_by_reason": not_added
            .iter()
            .map(|(name, count)| json!({ "name": name, "count": count }))
            .collect::<Vec<_>>(),
    })
}

/// A sitemap a seed could not read, with the Error code, skip reason or
/// refusal, the status when one answered, and why.
fn failure(url: &str, code: &str, status: Option<u16>, message: &str) -> Value {
    let mut failure = json!({ "url": url, "code": code, "message": message });
    if let Some(status) = status {
        failure["status"] = json!(status);
    }
    failure
}

fn handled(requests: &[(&str, &str)]) -> Vec<(String, String)> {
    requests
        .iter()
        .map(|(label, url)| ((*label).to_owned(), (*url).to_owned()))
        .collect()
}

/// A sitemap index of two sitemaps seeds every entry the scope admits,
/// each with the Label of the first rule matching its URL, an entry
/// resolved against its sitemap's URL; the sitemaps' fetches run no
/// handler; an entry no rule matches is counted `no_rule`, one outside the
/// scope `out_of_scope` and never fetched, and one listed twice
/// `duplicate`, each in its sitemap's counts.
///
/// Covers: story:6-203, story:6-205, operation:crawl.start, operation:run.seeds
#[test]
fn a_sitemap_index_seeds_its_entries_with_the_first_matching_rules_label() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |host: &str, path: &str| proving_ground.url(host, path);
    let index = url(SITE, "/sitemap-index.xml");
    let seed = json!({
        "sitemap": index,
        "rules": [
            { "pattern": "**/books/*", "label": "book" },
            { "pattern": "**/books/**", "label": "shelf" },
            { "pattern": "**/authors/*", "label": "author" },
        ],
    });

    let crawled = crawl_sitemap(&proving_ground, &seed, &json!({}));

    assert_eq!(
        (
            crawled.seeds,
            crawled.handled,
            crawled.report["sitemaps"].clone(),
            targets(&crawled.observations, PARTNER),
        ),
        (
            json!([added("1", &index)]),
            handled(&[
                ("author", &url(SITE, "/authors/1")),
                ("author", &url(SITE, "/authors/2")),
                ("book", &url(SITE, "/books/1")),
                ("book", &url(SITE, "/books/2")),
                ("book", &url(SITE, "/books/3")),
            ]),
            json!([counts(
                &index,
                3,
                &[],
                5,
                &[("duplicate", 1), ("no_rule", 1), ("out_of_scope", 1)]
            )]),
            Vec::<String>::new(),
        )
    );
}

/// A sitemap index's sitemaps are nested one level under their seed: with
/// `crawl.max_sitemap_nesting` at 0 neither of the index's two is fetched,
/// the seed's report keeps each with `max_sitemap_nesting`, no entry is
/// seeded and the Run ends `incomplete`, the bound named.
///
/// Covers: story:6-204, setting:crawl.max_sitemap_nesting
#[test]
fn a_sitemap_nested_deeper_than_crawl_max_sitemap_nesting_is_not_fetched() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seed = json!({
        "sitemap": proving_ground.url(SITE, "/sitemap-index.xml"),
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });

    let crawled = crawl_sitemap(
        &proving_ground,
        &seed,
        &json!({ "crawl": { "max_sitemap_nesting": 0 } }),
    );

    let sitemaps = &crawled.report["sitemaps"][0];
    let refused: Vec<&Value> = sitemaps["failures"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|failure| &failure["code"])
        .collect();
    let reasons: Vec<&Value> = crawled.report["completeness_reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|reason| &reason["code"])
        .collect();
    assert_eq!(
        (
            crawled.handled,
            &sitemaps["read"],
            refused,
            &crawled.report["completeness"],
            reasons,
            targets(&crawled.observations, SITE),
        ),
        (
            Vec::new(),
            &json!(1),
            vec![&json!("max_sitemap_nesting"), &json!("max_sitemap_nesting")],
            &json!("incomplete"),
            vec![&json!("crawl.max_sitemap_nesting")],
            vec!["/robots.txt".to_owned(), "/sitemap-index.xml".to_owned()],
        )
    );
}

/// A seed naming a site's robots.txt reads the sitemap its `Sitemap:` line
/// cross-submits on another host: it admits the site's own entries from
/// it, and the other host's are `out_of_scope`, so the partner's sitemap
/// never widens the crawl; the robots.txt is read once, as the copy the
/// sitemap's fetch read. With `site_preferences.respect_robots_txt` off,
/// the seed's robots.txt is still read for its `Sitemap:` lines, which the
/// crawl asked for, and the partner's robots.txt is never fetched (ADR
/// 0138).
///
/// Covers: story:6-205, story:6-203, operation:crawl.start
#[rstest]
#[case::respected(true, &["/cross.xml", "/robots.txt"])]
/// Covers: setting:site_preferences.respect_robots_txt
#[case::not_respected(false, &["/cross.xml"])]
fn a_robots_txt_seed_admits_its_own_hosts_entries_from_the_sitemap_it_cross_submits(
    #[case] respected: bool,
    #[case] partner_received: &[&str],
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |host: &str, path: &str| proving_ground.url(host, path);
    let robots = url(SITE, "/robots.txt");
    let seed = json!({
        "sitemap": robots,
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });

    let crawled = crawl_sitemap(
        &proving_ground,
        &seed,
        &json!({ "site_preferences": { "respect_robots_txt": respected } }),
    );

    assert_eq!(
        (
            crawled.handled,
            crawled.report["sitemaps"].clone(),
            targets(&crawled.observations, SITE),
            targets(&crawled.observations, PARTNER),
        ),
        (
            handled(&[("book", &url(SITE, "/books/4"))]),
            json!([counts(&robots, 1, &[], 1, &[("out_of_scope", 1)])]),
            vec!["/books/4".to_owned(), "/robots.txt".to_owned()],
            partner_received
                .iter()
                .map(|&target| target.to_owned())
                .collect::<Vec<_>>(),
        )
    );
}

/// A sitemap's own fetches, a nested one's too, are requests of their
/// domain: a sitemap whose host the domain policy refuses is never
/// fetched, one robots.txt disallows is never sent, and every request the
/// site receives, the sitemaps' and the pages' alike, waits its turn under
/// `politeness.domain.delay` after robots.txt; no handler is given a sitemap.
///
/// Covers: story:6-204, setting:politeness.domain.delay, setting:egress.blocked_domains, setting:fetch.tier
#[test]
fn a_sitemaps_own_fetches_obey_the_domain_policy_politeness_and_robots_txt() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |host: &str, path: &str| proving_ground.url(host, path);
    let index = url(SITE, "/polite-index.xml");
    let seed = json!({
        "sitemap": index,
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });
    let more = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "delay": DELAY_MILLISECONDS } },
        "egress": { "blocked_domains": [BLOCKED] },
    });

    let crawled = crawl_sitemap(&proving_ground, &seed, &more);

    let arrivals = received(&crawled.observations, SITE);
    let too_soon = before_their_turn(&arrivals, DELAY_MILLISECONDS * 1_000);
    assert_eq!(
        (
            crawled.handled,
            crawled.report["sitemaps"].clone(),
            arrivals.first().map(|(target, _)| target.as_str()),
            targets(&crawled.observations, SITE),
            targets(&crawled.observations, BLOCKED),
            too_soon,
        ),
        (
            handled(&[
                ("book", &url(SITE, "/books/1")),
                ("book", &url(SITE, "/books/3")),
            ]),
            json!([counts(
                &index,
                2,
                &[
                    failure(
                        &url(BLOCKED, "/sitemap.xml"),
                        "domain_policy",
                        None,
                        &format!(
                            "The Run's domain policy refuses the host of the sitemap {}, so it was not fetched.",
                            url(BLOCKED, "/sitemap.xml")
                        ),
                    ),
                    failure(
                        &url(SITE, "/private/hidden.xml"),
                        "robots_txt",
                        None,
                        &format!(
                            "The site's robots.txt disallows the sitemap {}, so it was not fetched.",
                            url(SITE, "/private/hidden.xml")
                        ),
                    ),
                ],
                2,
                &[("no_rule", 1)]
            )]),
            Some("/robots.txt"),
            [
                "/books/1",
                "/books/3",
                "/polite-index.xml",
                "/robots.txt",
                "/sitemaps/more.xml",
            ]
            .map(str::to_owned)
            .to_vec(),
            Vec::<String>::new(),
            Vec::<String>::new(),
        )
    );
}

/// A sitemap index past sitemaps.org's 50,000 URLs is not read: its seed's
/// report keeps it with why, and none of the sitemaps it lists is fetched.
///
/// Covers: story:6-207
#[test]
fn a_sitemap_index_past_fifty_thousand_sitemaps_is_reported_unread() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let index = proving_ground.url(OVER_LIMIT, "/sitemap-index.xml");
    let seed = json!({
        "sitemap": index,
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });

    let crawled = crawl_sitemap(
        &proving_ground,
        &seed,
        &json!({ "crawl": { "retry": { "max": 0 } } }),
    );

    assert_eq!(
        (
            crawled.report["sitemaps"].clone(),
            targets(&crawled.observations, OVER_LIMIT),
        ),
        (
            json!([counts(
                &index,
                0,
                &[failure(
                    &index,
                    "response_not_ok",
                    None,
                    &format!(
                        "The sitemap {index} could not be read: it lists more than sitemaps.org's 50,000 URLs."
                    ),
                )],
                0,
                &[],
            )]),
            vec!["/robots.txt".to_owned(), "/sitemap-index.xml".to_owned()],
        )
    );
}

/// Sitemaps are read plain and gzip-compressed alike, and one past
/// sitemaps.org's 50 MB uncompressed, a decompression bomb, broken XML, a
/// 500 and a 404 are each reported with its URL and why, as is one the
/// index names on another host, which is not fetched, in the report and
/// the progress alike, while the other sitemaps still seed (pitfall U19):
/// the seed's counts give the sitemaps read, those that failed, with
/// their status, and the entries added, never listing one.
///
/// Covers: story:6-206, story:6-207, setting:network.max_decompression_ratio, operation:run.progress
#[test]
fn a_gzip_and_a_plain_sitemap_seed_and_a_bad_one_never_stops_the_rest() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |host: &str, path: &str| proving_ground.url(host, path);
    let index = url(SITE, "/limits-index.xml");
    let seed = json!({
        "sitemap": index,
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });
    let more = json!({ "crawl": { "retry": { "max": 0 } } });
    let unreadable = |at: &str, why: &str| {
        failure(
            at,
            "response_not_ok",
            None,
            &format!("The sitemap {at} could not be read: {why}."),
        )
    };
    let elsewhere = url(OVER_LIMIT, "/sitemap-index.xml");
    let bomb = url(SITE, "/sitemaps/bomb.xml.gz");
    let error = url(SITE, "/sitemaps/error.xml");
    let missing = url(SITE, "/sitemaps/missing.xml");

    let crawled = crawl_sitemap(&proving_ground, &seed, &more);

    let expected = json!([counts(
        &index,
        3,
        &[
            failure(
                &bomb,
                "response_limit_exceeded",
                None,
                &format!(
                    "the body of {bomb} decodes to more bytes per received byte than `network.max_decompression_ratio` allows (100)"
                ),
            ),
            unreadable(
                &url(SITE, "/sitemaps/broken.xml"),
                "its XML ends inside an element"
            ),
            failure(
                &error,
                "response_unavailable",
                Some(500),
                &format!("The sitemap {error} answered 500."),
            ),
            failure(
                &missing,
                "response_not_ok",
                Some(404),
                &format!("The sitemap {missing} answered 404, not a sitemap."),
            ),
            unreadable(
                &url(SITE, "/sitemaps/oversized.xml"),
                "it is larger than sitemaps.org's 50 MB (52,428,800 bytes) uncompressed",
            ),
            failure(
                &elsewhere,
                "out_of_scope",
                None,
                &format!(
                    "The sitemap {elsewhere} is on another host than its sitemap seed's and the sitemap's that names it, where only a robots.txt seed's own `Sitemap:` lines name one that is fetched, so it was not fetched."
                ),
            ),
        ],
        4,
        &[],
    )]);
    assert_eq!(
        (
            crawled.handled,
            crawled.report["sitemaps"].clone(),
            crawled.progress["sitemaps"].clone(),
        ),
        (
            handled(&[
                ("book", &url(SITE, "/books/11")),
                ("book", &url(SITE, "/books/12")),
                ("book", &url(SITE, "/books/13")),
                ("book", &url(SITE, "/books/14")),
            ]),
            expected.clone(),
            expected,
        )
    );
}

/// A sitemap entry robots.txt disallows to `goodfetch-foxhound` ends
/// `skipped` with `robots_txt`: it is never sent and no handler is run for
/// it, while its sitemap's other entry is fetched and handled.
///
/// Covers: story:6-239, setting:fetch.tier
#[test]
fn a_sitemap_entry_robots_txt_disallows_ends_skipped_unsent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |host: &str, path: &str| proving_ground.url(host, path);
    let seed = json!({
        "sitemap": url(SITE, "/sitemaps/private.xml"),
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });
    let more = json!({ "fetch": { "tier": "http" } });

    let crawled = crawl_sitemap(&proving_ground, &seed, &more);

    assert_eq!(
        (
            crawled.handled,
            crawled.outcomes,
            targets(&crawled.observations, SITE),
        ),
        (
            handled(&[("book", &url(SITE, "/books/20"))]),
            vec![
                (url(SITE, "/books/20"), json!("succeeded"), Value::Null),
                (
                    url(SITE, "/private/books/21"),
                    json!("skipped"),
                    json!("robots_txt"),
                ),
            ],
            ["/books/20", "/robots.txt", "/sitemaps/private.xml"]
                .map(str::to_owned)
                .to_vec(),
        )
    );
}

/// The request seed of the `run.add_seed` cases below: an article no
/// sitemap of the family lists.
const ADDING: &str = "/books/30";

/// The sitemap seed the `run.add_seed` cases below add: the plain sitemap
/// of `/books/11` and `/books/12`, each an entry for the Label `book`.
fn plain_sitemap(proving_ground: &ProvingGround) -> Value {
    json!({
        "sitemap": proving_ground.url(SITE, "/sitemaps/plain.xml"),
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    })
}

/// A sitemap seed added to a running crawl, `run.add_seed(sitemap(...))`,
/// is committed at once and answered as an enqueue is: the sitemap is read
/// as a start's sitemap seed is, its entries handled with their rule's
/// Label and counted in the Run's report (ticket #914, CO-03).
///
/// Covers: story:6-208, story:6-203, operation:run.add_seed
#[test]
fn a_sitemap_seed_added_to_a_running_crawl_is_committed_at_once_and_read() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |path: &str| proving_ground.url(SITE, path);
    let sitemap = plain_sitemap(&proving_ground);
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let mut answers = Vec::new();
    let mut handled = Vec::new();

    let report = crawl_declaring(
        &mut engine,
        "adding",
        &[json!({ "url": url(ADDING), "label": "page" })],
        &json!({ "page": {}, "book": {} }),
        &settings(&proving_ground, &json!({})),
        &mut |inside, handle| {
            let request = &handle["crawl_request"];
            handled.push((
                request["label"].as_str().unwrap().to_owned(),
                request["url"].as_str().unwrap().to_owned(),
            ));
            if request["url"] == url(ADDING) {
                let reply = inside.call(
                    "run.add_seed",
                    &json!({ "run_id": "adding", "seed": sitemap }),
                );
                answers.push(reply.get("result").unwrap_or(&reply).clone());
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    handled.sort();

    assert_eq!(
        (
            report["result"]["lifecycle"].clone(),
            answers,
            handled,
            report["result"]["sitemaps"].clone(),
        ),
        (
            json!("finished"),
            vec![added("2", &url("/sitemaps/plain.xml"))],
            crate::sitemap::handled(&[
                ("book", &url("/books/11")),
                ("book", &url("/books/12")),
                ("page", &url(ADDING)),
            ]),
            json!([counts(&url("/sitemaps/plain.xml"), 1, &[], 2, &[])]),
        ),
        "{report}"
    );
}

/// A sitemap seed a handler adds through its context, `ctx.run.addSeed`,
/// is staged with its attempt: the attempt that fails leaves nothing of it,
/// so the sitemap is not read for it, and the retried attempt's own seed
/// commits with that attempt, its entries each handled once (ticket #914,
/// ADR 0093).
///
/// Covers: story:6-208, story:6-236, operation:run.add_seed
#[test]
fn a_sitemap_seed_staged_by_a_failed_attempt_is_discarded_and_its_retry_commits_its_own() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |path: &str| proving_ground.url(SITE, path);
    let sitemap = plain_sitemap(&proving_ground);
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let mut answers = Vec::new();
    let mut handled = Vec::new();

    let report = crawl_declaring(
        &mut engine,
        "staged",
        &[json!({ "url": url(ADDING), "label": "page" })],
        &json!({ "page": {}, "book": {} }),
        &settings(
            &proving_ground,
            &json!({ "crawl": { "retry": { "max": 2 } } }),
        ),
        &mut |inside, handle| {
            let request = &handle["crawl_request"];
            handled.push((
                request["label"].as_str().unwrap().to_owned(),
                request["url"].as_str().unwrap().to_owned(),
            ));
            if request["url"] != url(ADDING) {
                return json!({ "result": { "records": [] } });
            }
            let reply = inside.call(
                "run.add_seed",
                &json!({
                    "run_id": "staged",
                    "seed": sitemap,
                    "attempt": {
                        "crawl_request_id": request["id"],
                        "lease_epoch": handle["lease_epoch"],
                    },
                }),
            );
            answers.push(reply.get("result").unwrap_or(&reply).clone());
            if handle["attempt"] == 1 {
                json!({ "error": { "code": -32003, "message": "try again", "data": { "code": "retry_requested", "retryable": true } } })
            } else {
                json!({ "result": { "records": [] } })
            }
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());
    handled.sort();

    let sitemap_url = url("/sitemaps/plain.xml");
    assert_eq!(
        (
            report["result"]["lifecycle"].clone(),
            answers
                .iter()
                .map(|answer| answer["kind"].clone())
                .collect::<Vec<_>>(),
            handled,
            report["result"]["sitemaps"].clone(),
            targets(&observations, SITE)
                .iter()
                .filter(|target| target.as_str() == "/sitemaps/plain.xml")
                .count(),
        ),
        (
            json!("finished"),
            vec![json!("added"), json!("added")],
            crate::sitemap::handled(&[
                ("book", &url("/books/11")),
                ("book", &url("/books/12")),
                ("page", &url(ADDING)),
                ("page", &url(ADDING)),
            ]),
            json!([counts(&sitemap_url, 1, &[], 2, &[])]),
            1,
        ),
        "{report}\n{answers:?}"
    );
}
