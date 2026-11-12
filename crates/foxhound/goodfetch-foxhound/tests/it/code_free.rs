//! Code-free crawl cases (spec #582; ADR 0103), on Foxhound's executable
//! over stdio against the Proving ground: a `crawl.start` with no handler
//! takes `extraction`, and the engine handles every request itself,
//! committing one Record per page into the Dataset `pages`, keyed by
//! `url`, whose schema it generates from the formats request; the program
//! is never sent a `crawl_request.handle`. `follow` chooses the pages
//! fetched beyond the seeds: every in-scope link (`scope`), the entries of
//! the sitemaps each seed's site names (`sitemap`) or none. Named `fields`
//! are read by locators and given by Value transforms (ADR 0123), each
//! Record's Provenance naming each transform, and each field the page did
//! not give is null with its reason.

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ToolStatus;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, ReceivedRequests, TestEnv, browser_cache,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;
use crate::attached_with;
use crate::fetch_tiers::{settings, tiers};

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
    "fetched_at",
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
    run: (&str, &[Value]),
    extraction: &Value,
    more: &Value,
) -> (Value, Vec<Value>) {
    let (started, read) = crawl_read(engine, proving_ground, run, extraction, more);
    let values = read.iter().map(|record| record["value"].clone()).collect();
    (started, values)
}

