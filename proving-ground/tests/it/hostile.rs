//! ADR 0067's hostile input beside SSRF: the `redirects`, `headers`,
//! `downloads` and `bombs` families, each page served with its expected
//! result, which names what a correct tool refuses, with which typed error
//! where one is decided, or keeps.
//!
//! A refusing tool is played by a submission that reports the failure the
//! expected result names; a non-refusing tool by one that follows the page
//! and reports what it reached, or keeps a file under the name the page
//! suggests.
//! Covers: story:2-83
#![expect(
    clippy::disallowed_types,
    reason = "the test reads malformed heads over its own socket, as no HTTP client reads them"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 312;

fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    proving_ground.task(id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// A failure that reports the typed error `error`.
fn refusal(task: &str, error: &Value) -> Value {
    json!({ "task": task, "status": "failure", "error": error })
}

/// The origin of a URL on one of the Proving ground's host names.
fn origin(url: &str) -> &str {
    let rest = url.strip_prefix("http://").expect("an http URL");
    &url[..7 + rest.find('/').unwrap_or(rest.len())]
}

/// Follows same-host `302` redirects from `url`, as a tool with no limit
/// does, and returns the page it ends on and how many it followed.
fn follow(proving_ground: &ProvingGround, url: &str) -> (String, u32) {
    let mut url = url.to_owned();
    let mut followed = 0;
    loop {
        let answer = proving_ground.get(&url);
        let Some(location) = answer.header("location").map(str::to_owned) else {
            return (answer.body, followed);
        };
        assert!(followed < 100, "the redirects end");
        url = format!("{}{location}", origin(&url));
        followed += 1;
    }
}

/// The Record a kept page shows.
fn read_record(page: &str) -> Value {
    let id = page
        .split("data-id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the page shows a Record's id");
    let word = page
        .split("<h1 class=\"word\">")
        .nth(1)
        .and_then(|rest| rest.split('<').next())
        .expect("the page shows a Record's word");
    json!({ "id": id, "word": word })
}

#[rstest]
#[case::a_301_to_loopback("redirects/1", 301, "http://127.0.0.1/", "private_address")]
#[case::a_307_to_a_mapped_address(
    "redirects/10",
    307,
    "http://[::ffff:127.0.0.1]/",
    "private_address"
)]
#[case::a_302_to_a_file("redirects/11", 302, "file:///etc/passwd", "scheme")]
#[case::a_302_to_gopher("redirects/15", 302, "gopher://127.0.0.1:6379/_INFO", "scheme")]
fn a_redirect_case_answers_as_its_stand_in_and_its_typed_refusal_is_verified(
    #[case] id: &str,
    #[case] status: u16,
    #[case] location: &str,
    #[case] rule: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get(&start_url(&proving_ground, id));

    let answer = proving_ground.judge(&refusal(
        id,
        &json!({ "code": "egress_refused", "rule": rule }),
    ));

    assert_eq!(page.status, status);
    assert_eq!(page.header("location"), Some(location));
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_redirect_case_refused_with_another_typed_error_fails_and_names_both() {
    let proving_ground = ProvingGround::start(SEED);
    let reported = json!({ "code": "egress_refused", "rule": "scheme" });

    let answer = proving_ground.judge(&refusal("redirects/1", &reported));

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(
        answer["wrong_error"]["expected"],
        json!({ "code": "egress_refused", "rule": "private_address" }),
        "{answer}"
    );
    assert_eq!(
        answer["wrong_error"]["submitted"]["rule"], "scheme",
        "{answer}"
    );
}

#[rstest]
#[case::claimed_a_success("success", "false_success")]
#[case::reported_needs_review("needs-review", "failed")]
fn a_tool_that_does_not_refuse_a_redirect_case_is_judged_a_failure(
    #[case] status: &str,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission("redirects/6", status, &[]));

    assert_eq!(answer["outcome"], outcome, "{answer}");
}

#[rstest]
#[case::a_refresh_header("redirects/6")]
#[case::a_meta_refresh("redirects/7")]
#[case::a_script("redirects/8")]
fn a_redirect_without_a_status_names_its_target_in_the_page(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&start_url(&proving_ground, id));

    assert_eq!(page.status, 200);
    let named = page.header("refresh").unwrap_or_default().to_owned() + &page.body;
    assert!(named.contains("http://127.0.0.1/"), "{named}");
}

