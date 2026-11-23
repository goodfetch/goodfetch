//! What an attempt's ending tells its domain's Politeness: a handler's
//! `blocked`, read from the details as the protocol's `BlockedDetails`
//! writes them, so the detail the crawl reads a site's `Retry-After` from
//! cannot drift from the one the engine's `blocked` writes it under again
//! (fix-queue 44); and the backoff classes of what the engine's own fetch
//! of the document met (spec #579), the site's and never the proxy's.
#![expect(
    clippy::disallowed_methods,
    reason = "a pure rule's test takes one monotonic instant to measure the hold from (ADR 0059)"
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::admission::Ended;
use goodfetch_protocol::foxhound::crawl::CrawlRequestHandled;
use goodfetch_protocol::page::{BLOCKED, BlockedDetails, NAVIGATION_FAILED, PageResponse};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::super::attempt::Answer;
use super::super::document_answer;
use super::super::failure::AttemptFailure;
use super::domain_ending;

/// 2026-01-05T09:00:00Z.
fn wall() -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(491_001)
}

/// Covers: story:3-159
#[test]
fn a_blocked_rate_limit_holds_its_domain_until_the_time_its_details_name() {
    // The site's Retry-After a minute after the wall time.
    let details: BlockedDetails = serde_json::from_value(json!({
        "cause": "rate_limit",
        "url": "https://shop.test/1",
        "status": 429,
        "retry_after": "2026-01-05T09:01:00Z",
    }))
    .unwrap();
    let Value::Object(details) = serde_json::to_value(details).unwrap() else {
        panic!("details are a table");
    };
    let now = Instant::now();
    let answer = Answer::failed(AttemptFailure::entered(
        BLOCKED.name.to_owned(),
        true,
        String::new(),
        details,
    ));

    assert_eq!(
        domain_ending(&answer, None, false, now, wall()),
        Ended::TooManyRequests(Some(now + Duration::from_secs(60)))
    );
}

/// The request's own document, answered `status` with `headers`, from
/// `url`.
fn document(url: &str, status: u16, headers: &[(&str, &str)]) -> PageResponse {
    PageResponse {
        url: url.to_owned(),
        status,
        status_text: String::new(),
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        encoding: None,
        request_id: "1".to_owned(),
        from_service_worker: false,
    }
}

/// A handler that handled its page and committed nothing.
fn handled() -> Answer {
    Answer::Handled(CrawlRequestHandled {
        records: Vec::new(),
        skipped: None,
    })
}

/// An attempt whose fetch failed with `error`, Chrome's network error.
fn navigation_failed(error: &str) -> Answer {
    let Value::Object(details) = json!({ "url": "https://shop.test/1", "error": error }) else {
        panic!("details are a table");
    };
    Answer::failed(AttemptFailure::entered(
        NAVIGATION_FAILED.name.to_owned(),
        true,
        String::new(),
        details,
    ))
}

/// The class of answer the site's document gives its domain, whatever the
/// handler made of the page: what the engine's own fetch met, on the
/// `http` tier as on the `browser` tier (ADRs 0086 and 0118). A field
/// naming an error over a tunnel is the site's own intermediary's.
///
/// Covers: story:5-132
#[rstest]
#[case::a_429(document("https://shop.test/1", 429, &[]), Ended::TooManyRequests(None))]
#[case::a_502(document("https://shop.test/1", 502, &[]), Ended::BackOff(None))]
#[case::a_503(document("https://shop.test/1", 503, &[]), Ended::BackOff(None))]
#[case::a_504(document("https://shop.test/1", 504, &[]), Ended::BackOff(None))]
#[case::a_500(document("https://shop.test/1", 500, &[]), Ended::Done)]
#[case::a_200(document("https://shop.test/1", 200, &[("retry-after", "60")]), Ended::Done)]
#[case::a_429_through_a_tunnel_naming_an_error(
    document("https://shop.test/1", 429, &[("proxy-status", "cdn.test; error=http_request_denied")]),
    Ended::TooManyRequests(None)
)]
fn the_sites_answer_to_the_document_backs_its_domain_off(
    #[case] response: PageResponse,
    #[case] expected: Ended,
) {
    assert_eq!(
        domain_ending(&handled(), Some(&response), true, Instant::now(), wall()),
        expected
    );
}

