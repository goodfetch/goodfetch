//! Response decoding conformance cases (ADR 0119; pitfalls H16 and H17),
//! the content-type and Provenance lines, on Foxhound's executable against
//! the Proving ground: under the built-in `crawl.content_types` a PDF, a
//! ZIP, a JSON response, a PDF served with no `Content-Type` and a 404
//! `crawl.handle_statuses` hands over end `skipped` with `content_type`,
//! never retried and never handled, an endless PDF's body unread; a Label
//! listing `application/pdf` gets the PDF's bytes in its handler's
//! response and a page holding no document, whose document calls fail at
//! once with `response_not_document` and never escalate; and Provenance
//! names each encodings page's encoding and where it came from. On the
//! `browser` tier the check runs at the interception, before Chrome
//! renders the response, so no file starts a download; a handler's own
//! `goto` to a PDF is not checked and gives its `Response` with its body,
//! its page holding no document, on every tier; and a program's own page
//! is untouched by any of it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, provenance, settings};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 950;

/// The `files` family's host.
const FILES: &str = "files.test";

/// The settings of a crawl of the Proving ground on `tier`, with `more`.
fn on(proving_ground: &ProvingGround, tier: &str, more: &Value) -> Value {
    let mut more = more.clone();
    more["fetch"]["tier"] = json!(tier);
    more["politeness"] = json!({ "domain": { "delay": 0 } });
    more["site_preferences"] = json!({ "respect_robots_txt": false });
    settings(proving_ground, &more)
}

/// Each request's end by its URL's path: its kind, its skip reason, its
/// error's code and its attempts.
fn ended(engine: &mut Attached, run_id: &str) -> BTreeMap<String, Value> {
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
            let path = url.splitn(4, '/').nth(3).unwrap_or_default();
            (
                format!("/{path}"),
                json!([
                    outcome["kind"],
                    outcome["reason"],
                    outcome["error"]["code"],
                    outcome["attempts"]
                ]),
            )
        })
        .collect()
}

/// The bytes of the response at `url`, as the Proving ground serves them.
#[expect(
    clippy::disallowed_types,
    reason = "the test reads the Proving ground's bytes from outside the engine (ADR 0060)"
)]
fn bytes(proving_ground: &ProvingGround, url: &str) -> Vec<u8> {
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
    response.split_off(at + 4)
}

/// The bytes of the body `handle` names, read from its file.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's body file from outside the engine, on no async worker"
)]
fn body(handle: &Value) -> Vec<u8> {
    std::fs::read(handle["path"].as_str().unwrap()).unwrap()
}