#[test]
fn a_tool_that_refuses_the_redirect_to_the_proving_grounds_own_loopback_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get(&start_url(&proving_ground, "redirects/16"));
    let error = json!({ "code": "egress_refused", "rule": "private_address" });

    let answer = proving_ground.judge(&refusal("redirects/16", &error));

    assert_eq!(page.status, 302);
    assert_eq!(
        page.header("location"),
        Some(
            format!(
                "http://127.0.0.1:{}/redirects/landing",
                proving_ground.port()
            )
            .as_str()
        )
    );
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_tool_that_follows_the_redirect_to_its_own_loopback_is_recorded_reaching_it() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get(&start_url(&proving_ground, "redirects/16"));
    let landing = page.header("location").expect("a redirect").to_owned();
    let reached = proving_ground.control(
        landing
            .strip_prefix(&format!("http://127.0.0.1:{}", proving_ground.port()))
            .expect("a landing on the Proving ground's own address"),
    );
    let error = json!({ "code": "egress_refused", "rule": "private_address" });

    let answer = proving_ground.judge(&refusal("redirects/16", &error));

    assert_eq!(reached.status, 200);
    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(answer["findings"][0]["check"], "private_address_reached");
}

#[test]
fn a_chain_of_19_redirects_is_followed_to_its_record() {
    let proving_ground = ProvingGround::start(SEED);
    let (page, followed) = follow(&proving_ground, &start_url(&proving_ground, "redirects/17"));

    let answer = proving_ground.judge(&submission(
        "redirects/17",
        "success",
        &[read_record(&page)],
    ));

    assert_eq!(followed, 19);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// The chain a domain policy of `redirects.test` refuses at its third
/// redirect: two within the family's host, then one to `catalog.test`.
#[test]
fn a_chain_leaves_its_hosts_family_at_its_third_redirect() {
    let proving_ground = ProvingGround::start(SEED);
    let mut url = proving_ground.url("17.redirects.test", "/leaves/3");
    let mut locations = Vec::new();

    for _ in 0..3 {
        let answer = proving_ground.get(&url);
        let location = answer.header("location").expect("a redirect").to_owned();
        url = if location.starts_with('/') {
            format!("{}{location}", origin(&url))
        } else {
            location.clone()
        };
        locations.push(location);
    }
    let landed = proving_ground.get(&url);

    assert_eq!(
        (locations, landed.status),
        (
            vec![
                "/leaves/2".to_owned(),
                "/leaves/1".to_owned(),
                proving_ground.url("catalog.test", "/catalog/1"),
            ],
            200
        )
    );
}

#[rstest]
#[case::a_chain_of_20("redirects/18")]
#[case::a_loop("redirects/19")]
fn too_many_redirects_is_the_only_right_answer_past_19(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let refused = proving_ground.judge(&refusal(id, &json!({ "code": "too_many_redirects" })));
    let claimed = proving_ground.judge(&submission(id, "success", &[]));

    assert_eq!(refused["outcome"], "verified", "{refused}");
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
}

#[test]
fn a_tool_that_follows_a_chain_of_20_and_keeps_its_record_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let (page, followed) = follow(&proving_ground, &start_url(&proving_ground, "redirects/18"));

    let answer = proving_ground.judge(&submission(
        "redirects/18",
        "success",
        &[read_record(&page)],
    ));

    assert_eq!(followed, 20);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// The whole response to a `GET` of `url` over a new HTTP/1.1 connection,
/// as its bytes arrive, read to the connection's end: what a client reads
/// of a head no HTTP library parses. An `https` URL is read over plain
/// HTTP/1.1 too, which the Proving ground's one address also serves.
fn raw_get(proving_ground: &ProvingGround, url: &str) -> Vec<u8> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .expect("an http or https URL");
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let mut stream =
        TcpStream::connect(proving_ground.address()).expect("the Proving ground listens");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .expect("the request is sent");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("the response is read");
    response
}

