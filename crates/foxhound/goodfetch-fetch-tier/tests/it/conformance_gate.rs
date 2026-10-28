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
use goodfetch_fetch_tier::{Deadlines, Fetched, Hop, Transport, fetch};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{Stack, running_clock_runtime, transport};

/// Any seed: the families below are served on every one.
const SEED: u64 = 759;

/// `timeout.first_byte` and `timeout.navigation` at `first_byte` and 30 s.
fn deadlines(first_byte: u64) -> Deadlines {
    Deadlines {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, first_byte, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
    }
}

/// Fetches `path` on `host` over plain HTTP at the Proving ground, through
/// `transport`, under `deadlines`.
fn fetched(
    proving_ground: &ProvingGround,
    transport: &dyn Transport,
    host: &str,
    path: &str,
    deadlines: &Deadlines,
) -> Result<Fetched, CallError> {
    let hop = Hop::get(Url::parse(&proving_ground.url(host, path)).unwrap());
    running_clock_runtime().block_on(fetch(transport, &hop, deadlines, &Clock::system()))
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
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|seen| seen.starts_with(host))
        })
        .cloned()
        .collect()
}

/// Covers: story:5-128, story:5-122
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn decodes_every_coding_as_chrome_decodes_it_and_fails_where_chrome_fails(#[case] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    for ordinal in 1..=11 {
        let host = format!("{ordinal}.content-coding.test");
        let answer = fetched(
            &proving_ground,
            transport.as_ref(),
            &host,
            "/page",
            &deadlines(30_000),
        );

        if [1, 2, 6, 7].contains(&ordinal) {
            let error = answer.expect_err(&host);
            assert_eq!(
                code_and_network_error(&error),
                (
                    "navigation_failed".to_owned(),
                    Some("net::ERR_CONTENT_DECODING_FAILED".to_owned())
                ),
                "{host}"
            );
        } else {
            let page = answer.unwrap_or_else(|error| panic!("{host}: {error:?}"));
            let text = String::from_utf8(page.body.to_vec()).unwrap();
            assert!(
                text.contains(&format!(
                    "Content coding page {ordinal}, decoded once and as labelled."
                )),
                "{host}: {text}"
            );
        }
    }
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
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn refuses_a_malformed_head_and_keeps_a_folded_one(#[case] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    for path in ["/nul", "/bare-cr", "/no-colon"] {
        let error = fetched(
            &proving_ground,
            transport.as_ref(),
            "headers.test",
            path,
            &deadlines(30_000),
        )
        .expect_err(path);
        assert_eq!(
            code_and_network_error(&error),
            (
                "navigation_failed".to_owned(),
                Some("net::ERR_INVALID_HTTP_RESPONSE".to_owned())
            ),
            "{path}"
        );
    }
    let folded = fetched(
        &proving_ground,
        transport.as_ref(),
        "headers.test",
        "/folded",
        &deadlines(30_000),
    )
    .unwrap();
    assert_eq!(folded.status, 200);
}
