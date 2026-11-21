//! Tier table cases (spec #582's served-call table; ADR 0093), on
//! Foxhound's executable against the Proving ground and Chrome for
//! Testing: one handler's calls on the `http` tier, forced and under
//! `fetch.tier: auto`, and on `browser`. A document tier answers the
//! locators, the visibility of an element and the waits already true from
//! its markup, reads `formats`, and serves `goto`, `reload`, `goBack` and
//! `goForward` by fetching over the same tier through the gate; a call it
//! cannot serve exactly, such as `evaluate` or a wait for content its
//! document lacks, escalates the request to the browser under `auto`,
//! neither a retry nor a failure, and fails with `browser_required` under a
//! forced tier, so a script-rendered page never ends as an empty success
//! (pitfall H22). `page.read` answers a page's fields alike on a Label
//! forced to `http` and on one forced to `browser` (ADR 0147). The Page
//! formats cases read every Markdown page of the
//! Proving ground's `extraction/7` on `http` and on `browser`, each judged
//! by the oracle (ADR 0094).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::ToolStatus;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, provenance, settings, tiers};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 931;

/// The host of the `tiers` family.
const HOST: &str = "tiers.test";

/// One handler run as it saw it: its tier, and what each of its calls
/// answered.
type Run = (Value, Vec<(&'static str, Value)>);

/// What a call answered, as a case compares it: its error's code, else the
/// part of its result it reads.
pub(crate) fn read(reply: &Value, part: impl Fn(&Value) -> Value) -> Value {
    reply.get("error").map_or_else(
        || part(&reply["result"]),
        |error| error["data"]["code"].clone(),
    )
}

/// The path of the URL `url`.
fn path_of(url: &Value) -> Value {
    let url = url.as_str().unwrap_or_default();
    let host = url.find("://").map_or(0, |at| at + "://".len());
    json!(url[host..].find('/').map_or("", |at| &url[host + at..]))
}

/// Whether a wait's result says it reached its state.
fn reached(result: &Value) -> Value {
    json!(result == &json!({}))
}

/// A navigation's response: its status and its URL's path.
fn response(result: &Value) -> Value {
    json!([
        result["response"]["status"],
        path_of(&result["response"]["url"])
    ])
}

/// One call of the handler: its name in a case, its method, its params,
/// and what a case reads of its result.
type Call = (&'static str, &'static str, Value, fn(&Value) -> Value);

/// The handler's calls on the states page `page_id`, whose static
/// listing is at `products`: it waits for its visible heading, for each
/// element its markup hides to be hidden, for its link and for its URL
/// and load state, reads its Markdown, goes to the listing and waits for
/// its products, reloads it, goes back and forward, and last runs a
/// script.
fn calls(page_id: &Value, products: &str) -> Vec<Call> {
    let locator = |selector: &str| json!({ "page_id": page_id, "selector": selector });
    vec![
        (
            "visible",
            "locator.wait_for",
            json!({ "locator": locator("h1") }),
            reached,
        ),
        (
            "hidden attribute",
            "locator.wait_for",
            json!({ "locator": locator("#hidden-attribute"), "state": "hidden" }),
            reached,
        ),
        (
            "display none",
            "locator.wait_for",
            json!({ "locator": locator("#in-display-none"), "state": "hidden" }),
            reached,
        ),
        (
            "visibility hidden",
            "locator.wait_for",
            json!({ "locator": locator("#visibility-hidden"), "state": "hidden" }),
            reached,
        ),
        (
            "attached",
            "locator.wait_for",
            json!({ "locator": locator("a#next"), "state": "attached" }),
            reached,
        ),
        (
            "url",
            "page.wait_for_url",
            json!({ "page_id": page_id, "url": { "kind": "glob", "pattern": "**/states" } }),
            reached,
        ),
        (
            "load state",
            "page.wait_for_load_state",
            json!({ "page_id": page_id, "state": "domcontentloaded" }),
            reached,
        ),
        (
            "formats",
            "page.formats",
            json!({ "page_id": page_id, "formats": { "markdown": true } }),
            |r| r["main_content"].clone(),
        ),
        (
            "goto",
            "page.goto",
            json!({ "page_id": page_id, "url": products }),
            response,
        ),
        (
            "products",
            "locator.wait_for_count",
            json!({ "locator": locator("li.product"), "count": { "min": 3 } }),
            |r| r["count"].clone(),
        ),
        (
            "reload",
            "page.reload",
            json!({ "page_id": page_id }),
            response,
        ),
        (
            "back",
            "page.go_back",
            json!({ "page_id": page_id }),
            response,
        ),
        (
            "forward",
            "page.go_forward",
            json!({ "page_id": page_id }),
            response,
        ),
        (
            "script",
            "page.evaluate",
            json!({ "page_id": page_id, "function": "() => document.title" }),
            |r| r["value"].clone(),
        ),
    ]
}

/// The handler of the states page, which makes [`calls`] in turn; the
/// first call that fails ends the attempt with its error, as an SDK's
/// handler raising it would.
fn the_tables_calls(runs: &mut Vec<Run>) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        let products = handle["crawl_request"]["url"]
            .as_str()
            .unwrap()
            .replace("/states", "/products/2");
        let mut answered = Vec::new();
        let mut failed = None;
        for (name, method, params, part) in calls(&handle["page_id"], &products) {
            let reply = inside.call(method, &params);
            answered.push((name, read(&reply, part)));
            if let Some(error) = reply.get("error") {
                failed = Some(error.clone());
                break;
            }
        }
        runs.push((handle["tier"].clone(), answered));
        failed.map_or_else(
            || json!({ "result": { "records": [] } }),
            |error| json!({ "error": error }),
        )
    }
}