/// Under the built-in `crawl.content_types`, every file the `files` family
/// links ends `skipped` with `content_type` on its first attempt and its
/// handler never runs, a 404 `crawl.handle_statuses` hands over included,
/// while its HTML index is handled. On a forced `http` tier, and on a
/// forced `browser` tier, whose interception judges the response's head
/// before Chrome renders it, an endless PDF is skipped too, its body
/// unread: reading it would never end, and the crawl would outlast the
/// engine's deadline.
///
/// Covers: story:6-264, setting:crawl.content_types, setting:crawl.handle_statuses
#[rstest::rstest]
#[case::http("http")]
#[case::auto("auto")]
/// Covers: story:5-130
#[case::browser("browser")]
fn the_built_in_content_types_skip_every_file_before_its_handler(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("files-{tier}");
    let mut paths = vec![
        "/files",
        "/files/report.pdf",
        "/files/archive.zip",
        "/files/data.json",
        "/files/missing.csv",
        "/files/untyped-document",
    ];
    let mut seeds: Vec<Value> = paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url(FILES, path), "label": "page" }))
        .collect();
    // Browser-fetch, which a Session that may move sends through, reads a
    // body whole before it hands its head over.
    if tier != "auto" {
        paths.push("/endless/pdf");
        seeds.push(
            json!({ "url": proving_ground.url("fetch-edge.test", "/endless/pdf"), "label": "page" }),
        );
    }

    let mut handled = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        &run_id,
        &seeds,
        &json!({ "page": {} }),
        &on(
            &proving_ground,
            tier,
            &json!({ "crawl": { "handle_statuses": [404] } }),
        ),
        &mut |_: &mut Inside<'_>, handle: &Value| {
            handled.push(handle["crawl_request"]["url"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    // Under `auto` the index is a sample, handled on each tier.
    handled.dedup();
    assert_eq!(handled, vec![json!(proving_ground.url(FILES, "/files"))]);
    let skipped = json!(["skipped", "content_type", null, 1]);
    let expected: BTreeMap<String, Value> = paths
        .iter()
        .map(|path| {
            let end = if *path == "/files" {
                json!(["succeeded", null, null, 1])
            } else {
                skipped.clone()
            };
            ((*path).to_owned(), end)
        })
        .collect();
    assert_eq!(ended, expected);
}

/// A Label listing `application/pdf` gets the PDF: its handler's response
/// holds its status, its type and its bytes, readable through
/// `response.body`, and its page holds no document, so `page.formats` and
/// a locator's wait fail at once with `response_not_document` naming the
/// type and the call, and under `auto` nothing escalates. On a forced
/// `browser` tier the PDF is read at the interception and its navigation
/// stopped, so the same holds and no download starts.
///
/// Covers: story:6-264, setting:crawl.content_types, error:response_not_document, operation:response.body
#[rstest::rstest]
#[case::http("http")]
#[case::auto("auto")]
/// Covers: story:5-130
#[case::browser("browser")]
fn an_accepted_pdf_reaches_its_handler_with_its_bytes_and_no_document(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("pdf-{tier}");
    let url = proving_ground.url(FILES, "/files/report.pdf");
    let pdf = json!({ "settings": { "crawl": { "content_types": ["application/pdf"] } } });

    let mut seen = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        &run_id,
        &[json!({ "url": url, "label": "pdf" })],
        &json!({ "pdf": pdf }),
        &on(&proving_ground, tier, &json!({})),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let response = handle["response"].clone();
            let read = inside.call(
                "response.body",
                &json!({ "page_id": handle["page_id"], "request_id": response["request_id"] }),
            );
            let formats = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            let waited = inside.call(
                "locator.wait_for",
                &json!({ "locator": { "page_id": handle["page_id"], "selector": "body" } }),
            );
            seen.push((handle["tier"].clone(), response, read, formats, waited));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, &run_id);
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(
        ended["/files/report.pdf"],
        json!(["succeeded", null, null, 1])
    );
    let [(tier_seen, response, read, formats, waited)] = seen.as_slice() else {
        panic!("one handler run: {seen:?}");
    };
    let expected_tier = if tier == "browser" { "browser" } else { "http" };
    assert_eq!(tier_seen, expected_tier);
    assert_eq!(
        (&response["status"], &response["headers"]["content-type"]),
        (&json!(200), &json!("application/pdf")),
        "{response}"
    );
    assert_eq!(
        body(&read["result"]),
        bytes(&proving_ground, &url),
        "{read}"
    );
    for (call, answered) in [("page.formats", formats), ("locator.wait_for", waited)] {
        let data = &answered["error"]["data"];
        assert_eq!(
            (&data["code"], &data["content_type"], &data["call"]),
            (
                &json!("response_not_document"),
                &json!("application/pdf"),
                &json!(call)
            ),
            "{answered}"
        );
    }
    let escalated: Vec<Value> = provenance(&env, &run_id, "fetch_tier")
        .into_iter()
        .filter(|entry| entry["tier"] == "browser" && entry["chosen"] != "forced")
        .collect();
    assert_eq!(escalated, Vec::<Value>::new());
}

/// A handler's own `goto` to a PDF is not judged by `crawl.content_types`:
/// it answers the PDF's `Response`, its status, its type and its bytes
/// readable through `response.body`, and no download starts; its page then
/// holds no document, so `page.formats` fails with `response_not_document`;
/// a `goto` back to an HTML page reads its document again.
///
/// Covers: story:5-130, error:response_not_document, operation:page.goto, operation:response.body
#[rstest::rstest]
#[case::http("http")]
#[case::browser("browser")]
fn a_handlers_own_goto_to_a_pdf_answers_its_response_and_body_and_no_document(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let run_id = format!("goto-pdf-{tier}");
    let index = proving_ground.url(FILES, "/files");
    let pdf = proving_ground.url(FILES, "/files/report.pdf");

    let mut seen = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        &run_id,
        &[json!({ "url": index, "label": "page" })],
        &json!({ "page": {} }),
        &on(&proving_ground, tier, &json!({})),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let page_id = &handle["page_id"];
            let went = inside.call("page.goto", &json!({ "page_id": page_id, "url": pdf }));
            let response = went["result"]["response"].clone();
            let read = inside.call(
                "response.body",
                &json!({ "page_id": page_id, "request_id": response["request_id"] }),
            );
            let formats = inside.call(
                "page.formats",
                &json!({ "page_id": page_id, "formats": { "markdown": true } }),
            );
            let back = inside.call("page.goto", &json!({ "page_id": page_id, "url": index }));
            let again = inside.call(
                "page.formats",
                &json!({ "page_id": page_id, "formats": { "markdown": true } }),
            );
            seen.push((went, read, formats, back, again));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let [(went, read, formats, back, again)] = seen.as_slice() else {
        panic!("one handler run: {seen:?}");
    };
    let response = &went["result"]["response"];
    assert_eq!(
        (&response["status"], &response["headers"]["content-type"]),
        (&json!(200), &json!("application/pdf")),
        "{went}"
    );
    assert_eq!(
        body(&read["result"]),
        bytes(&proving_ground, &pdf),
        "{read}"
    );
    let data = &formats["error"]["data"];
    assert_eq!(
        (&data["code"], &data["content_type"], &data["call"]),
        (
            &json!("response_not_document"),
            &json!("application/pdf"),
            &json!("page.formats")
        ),
        "{formats}"
    );
    assert_eq!(back["result"]["response"]["status"], 200, "{back}");
    assert!(again["result"]["markdown"].is_string(), "{again}");
}

