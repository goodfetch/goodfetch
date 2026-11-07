//! Response decoding conformance cases (ADR 0119; pitfalls H13, H16, H17),
//! the tier-dependent lines, on Foxhound's executable against the Proving
//! ground and Chrome for Testing: every page of the `encodings` family
//! reads the oracle's text on `http` (reqwest), on browser-fetch (the
//! `http` tier of a Session that may move, under `auto`) and on `browser`,
//! in one order: a BOM, `fetch.encoding`, the `Content-Type` charset, the
//! `<meta>` prescan, then UTF-8. A page that declares nothing and is not
//! UTF-8 escalates to the browser under `auto`, recorded as `detected`, and
//! fails with `browser_required` naming `fetch.encoding` and `fetch.tier`
//! under a forced `http`, while `fetch.encoding` on its Label reads it
//! there; a BOM still wins. An XHTML page declaring its encoding only in
//! its XML declaration reads on every tier without escalating. A response
//! with no `Content-Type` is typed as the pinned Chromium's sniffer types
//! it, and a listed 301 with neither a type nor a body reaches its handler.
//! A failed content coding is `navigation_failed` with
//! `net::ERR_CONTENT_DECODING_FAILED`, retried under `crawl.retry.max`, an
//! unknown coding passes its body through and a cut compressed body is
//! `response_truncated`.
//! The browser tier reads an unlabelled UTF-8 page as UTF-8 too, the
//! engine setting its charset at the interception, while its handler's
//! response keeps the site's headers; Chrome alone would read such a page
//! in its locale's default.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

use goodfetch_test_client::Attached;
use goodfetch_test_support::ToolStatus;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings, tiers};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 764;

/// The `encodings` family's host and task.
const ENCODINGS: &str = "encodings.test";
const TASK: &str = "encodings/1";

/// Every page of the family, as its index links it.
const PAGES: [&str; 18] = [
    "shift-jis",
    "euc-kr",
    "gb18030",
    "big5",
    "windows-1251",
    "iso-8859-2",
    "iso-8859-15",
    "utf-16-bom",
    "contradicting-meta",
    "undeclared-utf-8",
    "undeclared-legacy",
    "latin1-label",
    "ascii-label",
    "meta-utf-16-label",
    "x-user-defined",
    "late-meta-in-head",
    "meta-after-body",
    "undecodable-byte",
];

/// The page only a guess reads.
const LEGACY: &str = "undeclared-legacy";

/// The pages that declare nothing Chrome reads and are valid UTF-8, the
/// one whose `<meta>` comes after its body included, which Chrome alone
/// reads in its locale's default.
const UNLABELLED_UTF_8: [&str; 2] = ["undeclared-utf-8", "meta-after-body"];

/// The settings of a crawl of the Proving ground on `tier`, with `more`.
fn on(proving_ground: &ProvingGround, tier: &str, more: &Value) -> Value {
    let mut more = more.clone();
    more["fetch"]["tier"] = json!(tier);
    more["politeness"] = json!({ "domain": { "delay": 0 } });
    more["site_preferences"] = json!({ "respect_robots_txt": false });
    settings(proving_ground, &more)
}

/// The id and text the Markdown `markdown` of the encodings page `id`
/// shows, as the oracle's Record holds them: the page's main content is its
/// text paragraph, after any filler, without its heading.
fn record(id: &str, markdown: &Value) -> Value {
    let text = markdown
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or_default();
    json!({ "id": id, "text": text })
}

/// The handler: it reads its page's Markdown and keeps it by tier and by
/// the page's id, the last segment of its path.
fn reading(
    read: &mut BTreeMap<(String, String), Value>,
) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        let formats = inside.call(
            "page.formats",
            &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
        );
        let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
        let id = url.rsplit('/').next().unwrap_or_default().to_owned();
        let tier = handle["tier"].as_str().unwrap_or_default().to_owned();
        read.insert((tier, id), formats["result"]["markdown"].clone());
        json!({ "result": { "records": [] } })
    }
}

/// Each request's end by the last segment of its path: its kind, its
/// error's code and details, and its attempts.
fn ended(engine: &mut Attached, run_id: &str) -> BTreeMap<String, (Value, Value, Value, Value)> {
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
            let url = outcome["request"]["url"].as_str().unwrap_or_default();
            (
                url.rsplit('/').next().unwrap_or_default().to_owned(),
                (
                    outcome["kind"].clone(),
                    outcome["error"]["code"].clone(),
                    outcome["error"]["details"].clone(),
                    outcome["attempts"].clone(),
                ),
            )
        })
        .collect()
}

