//! The Frontier's learned rules on Foxhound's executable over stdio (spec
//! #581's seam: the protocol against the Proving ground; ADR 0005): a
//! crawl under `fetch.tier: http`, whose engine fetches each page and
//! learns from its page signature, against the Proving ground's `learning`
//! pages, each threshold met on one path and one short of it on another.
//! The handler reads each page's links from `page.formats` and enqueues
//! them, as a program does. The Run report lists every rule with its
//! evidence and the URLs it merged, and its Completeness says what a trap
//! or an offset left unvouched for.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 851;

/// What a crawl showed: the answer to each URL's first enqueue, `added`
/// or the reason it was not; how many pages the engine fetched on each host and
/// handed a handler; and the Run report.
struct Crawled {
    answers: BTreeMap<String, String>,
    fetched: BTreeMap<String, usize>,
    handled: Vec<String>,
    report: Value,
}

/// The links of a page's Markdown, in order: each `](url)`.
fn links(markdown: &str) -> Vec<String> {
    markdown
        .match_indices("](")
        .filter_map(|(at, _)| {
            let rest = &markdown[at + 2..];
            rest.find(')').map(|end| rest[..end].to_owned())
        })
        .collect()
}

/// Crawls `seeds` (paths on hosts of the `learning` family, as
/// `(host, path)`) under `fetch.tier: http`: each handler reads its page's
/// links from `page.formats` and enqueues every one.
fn crawl(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    seeds: &[(&str, &str)],
) -> Crawled {
    let ip = proving_ground.address().ip().to_string();
    let mut resolve = serde_json::Map::new();
    let seeds: Vec<Value> = seeds
        .iter()
        .map(|(host, path)| {
            resolve.insert((*host).to_owned(), json!([ip]));
            json!({ "url": proving_ground.url(host, path), "label": "page" })
        })
        .collect();
    // The `www.` host the `/forms/` pages link.
    resolve.insert("www.learning.test".to_owned(), json!([ip]));
    let settings = json!({
        "fetch": { "tier": "http" },
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
        "network": { "resolve": resolve },
    });
    let mut engine = Attached::start(env);

    let mut answers = BTreeMap::new();
    let mut fetched: BTreeMap<String, usize> = BTreeMap::new();
    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        run_id,
        &seeds,
        &settings,
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap();
            let host = url.trim_start_matches("http://").split(':').next().unwrap();
            *fetched.entry(host.to_owned()).or_default() += 1;
            handled.push(url.to_owned());
            let read = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            let markdown = read["result"]["markdown"].as_str().unwrap_or_default();
            for href in links(markdown) {
                let answer = inside.enqueue(handle, &href, "page", None);
                let said = answer
                    .get("reason")
                    .unwrap_or(&answer["kind"])
                    .as_str()
                    .unwrap()
                    .to_owned();
                answers.entry(href).or_insert(said);
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    Crawled {
        answers,
        fetched,
        handled,
        report: report["result"].clone(),
    }
}

/// A Goodfetch home and a Proving ground of its own for one test.
fn ground() -> (TestEnv, ProvingGround) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    (env, proving_ground)
}

/// On a host where 3 pairs of URLs differing only by `ref` served one
/// page, `ref` is learned to have no effect: item 9 without `ref`, linked
/// after item 9 with it was fetched, is a `duplicate` and never fetched,
/// and the report lists the rule with its pairs and the URL it merged. On
/// a host of the same registrable domain with one pair, nothing is
/// learned and item 9 is fetched under both URLs.
///
/// Covers: story:6-216, story:6-310, operation:crawl_request.enqueue, setting:fetch.tier
#[test]
fn a_parameter_with_no_effect_merges_its_urls_only_on_its_host() {
    let (env, proving_ground) = ground();
    let crawled = crawl(
        &env,
        &proving_ground,
        "parameter",
        &[
            ("learning.test", "/shop/3?n=1"),
            ("shop.learning.test", "/shop/1?n=1"),
        ],
    );

    let on = |host: &str, path: &str| proving_ground.url(host, path);
    let shop = |number: u32, reference: Option<u32>| {
        let path = match reference {
            Some(reference) => format!("/shop/3?n={number}&ref={reference}"),
            None => format!("/shop/3?n={number}"),
        };
        on("learning.test", &path)
    };
    let evidence = format!(
        "3 pairs of URLs differing only by `ref` served pages of matching signatures, and no \
         pair pages of differing ones: {} and {}; {} and {}; {} and {}.",
        shop(1, None),
        shop(1, Some(1)),
        shop(2, None),
        shop(2, Some(2)),
        shop(3, None),
        shop(3, Some(3)),
    );
    assert_eq!(
        (
            crawled.answers.get(&on("learning.test", "/shop/3?n=9")),
            crawled
                .answers
                .get(&on("shop.learning.test", "/shop/1?n=9")),
            crawled.fetched.get("learning.test"),
            crawled.fetched.get("shop.learning.test"),
            &crawled.report["learned_rules"],
            &crawled.report["completeness"],
        ),
        (
            Some(&"duplicate".to_owned()),
            Some(&"added".to_owned()),
            Some(&7),
            Some(&4),
            &json!([{
                "kind": "no_effect_parameter",
                "host": "learning.test",
                "parameter": "ref",
                "evidence": evidence,
                "merged": [on("learning.test", "/shop/3?n=9")],
            }]),
            &json!("complete"),
        )
    );
}