/// A program's own page, outside any crawl handler, is Collie's as it is:
/// its `goto` to a PDF downloads the file and answers no response, and an
/// unlabelled UTF-8 page is read in Chrome's own default, the engine
/// setting no charset at its interception.
///
/// Covers: story:5-130
#[test]
fn a_programs_own_page_downloads_a_pdf_and_reads_an_unlabelled_page_as_chrome_does() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let settings = settings(&proving_ground, &json!({}));
    let mut none = |_: &Value| None;
    let started = engine.call(
        "start",
        "run.start",
        &json!({ "run_id": "program", "settings": settings }),
        &mut none,
    );
    assert!(started.get("result").is_some(), "{started}");
    let opened = engine.call(
        "page",
        "run.new_page",
        &json!({ "run_id": "program" }),
        &mut none,
    );
    let page_id = opened["result"]["page_id"].clone();
    let pdf = engine.call(
        "pdf",
        "page.goto",
        &json!({ "page_id": page_id, "url": proving_ground.url(FILES, "/files/report.pdf") }),
        &mut none,
    );
    let unlabelled = engine.call(
        "unlabelled",
        "page.goto",
        &json!({
            "page_id": page_id,
            "url": proving_ground.url("encodings.test", "/pages/undeclared-utf-8"),
        }),
        &mut none,
    );
    assert!(engine.finished());

    assert_eq!(pdf["result"], json!({ "response": null }), "{pdf}");
    let encoding = &unlabelled["result"]["response"]["encoding"];
    assert!(encoding.is_string() && encoding != "UTF-8", "{unlabelled}");
}

/// Provenance's `fetch_tier` entry of each encodings page names the
/// encoding its document was read with and where that came from, as the
/// oracle's `encodings/2` expects: `bom`, `header`, `meta` and `utf8` from
/// the run on browser-fetch, and `detected` from the browser's run of the
/// page only a guess reads; and `setting` for that page under
/// `fetch.encoding` on its Label.
///
/// Covers: story:6-265, setting:fetch.encoding
#[test]
fn provenance_names_each_documents_encoding_and_where_it_came_from() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let host = "2.encodings.test";
    let index = bytes(&proving_ground, &proving_ground.url(host, "/pages"));
    let index = String::from_utf8_lossy(&index);
    let pages: Vec<&str> = index
        .split("href=\"/pages/")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert_eq!(pages.len(), 18, "{index}");
    let labels: serde_json::Map<String, Value> = pages
        .iter()
        .map(|id| ((*id).to_owned(), json!({})))
        .collect();
    let seeds: Vec<Value> = pages
        .iter()
        .map(|id| json!({ "url": proving_ground.url(host, &format!("/pages/{id}")), "label": id }))
        .collect();

    let finished = crawl_declaring(
        &mut engine,
        "provenance-auto",
        &seeds,
        &Value::Object(labels),
        &on(&proving_ground, "auto", &json!({})),
        &mut |_: &mut Inside<'_>, _: &Value| json!({ "result": { "records": [] } }),
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let forced = json!({ "settings": { "fetch": { "encoding": "shift_jis" } } });
    let finished = crawl_declaring(
        &mut engine,
        "provenance-setting",
        &[
            json!({ "url": proving_ground.url(host, "/pages/undeclared-legacy"), "label": "forced" }),
        ],
        &json!({ "forced": forced }),
        &on(&proving_ground, "http", &json!({})),
        &mut |_: &mut Inside<'_>, _: &Value| json!({ "result": { "records": [] } }),
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert!(engine.finished());

    // Each page's encoding: where a document tier read it, else the
    // browser's guess.
    let entries = provenance(&env, "provenance-auto", "fetch_tier");
    let records: Vec<Value> = pages
        .iter()
        .map(|id| {
            let of_page = |tier: &str| {
                entries.iter().find(|entry| {
                    entry["label"] == *id
                        && entry["tier"] == tier
                        && entry["encoding_source"].is_string()
                })
            };
            let entry = of_page("http").or_else(|| of_page("browser"));
            json!({
                "id": id,
                "encoding": entry.map(|entry| entry["encoding"].clone()),
                "encoding_source": entry.map(|entry| entry["encoding_source"].clone()),
            })
        })
        .collect();
    let verdict = proving_ground
        .judge_records("encodings/2", "success", &records)
        .unwrap();
    let wrong: Vec<&Value> = verdict["wrong"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|wrong| wrong["field"] == "encoding" || wrong["field"] == "encoding_source")
        .collect();
    assert_eq!(wrong, Vec::<&Value>::new(), "{records:?}");
    let sources: Vec<&Value> = records
        .iter()
        .map(|record| &record["encoding_source"])
        .collect();
    for source in ["bom", "header", "meta", "utf8", "detected"] {
        assert!(sources.contains(&&json!(source)), "{source}: {sources:?}");
    }

    let set: Vec<Value> = provenance(&env, "provenance-setting", "fetch_tier")
        .into_iter()
        .map(|entry| json!([entry["tier"], entry["encoding"], entry["encoding_source"]]))
        .collect();
    assert_eq!(set, vec![json!(["http", "Shift_JIS", "setting"])]);
}
