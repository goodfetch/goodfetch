//! The lab's HTTP incidents and access-policy pages, ported (story 40), and
//! the content-coding pages (ADR 0119): each incident served on demand on
//! its task's shelf, and the oracle telling incomplete Records reported as a
//! success (a false success) from complete Records delivered late, judging
//! robots.txt (RFC 9309, the lab's policy cases carried over) from the
//! Proving ground's own observations, and holding each content-coding page's
//! decoded text or its decoding failure.

use std::io::Read as _;
use std::time::Duration;

use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;
use crate::raw_clients;

const SEED: u64 = 162;

/// The host name of task `id`, `<ordinal>.<family host>`.
fn task_host(proving_ground: &ProvingGround, id: &str) -> String {
    let url = proving_ground.task(id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    url.strip_prefix("https://")
        .and_then(|rest| rest.split(':').next())
        .expect("an https start URL")
        .to_owned()
}

/// The `id` and `price` of every Record on a shelf page's HTML.
fn records_on(html: &str) -> Vec<Value> {
    let mut records = Vec::new();
    for article in html.split("<article").skip(1) {
        let field = |class: &str| {
            article
                .split(&format!("class=\"{class}\">"))
                .nth(1)
                .and_then(|rest| rest.split('<').next())
                .map(str::to_owned)
        };
        if let (Some(id), Some(price)) = (field("id"), field("price")) {
            records.push(json!({ "id": id, "price": price }));
        }
    }
    records
}

/// Every Record of task `id`'s whole shelf, read from its pages' answers
/// once their incidents have passed.
fn shelf_records(proving_ground: &ProvingGround, host: &str) -> Vec<Value> {
    (1..=3)
        .flat_map(|page| {
            let path = format!("/catalog/{page}");
            let mut arrived = raw_clients::get(proving_ground, host, &path, &[]);
            if arrived.status() != Some(200) {
                arrived = raw_clients::get(proving_ground, host, &path, &[]);
            }
            records_on(&String::from_utf8_lossy(arrived.body()))
        })
        .collect()
}

/// Covers: story:2-40
#[test]
fn a_429_and_a_503_answer_once_then_the_page() {
    let proving_ground = ProvingGround::start(SEED);
    let limited = task_host(&proving_ground, "incidents/1");
    let unavailable = task_host(&proving_ground, "incidents/2");

    let first = raw_clients::get(&proving_ground, &limited, "/catalog/2", &[]);
    let then = raw_clients::get(&proving_ground, &limited, "/catalog/2", &[]);
    let once = raw_clients::get(&proving_ground, &unavailable, "/catalog/2", &[]);
    let after = raw_clients::get(&proving_ground, &unavailable, "/catalog/2", &[]);

    assert_eq!(first.status(), Some(429));
    assert_eq!(first.header("retry-after").as_deref(), Some("1"));
    assert_eq!(then.status(), Some(200));
    assert_eq!(once.status(), Some(503));
    assert_eq!(after.status(), Some(200));
    let records = shelf_records(&proving_ground, &limited);
    let answer = proving_ground
        .judge(&json!({ "task": "incidents/1", "status": "success", "records": records }));
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_truncated_body_a_dropped_connection_and_a_redirect_loop_never_give_the_page() {
    let proving_ground = ProvingGround::start(SEED);
    let truncated = task_host(&proving_ground, "incidents/3");
    let dropped = task_host(&proving_ground, "incidents/4");
    let looping = task_host(&proving_ground, "incidents/5");

    let cut = raw_clients::get(&proving_ground, &truncated, "/catalog/2", &[]);
    let gone = raw_clients::get(&proving_ground, &dropped, "/catalog/2", &[]);
    let back = raw_clients::get(&proving_ground, &looping, "/catalog/2", &[]);

    let declared: usize = cut
        .header("content-length")
        .and_then(|length| length.parse().ok())
        .expect("a declared length");
    assert!(cut.body().len() < declared);
    assert!(!cut.closed_cleanly);
    assert!(!String::from_utf8_lossy(cut.body()).contains("rel=\"next\""));
    assert_eq!(gone.header("transfer-encoding").as_deref(), Some("chunked"));
    assert!(!gone.closed_cleanly);
    assert_eq!(back.status(), Some(302));
    assert_eq!(back.header("location").as_deref(), Some("/catalog/2"));
}

#[test]
fn the_slow_page_gives_nothing_within_its_tasks_deadline() {
    let proving_ground = ProvingGround::start(SEED);
    let slow = task_host(&proving_ground, "incidents/6");

    let arrived = raw_clients::get_within(
        &proving_ground,
        &slow,
        "/catalog/2",
        &[],
        Some(Duration::from_millis(500)),
    );

    assert_eq!(arrived.bytes, Vec::<u8>::new());
}

/// The stalled-robots host answers its robots.txt's head and the start of
/// its body, and never the rest its length declares while the client
/// waits; its other paths are pages.
#[test]
fn the_stalled_robots_host_never_sends_the_end_of_its_robots_txt() {
    let proving_ground = ProvingGround::start(SEED);
    let host = "stalled-robots.access-policy.test";

    let robots = raw_clients::get_within(
        &proving_ground,
        host,
        "/robots.txt",
        &[],
        Some(Duration::from_secs(1)),
    );
    let page = raw_clients::get(&proving_ground, host, "/catalog/1", &[]);

    let declared: usize = robots
        .header("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or_default();
    assert_eq!(
        (
            robots.status(),
            String::from_utf8_lossy(robots.body()).starts_with("User-agent: *\nCrawl-delay: 2\n"),
            declared > robots.body().len(),
            robots.closed_cleanly,
            page.status(),
        ),
        (Some(200), true, true, false, Some(200))
    );
}

/// Covers: story:2-40
#[test]
fn the_oracle_tells_incomplete_records_claimed_as_success_from_complete_ones_delivered_late() {
    let proving_ground = ProvingGround::start(SEED);
    let host = task_host(&proving_ground, "incidents/6");
    let first = raw_clients::get(&proving_ground, &host, "/catalog/1", &[]);
    let third = raw_clients::get(&proving_ground, &host, "/catalog/3", &[]);
    let mut incomplete = records_on(&String::from_utf8_lossy(first.body()));
    incomplete.extend(records_on(&String::from_utf8_lossy(third.body())));
    let complete = shelf_records(&proving_ground, &host);

    let late = proving_ground
        .judge(&json!({ "task": "incidents/6", "status": "success", "records": complete }));
    let false_success = proving_ground
        .judge(&json!({ "task": "incidents/6", "status": "success", "records": incomplete }));
    let reported = proving_ground
        .judge(&json!({ "task": "incidents/6", "status": "failure", "records": incomplete }));

    assert_eq!(complete.len(), 6);
    assert_eq!(late["outcome"], "failed", "{late}");
    assert_eq!(late["late"], true);
    assert_eq!(false_success["outcome"], "false_success", "{false_success}");
    assert_eq!(false_success["missing"].as_array().map(Vec::len), Some(2));
    assert_eq!(false_success.get("late"), None);
    assert_eq!(reported["outcome"], "verified", "{reported}");
}

/// The lab's robots.txt cases (`identity-policy.test.ts`), carried over:
/// the oracle judges each request of the robots task by its `User-Agent`.
#[test]
fn the_robots_task_is_judged_by_rfc_9309_from_the_requests_made() {
    let proving_ground = ProvingGround::start(SEED);
    let host = task_host(&proving_ground, "access-policy/1");
    let robots = raw_clients::get(&proving_ground, &host, "/robots.txt", &[]);
    let first = raw_clients::get(
        &proving_ground,
        &host,
        "/catalog/1",
        &[("User-Agent", "Browser/1")],
    );
    let records = records_on(&String::from_utf8_lossy(first.body()));
    let submission = json!({ "task": "access-policy/1", "status": "success", "records": records });

    let obeyed = proving_ground.judge(&submission);
    for (agent, path) in [
        ("Browser/1", "/doc.pdf"),
        ("Browser/1", "/doc.pdf?download=1"),
        ("Browser/1", "/tie"),
        ("Browser/1", "/example/page/disallowed.gif"),
        ("GoodfetchLab/1.0", "/public/a"),
        ("GoodfetchLab/1.0", "/catalog/1"),
        ("Browser/1", "/private/catalog/2"),
    ] {
        raw_clients::get(&proving_ground, &host, path, &[("User-Agent", agent)]);
    }
    let disobeyed = proving_ground.judge(&submission);

    assert!(String::from_utf8_lossy(robots.body()).contains("Disallow: /private/"));
    assert!(String::from_utf8_lossy(first.body()).contains("href=\"/private/catalog/2\""));
    assert_eq!(obeyed["outcome"], "verified", "{obeyed}");
    assert_eq!(disobeyed["outcome"], "false_success", "{disobeyed}");
    assert_eq!(
        disobeyed["forbidden_requests"],
        json!([
            "/doc.pdf",
            "/example/page/disallowed.gif",
            "/catalog/1",
            "/private/catalog/2"
        ])
    );
}

/// The lab's other policy cases: an unreachable robots.txt and a
/// `Content-Signal` refuse the whole site to the purposes they name, `TDMRep`
/// reserves its catalog, and a page-level reservation, `noai`, `402`, `403`
/// and a challenge served with `200` each stop the crawl.
#[test]
fn site_policies_are_served_and_the_only_right_result_is_a_refusal() {
    let proving_ground = ProvingGround::start(SEED);
    let host = |ordinal: u64| task_host(&proving_ground, &format!("access-policy/{ordinal}"));
    let get = |ordinal, path| raw_clients::get(&proving_ground, &host(ordinal), path, &[]);

    assert_eq!(get(2, "/robots.txt").status(), Some(503));
    assert!(String::from_utf8_lossy(get(3, "/robots.txt").body()).contains("ai-train=no"));
    assert!(
        String::from_utf8_lossy(get(4, "/.well-known/tdmrep.json").body())
            .contains("\"location\":\"/catalog*\"")
    );
    assert_eq!(
        get(5, "/catalog/2").header("tdm-reservation").as_deref(),
        Some("1")
    );
    assert!(String::from_utf8_lossy(get(6, "/catalog/2").body()).contains("noai, noimageai"));
    assert_eq!(get(7, "/catalog/2").status(), Some(402));
    assert_eq!(get(8, "/catalog/2").status(), Some(403));
    let challenge = get(9, "/catalog/2");
    assert_eq!(challenge.status(), Some(200));
    assert!(String::from_utf8_lossy(challenge.body()).contains("Just a moment..."));

    let refused = |task: &str, records: &[Value]| {
        proving_ground.judge(&json!({ "task": task, "status": "failure", "records": records }))
    };
    let first_page = records_on(&String::from_utf8_lossy(get(7, "/catalog/1").body()));
    assert_eq!(
        refused("access-policy/7", &first_page)["outcome"],
        "verified"
    );
    let claimed = proving_ground
        .judge(&json!({ "task": "access-policy/7", "status": "success", "records": first_page }));
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
    assert_eq!(refused("access-policy/2", &[])["outcome"], "verified");
    get(3, "/catalog/1");
    let trained = refused("access-policy/3", &[]);
    assert_eq!(trained["outcome"], "failed", "{trained}");
    assert_eq!(trained["forbidden_requests"], json!(["/catalog/1"]));
}

/// The content-coding pages: each page's label and body, and the oracle's
/// decoded text or decoding failure.
#[test]
fn content_coding_pages_are_labelled_as_adr_0119_lists() {
    let proving_ground = ProvingGround::start(SEED);
    let page = |ordinal: u64| {
        let host = task_host(&proving_ground, &format!("content-coding/{ordinal}"));
        raw_clients::get(&proving_ground, &host, "/page", &[])
    };
    let gunzip = |bytes: &[u8]| {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut out)
            .map(|_| out)
    };

    let plain = page(1);
    assert_eq!(plain.header("content-encoding").as_deref(), Some("gzip"));
    assert!(plain.body().starts_with(b"<!doctype html>"));
    let invalid = page(2);
    assert_eq!(invalid.body()[..2], [0x1f, 0x8c]);
    assert_eq!(
        page(3).header("content-encoding").as_deref(),
        Some("gzip, br")
    );
    let deflated = page(4);
    assert_eq!(
        deflated.header("content-encoding").as_deref(),
        Some("deflate")
    );
    let mut inflated = String::new();
    flate2::read::DeflateDecoder::new(deflated.body())
        .read_to_string(&mut inflated)
        .expect("raw deflate");
    assert!(inflated.contains("Content coding page 4"));
    let unknown = page(5);
    assert_eq!(unknown.header("content-encoding").as_deref(), Some("utf-8"));
    let twice = page(6);
    let once = gunzip(twice.body()).expect("gzip once");
    assert!(once.starts_with(&[0x1f, 0x8b]));
    assert!(
        gunzip(&once)
            .expect("gzip twice")
            .starts_with(b"<!doctype html>")
    );
    let zstd = page(7);
    assert_eq!(zstd.header("content-encoding").as_deref(), Some("zstd"));
    assert_eq!(zstd.body()[..4], [0x28, 0xb5, 0x2f, 0xfd]);
    for (ordinal, label) in [(8, "gzip"), (9, "deflate"), (10, "br"), (11, "zstd")] {
        assert_eq!(
            page(ordinal).header("content-encoding").as_deref(),
            Some(label)
        );
    }
    let gzipped = gunzip(page(8).body()).expect("gzip");
    assert!(String::from_utf8_lossy(&gzipped).contains("Content coding page 8"));
    let mut zlib = String::new();
    flate2::read::ZlibDecoder::new(page(9).body())
        .read_to_string(&mut zlib)
        .expect("deflate with its zlib wrapper");
    assert!(zlib.contains("Content coding page 9"));

    let decoding_failed = json!({
        "code": "navigation_failed",
        "network_error": "net::ERR_CONTENT_DECODING_FAILED",
    });
    let failed = proving_ground.judge(
        &json!({ "task": "content-coding/1", "status": "failure", "error": decoding_failed }),
    );
    let other_error = proving_ground.judge(&json!({
        "task": "content-coding/6",
        "status": "failure",
        "error": { "code": "response_truncated" },
    }));
    let read = proving_ground.judge(&json!({
        "task": "content-coding/3",
        "status": "success",
        "records": [{ "id": "C-3", "text": "Content coding page 3, decoded once and as labelled." }],
    }));
    let garbled = proving_ground.judge(&json!({
        "task": "content-coding/7",
        "status": "success",
        "records": [{ "id": "C-7", "text": "(\u{fffd}/\u{fffd}" }],
    }));
    assert_eq!(failed["outcome"], "verified", "{failed}");
    assert_eq!(other_error["outcome"], "failed", "{other_error}");
    assert_eq!(other_error["wrong_error"]["expected"], decoding_failed);
    assert_eq!(read["outcome"], "verified", "{read}");
    assert_eq!(garbled["outcome"], "false_success", "{garbled}");
    let attempts = proving_ground.observations()["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .filter(|request| request["task"] == "content-coding/1")
        .count();
    assert_eq!(attempts, 1);
}
