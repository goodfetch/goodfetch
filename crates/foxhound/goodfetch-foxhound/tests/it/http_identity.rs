//! The identity a `standard` Session's own HTTP transport presents (ADR
//! 0021; issue 2315), on Foxhound's executable against the Proving ground
//! and Chrome for Testing: a crawl request forced onto `http` leaves with
//! the `User-Agent` and the document request headers its Session's browser
//! sends on a navigation, in the browser's order; its redirect hops and
//! the robots.txt read before it carry the same; and a request's own
//! headers stay over the Session's.
//!
//! The first test is also the guard of the headers the engine derives from
//! the pinned build without launching it: a new pin whose browser sends
//! another document request fails it.

use goodfetch_test_support::{ProvingGround, TestEnv, skipped_without_gui_session};
use rstest::rstest;
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::{Inside, crawl_declaring, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 2315;

/// A request's headers as a site received them, their names in lowercase,
/// in order, without `host`, which each client places itself.
type Headers = Vec<(String, String)>;

/// The headers of each request `host` received for `target`, in order.
fn received(proving_ground: &ProvingGround, host: &str, target: &str) -> Vec<Headers> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.split(':').next() == Some(host) && request.target == target)
        .map(|request| {
            request
                .headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .filter(|(name, _)| name != "host")
                .collect()
        })
        .collect()
}

/// The value of `name` among `headers`.
fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

/// A handler that answers no Record.
fn reads_nothing() -> impl FnMut(&mut Inside<'_>, &Value) -> Value {
    |_, _| json!({ "result": { "records": [] } })
}

/// On a plain HTTP site and on one in a secure context, where a browser
/// sends its client hints and fetch metadata: the document a Label forced
/// onto `http` fetches arrives with the headers, values and order of the
/// one a Label forced onto `browser` navigates to, the `User-Agent` among
/// them.
///
/// Covers: story:5-121
#[rstest]
#[case::a_plain_http_site("access.test")]
#[case::a_site_in_a_secure_context("access.localhost")]
fn a_standard_sessions_http_tier_sends_its_browsers_document_request(#[case] host: &str) {
    if skipped_without_gui_session(
        "the `http` tier sends the full build's identity, where the headless shell shows `HeadlessChrome`",
    ) {
        return;
    }
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let finished = crawl_declaring(
        &mut engine,
        "http-identity",
        &[
            json!({ "url": proving_ground.url(host, "/identity?tier=http"), "label": "http" }),
            json!({ "url": proving_ground.url(host, "/identity?tier=browser"), "label": "browser" }),
        ],
        &json!({
            "http": { "settings": { "fetch": { "tier": "http" } } },
            "browser": { "settings": { "fetch": { "tier": "browser" } } },
        }),
        &settings(&proving_ground, &json!({})),
        &mut reads_nothing(),
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["requests"]["succeeded"], 2, "{finished}");

    let over_http = received(&proving_ground, host, "/identity?tier=http");
    let navigated = received(&proving_ground, host, "/identity?tier=browser");
    assert_eq!(navigated.len(), 1, "{navigated:?}");
    assert!(
        header(&navigated[0], "user-agent").is_some_and(|agent| agent.contains(" Chrome/")),
        "{navigated:?}"
    );
    assert_eq!(over_http, navigated);
}

/// A crawl request forced onto `http` whose URL redirects: the robots.txt
/// read before it, its first hop and the hop its redirect leads to each
/// carry the Session's `User-Agent` and one header set.
///
/// Covers: story:5-121, story:5-124
#[test]
fn the_robots_txt_read_and_each_redirect_hop_carry_the_sessions_identity() {
    const HOST: &str = "fetch-edge.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let finished = crawl_with(
        &mut engine,
        "http-identity-hops",
        &[json!({ "url": proving_ground.url(HOST, "/redirect/301"), "label": "page" })],
        &settings(&proving_ground, &json!({ "fetch": { "tier": "http" } })),
        &mut reads_nothing(),
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["requests"]["succeeded"], 1, "{finished}");

    let robots_txt = received(&proving_ground, HOST, "/robots.txt");
    let first = received(&proving_ground, HOST, "/redirect/301");
    let hop = received(&proving_ground, HOST, "/target");
    assert_eq!(
        (robots_txt.len(), first.len(), hop.len()),
        (1, 1, 1),
        "{robots_txt:?} {first:?} {hop:?}"
    );
    assert!(
        header(&first[0], "user-agent").is_some_and(|agent| agent.contains(" Chrome/")),
        "{first:?}"
    );
    assert_eq!((&robots_txt[0], &hop[0]), (&first[0], &first[0]));
}

/// A `page.request` on a page the `http` tier served sets `Accept` and a
/// header of its own: the site receives both as the request set them, and
/// the Session's `User-Agent` beside them.
///
/// Covers: story:5-121
#[test]
fn a_requests_own_headers_stay_over_the_sessions() {
    const HOST: &str = "access.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let asked = proving_ground.url(HOST, "/identity?asked");

    let finished = crawl_with(
        &mut engine,
        "http-identity-own-headers",
        &[json!({ "url": proving_ground.url(HOST, "/identity"), "label": "page" })],
        &settings(&proving_ground, &json!({ "fetch": { "tier": "http" } })),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let answer = inside.call(
                "page.request",
                &json!({
                    "page_id": handle["page_id"],
                    "url": asked,
                    "headers": { "Accept": "application/json", "x-own": "1" },
                }),
            );
            assert_eq!(answer["result"]["status"], 200, "{answer}");
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["requests"]["succeeded"], 1, "{finished}");

    let document = received(&proving_ground, HOST, "/identity");
    let requested = received(&proving_ground, HOST, "/identity?asked");
    assert_eq!((document.len(), requested.len()), (1, 1), "{requested:?}");
    let accepts: Vec<&str> = requested[0]
        .iter()
        .filter(|(name, _)| name == "accept")
        .map(|(_, value)| value.as_str())
        .collect();
    assert_eq!(
        (
            accepts,
            header(&requested[0], "x-own"),
            header(&requested[0], "user-agent"),
        ),
        (
            vec!["application/json"],
            Some("1"),
            header(&document[0], "user-agent"),
        )
    );
    assert!(header(&document[0], "user-agent").is_some());
}