/// The oracle's verdict on the Records `read` on `tier` shows, but
/// `leaving_out`: its wrong texts and its missing pages.
fn judged(
    proving_ground: &ProvingGround,
    read: &BTreeMap<(String, String), Value>,
    tier: &str,
    leaving_out: &[&str],
) -> (Value, Value) {
    let records: Vec<Value> = read
        .iter()
        .filter(|((on, id), _)| on == tier && !leaving_out.contains(&id.as_str()))
        .map(|((_, id), markdown)| record(id, markdown))
        .collect();
    let verdict = proving_ground
        .judge_records(TASK, ToolStatus::Success, &records)
        .unwrap();
    // The Markdown holds no title, the oracle's other field.
    let wrong: Vec<Value> = verdict["wrong"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|wrong| wrong["field"] != "title")
        .cloned()
        .collect();
    (json!(wrong), verdict["missing"].clone())
}

/// The seeds of every encodings page but `leaving_out`, each of its own
/// Label, `label` naming it.
fn seeds(
    proving_ground: &ProvingGround,
    leaving_out: &[&str],
    label: fn(&str) -> String,
) -> Vec<Value> {
    PAGES
        .iter()
        .filter(|id| !leaving_out.contains(id))
        .map(|id| {
            json!({
                "url": proving_ground.url(ENCODINGS, &format!("/pages/{id}")),
                "label": label(id),
            })
        })
        .collect()
}

/// Under a forced `http` tier every page reads the oracle's text but the
/// one only a guess reads, which fails at once with `browser_required`
/// naming `fetch.encoding` and `fetch.tier`; under a forced `browser` every
/// page reads it as Chrome reads it.
///
/// Covers: story:5-129, setting:fetch.tier, operation:page.formats
#[rstest::rstest]
/// Covers: error:browser_required
#[case::http("http", &[], json!([LEGACY]))]
#[case::browser("browser", &[], json!([]))]
fn every_encodings_page_reads_the_oracles_text_under_a_forced_tier(
    #[case] tier: &str,
    #[case] leaving_out: &[&str],
    #[case] missing: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("encodings-{tier}");

    let mut read = BTreeMap::new();
    let finished = crawl_declaring(
        &mut engine,
        &run_id,
        &seeds(&proving_ground, leaving_out, |_| "page".to_owned()),
        &json!({ "page": {} }),
        &on(&proving_ground, tier, &json!({})),
        &mut reading(&mut read),
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(
        judged(&proving_ground, &read, tier, &[]),
        (json!([]), missing),
        "{read:?}"
    );
    if tier == "http" {
        let (kind, code, details, attempts) = &ended[LEGACY];
        assert_eq!(
            (kind, code, &details["call"], &details["key"], attempts),
            (
                &json!("failed"),
                &json!("browser_required"),
                &json!("fetch.encoding"),
                &json!("fetch.tier"),
                &json!(1)
            ),
            "{details}"
        );
        let fix = details["fix"].as_str().unwrap_or_default();
        assert!(
            fix.contains("`fetch.encoding`") && fix.contains("`fetch.tier"),
            "{fix}"
        );
    }
}

/// Under `auto`, each page the first of its own Label and so a sample, it
/// is read on browser-fetch and then in the browser, each reading the
/// oracle's text; the page only a guess reads escalates before its
/// handler runs on browser-fetch, its browser run recorded in Provenance
/// as escalating at `fetch.encoding`, its encoding `detected`, and no page
/// fails.
///
/// Covers: story:5-129, story:5-121, setting:fetch.tier
#[test]
fn every_encodings_page_reads_the_same_text_on_browser_fetch_and_in_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let labels: serde_json::Map<String, Value> = PAGES
        .iter()
        .map(|id| ((*id).to_owned(), json!({})))
        .collect();

    let mut read = BTreeMap::new();
    let finished = crawl_declaring(
        &mut engine,
        "encodings-auto",
        &seeds(&proving_ground, &[], ToOwned::to_owned),
        &Value::Object(labels),
        &on(&proving_ground, "auto", &json!({})),
        &mut reading(&mut read),
    );
    let ended = ended(&mut engine, "encodings-auto");
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert!(
        ended.values().all(|(kind, ..)| kind == "succeeded"),
        "{ended:?}"
    );
    assert_eq!(
        (
            judged(&proving_ground, &read, "http", &[]),
            judged(&proving_ground, &read, "browser", &[]),
        ),
        ((json!([]), json!([LEGACY])), (json!([]), json!([]))),
        "{read:?}"
    );
    let escalated: Vec<Value> =
        crate::fetch_tiers::provenance(&env, "encodings-auto", "fetch_tier")
            .into_iter()
            .filter(|entry| entry["label"] == LEGACY)
            .map(|entry| {
                json!([
                    entry["tier"],
                    entry["chosen"],
                    entry["call"],
                    entry["encoding_source"]
                ])
            })
            .collect();
    assert_eq!(
        escalated,
        vec![
            json!(["http", "sample", null, null]),
            json!(["browser", "sample", "fetch.encoding", "detected"]),
        ]
    );
}

