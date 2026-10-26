//! Code-free crawl cases (spec #582; ADR 0103), on Foxhound's executable
//! over stdio against the Proving ground: a `crawl.start` with no handler
//! takes `extraction`, and the engine handles every request itself,
//! committing one Record per page into the Dataset `pages`, keyed by
//! `url`, whose schema it generates from the formats request; the program
//! is never sent a `crawl_request.handle`. `follow` chooses the pages
//! fetched beyond the seeds: every in-scope link (`scope`), the entries of
//! the sitemaps each seed's site names (`sitemap`) or none.

use std::collections::BTreeSet;

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::fetch_tiers::settings;

/// The Proving ground's seed.
const SEED: u64 = 994;

/// The Markdown task, whose index links every Markdown page and whose
/// oracle judges each page's Markdown.
const MARKDOWN_TASK: &str = "extraction/7";

/// How many Markdown pages the task's index links.
const MARKDOWN_PAGES: usize = 31;

/// The site whose robots.txt names a sitemap on its partner host.
const SITEMAP_SITE: &str = "sitemap-seed.test";

/// The fields every Record of a Markdown crawl holds.
const FIELDS: [&str; 6] = [
    "base_url",
    "fetchedAt",
    "main_content",
    "markdown",
    "status",
    "url",
];

/// The path of the URL `url`.
fn path_of(url: &str) -> String {
    let host = url.find("://").map_or(0, |at| at + "://".len());
    url[host..]
        .find('/')
        .map_or_else(String::new, |at| url[host + at..].to_owned())
}

/// Starts the code-free crawl `run_id` of `seeds` reading `extraction`
/// under `more` settings, waits for its Run to end, and answers the
/// start's reply and every Record of `pages`; the engine must never send
/// the program a handler request.
fn crawl(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    (run_id, seeds): (&str, &[Value]),
    extraction: &Value,
    more: &Value,
) -> (Value, Vec<Value>) {
    let mut more = more.clone();
    more["politeness"] = json!({ "domain": { "delay": 0 } });
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "datasets": [],
            "extraction": extraction,
            "settings": settings(proving_ground, &more),
        }),
        &mut |handle| panic!("a code-free crawl sent its program {handle}"),
    );
    if started.get("error").is_some() {
        return (started, Vec::new());
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records",
        "params": { "run_id": run_id, "dataset": "pages" },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "records", "credits": 1000 },
    }));
    let mut records = Vec::new();
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the reader answered");
        assert_ne!(
            message["method"], "crawl_request.handle",
            "a code-free crawl sent its program {message}"
        );
        if message["method"] == "$/record" {
            records.push(message["params"]["record"]["value"].clone());
        } else if message["id"] == "records" {
            assert!(message.get("result").is_some(), "{message}");
            return (started, records);
        }
    }
}

/// The paths of `records`' URLs, sorted.
fn paths(records: &[Value]) -> Vec<String> {
    let mut paths: Vec<String> = records
        .iter()
        .map(|record| path_of(record["url"].as_str().unwrap()))
        .collect();
    paths.sort();
    paths
}

/// A crawl from the Markdown task's index with `formats: { markdown }` and
/// the built-in `follow: scope`, one link deep, fetches the index and
/// every in-scope page it links, with no code: `pages` holds one Record
/// per page, keyed by its URL, each
/// with its status, when it was fetched and its Markdown, and nothing
/// else; every Record validated against the schema the engine generated
/// from the request as it committed, and the oracle verifies each page's
/// Markdown.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[test]
fn a_crawl_with_no_code_fills_pages_keyed_by_url_with_each_pages_markdown() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("markdown", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": { "markdown": true } }),
        &json!({ "fetch": { "tier": "http" }, "crawl": { "max_depth": 1 } }),
    );
    assert!(engine.finish());

    let index_path = path_of(&index);
    let judged: Vec<Value> = records
        .iter()
        .filter(|record| path_of(record["url"].as_str().unwrap()) != index_path)
        .map(|record| {
            json!({ "id": path_of(record["url"].as_str().unwrap()), "markdown": record["markdown"] })
        })
        .collect();
    let verdict = proving_ground
        .judge_records(MARKDOWN_TASK, "success", &judged)
        .unwrap();
    let urls: BTreeSet<&str> = records
        .iter()
        .map(|record| record["url"].as_str().unwrap())
        .collect();
    let shapes: BTreeSet<(Vec<String>, Option<u64>, bool)> = records
        .iter()
        .map(|record| {
            (
                record.as_object().unwrap().keys().cloned().collect(),
                record["status"].as_u64(),
                record["fetchedAt"]
                    .as_str()
                    .is_some_and(|at| at.ends_with('Z') && at.contains('T')),
            )
        })
        .collect();
    assert_eq!(
        (
            started["result"]["run_id"].clone(),
            records.len(),
            urls.len(),
            urls.contains(index.as_str()),
            shapes,
            &verdict["outcome"],
        ),
        (
            json!("markdown"),
            MARKDOWN_PAGES + 1,
            MARKDOWN_PAGES + 1,
            true,
            BTreeSet::from([(
                FIELDS.iter().map(|field| (*field).to_owned()).collect(),
                Some(200),
                true
            )]),
            &json!("verified"),
        ),
        "{verdict}"
    );
}

/// `follow: none` fetches only the seeds: the index's links are never
/// requested, and `pages` holds the seeds' Records alone.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn follow_none_fetches_only_the_seeds() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let page = format!("{index}/headings/levels");

    let (_, records) = crawl(
        &mut engine,
        &proving_ground,
        (
            "seeds-only",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": page, "label": "page" }),
            ],
        ),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finish());

    let observations = proving_ground.observations().unwrap();
    let mut received: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request["target"] != "/robots.txt")
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .collect();
    received.sort();
    let mut seeds = vec![path_of(&index), path_of(&page)];
    seeds.sort();
    assert_eq!((paths(&records), received), (seeds.clone(), seeds));
}

/// `follow: sitemap` reads the sitemaps each seed's site names in its
/// robots.txt, here one its partner host holds: the seed and the site's
/// own entry are fetched, each a Record, and the partner's entry, outside
/// the scope, is not.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn follow_sitemap_reads_the_sitemap_the_sites_robots_txt_names() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);
    let about = proving_ground.url(SITEMAP_SITE, "/about");

    let (_, records) = crawl(
        &mut engine,
        &proving_ground,
        ("sitemap", &[json!({ "url": about, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "sitemap" }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finish());

    let hosts: BTreeSet<String> = records
        .iter()
        .map(|record| {
            let url = record["url"].as_str().unwrap();
            url.split("://")
                .nth(1)
                .unwrap()
                .split(':')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        (paths(&records), hosts),
        (
            vec!["/about".to_owned(), "/books/4".to_owned()],
            BTreeSet::from([SITEMAP_SITE.to_owned()])
        )
    );
}

/// A code-free crawl declares its own Dataset: a start that also declares
/// one is refused with `invalid_params` before its Run begins.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn a_code_free_start_that_declares_a_dataset_is_refused() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "declared",
            "seeds": [{ "url": "http://example.test/", "label": "page" }],
            "datasets": [{ "name": "pages" }],
            "extraction": { "formats": { "markdown": true } },
        }),
        &mut |handle| panic!("a refused crawl sent its program {handle}"),
    );
    assert!(engine.finish());
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("invalid_params"),
        "{refused}"
    );
}
