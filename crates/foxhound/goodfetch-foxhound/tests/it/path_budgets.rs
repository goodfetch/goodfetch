//! Per-path budgets and facets, on Foxhound's executable over stdio (spec
//! #581's seam: the protocol against the Proving ground; ADR 0094), on the
//! Proving ground's faceted shop, `facets.test`, whose listing links every
//! value of every facet it has not set: 400 listings once the URL key sorts
//! their parameters, and 24 products. `crawl.path_budgets` refuses a
//! listing past its entry's count with `path_budget` and ends the Run
//! `incomplete`, the pattern named in its reasons (CO-08); facets get no
//! default rule, so the shop is crawled whole unless a `crawl.scope.exclude`
//! pattern on their parameters or `crawl.max_requests` bounds it (ADR 0120;
//! pitfall U02).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::{Inside, attributes, crawl_under, fetch};

/// The Proving ground's seed.
const SEED: u64 = 847;

/// The shop's faceted listings: one per combination of four colours, four
/// sizes, three materials and three sort orders, each facet set or not.
const LISTINGS: usize = 5 * 5 * 4 * 4;

/// The shop's products.
const PRODUCTS: usize = 24;

/// What a crawl of the shop showed: how many enqueues got each answer,
/// `added` or the reason it was not, how many faceted listings (`/shop`
/// with a query) and products the Proving ground received, and the Run
/// report's Completeness and its reasons' codes and messages, as
/// `run.finish` answered them and as an engine started afterwards reads
/// them from the Run's state.
#[derive(Debug, PartialEq)]
struct Crawled {
    answers: BTreeMap<String, usize>,
    listings: usize,
    products: usize,
    completeness: Value,
    reasons: Value,
    read_back: (Value, Value),
}

/// Crawls the shop from `/shop` under the `crawl` settings `settings`:
/// each page's handler fetches it and enqueues every link it holds.
fn crawl_shop(settings: &Value) -> Crawled {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url("facets.test", "/shop"), "label": "page" })];
    let mut engine = attached(&env);

    let mut answers = BTreeMap::new();
    let report = crawl_under(
        &mut engine,
        "shop",
        &seeds,
        settings,
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap();
            let page = fetch(&proving_ground, url);
            assert_eq!(page.status, 200, "{url}");
            for href in attributes(&page.body, "a href") {
                let href = href.replace("&amp;", "&");
                let answer = inside.enqueue(handle, &href, "page", None);
                let answered = answer
                    .get("reason")
                    .unwrap_or(&answer["kind"])
                    .as_str()
                    .unwrap()
                    .to_owned();
                *answers.entry(answered).or_insert(0) += 1;
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());
    let received: Vec<String> = ReceivedRequests::of(&observations)
        .of_the_case()
        .map(|request| request.target.as_str().to_owned())
        .collect();
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");

    let mut engine = attached(&env);
    let again = engine.call(
        "again",
        "run.finish",
        &json!({ "run_id": "shop" }),
        &mut |_| None,
    );
    assert!(engine.finished());
    Crawled {
        answers,
        listings: received
            .iter()
            .filter(|target| target.starts_with("/shop?"))
            .count(),
        products: received
            .iter()
            .filter(|target| target.starts_with("/products/"))
            .count(),
        completeness: report["result"]["completeness"].clone(),
        reasons: report["result"]["completeness_reasons"].clone(),
        read_back: (
            again["result"]["completeness"].clone(),
            again["result"]["completeness_reasons"].clone(),
        ),
    }
}

