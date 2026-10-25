//! Response decoding conformance cases (ADR 0119; pitfalls H16 and H17),
//! the content-type and Provenance lines, on Foxhound's executable against
//! the Proving ground: under the built-in `crawl.content_types` a PDF, a
//! ZIP, a JSON response, a PDF served with no `Content-Type` and a 404
//! `crawl.handle_statuses` hands over end `skipped` with `content_type`,
//! never retried and never handled, an endless PDF's body unread; a Label
//! listing `application/pdf` gets the PDF's bytes in its handler's
//! response and a page holding no document, whose document calls fail at
//! once with `response_not_document` and never escalate; and Provenance
//! names each encodings page's encoding and where it came from.
//!
//! The browser tier's own lines (the check at its interception, no
//! Download) are #765's.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
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
/// while its HTML index is handled. On a forced `http` tier an endless PDF
/// is skipped too, its body unread: reading it would never end, and the
/// crawl would outlast the engine's deadline.
///
/// Covers: story:6-264, setting:crawl.content_types, setting:crawl.handle_statuses
#[rstest::rstest]
#[case::http("http")]
#[case::auto("auto")]
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
    if tier == "http" {
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
    assert!(engine.finish());

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
/// type and the call, and under `auto` nothing escalates.
///
/// Covers: story:6-264, setting:crawl.content_types, error:response_not_document, operation:response.body
#[rstest::rstest]
#[case::http("http")]
#[case::auto("auto")]
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
    assert!(engine.finish());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(
        ended["/files/report.pdf"],
        json!(["succeeded", null, null, 1])
    );
    let [(tier_seen, response, read, formats, waited)] = seen.as_slice() else {
        panic!("one handler run: {seen:?}");
    };
    assert_eq!(tier_seen, "http");
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
        .filter(|entry| entry["tier"] == "browser")
        .collect();
    assert_eq!(escalated, Vec::<Value>::new());
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
    assert!(engine.finish());

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