/// The head of a raw response, up to its blank line.
fn head(response: &[u8]) -> &[u8] {
    let end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("the response has a whole head");
    &response[..end + 2]
}

/// The body of a raw response, after its blank line, as text.
fn body(response: &[u8]) -> String {
    let start = head(response).len() + 2;
    String::from_utf8(response[start..].to_vec()).expect("the body is UTF-8")
}

fn holds(bytes: &[u8], line: &[u8]) -> bool {
    bytes.windows(line.len()).any(|window| window == line)
}

#[rstest]
#[case::a_conflicting_content_length("headers/2", b"Content-Length: ".as_slice())]
#[case::a_nul_in_a_value("headers/5", b"X-Note: one\0two\r\n".as_slice())]
#[case::a_bare_cr_in_a_value("headers/6", b"X-Note: one\rSet-Cookie: injected=1\r\n".as_slice())]
#[case::a_line_without_a_colon("headers/7", b"\r\nX-Note one two\r\n".as_slice())]
fn a_malformed_head_is_sent_as_its_bytes_and_only_a_refusal_is_verified(
    #[case] id: &str,
    #[case] line: &[u8],
) {
    let proving_ground = ProvingGround::start(SEED);
    let response = raw_get(&proving_ground, &start_url(&proving_ground, id));
    let kept = read_record(&body(&response));

    let refused = proving_ground.judge(&submission(id, "failure", &[]));
    let claimed = proving_ground.judge(&submission(id, "success", &[kept]));

    assert!(
        holds(head(&response), line),
        "{}",
        String::from_utf8_lossy(head(&response))
    );
    assert_eq!(refused["outcome"], "verified", "{refused}");
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
}

#[test]
fn two_different_content_lengths_are_both_sent() {
    let proving_ground = ProvingGround::start(SEED);

    let response = raw_get(&proving_ground, &start_url(&proving_ground, "headers/2"));

    let head = String::from_utf8_lossy(head(&response)).into_owned();
    assert_eq!(head.matches("Content-Length: ").count(), 2, "{head}");
}

#[rstest]
#[case::a_repeated_content_length("headers/4", b"Content-Length: ".as_slice())]
#[case::a_folded_line("headers/8", b"X-Note: one\r\n two\r\n".as_slice())]
fn a_head_the_browser_keeps_is_sent_as_its_bytes_and_its_record_is_verified(
    #[case] id: &str,
    #[case] line: &[u8],
) {
    let proving_ground = ProvingGround::start(SEED);
    let response = raw_get(&proving_ground, &start_url(&proving_ground, id));
    let kept = read_record(&body(&response));

    let answer = proving_ground.judge(&submission(id, "success", &[kept]));
    let refused = proving_ground.judge(&submission(id, "failure", &[]));

    assert!(holds(head(&response), line));
    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(refused["outcome"], "failed", "{refused}");
}

#[test]
fn an_oversized_head_is_sent_and_only_a_refusal_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let response = raw_get(&proving_ground, &start_url(&proving_ground, "headers/1"));

    let refused = proving_ground.judge(&submission("headers/1", "failure", &[]));
    let claimed = proving_ground.judge(&submission(
        "headers/1",
        "success",
        &[read_record(&body(&response))],
    ));

    assert!(head(&response).len() > 256 * 1024);
    assert_eq!(refused["outcome"], "verified", "{refused}");
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
}

#[test]
fn a_redirect_with_two_locations_is_sent_with_both() {
    let proving_ground = ProvingGround::start(SEED);

    let response = raw_get(&proving_ground, &start_url(&proving_ground, "headers/3"));

    let head = String::from_utf8_lossy(head(&response)).to_lowercase();
    assert!(
        head.contains("location: /two-locations/first\r\n"),
        "{head}"
    );
    assert!(
        head.contains("location: /two-locations/second\r\n"),
        "{head}"
    );
}

