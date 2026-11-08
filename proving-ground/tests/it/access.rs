//! The `access` family, the first rung of ADR 0035's Access family on the
//! local copy: fingerprint consistency from HTTP to the browser judged from
//! the server's own observations, self-hosted probes in the manner of `BotD`
//! and `CreepJS`, a probe of the debugging protocol's side effects, the version
//! check and the block-detection corpus. Every result can carry the Access
//! dimensions it was run under, which the answer names with the build's age,
//! so stock headless and stock headful Chrome serve as controls.
//!
//! A passing tool sends every page the same fingerprint, or posts nothing
//! but that a probe ran; a failing tool changes its fingerprint between
//! pages, or posts the signal a probe found.
//! Covers: story:2-33, story:2-37, story:2-39

use std::fmt::Write as _;

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::clients::{self, Http2Settings};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 167;

/// A Chrome Stable user agent of the major `major`, as headful Chrome sends.
fn chrome(major: u32) -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    )
}

/// The user agent stock headless Chrome sends.
fn headless_chrome(major: u32) -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/{major}.0.0.0 Safari/537.36"
    )
}

/// The client hints of Chrome's major `major`.
fn brands(major: u32) -> String {
    format!(r#""Chromium";v="{major}", "Google Chrome";v="{major}", "Not.A/Brand";v="99""#)
}

/// The first capture of `pattern` in `text`.
fn capture(text: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .unwrap_or_else(|| panic!("{pattern} in {text}"))[1]
        .to_owned()
}

/// The task `id`'s start URL.
fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .find(|task| task["id"] == id)
        .and_then(|task| task["start_url"].as_str())
        .unwrap_or_else(|| panic!("the task list names {id}"))
        .to_owned()
}

/// The host name of a URL.
fn host(url: &str) -> String {
    capture(url, r"http://([^:/]+)")
}

/// The origin of a URL: its scheme, host and port.
fn origin(url: &str) -> String {
    capture(url, r"^(http://[^/]+)")
}