/// A `Retry-After` on the document's answer holds its domain until the
/// time it names: seconds, or a date measured against the answer's own
/// `Date` (ADR 0118).
///
/// Covers: story:5-132
#[rstest]
#[case::in_seconds(429, &[("retry-after", "60")], Ended::TooManyRequests)]
#[case::as_a_date_against_its_date(
    503,
    &[("retry-after", "Mon, 05 Jan 2026 08:01:00 GMT"), ("date", "Mon, 05 Jan 2026 08:00:00 GMT")],
    Ended::BackOff
)]
fn the_sites_retry_after_holds_its_domain_until_the_time_it_names(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] ended: fn(Option<Instant>) -> Ended,
) {
    // The site's clock runs an hour behind the wall time.
    let now = Instant::now();
    let response = document("https://shop.test/1", status, headers);
    let until = Some(now + Duration::from_secs(60));

    assert_eq!(
        domain_ending(&handled(), Some(&response), false, now, wall()),
        ended(until)
    );
}

/// A forward proxy's own 429 or 502, its `Proxy-Status` naming its error,
/// backs no domain off: the proxy's limits are its own (ADR 0053).
///
/// Covers: story:5-133
#[rstest]
#[case::its_own_429(429)]
#[case::its_own_502(502)]
fn a_forward_proxy_s_own_answer_backs_no_domain_off(#[case] status: u16) {
    let response = document(
        "http://shop.test/1",
        status,
        &[("proxy-status", "proving-ground; error=http_request_denied")],
    );

    assert_eq!(
        domain_ending(&handled(), Some(&response), true, Instant::now(), wall()),
        Ended::Done
    );
}

/// The `Retry-After` of a forward proxy's own 503 is the proxy's, not the
/// site's: the engine reads none, so a value that does not parse is never
/// noted against the site (ADRs 0053 and 0118).
///
/// Covers: story:3-159
#[test]
fn a_forward_proxy_s_own_retry_after_is_not_read() {
    let response = document(
        "http://shop.test/1",
        503,
        &[
            ("proxy-status", "proving-ground; error=http_request_denied"),
            ("retry-after", "soon"),
        ],
    );

    let asked = document_answer::asked(&response, true, wall());

    assert_eq!(asked.retry_after, None);
}

/// A fetch that failed backs its domain off when the site refused or
/// timed out the connection or reset its stream with `ENHANCE_YOUR_CALM`,
/// never when the proxy refused the tunnel (ADRs 0053, 0086 and 0118).
///
/// Covers: story:5-132, story:5-133
#[rstest]
#[case::a_refused_connection("net::ERR_CONNECTION_REFUSED", Ended::BackOff(None))]
#[case::a_timed_out_connection("net::ERR_CONNECTION_TIMED_OUT", Ended::BackOff(None))]
#[case::enhance_your_calm("net::ERR_HTTP2_PROTOCOL_ERROR", Ended::BackOff(None))]
#[case::a_reset_connection("net::ERR_CONNECTION_RESET", Ended::Done)]
#[case::a_tunnel_the_proxy_refused("net::ERR_TUNNEL_CONNECTION_FAILED", Ended::Done)]
fn a_failed_fetch_backs_its_domain_off_by_its_network_error(
    #[case] error: &str,
    #[case] expected: Ended,
) {
    assert_eq!(
        domain_ending(
            &navigation_failed(error),
            None,
            true,
            Instant::now(),
            wall()
        ),
        expected
    );
}