#[test]
fn a_tool_that_sends_an_injected_cookie_back_as_set_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "headers/9");
    let first = proving_ground.get(&start);
    let cookie = first.cookie("note").expect("the page sets its cookie");
    let next = format!(
        "{}{}",
        origin(&start),
        first.header("location").expect("a redirect")
    );

    let page = proving_ground.get_with_cookie(&next, Some(&cookie));
    let answer = proving_ground.judge(&submission(
        "headers/9",
        "success",
        &[read_record(&page.body)],
    ));

    assert_eq!(cookie, "note=one%0D%0AX-Injected%3A%20yes");
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_tool_that_decodes_an_injected_cookie_is_recorded_and_fails() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "headers/9");
    let next = format!("{}/cookie-injection/next", origin(&start));

    let page = proving_ground.get_with_cookie(&next, Some("note=one"));
    let answer = proving_ground.judge(&submission(
        "headers/9",
        "success",
        &[read_record(&page.body)],
    ));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["findings"][0]["check"], "header_injection",
        "{answer}"
    );
}

#[test]
fn a_location_holding_an_encoded_crlf_is_followed_with_its_encoding_untouched() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "headers/10");

    let (page, followed) = follow(&proving_ground, &start);
    let answer = proving_ground.judge(&submission("headers/10", "success", &[read_record(&page)]));

    assert_eq!(followed, 1);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// A result that returns the task's download, saved under `name`.
fn saved_as(proving_ground: &ProvingGround, id: &str, name: &str) -> Value {
    let file = proving_ground.get_raw(&start_url(proving_ground, id));
    assert_eq!(file.status, 200);
    json!({
        "task": id,
        "status": "success",
        "files": [{ "name": name, "hex": data_encoding::HEXLOWER.encode(&file.body) }],
    })
}

#[rstest]
#[case::a_parent_path(
    "downloads/2",
    "attachment; filename=\"../../../../tmp/goodfetch-traversal.csv\""
)]
#[case::a_windows_drive(
    "downloads/5",
    "attachment; filename=\"C:\\\\Windows\\\\goodfetch-drive.csv\""
)]
#[case::an_encoded_parent_path(
    "downloads/13",
    "attachment; filename*=UTF-8''..%2F..%2Fgoodfetch-encoded.csv"
)]
fn a_download_suggests_its_hostile_name(#[case] id: &str, #[case] disposition: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&start_url(&proving_ground, id));

    assert_eq!(page.header("content-disposition"), Some(disposition));
}

