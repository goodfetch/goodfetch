//! The fetch edge cases of the `http` tier (ADR 0118; spec #579): reqwest
//! under the fetch driver, against the Proving ground's `fetch-edge.test`
//! pages. A cut body fails with `response_truncated` and a body ended by
//! its connection's close is whole (pitfall H15); a request the site never
//! processed is sent once more on a new connection, never a second time,
//! and any other transport failure is `navigation_failed` (pitfall H05).

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_fetch_tier::{Deadlines, Fetched, Hop, Reqwest, fetch};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{reqwest, running_clock_runtime};

/// Any seed: the fetch edge pages are served on every one.
const SEED: u64 = 763;

/// The fetch edge pages' host.
const HOST: &str = "fetch-edge.test";

/// `timeout.first_byte` and `timeout.navigation` at 30 s each.
fn deadlines() -> Deadlines {
    Deadlines {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
    }
}

/// Fetches each of `hops` in turn through `transport`, on one runtime, so
/// a later hop may reuse an earlier one's connection.
fn fetched_in_turn(transport: &Reqwest, hops: &[Hop]) -> Vec<Result<Fetched, CallError>> {
    running_clock_runtime().block_on(async {
        let mut answers = Vec::new();
        for hop in hops {
            answers.push(fetch(transport, hop, &deadlines(), &Clock::system()).await);
        }
        answers
    })
}

/// A `GET` of `path` on `host` over plain HTTP.
fn plain(proving_ground: &ProvingGround, host: &str, path: &str) -> Hop {
    Hop::get(Url::parse(&proving_ground.url(host, path)).unwrap())
}

/// The requests the Proving ground received for `path` on `host`.
fn requests_for(proving_ground: &ProvingGround, host: &str, path: &str) -> Vec<Value> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|seen| seen.split(':').next() == Some(host))
                && request["target"] == path
        })
        .cloned()
        .collect()
}

/// Covers: story:5-126
#[rstest]
#[case::a_body_shorter_than_its_content_length("/cut/content-length", true)]
#[case::a_chunked_body_missing_its_last_chunk("/cut/chunked", true)]
#[case::a_body_its_connection_cut("/cut/connection", true)]
#[case::a_compressed_body_whose_transfer_is_cut("/cut/gzip-content-length", true)]
#[case::a_compressed_stream_ending_early_in_a_whole_transfer("/cut/gzip", false)]
fn a_cut_body_fails_with_response_truncated(#[case] path: &str, #[case] transfer_cut: bool) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let hop = plain(&proving_ground, HOST, path);

    let error = fetched_in_turn(&transport, &[hop.clone()])
        .remove(0)
        .unwrap_err();

    assert_eq!(error.code(), "response_truncated", "{error:?}");
    assert!(error.retryable());
    let details = error.details().unwrap();
    assert_eq!(details["url"], hop.url.as_str());
    let received = details["received"].as_u64().unwrap();
    assert!(received > 0, "{details:?}");
    if let Some(expected) = details["expected"].as_u64() {
        assert_eq!(received < expected, transfer_cut, "{details:?}");
    }
}

/// Covers: story:5-126
#[test]
fn a_body_its_connection_s_close_delimits_is_whole_at_the_close() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let hop = plain(&proving_ground, HOST, "/close-delimited");

    let page = fetched_in_turn(&transport, &[hop]).remove(0).unwrap();

    assert_eq!(page.status, 200);
    let text = String::from_utf8(page.body).unwrap();
    assert!(
        text.contains("Line 40 of the fetch edge page at /close-delimited.")
            && text.ends_with("</html>"),
        "{text}"
    );
}