/// With no bound set, nothing cuts the faceted listings: every one of them
/// and every product is fetched, no link is refused but as a duplicate, and
/// the Run is `complete`. An exclude pattern naming the facet parameters
/// keeps the crawl to `/shop` and its products, every facet link
/// `out_of_scope`, still `complete`; a
/// request cap cuts it, the Run `incomplete`.
///
/// Covers: story:6-230, setting:crawl.scope.exclude, setting:crawl.max_requests
#[test]
fn a_faceted_listing_is_crawled_whole_unless_an_exclude_pattern_or_the_request_cap_bounds_it() {
    let whole = crawl_shop(&json!({}));
    let excluded =
        crawl_shop(&json!({ "scope": { "exclude": ["**{colour,size,material,sort}=**"] } }));
    let capped = crawl_shop(&json!({ "max_requests": 50 }));

    let whole_answers = whole.answers.clone();
    assert_eq!(
        whole,
        Crawled {
            answers: whole_answers,
            listings: LISTINGS - 1,
            products: PRODUCTS,
            completeness: json!("complete"),
            reasons: Value::Null,
            read_back: (json!("complete"), Value::Null),
        }
    );
    assert_eq!(
        whole.answers.keys().collect::<Vec<_>>(),
        ["added", "duplicate"]
    );
    assert_eq!(whole.answers["added"], LISTINGS - 1 + PRODUCTS);

    let excluded_answers = excluded.answers.clone();
    assert_eq!(
        excluded,
        Crawled {
            answers: excluded_answers,
            listings: 0,
            products: PRODUCTS,
            completeness: json!("complete"),
            reasons: Value::Null,
            read_back: (json!("complete"), Value::Null),
        }
    );
    assert_eq!(
        excluded.answers.keys().collect::<Vec<_>>(),
        ["added", "out_of_scope"]
    );
    assert_eq!(excluded.answers["added"], PRODUCTS);

    let cut = json!([{
        "code": "crawl.max_requests",
        "message": format!(
            "`crawl.max_requests` refused {} URLs the crawl would otherwise have admitted.",
            capped.answers["max_requests"]
        ),
        "count": capped.answers["max_requests"],
    }]);
    assert_eq!(
        (
            capped.listings + capped.products,
            capped.completeness,
            capped.reasons,
            capped.read_back
        ),
        (
            49,
            json!("incomplete"),
            cut.clone(),
            (json!("incomplete"), cut)
        )
    );
}

/// `crawl.path_budgets` lets the listings' entry admit 20 requests and
/// refuses the next with `path_budget`; `/shop` itself, which the first
/// entry does not match, is counted by the second alone, whose budget of 1
/// it uses up without refusing anything, so only the first entry is named.
/// The products, which no entry matches, are all fetched, and the Run ends
/// `incomplete`, its reason naming the pattern and the counts.
///
/// Covers: story:6-229, setting:crawl.path_budgets, operation:crawl_request.enqueue
#[test]
fn a_path_budget_refuses_past_its_count_and_the_run_ends_incomplete() {
    let crawled = crawl_shop(&json!({ "path_budgets": [
        { "pattern": "/shop?*", "max_requests": 20 },
        { "pattern": "/shop**", "max_requests": 1 },
    ] }));

    let refused = crawled.answers["path_budget"];
    let reasons = json!([{
        "code": "crawl.path_budgets",
        "message": format!(
            "`crawl.path_budgets`'s entry `/shop?*` refused {refused} URLs past its 20 requests the crawl would otherwise have admitted."
        ),
        "count": refused,
    }]);
    let answers_given = crawled.answers.clone();
    assert_eq!(
        crawled,
        Crawled {
            answers: answers_given,
            listings: 20,
            products: PRODUCTS,
            completeness: json!("incomplete"),
            reasons: reasons.clone(),
            read_back: (json!("incomplete"), reasons),
        }
    );
    assert_eq!(
        crawled.answers.keys().collect::<Vec<_>>(),
        ["added", "duplicate", "path_budget"]
    );
}

/// A path budget whose pattern the engine cannot match fails the start
/// with `pattern_unsupported`, before the Run begins.
///
/// Covers: story:6-229, setting:crawl.path_budgets, error:pattern_unsupported
#[test]
fn a_path_budget_pattern_the_engine_cannot_match_fails_the_start() {
    let env = TestEnv::new();
    let mut engine = attached(&env);

    let answer = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "unmatched",
            "seeds": [{ "url": "https://shop.test/", "label": "page" }],
            "settings": { "crawl": { "path_budgets": [
                { "pattern": "/shop/{a,{b,c}}", "max_requests": 1 },
            ] } },
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