/// The Record a page of the family shows.
fn read_record(page: &str) -> Value {
    json!({
        "id": capture(page, r#"<article class="record" data-id="([^"]+)""#),
        "code": capture(page, r#"<span class="code">([^<]+)</span>"#),
    })
}

/// The checks the answer's findings name, in order.
fn checks(answer: &Value) -> Vec<String> {
    answer["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .map(|finding| finding["check"].as_str().expect("a check").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The HTTP/2 settings the test's browser sends.
const SETTINGS: Http2Settings = Http2Settings {
    initial_window_size: 6_291_456,
    initial_connection_window_size: 15_728_640,
    max_concurrent_streams: 1000,
    header_table_size: 65_536,
};

/// Fetches `path` on `host` over HTTP/2 with TLS, sending `headers`.
fn fetch_http2(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> String {
    let config = clients::trusting(&proving_ground.certificate_authority(), &["h2"]);
    let answers = clients::http2(
        proving_ground.address(),
        host,
        config,
        &SETTINGS,
        &[path],
        headers,
    );
    String::from_utf8(answers[0].body.clone()).expect("a UTF-8 page")
}

/// Fetches `path` on `host` over HTTP/1.1 with TLS, sending `headers` after
/// `Host`.
fn fetch_http1(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> String {
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n");
    for (name, value) in headers {
        write!(request, "{name}: {value}\r\n").expect("writing to a String succeeds");
    }
    request.push_str("Connection: close\r\n\r\n");
    let exchange = clients::raw_tls(proving_ground.address(), host, config, request.as_bytes())
        .expect("the handshake succeeds");
    String::from_utf8(exchange.body().to_vec()).expect("a UTF-8 page")
}

/// How the second page's request differs from the first's.
enum Change {
    Nothing,
    Protocol,
    HeaderOrder,
    UserAgent,
    ClientHints,
}

/// Fetches the consistency task's two pages, the second with `change`, and
/// returns their Records.
fn fetch_consistency_pages(proving_ground: &ProvingGround, change: &Change) -> Vec<Value> {
    let url = start_url(proving_ground, "access/1");
    let host = host(&url);
    let user_agent = chrome(155);
    let hints = brands(155);
    let headers = [
        ("sec-ch-ua", hints.as_str()),
        ("sec-ch-ua-mobile", "?0"),
        ("user-agent", user_agent.as_str()),
        ("accept", "text/html"),
    ];
    let first = fetch_http2(proving_ground, &host, "/consistency/1", &headers);
    let second_path = capture(&first, r#"<a class="next" href="([^"]+)""#);
    let other_agent = chrome(154);
    let other_hints = brands(154);
    let second = match change {
        Change::Nothing => fetch_http2(proving_ground, &host, &second_path, &headers),
        Change::Protocol => fetch_http1(proving_ground, &host, &second_path, &headers),
        Change::HeaderOrder => {
            let reordered = [headers[2], headers[0], headers[1], headers[3]];
            fetch_http2(proving_ground, &host, &second_path, &reordered)
        }
        Change::UserAgent => {
            let changed = [
                headers[0],
                headers[1],
                ("user-agent", other_agent.as_str()),
                headers[3],
            ];
            fetch_http2(proving_ground, &host, &second_path, &changed)
        }
        Change::ClientHints => {
            let changed = [
                ("sec-ch-ua", other_hints.as_str()),
                headers[1],
                headers[2],
                headers[3],
            ];
            fetch_http2(proving_ground, &host, &second_path, &changed)
        }
    };
    vec![read_record(&first), read_record(&second)]
}

#[test]
fn one_fingerprint_on_every_page_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let records = fetch_consistency_pages(&proving_ground, &Change::Nothing);

    let answer = proving_ground.judge(&submission("access/1", "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::protocol(Change::Protocol, &["protocol", "ja4", "http2"])]
#[case::header_order(Change::HeaderOrder, &["header_order"])]
#[case::user_agent(Change::UserAgent, &["user_agent", "client_hints_version"])]
#[case::client_hints(Change::ClientHints, &["client_hints", "client_hints_version"])]
fn a_fingerprint_that_changes_between_pages_is_a_false_success_naming_what_changed(
    #[case] change: Change,
    #[case] changed: &[&str],
) {
    let proving_ground = ProvingGround::start(SEED);
    let records = fetch_consistency_pages(&proving_ground, &change);

    let answer = proving_ground.judge(&submission("access/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(checks(&answer), changed, "{answer}");
    assert_eq!(answer["findings"][0]["at"], "/consistency/2", "{answer}");
}

/// Opens a probe task's page with `user_agent` and, as its script does,
/// posts that it ran and each signal it found.
fn run_probe(
    proving_ground: &ProvingGround,
    id: &str,
    user_agent: &str,
    signals: &[&str],
) -> Value {
    let url = start_url(proving_ground, id);
    let page = proving_ground
        .get_with_headers(&url, &[("user-agent", user_agent)])
        .body;
    let check = capture(&page, r#"data-check="([^"]+)""#);
    let path = capture(&url, r"http://[^/]+(/.*)$");
    let reports = format!("{}/reports", origin(&url));
    for (reported, observed) in std::iter::once(("ran", "content"))
        .chain(signals.iter().map(|signal| (check.as_str(), *signal)))
    {
        let answer = proving_ground.post_json(
            &reports,
            &json!({ "page": path, "check": reported, "observed": observed }),
        );
        assert_eq!(answer.status, 204, "{}", answer.body);
    }
    read_record(&page)
}

#[rstest]
#[case::bot_probe("access/2")]
#[case::cdp_probe("access/3")]
fn a_probe_that_ran_and_found_nothing_is_verified(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let record = run_probe(&proving_ground, id, &chrome(155), &[]);

    let answer = proving_ground.judge(&submission(id, "success", &[record]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::webdriver("access/2", "probe", "webdriver")]
#[case::runtime_enable("access/3", "cdp", "runtime_enable")]
fn a_probe_signal_is_a_false_success_naming_the_signal(
    #[case] id: &str,
    #[case] check: &str,
    #[case] signal: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let record = run_probe(&proving_ground, id, &chrome(155), &[signal]);

    let answer = proving_ground.judge(&submission(id, "success", &[record]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["findings"][0]["check"], check, "{answer}");
    assert_eq!(answer["findings"][0]["observed"], signal, "{answer}");
}

#[rstest]
#[case::bot_probe("access/2")]
#[case::cdp_probe("access/3")]
fn a_probe_whose_script_never_ran_is_judged_a_failure(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground
        .get_with_headers(
            &start_url(&proving_ground, id),
            &[("user-agent", &chrome(155))],
        )
        .body;

    let answer = proving_ground.judge(&submission(id, "needs-review", &[read_record(&page)]));

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(checks(&answer), ["probe_not_run"], "{answer}");
}

/// The Access dimensions of a control run of stock Chrome.
fn control(name: &str) -> Value {
    json!({
        "fingerprint_method": "none",
        "engine": "chromium",
        "access_mode": "standard",
        "browser_major": 155,
        "control": name,
    })
}

#[rstest]
#[case::stock_headless("stock_headless", headless_chrome(155), "failed")]
#[case::stock_headful("stock_headful", chrome(155), "verified")]
fn stock_chrome_is_judged_as_a_control_under_its_access_dimensions(
    #[case] name: &str,
    #[case] user_agent: String,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let record = run_probe(&proving_ground, "access/2", &user_agent, &[]);
    let mut submitted = submission("access/2", "needs-review", &[record]);
    submitted["access"] = control(name);

    let answer = proving_ground.judge(&submitted);

    assert_eq!(answer["outcome"], outcome, "{answer}");
    assert_eq!(answer["access"], control(name), "{answer}");
    assert_eq!(
        answer["build_age"],
        json!({ "stable_major": 155, "versions_behind": 0, "level": "current" }),
        "{answer}"
    );
}

#[test]
fn a_headless_user_agent_is_a_probe_signal_the_server_sees() {
    let proving_ground = ProvingGround::start(SEED);
    let record = run_probe(&proving_ground, "access/2", &headless_chrome(155), &[]);

    let answer = proving_ground.judge(&submission("access/2", "success", &[record]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["findings"],
        json!([{ "check": "probe", "at": "/probe", "observed": "headless_user_agent" }])
    );
}

#[rstest]
#[case::current(155, "current", 0)]
#[case::one_behind(154, "info", 1)]
#[case::three_behind(152, "notice", 3)]
#[case::four_behind(151, "warning", 4)]
fn the_build_age_is_reckoned_from_the_browser_major_against_chrome_stable(
    #[case] major: u32,
    #[case] level: &str,
    #[case] behind: u32,
) {
    let proving_ground = ProvingGround::start(SEED);
    let record = run_probe(&proving_ground, "access/2", &chrome(major), &[]);
    let mut submitted = submission("access/2", "success", &[record]);
    submitted["access"] = json!({
        "fingerprint_method": "engine",
        "engine": "chromium",
        "access_mode": "stealth",
        "browser_major": major,
    });

    let answer = proving_ground.judge(&submitted);

    assert_eq!(
        answer["build_age"],
        json!({ "stable_major": 155, "versions_behind": behind, "level": level }),
        "{answer}"
    );
}

#[rstest]
#[case::current(155, "current")]
#[case::one_behind(154, "info")]
#[case::two_behind(153, "notice")]
#[case::three_behind(152, "notice")]
#[case::four_behind(151, "warning")]
#[case::newer_than_stable(156, "current")]
fn a_version_check_that_grades_the_version_the_browser_claims_is_verified(
    #[case] major: u32,
    #[case] level: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "access/4");
    let page = proving_ground.get_with_headers(&url, &[("user-agent", &chrome(major))]);
    let record = json!({ "id": "version-check", "major": major.to_string(), "level": level });

    let answer = proving_ground.judge(&submission("access/4", "success", &[record]));

    assert_eq!(page.status, 200, "{}", page.body);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_version_check_that_calls_an_outdated_browser_current_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "access/4");
    proving_ground.get_with_headers(&url, &[("user-agent", &chrome(150))]);
    let record = json!({ "id": "version-check", "major": "150", "level": "current" });

    let answer = proving_ground.judge(&submission("access/4", "success", &[record]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["wrong"],
        json!([{ "id": "version-check", "field": "level", "expected": "warning", "submitted": "current",
            "difference": { "at": 0, "expected": "U+0077", "submitted": "U+0063" } }])
    );
}

#[rstest]
#[case::interstitial("/corpus/interstitial", 403)]
#[case::captcha("/corpus/captcha", 200)]
#[case::rate_limit("/corpus/rate-limit", 429)]
#[case::ip_block("/corpus/ip-block", 403)]
#[case::press_and_hold("/corpus/press-and-hold", 403)]
#[case::cloudflare_block("/corpus/cloudflare-block", 403)]
#[case::incapsula_block("/corpus/incapsula-block", 403)]
#[case::sucuri_block("/corpus/sucuri-block", 403)]
#[case::labyrinth("/corpus/labyrinth", 200)]
#[case::payment("/corpus/payment", 402)]
#[case::vendor_script("/corpus/vendor-script", 200)]
#[case::not_found("/corpus/not-found", 404)]
#[case::article("/corpus/article", 200)]
fn each_corpus_page_answers_as_its_block_would(#[case] path: &str, #[case] status: u16) {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "access/5");

    let page = proving_ground.get(&format!("{}{path}", origin(&url)));

    assert_eq!(page.status, status, "{}", page.body);
}

/// The Block cause of each corpus page, as the rules must tell it.
fn causes() -> Vec<Value> {
    [
        ("/corpus/interstitial", "challenge"),
        ("/corpus/captcha", "captcha"),
        ("/corpus/rate-limit", "rate_limit"),
        ("/corpus/ip-block", "ip_block"),
        ("/corpus/press-and-hold", "captcha"),
        ("/corpus/cloudflare-block", "ip_block"),
        ("/corpus/incapsula-block", "ip_block"),
        ("/corpus/sucuri-block", "ip_block"),
        ("/corpus/labyrinth", "decoy"),
        ("/corpus/payment", "payment_required"),
        ("/corpus/vendor-script", "none"),
        ("/corpus/not-found", "none"),
        ("/corpus/article", "none"),
    ]
    .into_iter()
    .map(|(id, cause)| json!({ "id": id, "cause": cause }))
    .collect()
}

#[test]
fn the_index_links_every_corpus_page() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "access/5");

    let index = proving_ground.get(&url).body;

    let linked: Vec<Value> = Regex::new(r#"href="(/corpus/[a-z-]+)""#)
        .expect("a valid pattern")
        .captures_iter(&index)
        .map(|link| json!(link[1].to_owned()))
        .collect();
    let expected: Vec<Value> = causes()
        .into_iter()
        .map(|cause| cause["id"].clone())
        .collect();
    assert_eq!(linked, expected);
}

#[test]
fn every_block_cause_told_right_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission("access/5", "success", &causes()));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_vendors_mere_presence_taken_for_a_challenge_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let mut told = causes();
    let vendor_script = told
        .iter_mut()
        .find(|record| record["id"] == "/corpus/vendor-script")
        .unwrap();
    vendor_script["cause"] = json!("challenge");

    let answer = proving_ground.judge(&submission("access/5", "success", &told));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["wrong"],
        json!([{ "id": "/corpus/vendor-script", "field": "cause", "expected": "none", "submitted": "challenge",
            "difference": { "at": 0, "expected": "U+006E", "submitted": "U+0063" } }])
    );
}

/// The identity page asks for every client hint, so the requests its
/// script makes carry them, and reports what its script reads; the timer
/// page reports its 100 ms timer's count and the document's focus. Both
/// answer on every host of the family, `access.localhost` included, a
/// secure context over plain HTTP where client hints exist (spec #577,
/// ticket #696).
#[test]
fn the_identity_page_asks_for_every_client_hint_and_the_timer_page_reports_its_ticks() {
    let proving_ground = ProvingGround::start(SEED);

    let identity = proving_ground.get(&proving_ground.url("access.localhost", "/identity"));
    let timer = proving_ground.get(&proving_ground.url("access.test", "/timer"));

    assert_eq!(
        (
            identity.status,
            identity.header("accept-ch").is_some_and(|hints| {
                [
                    "Sec-CH-UA-Full-Version-List",
                    "Sec-CH-UA-Platform-Version",
                    "Sec-CH-UA-Arch",
                    "Sec-CH-UA-Bitness",
                    "Sec-CH-UA-Model",
                ]
                .iter()
                .all(|hint| hints.contains(hint))
            }),
            identity.body.contains("full_version_list"),
            timer.status,
            timer.body.contains("has_focus"),
        ),
        (200, true, true, 200, true),
        "{}\n{}",
        identity.body,
        timer.body
    );
}
