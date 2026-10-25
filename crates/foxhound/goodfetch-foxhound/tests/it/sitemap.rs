//! Crawl conformance cases for the sitemap seed (spec #581's seam: the
//! protocol against the Proving ground; ADRs 0093 and 0120): crawls on
//! Foxhound's executable over stdio of the Proving ground's `sitemap-seed`
//! family. A sitemap index's two sitemaps seed their entries, each with the
//! Label of the first rule matching it, one matching none counted
//! `no_rule` and one outside the scope `out_of_scope` in its sitemap's
//! counts in the Run's report; a robots.txt seed admits its own host's
//! entries from the sitemap it cross-submits on another host, and no other
//! host's (pitfall U20); and a sitemap's own fetches go through the domain
//! policy, Politeness and robots.txt and run no handler.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{added, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 903;

/// The site whose sitemaps the seeds read.
const SITE: &str = "sitemap-seed.test";

/// The host the site's robots.txt cross-submits a sitemap on.
const PARTNER: &str = "sitemap-partner.test";

/// The host the Politeness case's domain policy refuses.
const BLOCKED: &str = "sitemap-blocked.test";

/// The delay between starts of the Politeness case: long enough that two
/// requests of one domain sent without it would arrive closer.
const DELAY_MILLISECONDS: u64 = 1_000;

/// The settings of a crawl of the family's hosts on `proving_ground`, with
/// `more` merged in.
fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let ip = proving_ground.address().ip().to_string();
    let mut settings = json!({
        "network": { "resolve": { SITE: [&ip], PARTNER: [&ip], BLOCKED: [&ip] } },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    settings
}

/// What a crawl of the sitemap seed `seed` under `more` settings gave: the
/// Run's report, each seed's answer, each crawl request its handler was
/// given (its Label and URL, sorted) and the Proving ground's
/// observations.
struct Crawled {
    report: Value,
    seeds: Value,
    handled: Vec<(String, String)>,
    observations: Value,
}

fn crawl_sitemap(proving_ground: &ProvingGround, seed: &Value, more: &Value) -> Crawled {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
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
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    handled.sort();
    Crawled {
        report: report["result"].clone(),
        seeds: seeds["result"]["seeds"].clone(),
        handled,
        observations,
    }
}

/// The targets `host` received, in arrival order, with each arrival in
/// microseconds.
fn received(observations: &Value, host: &str) -> Vec<(String, u64)> {
    let mut received: Vec<(String, u64)> = ReceivedRequests::of(observations)
        .every()
        .filter(|request| request["host"] == host)
        .map(|request| {
            (
                request["target"].as_str().unwrap().to_owned(),
                request["timing"]["arrived_microseconds"].as_u64().unwrap(),
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

/// A sitemap's counts as the Run's report gives them.
fn counts(url: &str, added: u64, not_added: &[(&str, u64)]) -> Value {
    json!({
        "url": url,
        "added": added,
        "not_added": not_added.iter().map(|(_, count)| count).sum::<u64>(),
        "not_added_by_reason": not_added
            .iter()
            .map(|(name, count)| json!({ "name": name, "count": count }))
            .collect::<Vec<_>>(),
    })
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
            json!([
                counts(&index, 2, &[]),
                counts(
                    &url(SITE, "/sitemaps/books.xml"),
                    3,
                    &[("no_rule", 1), ("out_of_scope", 1)]
                ),
                counts(&url(SITE, "/sitemaps/more.xml"), 2, &[("duplicate", 1)]),
            ]),
            Vec::<String>::new(),
        )
    );
}

/// A seed naming a site's robots.txt reads the sitemap its `Sitemap:` line
/// cross-submits on another host: it admits the site's own entries from
/// it, and the other host's are `out_of_scope`, so the partner's sitemap
/// never widens the crawl; the robots.txt is read once, as the copy the
/// sitemap's fetch read.
///
/// Covers: story:6-205, story:6-203, operation:crawl.start
#[test]
fn a_robots_txt_seed_admits_its_own_hosts_entries_from_the_sitemap_it_cross_submits() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = |host: &str, path: &str| proving_ground.url(host, path);
    let robots = url(SITE, "/robots.txt");
    let seed = json!({
        "sitemap": robots,
        "rules": [{ "pattern": "**/books/*", "label": "book" }],
    });

    let crawled = crawl_sitemap(&proving_ground, &seed, &json!({}));

    assert_eq!(
        (
            crawled.handled,
            crawled.report["sitemaps"].clone(),
            targets(&crawled.observations, SITE),
            targets(&crawled.observations, PARTNER),
        ),
        (
            handled(&[("book", &url(SITE, "/books/4"))]),
            json!([
                counts(&robots, 1, &[]),
                counts(&url(PARTNER, "/cross.xml"), 1, &[("out_of_scope", 1)]),
            ]),
            vec!["/robots.txt".to_owned()],
            vec!["/cross.xml".to_owned(), "/robots.txt".to_owned()],
        )
    );
}

/// A sitemap's own fetches, a nested one's too, are requests of their
/// domain: a sitemap whose host the domain policy refuses is never
/// fetched, one robots.txt disallows is never sent, and every request the
/// site receives, the sitemaps' and the pages' alike, arrives at least
/// `politeness.domain.delay` after the one before; no handler is given a
/// sitemap.
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
    // The Proving ground stamps an arrival a little after the engine sent it.
    let least = DELAY_MILLISECONDS * 1_000 * 9 / 10;
    let too_soon: Vec<&String> = arrivals
        .windows(2)
        .filter(|pair| pair[1].1 - pair[0].1 < least)
        .map(|pair| &pair[1].0)
        .collect();
    assert_eq!(
        (
            crawled.handled,
            crawled.report["sitemaps"].clone(),
            targets(&crawled.observations, SITE),
            targets(&crawled.observations, BLOCKED),
            too_soon,
        ),
        (
            handled(&[
                ("book", &url(SITE, "/books/1")),
                ("book", &url(SITE, "/books/3")),
            ]),
            json!([
                counts(&index, 2, &[("domain_policy", 1)]),
                counts(&url(SITE, "/sitemaps/more.xml"), 2, &[("no_rule", 1)]),
            ]),
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
            Vec::<&String>::new(),
        )
    );
}