/// As [`crawl`], answering each Record of `pages` as `run.records` reads
/// it: its value, its Provenance and the rest.
fn crawl_read(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    (run_id, seeds): (&str, &[Value]),
    extraction: &Value,
    more: &Value,
) -> (Value, Vec<Value>) {
    let mut more = more.clone();
    more["politeness"]["domain"]["delay"] = json!(0);
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
            records.push(message["params"]["record"].clone());
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
    let mut engine = attached(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("markdown", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": { "markdown": true } }),
        &json!({ "fetch": { "tier": "http" }, "crawl": { "max_depth": 1 } }),
    );
    assert!(engine.finished());

    let index_path = path_of(&index);
    let judged: Vec<Value> = records
        .iter()
        .filter(|record| path_of(record["url"].as_str().unwrap()) != index_path)
        .map(|record| {
            json!({ "id": path_of(record["url"].as_str().unwrap()), "markdown": record["markdown"] })
        })
        .collect();
    let verdict = proving_ground
        .judge_records(MARKDOWN_TASK, ToolStatus::Success, &judged)
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
                record["fetched_at"]
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
    let mut engine = attached(&env);
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
    assert!(engine.finished());

    let observations = proving_ground.observations().unwrap();
    let mut received: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.target != "/robots.txt")
        .map(|request| request.target.as_str().to_owned())
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
    let mut engine = attached(&env);
    let about = proving_ground.url(SITEMAP_SITE, "/about");

    let (_, records) = crawl(
        &mut engine,
        &proving_ground,
        ("sitemap", &[json!({ "url": about, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "sitemap" }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finished());

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
    let mut engine = attached(&env);
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
    assert!(engine.finished());
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("invalid_params"),
        "{refused}"
    );
}

/// The fields of the value page, in German: a name, a SKU picked from its
/// sentence by a `match`, a price, a release date and two date-times, one
/// stating no offset and one stating its own, a relative link, and a
/// discount the page does not show.
fn value_fields() -> Value {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    json!({
        "name": { "locator": css("h1") },
        "sku": { "locator": css(".sku"), "match": { "pattern": "Artikelnummer: (\\S+)" } },
        "price": { "locator": css(".price"), "transform": { "kind": "number", "locale": "de-DE" } },
        "released": { "locator": css("time.released"), "transform": { "kind": "date", "locale": "de-DE" } },
        "available": {
            "locator": css("time.available"),
            "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" },
        },
        "updated": {
            "locator": css("time.updated"),
            "transform": { "kind": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" },
        },
        "manual": { "locator": css("a.manual"), "attribute": "href" },
        "discount": { "locator": css(".discount"), "transform": { "kind": "number", "locale": "de-DE" } },
    })
}

/// What the value page's Record holds of its fields, and what its
/// Provenance names: each transform with the text it read, its locale and
/// its zone, and the discount null for want of an element.
fn value_record(origin: &str) -> (Value, Value, Value) {
    (
        json!({
            "name": "Delta-Drachen",
            "sku": "DK-0042",
            "price": 1099.98,
            "released": "2026-02-01",
            "available": "2026-02-01T10:00:00+01:00",
            "updated": "2026-02-01T10:00:00+05:30",
            "manual": format!("{origin}/manuals/delta.pdf"),
            "discount": null,
        }),
        json!([
            { "field": "/available", "text": "2026-02-01T10:00", "transform": "date_time", "locale": "de-DE", "time_zone": "Europe/Berlin" },
            { "field": "/manual", "text": "manuals/delta.pdf", "transform": "text" },
            { "field": "/name", "text": "\n  Delta-Drachen\n", "transform": "text" },
            { "field": "/price", "text": "1099.98", "transform": "number", "locale": "de-DE" },
            { "field": "/released", "text": "2026-02-01", "transform": "date", "locale": "de-DE" },
            { "field": "/sku", "text": "Artikelnummer: DK-0042 (neu)", "transform": "match" },
            { "field": "/sku", "text": "DK-0042", "transform": "text" },
            { "field": "/updated", "text": "2026-02-01T10:00:00+05:30", "transform": "date_time", "locale": "de-DE" },
        ]),
        json!([{ "field": "/discount", "reason": "no_element" }]),
    )
}

/// The value page's one Record: its fields' values, its Provenance's
/// Value transforms and its unread fields.
fn fields_of(records: &[Value]) -> (Value, Value, Value) {
    let [record] = records else {
        panic!("one Record of the value page, not {records:?}");
    };
    let names = value_fields();
    let values: serde_json::Map<String, Value> = names
        .as_object()
        .unwrap()
        .keys()
        .map(|name| (name.clone(), record["value"][name].clone()))
        .collect();
    (
        Value::Object(values),
        record["provenance"]["value_transforms"].clone(),
        record["provenance"]["unread_fields"].clone(),
    )
}

/// A code-free crawl with `fields` fills each on the `http` tier, with no
/// code: a text and a link read as they are, a SKU picked by a `match`, a
/// price, a date and two date-times given by their Value transforms from
/// the page's machine-readable values, typed as the Dataset's generated
/// schema holds them (the Record validated against it as it committed);
/// the Record's Provenance names each transform with the text it read, its
/// locale and its zone; and a locator that finds nothing leaves its field
/// null with its reason, not an error.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[test]
fn a_crawl_with_fields_fills_each_by_its_value_transform_and_provenance_names_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = attached(&env);
    let page = proving_ground.url("extraction.test", "/values");
    let origin = proving_ground.url("extraction.test", "");

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("fields", &[json!({ "url": page, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none", "fields": value_fields() }),
        &json!({ "fetch": { "tier": "http" } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(fields_of(&records), value_record(&origin));
    assert_eq!(records[0]["provenance"]["tier"]["tier"], json!("http"));
}

/// An `XPath` field reads alike on every tier: the `http` tier reads it
/// from the served document; under `auto` the page's sample reads it from
/// the served document with no escalation, then on the browser, whose run
/// commits; the browser tier's page reads it itself. Each Record holds the
/// same values and Provenance as the CSS fields' crawl, and names no call
/// that escalated it.
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[rstest]
#[case::http("http", json!("http"))]
#[case::auto_sampled_with_no_escalation("auto", json!("browser"))]
#[case::browser("browser", json!("browser"))]
fn an_xpath_field_reads_alike_on_every_tier(#[case] tier: &str, #[case] read_on: Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let page = proving_ground.url("extraction.test", "/values");
    let origin = proving_ground.url("extraction.test", "");
    let mut fields = value_fields();
    fields["name"]["locator"] = json!({ "kind": "xpath", "selector": "//article/h1" });
    fields["manual"]["locator"] =
        json!({ "kind": "xpath", "selector": "//a[contains(@class, 'manual')]" });

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("xpath", &[json!({ "url": page, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none", "fields": fields }),
        &json!({ "fetch": { "tier": tier } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(
        (
            fields_of(&records),
            records[0]["provenance"]["tier"]["tier"].clone(),
            records[0]["provenance"]["tier"]["call"].clone(),
        ),
        (value_record(&origin), read_on, Value::Null)
    );
}

/// Provenance names the tier that sent a document, whoever chose it
/// (issue 2409): under `auto`, in an engine with browser pages, a Label's
/// samples send their cheaper run from inside the Session's browser, and
/// so does each request once the Label is learned onto the document tier;
/// every such run is recorded on `browser_fetch`, none on `http`, and how
/// it was chosen stays `sample` or `compared`.
///
/// Covers: story:5-116, story:6-304, operation:crawl.start
#[test]
fn a_document_sent_from_the_sessions_browser_under_auto_is_recorded_on_browser_fetch() {
    const HOST: &str = "tiers.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let seeds: Vec<Value> = (1..=5)
        .map(|page| {
            let url = proving_ground.url(HOST, &format!("/products/{page}"));
            json!({ "url": url, "label": "page" })
        })
        .collect();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("sent-from-the-browser", &seeds),
        &json!({
            "formats": {},
            "follow": "none",
            "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } },
        }),
        &json!({ "fetch": { "tier": "auto" }, "politeness": { "domain": { "concurrency": 1 } } }),
    );
    assert!(engine.finished());

    assert!(started.get("result").is_some(), "{started}");
    let recorded = tiers(&env, "sent-from-the-browser");
    let learned: Vec<&Value> = recorded
        .iter()
        .filter(|entry| entry[2] == "compared")
        .collect();
    assert_eq!(
        (
            records.len(),
            learned.is_empty(),
            learned
                .iter()
                .all(|entry| **entry == json!(["page", "browser_fetch", "compared", null, null])),
            recorded.iter().filter(|entry| entry[1] == "http").count(),
            recorded
                .iter()
                .filter(|entry| **entry == json!(["page", "browser_fetch", "sample", null, null]))
                .count(),
        ),
        (5, false, true, 0, 3),
        "{recorded:?}"
    );
}

/// Each attempt's browser page carries that attempt's own lease, however
/// many attempts run at once: of 16 seeds on the `browser` tier at a
/// concurrency of 16, one page answers 3 s late and 15 at once, and each
/// attempt keeps its page's commands going after the others ended and
/// gave their leases back, so every page commits a Record from its first
/// and only request (#2083: each page took the lease of the attempt that
/// handed its Run's pages over last, and its commands were withheld once
/// that attempt ended).
///
/// Covers: story:6-304, operation:crawl.start, operation:run.records
#[test]
fn concurrent_browser_attempts_each_keep_their_own_lease() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let mut pages = vec!["/products/1?hold=3000".to_owned()];
    pages.extend((2..=16).map(|number| format!("/products/{number}")));
    let seeds: Vec<Value> = pages
        .iter()
        .map(|path| json!({ "url": proving_ground.url("tiers.test", path), "label": "page" }))
        .collect();

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("concurrent", &seeds),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "browser" },
            "politeness": { "sharing": "run", "domain": { "concurrency": 16 } },
            "crawl": { "handler": { "concurrency": 16 } },
        }),
    );
    assert!(engine.finished());

    let observations = proving_ground.observations().unwrap();
    let mut received: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .map(|request| request.target.as_str())
        .filter(|target| target.starts_with("/products/"))
        .map(str::to_owned)
        .collect();
    received.sort();
    pages.sort();
    assert!(started.get("result").is_some(), "{started}");
    assert_eq!((paths(&records), received), (pages.clone(), pages));
}

/// An `XPath` field the engine does not read to nodes fails the start
/// with `invalid_params` before its Run begins.
///
/// Covers: story:6-304, operation:crawl.start
#[test]
fn a_code_free_start_with_an_xpath_the_engine_refuses_is_refused() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "refused-xpath",
            "seeds": [{ "url": "http://example.test/", "label": "page" }],
            "datasets": [],
            "extraction": {
                "formats": { "markdown": true },
                "fields": { "count": { "locator": { "kind": "xpath", "selector": "count(//h1)" } } },
            },
        }),
        &mut |handle| panic!("a refused crawl sent its program {handle}"),
    );
    assert!(engine.finished());
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("invalid_params"),
        "{refused}"
    );
}

/// The maximum-throughput combination (ADR 0138, `docs/throughput.md`):
/// a code-free crawl that asks for no format, with learning, the
/// Provenance log, the Records and the Block classification switched off,
/// still fetches the index and every page it links, once each, but keeps
/// no Record, each outcome saying why, and writes no Provenance log; its
/// report names each switch in its warnings and `weakened`, beside the
/// Proving ground's private ranges and no delay, and the Audit log records
/// each new switch once.
///
/// Covers: story:6-304, operation:crawl.start, setting:provenance.log, setting:dataset.records, setting:fetch.block_classification, setting:frontier.learning
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn with_every_per_page_feature_off_the_crawl_fetches_and_follows_and_keeps_nothing_else() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = attached(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let switched_off = json!({
        "fetch": { "tier": "http", "block_classification": false },
        "crawl": { "max_depth": 1 },
        "frontier": { "learning": false },
        "provenance": { "log": false },
        "dataset": { "records": false },
    });

    let (started, records) = crawl(
        &mut engine,
        &proving_ground,
        ("throughput", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": {} }),
        &switched_off,
    );
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "throughput" }),
        &mut |_| None,
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "throughput" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let observations = proving_ground.observations().unwrap();
    let received: BTreeSet<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.target != "/robots.txt")
        .map(|request| request.target.as_str().to_owned())
        .collect();
    let marks: BTreeSet<String> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {outcomes}"))
        .iter()
        .map(|outcome| outcome["nothing_committed"].to_string())
        .collect();
    let codes = |field: &str, key: &str| strings(&report["result"][field], key);
    let log = HomeLayout::of(env.home()).run_trace("throughput", RUN_TRACE_PROVENANCE);
    assert_eq!(
        (
            started["result"]["run_id"].clone(),
            records.len(),
            received.len(),
            marks,
            codes("warnings", "code")
                .into_iter()
                .filter(|code| !crate::on_the_calendar(code))
                .collect::<BTreeSet<_>>(),
            codes("weakened", "key"),
            switched_off_in_audit_log(&env),
            log.exists(),
        ),
        (
            json!("throughput"),
            0,
            MARKDOWN_PAGES + 1,
            BTreeSet::from(["\"records_off\"".to_owned()]),
            [
                "block_classification_off",
                "learning_off",
                "provenance_off",
                "records_off",
            ]
            .map(str::to_owned)
            .into(),
            [
                "dataset.records",
                "egress.private_ranges",
                "fetch.block_classification",
                "frontier.learning",
                "politeness.domain.delay",
                "provenance.log",
            ]
            .map(str::to_owned)
            .into(),
            BTreeSet::from([
                "dataset.records".to_owned(),
                "fetch.block_classification".to_owned(),
                "provenance.log".to_owned(),
            ]),
            false,
        ),
        "{report}"
    );
}

/// The `key` of each entry of the array `entries`.
fn strings(entries: &Value, key: &str) -> BTreeSet<String> {
    entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry[key].as_str().map(str::to_owned))
        .collect()
}

/// The setting of each `feature_switched_off` entry of `env`'s Audit log.
fn switched_off_in_audit_log(env: &TestEnv) -> BTreeSet<String> {
    let exported = goodfetch_test_support::cli(env, env!("CARGO_BIN_EXE_goodfetch-foxhound"))
        .args(["audit", "export"])
        .run()
        .unwrap();
    exported
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["kind"] == "feature_switched_off")
        .filter_map(|entry| entry["details"]["setting"].as_str().map(str::to_owned))
        .collect()
}