/// On a forced `browser` tier every page a document tier reads is read as
/// that tier reads it: the same text, the same encoding from the same
/// source in Provenance, and the handler's response's `Content-Type` the
/// site's own, whatever charset the engine set at the interception. The
/// two pages that declare nothing Chrome reads and are valid UTF-8 are read
/// as UTF-8, from `utf8`.
///
/// Covers: story:5-129, setting:fetch.tier, operation:page.formats
#[test]
fn the_browser_reads_every_page_as_the_http_tier_does_and_keeps_the_sites_headers() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let labels: serde_json::Map<String, Value> = PAGES
        .iter()
        .map(|id| ((*id).to_owned(), json!({})))
        .collect();

    let mut seen: BTreeMap<(String, String), Value> = BTreeMap::new();
    for tier in ["http", "browser"] {
        let finished = crawl_declaring(
            &mut engine,
            &format!("same-{tier}"),
            &seeds(&proving_ground, &[LEGACY], ToOwned::to_owned),
            &Value::Object(labels.clone()),
            &on(&proving_ground, tier, &json!({})),
            &mut |inside: &mut Inside<'_>, handle: &Value| {
                let formats = inside.call(
                    "page.formats",
                    &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
                );
                let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
                let id = url.rsplit('/').next().unwrap_or_default().to_owned();
                seen.insert(
                    (tier.to_owned(), id.clone()),
                    json!({
                        "record": record(&id, &formats["result"]["markdown"]),
                        "content_type": handle["response"]["headers"]["content-type"],
                    }),
                );
                json!({ "result": { "records": [] } })
            },
        );
        assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    }
    assert!(engine.finished());

    let read = |tier: &str| -> BTreeMap<String, Value> {
        let entries = crate::fetch_tiers::provenance(&env, &format!("same-{tier}"), "fetch_tier");
        PAGES
            .iter()
            .filter(|id| **id != LEGACY)
            .map(|id| {
                let entry = entries.iter().find(|entry| entry["label"] == *id);
                let mut read = seen
                    .get(&(tier.to_owned(), (*id).to_owned()))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                read["encoding"] = entry.map_or(Value::Null, |entry| entry["encoding"].clone());
                read["encoding_source"] =
                    entry.map_or(Value::Null, |entry| entry["encoding_source"].clone());
                ((*id).to_owned(), read)
            })
            .collect()
    };
    let (http, browser) = (read("http"), read("browser"));
    assert_eq!(browser, http);
    for id in UNLABELLED_UTF_8 {
        assert_eq!(
            (&browser[id]["encoding"], &browser[id]["encoding_source"]),
            (&json!("UTF-8"), &json!("utf8")),
            "{id}"
        );
    }
    for (id, read) in &browser {
        let head = served(
            &proving_ground,
            &proving_ground.url(ENCODINGS, &format!("/pages/{id}")),
        )
        .0;
        let site = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-type")
                    .then(|| json!(value.trim()))
            })
            .unwrap_or(Value::Null);
        assert_eq!(read["content_type"], site, "{id}");
    }
}

