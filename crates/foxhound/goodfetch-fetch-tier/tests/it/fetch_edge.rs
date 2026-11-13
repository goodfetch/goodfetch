//! The fetch edge cases of the `http` tier (ADR 0118; spec #579): reqwest
//! and `wreq`, each under the fetch driver, against the Proving ground's `fetch-edge.test`
//! pages. A cut body fails with `response_truncated` and a body ended by
//! its connection's close is whole (pitfall H15); a request the site never
//! processed is sent once more on a new connection, never a second time,
//! and any other transport failure is `navigation_failed` (pitfall H05).

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

use crate::{Stack, requests_for, running_clock_runtime, transport};

/// Any seed: the fetch edge pages are served on every one.
const SEED: u64 = 763;

/// The fetch edge pages' host.
const HOST: &str = "fetch-edge.test";

/// `timeout.first_byte` and `timeout.navigation` at 30 s each.
fn deadlines() -> FetchBounds {
    FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    }
}

/// Fetches each of `hops` in turn through `transport`, on one runtime, so
/// a later hop may reuse an earlier one's connection.
fn fetched_in_turn(transport: &dyn Transport, hops: &[Hop]) -> Vec<Result<Fetched, CallError>> {
    running_clock_runtime().block_on(async {
        let mut answers = Vec::new();
        for hop in hops {
            answers.push(
                fetch(transport, hop, &deadlines(), &Clock::system())
                    .await
                    .map_err(FetchFailure::into_error),
            );
        }
        answers
    })
}

/// A `GET` of `path` on `host` over plain HTTP.
fn plain(proving_ground: &ProvingGround, host: &str, path: &str) -> Hop {
    Hop::get(Url::parse(&proving_ground.url(host, path)).unwrap())
}

/// How a hop reaches the Proving ground.
#[derive(Clone, Copy, Debug)]
enum Over {
    /// Plain HTTP/1.1.
    Plain,
    /// TLS, where ALPN offers HTTP/2 first.
    Tls,
}

/// A `GET` of `path` on `host` over `over`.
fn get(proving_ground: &ProvingGround, over: Over, host: &str, path: &str) -> Hop {
    match over {
        Over::Plain => plain(proving_ground, host, path),
        Over::Tls => Hop::get(
            Url::parse(&format!(
                "https://{host}:{}{path}",
                proving_ground.address().port()
            ))
            .unwrap(),
        ),
    }
}

/// Covers: story:5-126, story:5-122
#[rstest]
#[case::a_body_shorter_than_its_content_length("/cut/content-length", true)]
#[case::a_chunked_body_missing_its_last_chunk("/cut/chunked", true)]
#[case::a_body_its_connection_cut("/cut/connection", true)]
#[case::a_compressed_body_whose_transfer_is_cut("/cut/gzip-content-length", true)]
#[case::a_compressed_stream_ending_early_in_a_whole_transfer("/cut/gzip", false)]
fn a_cut_body_fails_with_response_truncated(
    #[case] path: &str,
    #[case] transfer_cut: bool,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let hop = plain(&proving_ground, HOST, path);

    let error = fetched_in_turn(transport.as_ref(), std::slice::from_ref(&hop))
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

/// Covers: story:5-126, story:5-122
#[rstest]
fn a_body_its_connection_s_close_delimits_is_whole_at_the_close(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let hop = plain(&proving_ground, HOST, "/close-delimited");

    let page = fetched_in_turn(transport.as_ref(), &[hop])
        .remove(0)
        .unwrap();

    assert_eq!(page.status, 200);
    let text = String::from_utf8(page.body.to_vec()).unwrap();
    assert!(
        text.contains("Line 40 of the fetch edge page at /close-delimited.")
            && text.ends_with("</html>"),
        "{text}"
    );
}

/// Covers: story:5-126, story:5-122
#[rstest]
fn an_http2_stream_reset_after_its_headers_fails_with_response_truncated(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let hop = get(&proving_ground, Over::Tls, HOST, "/cut/reset");

    let error = fetched_in_turn(transport.as_ref(), &[hop])
        .remove(0)
        .unwrap_err();

    assert_eq!(error.code(), "response_truncated", "{error:?}");
    let requests = requests_for(&proving_ground, HOST, "/cut/reset");
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["protocol"], "h2");
    assert_eq!(requests[0]["delivery"]["end"]["kind"], "stream_reset");
}