/// What every call but the script answers on a tier that serves it.
fn served() -> Vec<(&'static str, Value)> {
    vec![
        ("visible", json!(true)),
        ("hidden attribute", json!(true)),
        ("display none", json!(true)),
        ("visibility hidden", json!(true)),
        ("attached", json!(true)),
        ("url", json!(true)),
        ("load state", json!(true)),
        ("formats", json!("found")),
        ("goto", json!([200, "/products/2"])),
        ("products", json!(3)),
        ("reload", json!([200, "/products/2"])),
        ("back", json!([200, "/states"])),
        ("forward", json!([200, "/products/2"])),
    ]
}

/// `served()`, then the script's answer `script`.
fn served_then(script: Value) -> Vec<(&'static str, Value)> {
    let mut answered = served();
    answered.push(("script", script));
    answered
}

/// Each request's URL path, how it ended, its error's code and how many
/// attempts it took.
pub(crate) fn ended(engine: &mut Attached, run_id: &str) -> Vec<Value> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            json!([
                path_of(&outcome["request"]["url"]),
                outcome["kind"],
                outcome["error"]["code"],
                outcome["attempts"]
            ])
        })
        .collect()
}

/// The handler's calls under a forced tier: `http` serves every row the
/// table gives it, answering the waits and the questions from the states
/// page's markup and navigating over HTTP through the gate, exactly as
/// `browser` answers them, and refuses the script with `browser_required`,
/// nothing escalating; `browser` serves the script too.
///
/// Covers: story:6-259, story:5-115, operation:locator.wait_for, operation:page.wait_for_url, operation:page.wait_for_load_state, operation:page.formats, operation:page.goto, operation:page.reload, operation:page.go_back, operation:page.go_forward, operation:locator.wait_for_count, operation:page.evaluate
#[rstest::rstest]
/// Covers: error:browser_required
#[case::http("http", served_then(json!("browser_required")), json!(["/states", "failed", "browser_required", 1]))]
#[case::browser("browser", served_then(json!("products 2")), json!(["/states", "succeeded", null, 1]))]
fn under_a_forced_tier_each_row_is_served_as_the_table_says_or_refused(
    #[case] tier: &str,
    #[case] expected: Vec<(&'static str, Value)>,
    #[case] outcome: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("table-{tier}");

    let mut runs = Vec::new();
    crawl_declaring(
        &mut engine,
        &run_id,
        &[json!({ "url": proving_ground.url(HOST, "/states"), "label": "page" })],
        &json!({ "page": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": tier } })),
        &mut the_tables_calls(&mut runs),
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    assert_eq!(
        (runs, ended, tiers(&env, &run_id)),
        (
            vec![(json!(tier), expected)],
            vec![outcome],
            vec![json!(["page", tier, "forced", "run", null])],
        )
    );
}

/// The same handler under `auto`: its run on `http` is served call by call
/// until the script, which the tier cannot serve exactly, ends that run,
/// and the same handler runs again in the browser in the same attempt,
/// where every call is served; the request ends `succeeded` in one
/// attempt, and Provenance names the script as the call that escalated.
///
/// Covers: story:6-259, story:5-115, operation:page.evaluate, error:attempt_ended
#[test]
fn under_auto_the_first_call_a_document_tier_cannot_serve_escalates_the_request_once() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut runs = Vec::new();
    crawl_declaring(
        &mut engine,
        "table-auto",
        &[json!({ "url": proving_ground.url(HOST, "/states"), "label": "page" })],
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "http_from": "browser" } }),
        ),
        &mut the_tables_calls(&mut runs),
    );
    let ended = ended(&mut engine, "table-auto");
    assert!(engine.finished());

    assert_eq!(
        (runs, ended, tiers(&env, "table-auto")),
        (
            vec![
                (json!("http"), served_then(json!("attempt_ended"))),
                (json!("browser"), served_then(json!("products 2"))),
            ],
            vec![json!(["/states", "succeeded", null, 1])],
            vec![
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, "page.evaluate"]),
            ],
        )
    );
}