#[rstest]
#[case::a_parent_path("downloads/2", "goodfetch-traversal.csv")]
#[case::a_windows_drive("downloads/5", "goodfetch-drive.csv")]
#[case::a_reserved_name("downloads/8", "CON_.csv")]
#[case::a_trailing_dot("downloads/10", "report.csv")]
#[case::an_alternate_data_stream("downloads/12", "report.csv_hidden")]
#[case::a_control_character("downloads/15", "report_.csv")]
#[case::an_overlong_name("downloads/18", "aaaa.csv")]
fn a_hostile_name_kept_as_one_safe_name_is_verified(#[case] id: &str, #[case] name: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&saved_as(&proving_ground, id, name));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::a_parent_path("downloads/2", "../../../../tmp/goodfetch-traversal.csv")]
#[case::a_kept_backslash_path("downloads/4", "..\\..\\goodfetch-traversal.csv")]
#[case::a_unc_path("downloads/7", "\\\\server\\share\\goodfetch-unc.csv")]
#[case::a_reserved_name("downloads/8", "CON.csv")]
#[case::a_numbered_device("downloads/9", "LPT1")]
#[case::a_trailing_space("downloads/11", "report.csv ")]
#[case::an_alternate_data_stream("downloads/12", "report.csv:hidden")]
#[case::a_decoded_parent_path("downloads/13", "../../goodfetch-encoded.csv")]
#[case::a_control_character("downloads/15", "report\u{7}\n.csv")]
fn a_tool_that_saves_under_the_hostile_name_is_judged_a_failure(
    #[case] id: &str,
    #[case] name: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&saved_as(&proving_ground, id, name));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["unsafe_files"], json!([name]), "{answer}");
}

#[test]
fn a_name_too_long_for_a_file_system_is_refused_as_saved() {
    let proving_ground = ProvingGround::start(SEED);
    let name = format!("{}.csv", "a".repeat(296));

    let answer = proving_ground.judge(&saved_as(&proving_ground, "downloads/18", &name));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[rstest]
#[case::a_plain_name("downloads/1", "export.csv")]
#[case::a_latin_1_name("downloads/16", "r\u{e9}sum\u{e9}.csv")]
fn a_harmless_name_is_kept_exactly(#[case] id: &str, #[case] name: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let kept = proving_ground.judge(&saved_as(&proving_ground, id, name));
    let renamed = proving_ground.judge(&saved_as(&proving_ground, id, "download.csv"));

    assert_eq!(kept["outcome"], "verified", "{kept}");
    assert_ne!(renamed["outcome"], "verified", "{renamed}");
}

// --- bombs ---

/// The body of a raw response, after its blank line, as its bytes: a bomb's
/// compressed body, which is not UTF-8.
fn body_bytes(response: &[u8]) -> &[u8] {
    let start = head(response).len() + 2;
    &response[start..]
}

/// The bytes `body` decodes to under gzip, as a tool with no limit would
/// inflate them.
fn gunzip(body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(body)
        .read_to_end(&mut decoded)
        .expect("the gzip body decodes");
    decoded
}

/// The typed error a decompression bomb's refusal names.
fn ratio_refused() -> Value {
    json!({ "code": "response_limit_exceeded", "setting": "network.max_decompression_ratio" })
}

#[rstest]
#[case::gzip("bombs/1", "gzip")]
#[case::deflate("bombs/2", "deflate")]
#[case::brotli("bombs/3", "br")]
#[case::zstd("bombs/4", "zstd")]
#[case::double_gzip("bombs/5", "gzip, gzip")]
fn a_decompression_bomb_is_served_compressed_and_only_a_refusal_is_verified(
    #[case] id: &str,
    #[case] encoding: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let response = raw_get(&proving_ground, &start_url(&proving_ground, id));

    assert!(
        holds(
            head(&response),
            format!("content-encoding: {encoding}").as_bytes()
        ),
        "served under its coding: {}",
        String::from_utf8_lossy(head(&response))
    );
    assert!(
        body_bytes(&response).len() < 256 * 1024,
        "the body is small and compressed, {} bytes",
        body_bytes(&response).len()
    );

    let refused = proving_ground.judge(&refusal(id, &ratio_refused()));
    let claimed = proving_ground.judge(&submission(id, "success", &[]));

    assert_eq!(refused["outcome"], "verified", "{refused}");
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
}

#[test]
fn a_gzip_bomb_decodes_far_past_the_ratio_limit() {
    let proving_ground = ProvingGround::start(SEED);
    let response = raw_get(&proving_ground, &start_url(&proving_ground, "bombs/1"));
    let compressed = body_bytes(&response);

    let decoded = gunzip(compressed);

    assert!(
        decoded.len() >= 10 * 1024 * 1024,
        "the body decodes to megabytes, not {} bytes",
        decoded.len()
    );
    assert!(
        decoded.len() / compressed.len() > 100,
        "far past the 100:1 ratio the engine allows"
    );
}

#[test]
fn a_bomb_refused_with_the_wrong_limit_setting_fails_and_names_both() {
    let proving_ground = ProvingGround::start(SEED);
    let reported = json!({ "code": "response_limit_exceeded", "setting": "network.max_body_size" });

    let answer = proving_ground.judge(&refusal("bombs/1", &reported));

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(
        answer["wrong_error"]["expected"],
        ratio_refused(),
        "{answer}"
    );
}

#[test]
fn the_safe_compressed_page_is_kept() {
    let proving_ground = ProvingGround::start(SEED);
    let response = raw_get(&proving_ground, &start_url(&proving_ground, "bombs/6"));
    assert!(holds(head(&response), b"content-encoding: gzip"));

    let page = String::from_utf8(gunzip(body_bytes(&response))).expect("the safe page is UTF-8");
    let answer = proving_ground.judge(&submission("bombs/6", "success", &[read_record(&page)]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}