/// Covers: story:5-127, story:5-122
#[rstest]
#[case::refused_stream(Over::Tls, HOST, &[], "/refused")]
#[case::http_1_1_required(Over::Tls, HOST, &[], "/http1-required")]
#[case::a_goaway_below_its_stream(Over::Tls, "goaway.fetch-edge.test", &["/target"], "/target")]
#[case::misdirected(Over::Plain, HOST, &[], "/misdirected")]
#[case::a_reused_connection_closing(Over::Plain, HOST, &["/target"], "/reuse/close")]
#[case::a_reused_connection_answering_408(Over::Plain, HOST, &["/target"], "/reuse/408")]
fn a_request_the_site_never_processed_is_sent_once_more_on_a_new_connection(
    #[case] over: Over,
    #[case] host: &str,
    #[case] earlier: &[&str],
    #[case] path: &str,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let mut hops: Vec<Hop> = earlier
        .iter()
        .map(|earlier| get(&proving_ground, over, host, earlier))
        .collect();
    hops.push(get(&proving_ground, over, host, path));

    let answer = fetched_in_turn(transport.as_ref(), &hops)
        .pop()
        .unwrap()
        .unwrap();

    assert_eq!(answer.status, 200);
    let requests = requests_for(&proving_ground, host, path);
    let last_two = &requests[requests.len().saturating_sub(2)..];
    assert_eq!(
        last_two.len(),
        2,
        "the request is sent once more: {requests:?}"
    );
    assert_ne!(
        last_two[0]["connection"], last_two[1]["connection"],
        "on a new connection: {requests:?}"
    );
}

/// Covers: story:5-127, story:5-122
#[rstest]
fn http_1_1_required_keeps_its_origin_on_http_1_1(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let hops = [
        get(&proving_ground, Over::Tls, HOST, "/http1-required"),
        get(&proving_ground, Over::Tls, HOST, "/target"),
    ];

    let answers = fetched_in_turn(transport.as_ref(), &hops);

    assert!(answers.iter().all(Result::is_ok), "{answers:?}");
    let protocols: Vec<Value> = requests_for(&proving_ground, HOST, "/http1-required")
        .into_iter()
        .chain(requests_for(&proving_ground, HOST, "/target"))
        .map(|request| request["protocol"].clone())
        .collect();
    assert_eq!(protocols, ["h2", "http/1.1", "http/1.1"]);
}

/// Covers: story:5-127, story:5-122
#[rstest]
fn a_request_refused_again_is_never_sent_a_third_time(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let hop = get(&proving_ground, Over::Tls, HOST, "/refused/always");

    let error = fetched_in_turn(transport.as_ref(), &[hop])
        .remove(0)
        .unwrap_err();

    assert_eq!(error.code(), "navigation_failed");
    assert_eq!(
        error.details().unwrap()["error"],
        "net::ERR_HTTP2_SERVER_REFUSED_STREAM"
    );
    assert_eq!(
        requests_for(&proving_ground, HOST, "/refused/always").len(),
        2
    );
}

/// A POST of `path` on the fetch edge host over plain HTTP, a read or not.
fn post(proving_ground: &ProvingGround, path: &str, read: bool) -> Hop {
    let mut post = plain(proving_ground, HOST, path);
    post.method = http::Method::POST;
    post.body = Some(bytes::Bytes::from_static(b"q=1"));
    post.read = read;
    post
}

/// Covers: story:5-127, story:5-122
#[rstest]
fn a_post_that_is_no_read_is_not_sent_again_when_its_reused_connection_closes(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    // A handler's `page.request` POST on the `http` tier.
    let hops = [
        plain(&proving_ground, HOST, "/target"),
        post(&proving_ground, "/reuse/close", false),
    ];

    let error = fetched_in_turn(transport.as_ref(), &hops)
        .pop()
        .unwrap()
        .unwrap_err();

    assert_eq!(error.code(), "navigation_failed", "{error:?}");
    assert_eq!(error.details().unwrap()["error"], "net::ERR_EMPTY_RESPONSE");
    assert_eq!(requests_for(&proving_ground, HOST, "/reuse/close").len(), 1);
}

/// Covers: story:5-127, story:5-122
#[rstest]
fn a_crawl_request_s_post_is_a_read_sent_again_when_its_reused_connection_closes(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    // A crawl request's own POST, a read whatever its method (ADR 0106).
    let hops = [
        plain(&proving_ground, HOST, "/target"),
        post(&proving_ground, "/reuse/close", true),
    ];

    let answer = fetched_in_turn(transport.as_ref(), &hops)
        .pop()
        .unwrap()
        .unwrap();

    assert_eq!(answer.status, 200);
    assert_eq!(requests_for(&proving_ground, HOST, "/reuse/close").len(), 2);
}

/// Covers: story:5-127
#[test]
fn a_hop_is_a_read_for_an_idempotent_method_alone() {
    let reads: Vec<bool> = [
        "GET", "HEAD", "OPTIONS", "TRACE", "PUT", "DELETE", "POST", "PATCH",
    ]
    .into_iter()
    .map(|method| Hop::idempotent(&http::Method::from_bytes(method.as_bytes()).unwrap()))
    .collect();

    assert_eq!(reads, [true, true, true, true, true, true, false, false]);
}