/// The calendar's months, alike and paging on without end, close as a
/// trap once 100 of them after the first brought no new page and no URL
/// outside it: the next month is not added, with `trap_path`, and the Run
/// is `unverified`, naming the count. The agenda's 100 months, one short
/// of it, close nothing, and the Run is `complete`.
///
/// Covers: story:6-216, story:6-310, operation:crawl_request.enqueue
#[test]
fn a_calendar_trap_closes_after_a_hundred_months_and_leaves_the_run_unverified() {
    let (env, proving_ground) = ground();
    let calendar = crawl(
        &env,
        &proving_ground,
        "calendar",
        &[("learning.test", "/calendar/2026/01")],
    );
    let (agenda_env, agenda_ground) = ground();
    let agenda = crawl(
        &agenda_env,
        &agenda_ground,
        "agenda",
        &[("learning.test", "/agenda/2026/01")],
    );

    let month = |index: u32| {
        proving_ground.url(
            "learning.test",
            &format!("/calendar/{}/{:02}", 2026 + index / 12, index % 12 + 1),
        )
    };
    let refused: Vec<&String> = calendar
        .answers
        .iter()
        .filter(|(_, answer)| *answer == "trap_path")
        .map(|(url, _)| url)
        .collect();
    assert_eq!(
        (
            refused,
            calendar.fetched.get("learning.test"),
            &calendar.report["learned_rules"],
            &calendar.report["completeness"],
            &calendar.report["completeness_reasons"],
        ),
        (
            vec![&month(101)],
            Some(&102),
            &json!([{
                "kind": "trap_path",
                "host": "learning.test",
                "path": "/calendar/{number}/{number}",
                "evidence": format!(
                    "100 pages under `/calendar/{{number}}/{{number}}` in a row, from {} to {}, \
                     brought no new page signature and no URL outside it.",
                    month(1),
                    month(100)
                ),
                "merged": [month(101)],
            }]),
            &json!("unverified"),
            &json!([{
                "code": "trap_path",
                "message": "A learned trap refused 1 URL the crawl would otherwise have admitted.",
                "count": 1,
            }]),
        )
    );
    assert_eq!(
        (
            agenda.fetched.get("learning.test"),
            &agenda.report["learned_rules"],
            &agenda.report["completeness"],
        ),
        (Some(&101), &json!([]), &json!("complete"))
    );
}

/// A listing paged by `offset` that also links a cursor is read through
/// the cursor once its offset is known: the offset URLs not yet fetched
/// are `duplicate`, the Proving ground serves each row's page once, and
/// the Run is `complete`. The same rows paged by offset alone leave the
/// Run `unverified`, and two pages of it, one short of an offset, do not.
///
/// Covers: story:6-216, story:6-310, operation:crawl_request.enqueue
#[test]
fn an_offset_listing_gives_way_to_its_cursor() {
    let (env, proving_ground) = ground();
    let feed = crawl(
        &env,
        &proving_ground,
        "feed",
        &[("learning.test", "/feed?offset=0")],
    );
    let list = crawl(
        &env,
        &proving_ground,
        "list",
        &[("learning.test", "/list?offset=0")],
    );
    let pair = crawl(
        &env,
        &proving_ground,
        "pair",
        &[("learning.test", "/pair?offset=0")],
    );

    let feed_url = |query: &str| proving_ground.url("learning.test", &format!("/feed?{query}"));
    let not_added: Vec<&String> = feed
        .answers
        .iter()
        .filter(|(_, answer)| *answer == "duplicate")
        .map(|(url, _)| url)
        .filter(|url| url.contains("offset="))
        .collect();
    let rule = &feed.report["learned_rules"][0];
    assert_eq!(
        (
            not_added,
            &rule["kind"],
            &rule["path"],
            &rule["parameter"],
            &rule["merged"],
            &feed.report["completeness"],
        ),
        (
            vec![&feed_url("offset=60")],
            &json!("cursor_pagination"),
            &json!("/feed"),
            &json!("offset"),
            &json!([feed_url("offset=60")]),
            &json!("complete"),
        )
    );
    assert_eq!(
        (
            &list.report["learned_rules"],
            &list.report["completeness"],
            &list.report["completeness_reasons"][0]["code"],
            &pair.report["learned_rules"],
            &pair.report["completeness"],
        ),
        (
            &json!([]),
            &json!("unverified"),
            &json!("offset_pagination"),
            &json!([]),
            &json!("complete"),
        )
    );
}