/// `fetch.encoding` on a Label reads its page on a forced `http`, where it
/// declares nothing, with the setting; a BOM still beats it.
///
/// Covers: story:5-129, setting:fetch.encoding
#[test]
fn fetch_encoding_on_a_label_reads_an_undeclared_page_and_a_bom_still_wins() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let forced = json!({ "settings": { "fetch": { "encoding": "shift_jis" } } });
    let seed = |id: &str| json!({ "url": proving_ground.url(ENCODINGS, &format!("/pages/{id}")), "label": "forced" });

    let mut read = BTreeMap::new();
    let finished = crawl_declaring(
        &mut engine,
        "encodings-forced",
        &[seed(LEGACY), seed("utf-16-bom")],
        &json!({ "forced": forced }),
        &on(&proving_ground, "http", &json!({})),
        &mut reading(&mut read),
    );
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let (wrong, missing) = judged(&proving_ground, &read, "http", &[]);
    assert_eq!(wrong, json!([]), "{read:?}");
    let missing: Vec<&str> = missing
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        !missing.contains(&LEGACY) && !missing.contains(&"utf-16-bom"),
        "{missing:?}"
    );
}

/// An XHTML page in windows-1251, declared only in its XML declaration,
/// reads the same text on every tier, and under `auto` its sample's run on
/// browser-fetch does not escalate.
///
/// Covers: story:5-129
#[rstest::rstest]
#[case::http("http")]
#[case::browser("browser")]
#[case::auto("auto")]
fn an_xhtml_page_declared_by_its_xml_declaration_reads_on_every_tier(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("xhtml-{tier}");

    let mut read = BTreeMap::new();
    let finished = crawl_declaring(
        &mut engine,
        &run_id,
        &[json!({ "url": proving_ground.url(ENCODINGS, "/xhtml/declared"), "label": "page" })],
        &json!({ "page": {} }),
        &on(&proving_ground, tier, &json!({})),
        &mut reading(&mut read),
    );
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let tiers_read: Vec<&str> = read.keys().map(|(tier, _)| tier.as_str()).collect();
    let expected: &[&str] = match tier {
        "auto" => &["browser", "http"],
        other => &[other],
    };
    assert_eq!(tiers_read, expected);
    for markdown in read.values() {
        assert!(
            markdown
                .as_str()
                .is_some_and(|text| text.contains("Москва — столица России.")),
            "{markdown}"
        );
    }
    if tier == "auto" {
        assert!(
            tiers(&env, &run_id).iter().all(|entry| entry[4].is_null()),
            "{:?}",
            tiers(&env, &run_id)
        );
    }
}

/// The head and the bytes of the page at `url`, as the Proving ground
/// serves them.
#[expect(
    clippy::disallowed_types,
    reason = "the test reads the Proving ground's bytes from outside the engine (ADR 0060)"
)]
fn served(proving_ground: &ProvingGround, url: &str) -> (String, Vec<u8>) {
    let rest = url.strip_prefix("http://").unwrap();
    let (authority, path) = rest.split_at(rest.find('/').unwrap());
    let mut stream = std::net::TcpStream::connect(proving_ground.address()).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let at = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let body = response.split_off(at + 4);
    (String::from_utf8_lossy(&response).into_owned(), body)
}

/// The bytes of the page at `url`, as the Proving ground serves them.
fn bytes(proving_ground: &ProvingGround, url: &str) -> Vec<u8> {
    served(proving_ground, url).1
}