/// A handler that waits for a script-rendered listing's entries and
/// answers how many it saw: on `http` its document holds none, so under
/// `auto` the wait escalates the request rather than answering an empty
/// listing, and the browser's run, in the same attempt, answers the three
/// entries; under a forced `http` the wait fails with `browser_required`,
/// and the request ends `failed`, never `succeeded` with nothing (pitfall
/// H22). Neither counts a retry. Under `auto`, its HTTP leaves from the
/// browser (`fetch.http_from: browser`).
///
/// Covers: story:6-260, story:6-259, operation:locator.wait_for_count
#[rstest::rstest]
/// Covers: error:attempt_ended
#[case::auto("auto", &[("http", "attempt_ended"), ("browser", "3")], json!(["/blog/1", "succeeded", null, 1]))]
/// Covers: error:browser_required
#[case::forced("http", &[("http", "browser_required")], json!(["/blog/1", "failed", "browser_required", 1]))]
fn a_script_rendered_page_never_ends_as_an_empty_success_on_http(
    #[case] tier: &str,
    #[case] expected: &[(&str, &str)],
    #[case] outcome: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("empty-{tier}");

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        &run_id,
        &[json!({ "url": proving_ground.url(HOST, "/blog/1"), "label": "blog" })],
        &json!({ "blog": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": tier, "http_from": "browser" } }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let waited = inside.call(
                "locator.wait_for_count",
                &json!({
                    "locator": { "page_id": handle["page_id"], "selector": "li.item" },
                    "count": { "min": 1 },
                }),
            );
            let answer = read(&waited, |result| json!(result["count"].to_string()));
            seen.push((
                handle["tier"].as_str().unwrap().to_owned(),
                answer.as_str().unwrap().to_owned(),
            ));
            waited.get("error").map_or_else(
                || json!({ "result": { "records": [] } }),
                |error| json!({ "error": error }),
            )
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(tier, answer)| ((*tier).to_owned(), (*answer).to_owned()))
        .collect();
    assert_eq!((seen, ended), (expected, vec![outcome]));
}

