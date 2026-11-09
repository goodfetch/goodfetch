//! The `body-limits` family (ADR 0067, SC-05): a body one byte past 64 MiB,
//! a chunked body and a body with no length that never end, and JSON nested
//! one level past 128, each verified only as a refusal naming its setting;
//! JSON nested exactly 128 deep is kept; and a gzip body within the
//! decompression ratio that decodes past the 256 MiB a browser's pipe
//! frames, at a task's start page and at a named host's robots.txt.
//! Covers: story:2-83

use flate2::read::GzDecoder;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::raw_clients;

const SEED: u64 = 644;

/// 64 MiB, `network.max_body_size`'s default.
const MAX_BODY_SIZE: usize = 64 * 1024 * 1024;

/// The 256 MiB a browser's pipe frames in one message (`MAX_FRAME`).
const MAX_FRAME: u64 = 256 * 1024 * 1024;

/// The decoded bytes per received byte `network.max_decompression_ratio`
/// allows by default.
const MAX_DECOMPRESSION_RATIO: u64 = 100;

/// The named host whose robots.txt is the coded body.
const ROBOTS_HOST: &str = "robots.body-limits.test";

/// The host and path of a task's start URL.
fn start(proving_ground: &ProvingGround, id: &str) -> (String, String) {
    let url = proving_ground.task(id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    let rest = url.strip_prefix("https://").expect("an https URL");
    let (authority, path) = rest.split_at(rest.find('/').expect("a path"));
    let host = authority.split(':').next().expect("a host");
    (host.to_owned(), path.to_owned())
}

/// A failure that reports `response_limit_exceeded` naming `setting`.
fn refusal(task: &str, setting: &str) -> Value {
    json!({
        "task": task,
        "status": "failure",
        "error": { "code": "response_limit_exceeded", "setting": setting },
    })
}

/// How deep `json` nests its arrays and objects, by its brackets: the
/// family's JSON holds none inside its strings.
fn depth(json: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut deepest = 0;
    for byte in json {
        match byte {
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth -= 1,
            _ => {}
        }
    }
    assert_eq!(depth, 0, "every bracket is closed");
    deepest
}

#[test]
fn the_body_past_the_size_says_its_length_and_sends_it_whole() {
    let proving_ground = ProvingGround::start(SEED);
    let (host, path) = start(&proving_ground, "body-limits/1");

    let head = raw_clients::get_prefix(&proving_ground, &host, &path, 1024);
    let total = raw_clients::count(&proving_ground, &host, &path);

    assert_eq!(head.status(), Some(200));
    assert_eq!(
        head.header("content-length"),
        Some((MAX_BODY_SIZE + 1).to_string())
    );
    let head_length = head.bytes.len() - head.body().len();
    assert_eq!(total - head_length, MAX_BODY_SIZE + 1);
}

#[rstest]
#[case::chunked("body-limits/2", Some("chunked"), None)]
#[case::until_closed("body-limits/3", None, Some("close"))]
fn an_endless_body_keeps_coming_past_the_size(
    #[case] id: &str,
    #[case] transfer_encoding: Option<&str>,
    #[case] connection: Option<&str>,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (host, path) = start(&proving_ground, id);

    let arrived = raw_clients::get_prefix(&proving_ground, &host, &path, MAX_BODY_SIZE + 4096);

    assert_eq!(arrived.status(), Some(200));
    assert_eq!(arrived.header("content-length"), None);
    assert_eq!(
        arrived.header("transfer-encoding").as_deref(),
        transfer_encoding
    );
    assert_eq!(arrived.header("connection").as_deref(), connection);
    assert!(
        arrived.body().len() > MAX_BODY_SIZE,
        "still sending past 64 MiB, {} bytes",
        arrived.body().len()
    );
}

#[rstest]
#[case::past_the_depth("body-limits/4", 129)]
#[case::at_the_depth("body-limits/5", 128)]
fn the_apis_json_nests_as_deep_as_its_task_says(#[case] id: &str, #[case] expected: usize) {
    let proving_ground = ProvingGround::start(SEED);
    let (host, path) = start(&proving_ground, id);

    let arrived = raw_clients::get(&proving_ground, &host, &path, &[]);

    assert_eq!(
        arrived.header("content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(depth(arrived.body()), expected);
}

/// Each JSON task's host serves its JSON as its `TDMRep` file too, the read
/// of JSON a crawl makes before it sends a request; another task's does not.
#[rstest]
#[case::past_the_depth("body-limits/4", 200, 129)]
#[case::at_the_depth("body-limits/5", 200, 128)]
#[case::no_json_task("body-limits/1", 404, 0)]
fn a_json_tasks_host_serves_its_json_as_its_tdmrep_file(
    #[case] id: &str,
    #[case] status: u16,
    #[case] expected: usize,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (host, _) = start(&proving_ground, id);

    let arrived = raw_clients::get(&proving_ground, &host, "/.well-known/tdmrep.json", &[]);

    assert_eq!(arrived.status(), status);
    assert_eq!(depth(arrived.body()), expected);
}

/// How many bytes `body` decodes to through gzip, which a body whose
/// checksum or length is wrong fails.
fn gunzipped(body: &[u8]) -> u64 {
    std::io::copy(&mut GzDecoder::new(body), &mut std::io::sink())
        .expect("the body decodes, its checksum and length right")
}

#[rstest]
#[case::start_page(None)]
#[case::robots_txt(Some((ROBOTS_HOST, "/robots.txt")))]
fn the_coded_body_decodes_past_the_frame_within_the_ratio(#[case] at: Option<(&str, &str)>) {
    let proving_ground = ProvingGround::start(SEED);
    let (host, path) = at.map_or_else(
        || start(&proving_ground, "body-limits/6"),
        |(host, path)| (host.to_owned(), path.to_owned()),
    );

    let arrived = raw_clients::get(&proving_ground, &host, &path, &[]);

    assert_eq!(arrived.status(), Some(200));
    assert_eq!(arrived.header("content-encoding").as_deref(), Some("gzip"));
    let sent = u64::try_from(arrived.body().len()).unwrap();
    let decoded = gunzipped(arrived.body());
    assert!(decoded > MAX_FRAME, "decodes to {decoded} bytes");
    assert!(
        decoded < sent * MAX_DECOMPRESSION_RATIO / 2,
        "{decoded} decoded from {sent} sent, well within the ratio"
    );
}

#[test]
fn the_robots_hosts_other_pages_are_small_and_plain() {
    let proving_ground = ProvingGround::start(SEED);

    let arrived = raw_clients::get(&proving_ground, ROBOTS_HOST, "/page", &[]);

    assert_eq!(arrived.status(), Some(200));
    assert_eq!(arrived.header("content-encoding"), None);
    assert!(
        arrived.body().len() < 1024,
        "{} bytes",
        arrived.body().len()
    );
}

#[rstest]
#[case::past_the_size("body-limits/1", "network.max_body_size")]
#[case::endless_chunked("body-limits/2", "network.max_body_size")]
#[case::endless_until_closed("body-limits/3", "network.max_body_size")]
#[case::json_past_the_depth("body-limits/4", "network.max_json_depth")]
#[case::coded_past_the_frame("body-limits/6", "network.max_body_size")]
fn only_a_refusal_naming_the_setting_is_verified(#[case] id: &str, #[case] setting: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let refused = proving_ground.judge(&refusal(id, setting));
    let claimed = proving_ground.judge(&submission(id, "success", &[]));
    let other = proving_ground.judge(&refusal(id, "network.max_decompression_ratio"));

    assert_eq!(refused["outcome"], "verified", "{refused}");
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
    assert_eq!(other["outcome"], "failed", "{other}");
}

#[test]
fn the_json_at_the_depth_is_kept_as_its_innermost_record() {
    let proving_ground = ProvingGround::start(SEED);
    let record = json!({ "id": "B-5", "text": "Nested as deep as the engine keeps." });

    let kept = proving_ground.judge(&submission("body-limits/5", "success", &[record]));

    assert_eq!(kept["outcome"], "verified", "{kept}");
}