/// A response with no `Content-Type` is typed as the pinned Chromium's own
/// sniffer types it, over at most its first 1,024 bytes: the browser
/// renders HTML, XML and plain text as the type the non-browser tiers'
/// sniffer gives their bytes, and renders no document for the binary
/// bytes and the PDF, which the sniffer types as no text. The browser's
/// own sniffer is read on a program's page, which no document reader
/// judges: on a crawl handler's page the engine types such a response
/// with the same sniffer at its interception.
///
/// Covers: story:5-129
#[test]
fn a_response_with_no_content_type_is_typed_as_the_pinned_chromium_sniffs_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let cases = ["html", "xml", "text", "binary", "pdf"];
    let urls: Vec<String> = cases
        .iter()
        .map(|case| proving_ground.url("fetch-edge.test", &format!("/sniffed/{case}")))
        .collect();

    let mut none = |_: &Value| None;
    let started = engine.call(
        "start",
        "run.start",
        &json!({ "run_id": "sniffed", "settings": settings(&proving_ground, &json!({})) }),
        &mut none,
    );
    assert!(started.get("result").is_some(), "{started}");
    let mut seen = BTreeMap::new();
    for (n, url) in urls.iter().enumerate() {
        let opened = engine.call(
            &format!("page-{n}"),
            "run.new_page",
            &json!({ "run_id": "sniffed" }),
            &mut none,
        );
        let page_id = opened["result"]["page_id"].clone();
        engine.call(
            &format!("goto-{n}"),
            "page.goto",
            &json!({ "page_id": page_id, "url": url }),
            &mut none,
        );
        let typed = engine.call(
            &format!("typed-{n}"),
            "page.evaluate",
            &json!({
                "page_id": page_id,
                "function": "() => [document.contentType, location.href]",
            }),
            &mut none,
        );
        seen.insert(url.clone(), typed["result"]["value"].clone());
    }
    assert!(engine.finished());

    for url in &urls {
        let body = bytes(&proving_ground, url);
        let path = url
            .rsplit_once(':')
            .map_or("", |(_, rest)| &rest[rest.find('/').unwrap()..]);
        let sniffed = goodfetch_fetch_tier::sniff(&body, path);
        let rendered = seen.get(url).cloned().unwrap_or(Value::Null);
        if ["text/html", "text/xml", "text/plain"].contains(&sniffed) {
            assert_eq!(rendered, json!([sniffed, url]), "{url}");
        } else {
            assert_ne!(
                rendered[1],
                json!(url),
                "{url} rendered as {rendered}, sniffed {sniffed}"
            );
        }
    }
}

/// A 301 that `crawl.handle_statuses` lists, with neither a
/// `Content-Type` nor a body, reaches its handler unfollowed: there is
/// nothing to type.
///
/// Covers: story:5-129, setting:crawl.handle_statuses
#[test]
fn a_listed_301_with_no_type_and_no_body_reaches_its_handler() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let url = proving_ground.url("fetch-edge.test", "/redirect/301/untyped");

    let mut handled = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "untyped-301",
        &[json!({ "url": url, "label": "page" })],
        &json!({ "page": {} }),
        &on(
            &proving_ground,
            "http",
            &json!({ "crawl": { "handle_statuses": [301] } }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let formats = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            handled.push((handle["tier"].clone(), formats.get("error").cloned()));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "untyped-301");
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(handled, vec![(json!("http"), None)]);
    assert_eq!(ended["untyped"].0, json!("succeeded"));
}

/// On the crawl: a body labelled gzip that is plain fails with
/// `navigation_failed` carrying `net::ERR_CONTENT_DECODING_FAILED`, tried
/// again under `crawl.retry.max`; a cut compressed body fails with
/// `response_truncated`, tried again too; an unknown coding passes its
/// body through to the handler.
///
/// Covers: story:5-128, story:5-129, setting:crawl.retry.max, error:navigation_failed, error:response_truncated
#[test]
fn content_codings_fail_as_chrome_fails_them_and_are_retried_on_the_crawl() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let mislabelled = proving_ground.url("1.content-coding.test", "/page");
    let unknown = proving_ground.url("5.content-coding.test", "/page");
    let cut = proving_ground.url("fetch-edge.test", "/cut/gzip-content-length");

    let mut read = BTreeMap::new();
    let finished = crawl_declaring(
        &mut engine,
        "codings",
        &[&mislabelled, &unknown, &cut]
            .iter()
            .map(|url| json!({ "url": url, "label": "page" }))
            .collect::<Vec<_>>(),
        &json!({ "page": {} }),
        &on(
            &proving_ground,
            "http",
            &json!({ "crawl": { "retry": { "max": 1 } } }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let formats = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
            read.insert(url, formats["result"]["markdown"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "codings" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let outcomes: BTreeMap<String, Value> = answer["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                json!([
                    outcome["kind"],
                    outcome["error"]["code"],
                    outcome["error"]["details"]["error"],
                    outcome["attempts"]
                ]),
            )
        })
        .collect();
    assert_eq!(
        (
            &outcomes[&mislabelled],
            &outcomes[&unknown],
            &outcomes[&cut]
        ),
        (
            &json!([
                "failed",
                "navigation_failed",
                "net::ERR_CONTENT_DECODING_FAILED",
                2
            ]),
            &json!(["succeeded", null, null, 1]),
            &json!(["failed", "response_truncated", null, 2]),
        )
    );
    assert!(
        read[&unknown]
            .as_str()
            .is_some_and(|text| text.contains("Content coding page 5")),
        "{read:?}"
    );
}