/// The Markdown task of the Proving ground, whose pages and oracle judge
/// a page's Markdown.
const MARKDOWN_TASK: &str = "extraction/7";

/// Page formats conformance cases on Foxhound's executable (ADRs 0088 and
/// 0094): a crawl of every Markdown page of `extraction/7` reads each
/// page's main content through `formats` and answers it as a Record, and
/// the oracle verifies the Markdown of every page on `http` as on
/// `browser`, so a handler reads the same Markdown on either tier.
///
/// Covers: story:6-65, story:6-259, operation:page.formats
#[rstest::rstest]
#[case::http("http")]
#[case::browser("browser")]
fn every_page_formats_the_same_markdown_on_its_tier_as_the_oracle_holds(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let listing = ureq_get(&proving_ground, &index);
    let origin = index[..index.find("/markdown").unwrap()].to_owned();
    let seeds: Vec<Value> = listing
        .split("href=\"")
        .skip(1)
        .filter_map(|link| link.split('"').next())
        .map(|path| json!({ "url": format!("{origin}{path}"), "label": "page" }))
        .collect();
    let run_id = format!("formats-{tier}");

    let mut records = Vec::new();
    crawl_declaring(
        &mut engine,
        &run_id,
        &seeds,
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": tier },
                "politeness": { "domain": { "delay": 0 } },
            }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let read = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            let url = handle["crawl_request"]["url"].clone();
            records.push(json!({ "id": path_of(&url), "markdown": read["result"]["markdown"] }));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    let verdict = proving_ground
        .judge_records(MARKDOWN_TASK, ToolStatus::Success, &records)
        .unwrap();
    assert_eq!(
        (
            seeds.len(),
            ended
                .iter()
                .filter(|outcome| outcome[1] == "succeeded")
                .count(),
            &verdict["outcome"],
        ),
        (31, 31, &json!("verified")),
        "{verdict}"
    );
}

/// The host of the Proving ground's Clean-up rules pages.
const CLEAN_UP_HOST: &str = "clean-up.test";

