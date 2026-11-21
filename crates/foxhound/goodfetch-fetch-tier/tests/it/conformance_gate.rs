//! The transport conformance gate (pitfall H13's acceptance test; spec
//! #579): reqwest and `wreq`, each under the fetch driver, against the
//! Proving ground's
//! content-coding pages decodes every coding as Chrome does and fails
//! where Chrome fails, follows no redirect itself and stops at the
//! engine's timeouts. Malformed heads are refused and a folded one kept,
//! as the `headers` family asks. Browser-fetch, which sends from a
//! Session's browser context, runs the gate on Foxhound's executable
//! (`goodfetch-foxhound`'s `tests/it/browser_fetch.rs`).

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_limits::Limits;
use goodfetch_fetch_tier::{FetchBounds, FetchFailure, Fetched, Hop, Transport, fetch};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{Stack, running_clock_runtime, transport};

/// Any seed: the families below are served on every one.
const SEED: u64 = 759;

/// `timeout.first_byte` and `timeout.navigation` at `first_byte` and 30 s.
fn deadlines(first_byte: u64) -> FetchBounds {
    FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, first_byte, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    }
}

/// Fetches `path` on `host` over plain HTTP at the Proving ground, through
/// `transport`, under `deadlines`.
fn fetched(
    proving_ground: &ProvingGround,
    transport: &dyn Transport,
    host: &str,
    path: &str,
    deadlines: &FetchBounds,
) -> Result<Fetched, CallError> {
    let hop = Hop::get(Url::parse(&proving_ground.url(host, path)).unwrap());
    running_clock_runtime()
        .block_on(fetch(transport, &hop, deadlines, &Clock::system()))
        .map_err(FetchFailure::into_error)
}

/// The error's code and its details' network error.
fn code_and_network_error(error: &CallError) -> (String, Option<String>) {
    (
        error.code().to_owned(),
        error
            .details()
            .and_then(|details| details.get("error"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    )
}

/// The page requests the Proving ground received for `host`.
fn requests_to(proving_ground: &ProvingGround, host: &str) -> Vec<Value> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.starts_with(host))
        .map(|request| serde_json::to_value(request).unwrap())
        .collect()
}

/// The page's text when the fetch decoded it.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the case table reads `decoded` beside `refused`, both an `Answer`"
)]
fn decoded(text: &str) -> Answer<String> {
    Ok(text.to_owned())
}

/// `navigation_failed` with `network_error`, as Chrome fails.
fn refused<T>(network_error: &str) -> Answer<T> {
    Err((
        "navigation_failed".to_owned(),
        Some(network_error.to_owned()),
    ))
}

/// What a fetch answered: its page's part under test, or its Error code and
/// network error.
type Answer<T> = Result<T, (String, Option<String>)>;

/// The text of a `content-coding` page's `<p class="text">`.
fn page_text(page: &Fetched) -> String {
    let body = String::from_utf8(page.body.to_vec()).unwrap();
    body.split("<p class=\"text\">")
        .nth(1)
        .and_then(|rest| rest.split("</p>").next())
        .unwrap_or(&body)
        .to_owned()
}

/// Each `content-coding` page by its ordinal, with what Chrome makes of it.
///
/// Covers: story:5-128, story:5-122
#[rstest]
#[case::plain_labelled_gzip(1, refused("net::ERR_CONTENT_DECODING_FAILED"))]
#[case::invalid_gzip_header(2, refused("net::ERR_CONTENT_DECODING_FAILED"))]
#[case::gzip_then_brotli(3, decoded("Content coding page 3, decoded once and as labelled."))]
#[case::raw_deflate(4, decoded("Content coding page 4, decoded once and as labelled."))]
#[case::unknown(5, decoded("Content coding page 5, decoded once and as labelled."))]
#[case::gzip_twice(6, refused("net::ERR_CONTENT_DECODING_FAILED"))]
#[case::zstd_twice(7, refused("net::ERR_CONTENT_DECODING_FAILED"))]
#[case::gzip(8, decoded("Content coding page 8, decoded once and as labelled."))]
#[case::zlib_deflate(9, decoded("Content coding page 9, decoded once and as labelled."))]
#[case::brotli(10, decoded("Content coding page 10, decoded once and as labelled."))]
#[case::zstd(11, decoded("Content coding page 11, decoded once and as labelled."))]
fn decodes_every_coding_as_chrome_decodes_it_and_fails_where_chrome_fails(
    #[case] ordinal: u64,
    #[case] expected: Answer<String>,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    let answer = fetched(
        &proving_ground,
        transport.as_ref(),
        &format!("{ordinal}.content-coding.test"),
        "/page",
        &deadlines(30_000),
    );

    assert_eq!(
        answer
            .as_ref()
            .map(page_text)
            .map_err(code_and_network_error),
        expected
    );
}

/// Covers: story:5-128, story:5-122
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn asks_for_every_coding_chrome_asks_for(#[case] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    fetched(
        &proving_ground,
        transport.as_ref(),
        "8.content-coding.test",
        "/page",
        &deadlines(30_000),
    )
    .unwrap();

    let requests = requests_to(&proving_ground, "8.content-coding.test");
    let accept_encoding = requests[0]["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|header| {
            header[0]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("accept-encoding"))
        })
        .map(|header| header[1].clone());
    assert_eq!(
        accept_encoding,
        Some(Value::from("gzip, deflate, br, zstd")),
        "{requests:?}"
    );
}

/// Covers: story:5-128, story:5-122
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn follows_no_redirect_itself(#[case] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    let page = fetched(
        &proving_ground,
        transport.as_ref(),
        "fetch-edge.test",
        "/redirect/301",
        &deadlines(30_000),
    )
    .unwrap();

    assert_eq!(page.status, 301);
    assert!(
        page.headers.iter().any(|(name, _)| name == "location"),
        "{:?}",
        page.headers
    );
    assert_eq!(requests_to(&proving_ground, "fetch-edge.test").len(), 1);
}

/// Covers: story:5-128, story:5-122
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn stops_at_the_engine_s_first_byte_timeout(#[case] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    // The sixth incident's second page answers after 8 s.
    let error = fetched(
        &proving_ground,
        transport.as_ref(),
        "6.incidents.test",
        "/catalog/2",
        &deadlines(1_000),
    )
    .unwrap_err();

    assert_eq!(error.code(), "deadline_exceeded");
    assert_eq!(
        error.details().unwrap()["limit"]["key"],
        "timeout.first_byte"
    );
}

/// Covers: story:5-128, story:5-122
#[rstest]
#[case::a_nul("/nul", refused("net::ERR_INVALID_HTTP_RESPONSE"))]
#[case::a_bare_cr("/bare-cr", refused("net::ERR_INVALID_HTTP_RESPONSE"))]
#[case::no_colon("/no-colon", refused("net::ERR_INVALID_HTTP_RESPONSE"))]
#[case::a_folded_header("/folded", Ok(200))]
fn refuses_a_malformed_head_and_keeps_a_folded_one(
    #[case] path: &str,
    #[case] expected: Answer<u16>,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    let answer = fetched(
        &proving_ground,
        transport.as_ref(),
        "headers.test",
        path,
        &deadlines(30_000),
    );

    assert_eq!(
        answer
            .as_ref()
            .map(|page| page.status)
            .map_err(code_and_network_error),
        expected
    );
}