/// The URL identity conformance cases of learned forms (ADR 0120) on the
/// Proving ground's `/forms/` pages: two pairs of a guide with and without
/// its trailing slash teach that form, and the third guide's slashed URL,
/// still pending, ends `skipped` with `duplicate` and is never handed a
/// handler; three docs pages naming their folder as `rel=canonical` teach
/// the `index.html` form, ending the folders already pending and making
/// the next a `duplicate`; news pages whose canonical names the section's
/// index teach nothing and are each fetched; and pages that read otherwise
/// on `www.` and on the bare host are both kept. The report lists each
/// `equivalent_form` rule with its evidence and the URLs it merged.
///
/// Covers: story:6-217, story:6-219, story:6-222, story:6-310, operation:crawl_request.enqueue, setting:fetch.tier
#[test]
fn equivalent_forms_merge_only_on_evidence() {
    let (env, proving_ground) = ground();
    let crawled = crawl(
        &env,
        &proving_ground,
        "forms",
        &[("learning.test", "/forms/")],
    );

    let on = |path: &str| proving_ground.url("learning.test", path);
    let on_www = |path: &str| proving_ground.url("www.learning.test", path);
    let handled = |url: &str| crawled.handled.iter().any(|handled| handled == url);
    let skipped = crawled.report["requests"]["by_skip_reason"].clone();
    assert_eq!(
        (
            handled(&on("/forms/slash/3")),
            handled(&on("/forms/slash/3/")),
            handled(&on("/forms/docs/1/")),
            handled(&on("/forms/docs/2/")),
            crawled.answers.get(&on("/forms/docs/3/")),
            [1, 2, 3].map(|page| handled(&on(&format!("/forms/news/{page}")))),
            [1, 2, 3].map(|page| {
                let path = format!("/forms/split/{page}");
                (handled(&on(&path)), handled(&on_www(&path)))
            }),
            skipped,
        ),
        (
            true,
            false,
            false,
            false,
            Some(&"duplicate".to_owned()),
            [true; 3],
            [(true, true); 3],
            json!([{ "name": "duplicate", "count": 3 }]),
        )
    );
    assert_eq!(
        crawled.report["learned_rules"],
        json!([
            {
                "kind": "equivalent_form",
                "host": "learning.test",
                "form": "trailing_slash",
                "evidence": format!(
                    "2 pairs of URLs differing only by a trailing slash served pages of matching \
                     signatures, and no pair pages of differing ones: {} and {}; {} and {}.",
                    on("/forms/slash/1"),
                    on("/forms/slash/1/"),
                    on("/forms/slash/2"),
                    on("/forms/slash/2/"),
                ),
                "merged": [on("/forms/slash/3/"), on("/forms/docs/3/")],
            },
            {
                "kind": "equivalent_form",
                "host": "learning.test",
                "form": "index_html",
                "evidence": format!(
                    "3 pages named as their `rel=canonical` their own URL but for `index.html`: \
                     {} names {}; {} names {}; {} names {}.",
                    on("/forms/docs/1/index.html"),
                    on("/forms/docs/1/"),
                    on("/forms/docs/2/index.html"),
                    on("/forms/docs/2/"),
                    on("/forms/docs/3/index.html"),
                    on("/forms/docs/3/"),
                ),
                "merged": [on("/forms/docs/1/"), on("/forms/docs/2/"), on("/forms/docs/3/")],
            },
        ])
    );
}