/// The cosmetic Clean-up rules apply on every tier, HTTP included (ADR
/// 0047): on the Proving ground's `/hiding`, whose ad EasyList's
/// `##.ad-slot` matches, a handler's `formats` read on `http` leaves the ad
/// out of the Markdown and the cleaned HTML as on `browser`, and
/// Provenance records the rules' version and what they removed.
///
/// Covers: story:6-10, operation:page.formats
#[rstest::rstest]
#[case::http("http")]
#[case::browser("browser")]
fn a_cosmetic_rule_takes_an_ad_out_of_a_handlers_formats_on_its_tier(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("cosmetic-{tier}");

    let mut read = Vec::new();
    crawl_declaring(
        &mut engine,
        &run_id,
        &[json!({ "url": proving_ground.url(CLEAN_UP_HOST, "/hiding"), "label": "article" })],
        &json!({ "article": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": tier } })),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let formats = inside.call(
                "page.formats",
                &json!({
                    "page_id": handle["page_id"],
                    "formats": { "markdown": true, "html": { "kind": "cleaned" } },
                }),
            );
            let markdown = formats["result"]["markdown"].as_str().unwrap_or_default();
            let html = formats["result"]["html"].as_str().unwrap_or_default();
            read.push((
                handle["tier"].clone(),
                markdown.contains("The article's text begins here"),
                markdown.contains("An ad's text"),
                html.contains("ad-slot"),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    let recorded: Vec<(bool, bool)> = provenance(&env, &run_id, "formats")
        .iter()
        .map(|entry| {
            (
                entry["clean_up_rules"].is_string(),
                entry["removed"]
                    .as_array()
                    .is_some_and(|removed| !removed.is_empty()),
            )
        })
        .collect();
    assert_eq!(
        (read, recorded, ended.len()),
        (
            vec![(json!(tier), true, false, false)],
            vec![(true, true)],
            1
        )
    );
}

/// The extraction task whose navigation page holds a figure.
const CONTENT_MODES_TASK: &str = "extraction/8";

/// A Label's `content.extractor` chooses the main-content step of its
/// requests' pages, over the Run's, on every tier (spec #580's Settings
/// table): on the page with a figure, dom-smoothie's Markdown holds the
/// figure's image and rs-trafilatura's does not, while both keep the
/// article's paragraph.
///
/// Covers: setting:content.extractor, operation:page.formats
#[rstest::rstest]
#[case::http("http")]
#[case::browser("browser")]
fn a_labels_content_extractor_reads_its_pages_main_content(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let index = proving_ground.start_url(CONTENT_MODES_TASK).unwrap();
    let origin = index[..index.find("/content-modes").unwrap()].to_owned();
    let figure = format!("{origin}/content-modes/navigation-and-related");
    let run_id = format!("label-extractor-{tier}");

    let mut read = Vec::new();
    crawl_declaring(
        &mut engine,
        &run_id,
        &[
            json!({ "url": figure, "label": "smoothie" }),
            json!({ "url": format!("{figure}?again"), "label": "trafilatura" }),
        ],
        &json!({
            "smoothie": {},
            "trafilatura": { "settings": { "content": { "extractor": "trafilatura" } } },
        }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": tier },
                "politeness": { "domain": { "delay": 0 } },
            }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let formats = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            let markdown = formats["result"]["markdown"].as_str().unwrap_or_default();
            read.push((
                handle["crawl_request"]["label"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                markdown.contains("Braided polyester is the usual choice for single-line kites"),
                markdown.contains("/images/line.jpg"),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    read.sort();
    assert_eq!(
        (read, ended.len()),
        (
            vec![
                ("smoothie".to_owned(), true, true),
                ("trafilatura".to_owned(), true, false),
            ],
            2
        )
    );
}

/// What a handler's page answers of one extraction task's formats, as the
/// task's oracle reads a Record: the call or calls it makes through
/// `inside` on the page `page_id`, and the Record's fields.
type Recording = fn(&mut Inside<'_>, &Value) -> serde_json::Map<String, Value>;

/// Each form `extraction/8`'s Records hold: its field, the request that
/// asks for it and the key of the result that holds it.
const CONTENT_FORMS: [(&str, &str, &str); 7] = [
    (
        "markdown",
        r#"{ "markdown": { "content": "main" } }"#,
        "markdown",
    ),
    (
        "markdown_full",
        r#"{ "markdown": { "content": "full" } }"#,
        "markdown",
    ),
    (
        "markdown_fit",
        r#"{ "markdown": { "content": "fit" } }"#,
        "markdown",
    ),
    ("html_raw", r#"{ "html": { "kind": "raw" } }"#, "html"),
    (
        "html_cleaned",
        r#"{ "html": { "kind": "cleaned" } }"#,
        "html",
    ),
    ("html_main", r#"{ "html": { "kind": "main" } }"#, "html"),
    ("text", r#"{ "text": true }"#, "text"),
];

/// The content-mode Record of the page `page_id`: each Markdown content
/// mode, each HTML kind and the text, one call each.
fn content_modes(inside: &mut Inside<'_>, page_id: &Value) -> serde_json::Map<String, Value> {
    CONTENT_FORMS
        .iter()
        .map(|(field, request, key)| {
            let request: Value = serde_json::from_str(request).unwrap();
            let read = inside.call(
                "page.formats",
                &json!({ "page_id": page_id, "formats": request }),
            );
            ((*field).to_owned(), read["result"][key].clone())
        })
        .collect()
}

/// The reference Record of the page `page_id`: its links, images, media
/// and tables, from one call.
fn references(inside: &mut Inside<'_>, page_id: &Value) -> serde_json::Map<String, Value> {
    let read = inside.call(
        "page.formats",
        &json!({
            "page_id": page_id,
            "formats": { "links": true, "images": true, "media": true, "tables": true },
        }),
    );
    ["links", "images", "media", "tables"]
        .into_iter()
        .map(|field| (field.to_owned(), read["result"][field].clone()))
        .collect()
}

/// The structured Record of the page `page_id`: its metadata and its
/// contacts, from one call.
fn structured(inside: &mut Inside<'_>, page_id: &Value) -> serde_json::Map<String, Value> {
    let read = inside.call(
        "page.formats",
        &json!({ "page_id": page_id, "formats": { "metadata": true, "contacts": true } }),
    );
    ["metadata", "contacts"]
        .into_iter()
        .map(|field| (field.to_owned(), read["result"][field].clone()))
        .collect()
}

/// Page formats conformance cases on Foxhound's executable for every
/// format beside the Markdown (spec #580's Testing Decisions; ADR 0094): a
/// crawl of every page of the `extraction` family's content-mode,
/// reference and structured tasks reads each page's formats through
/// `formats` as a handler does and answers them as Records, and each
/// task's oracle verifies them on `http` as on `browser`: the HTML kinds,
/// the text, links, images, media, tables, metadata and contacts.
///
/// Covers: story:6-259, operation:page.formats
#[rstest::rstest]
#[case::content_modes_on_http("extraction/8", "http", content_modes as Recording, 2)]
#[case::content_modes_on_browser("extraction/8", "browser", content_modes as Recording, 2)]
#[case::references_on_http("extraction/9", "http", references as Recording, 3)]
#[case::references_on_browser("extraction/9", "browser", references as Recording, 3)]
#[case::structured_on_http("extraction/10", "http", structured as Recording, 3)]
#[case::structured_on_browser("extraction/10", "browser", structured as Recording, 3)]
fn every_format_of_a_handlers_page_matches_the_oracle_on_its_tier(
    #[case] task: &str,
    #[case] tier: &str,
    #[case] recording: Recording,
    #[case] pages: usize,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let index = proving_ground.start_url(task).unwrap();
    let listing = ureq_get(&proving_ground, &index);
    let host = index.find("://").unwrap() + "://".len();
    let origin = index[..host + index[host..].find('/').unwrap()].to_owned();
    let seeds: Vec<Value> = listing
        .split("href=\"")
        .skip(1)
        .filter_map(|link| link.split('"').next())
        .map(|path| json!({ "url": format!("{origin}{path}"), "label": "page" }))
        .collect();
    let run_id = format!("formats-{}-{tier}", task.replace('/', "-"));

    let mut records = Vec::new();
    crawl_declaring(
        &mut engine,
        &run_id,
        &seeds,
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": tier },
                "politeness": { "domain": { "delay": 0 } },
            }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let mut record = recording(inside, &handle["page_id"]);
            record.insert("id".to_owned(), path_of(&handle["crawl_request"]["url"]));
            records.push(Value::Object(record));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    let verdict = proving_ground
        .judge_records(task, ToolStatus::Success, &records)
        .unwrap();
    assert_eq!(
        (
            seeds.len(),
            ended
                .iter()
                .filter(|outcome| outcome[1] == "succeeded")
                .count(),
            &verdict["outcome"],
        ),
        (pages, pages, &json!("verified")),
        "{verdict}"
    );
}

/// The page at `url` of `proving_ground`, fetched as a test reads it.
fn ureq_get(proving_ground: &ProvingGround, url: &str) -> String {
    crate::frontier::fetch(proving_ground, url).body
}

/// The fields a handler reads of the catalog's product and listing pages
/// in one `page.read`: value fields by CSS and `XPath`, a text and
/// attributes, a list, rows, fields neither page has, and a row's field
/// only the first row has.
fn catalog_fields() -> Value {
    let css = |selector: &str| json!({ "kind": "css", "selector": selector });
    let xpath = |selector: &str| json!({ "kind": "xpath", "selector": selector });
    json!({
        "heading": { "locator": css("h1") },
        "id": { "locator": xpath("//span[@class='id']") },
        "price": { "locator": css("span.price"), "attribute": "data-price" },
        "price_digits": {
            "locator": css("span.price"),
            "match": { "pattern": r"\$([0-9.]+)" },
        },
        "back": { "locator": css("main > a"), "attribute": "href" },
        "rating": { "locator": css(".rating") },
        "links": { "list": { "locator": css("a"), "attribute": "href" } },
        "products": {
            "rows": css("li.product"),
            "fields": {
                "name": { "locator": css("a") },
                "price": { "locator": xpath("./span[@class='price']") },
                "blurb": { "locator": css(".blurb") },
                "lead": {
                    "locator": xpath("self::li[not(preceding-sibling::li[@class='product'])]/a"),
                },
            },
        },
    })
}

/// `page.read` answers a page's fields alike on a Label forced to `http`
/// and on one forced to `browser` (ADR 0147): the same fields of the
/// catalog's product page and of its first listing page, crawled once on
/// each Label, read the same values, the same nulls and the same reasons,
/// a listing row keeping its other fields beside its missing blurb, and
/// only the first row reading its lead.
///
/// Covers: operation:page.read, setting:fetch.tier
#[test]
fn a_pages_fields_read_the_same_on_a_label_forced_to_http_and_on_one_forced_to_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let product = proving_ground.start_url("catalog/2").unwrap();
    let listing = proving_ground.start_url("catalog/1").unwrap();
    let run_id = "page-read-on-each-tier";
    let seeds: Vec<Value> = [&product, &listing]
        .into_iter()
        .flat_map(|url| {
            [
                json!({ "url": url, "label": "on_http" }),
                json!({ "url": format!("{url}?again"), "label": "on_browser" }),
            ]
        })
        .collect();

    let mut read = std::collections::BTreeMap::new();
    crawl_declaring(
        &mut engine,
        run_id,
        &seeds,
        &json!({
            "on_http": { "settings": { "fetch": { "tier": "http" } } },
            "on_browser": { "settings": { "fetch": { "tier": "browser" } } },
        }),
        &settings(
            &proving_ground,
            &json!({ "politeness": { "domain": { "delay": 0 } } }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let answer = inside.call(
                "page.read",
                &json!({ "page_id": handle["page_id"], "fields": catalog_fields() }),
            );
            let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
            read.insert(
                (
                    url.trim_end_matches("?again").to_owned(),
                    handle["tier"].as_str().unwrap_or_default().to_owned(),
                ),
                answer.get("result").cloned().unwrap_or(answer),
            );
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, run_id);
    assert!(engine.finished());

    assert_eq!(ended.len(), 4, "{ended:?}");
    for url in [&product, &listing] {
        let on = |tier: &str| read.get(&(url.clone(), tier.to_owned())).cloned();
        let (http, browser) = (on("http"), on("browser"));
        assert!(
            http.as_ref().is_some_and(|http| http["values"].is_object()),
            "{url}: {http:?}"
        );
        assert_eq!(browser, http, "{url}");
    }
    let listed = &read[&(listing.clone(), "http".to_owned())];
    let rows = listed["values"]["products"].as_array().unwrap();
    assert!(
        rows.len() > 1
            && rows.iter().all(|row| {
                row["name"].is_string() && row["price"].is_string() && row["blurb"].is_null()
            }),
        "{listed}"
    );
    let leads: Vec<bool> = rows.iter().map(|row| row["lead"].is_string()).collect();
    assert_eq!(
        (leads[0], leads[1..].iter().any(|lead| *lead)),
        (true, false),
        "{listed}"
    );
}
