//! How a response the browser reports is read: its headers, its declared
//! length, and whether its body broke off, after a failure or once the
//! browser reported its load finished (NEW-2, #1732: the pinned Chromium
//! can finish a load whose body broke off, with no bytes counted, when the
//! network's failure reached its interception only after the response went
//! on).

use std::collections::BTreeMap;

use goodfetch_cdp::NetError;
use goodfetch_cdp::generated::fetch::HeaderEntry;
use rstest::rstest;

use super::{cut_after_sent_on, cut_by, cut_though_finished, declared_length, paused_headers};

fn entry(name: &str, value: &str) -> HeaderEntry {
    HeaderEntry {
        name: name.to_owned(),
        value: value.to_owned(),
    }
}

/// A paused response's repeated header keeps each of its values, on lines
/// of their own as the Network domain gives them, so a route's `fetch`
/// shows the same headers as the navigation and `page.request` do.
#[test]
fn a_repeated_header_of_a_paused_response_keeps_each_value_on_its_own_line() {
    let entries = [
        entry("Set-Cookie", "a=1"),
        entry("Content-Type", "text/html"),
        entry("set-cookie", "b=2"),
    ];

    assert_eq!(
        paused_headers(&entries),
        BTreeMap::from([
            ("content-type".to_owned(), "text/html".to_owned()),
            ("set-cookie".to_owned(), "a=1\nb=2".to_owned()),
        ])
    );
}

#[rstest]
#[case::declared("410", Some(410))]
#[case::padded(" 410 ", Some(410))]
#[case::not_a_number("ten", None)]
#[case::declared_twice("410\n410", None)]
fn a_content_length_is_read_as_its_number(#[case] value: &str, #[case] expected: Option<u64>) {
    let headers = BTreeMap::from([("content-length".to_owned(), value.to_owned())]);

    assert_eq!(declared_length(&headers), expected);
}

#[test]
fn a_response_with_no_content_length_declares_none() {
    assert_eq!(declared_length(&BTreeMap::new()), None);
}

/// A network error after the response arrived cuts its body, but the
/// browser's own cancel and a content coding that failed to decode (ADR
/// 0118's #699 note).
#[rstest]
#[case::length_mismatch("net::ERR_CONTENT_LENGTH_MISMATCH", true)]
#[case::incomplete_chunked("net::ERR_INCOMPLETE_CHUNKED_ENCODING", true)]
#[case::http2_stream("net::ERR_HTTP2_PROTOCOL_ERROR", true)]
#[case::connection_reset("net::ERR_CONNECTION_RESET", true)]
#[case::the_browsers_cancel("net::ERR_ABORTED", false)]
#[case::decoding_failed("net::ERR_CONTENT_DECODING_FAILED", false)]
#[case::zstd_window_too_big("net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG", false)]
fn a_failure_after_the_response_cuts_its_body_unless_cancelled_or_undecodable(
    #[case] error: &str,
    #[case] cut: bool,
) {
    assert_eq!(cut_by(&NetError::of(error)), cut);
}

/// A network error after the engine read a response and sent it on to its
/// own page cuts the body it read, the browser's cancel included: the
/// browser cancels such a load when it turns the page's copy into a
/// download. Only a content coding that failed to decode does not.
#[rstest]
#[case::length_mismatch("net::ERR_CONTENT_LENGTH_MISMATCH", true)]
#[case::connection_reset("net::ERR_CONNECTION_RESET", true)]
#[case::the_browsers_cancel("net::ERR_ABORTED", true)]
#[case::decoding_failed("net::ERR_CONTENT_DECODING_FAILED", false)]
#[case::zstd_window_too_big("net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG", false)]
fn a_failure_after_a_response_sent_on_cuts_its_body_unless_undecodable(
    #[case] error: &str,
    #[case] cut: bool,
) {
    assert_eq!(cut_after_sent_on(&NetError::of(error)), cut);
}

/// A load from the network the browser finished with no bytes counted is
/// cut, whatever its body's framing or content coding, which the rule
/// never reads: the network finishes a load it read whole only once it
/// told the browser so, so the failure of this one was lost on the way
/// (#1732).
#[test]
fn a_load_from_the_network_finished_with_no_bytes_counted_is_cut() {
    assert!(cut_though_finished(None, 200, true, 0.0));
}

#[rstest]
#[case::counted_by_the_network(None, 200, true, 475.0)]
#[case::from_the_cache_whole(None, 200, false, 0.0)]
#[case::a_head_request(Some("HEAD"), 200, true, 0.0)]
#[case::a_response_with_no_body(None, 204, true, 0.0)]
fn a_load_finished_whole_is_never_cut(
    #[case] method: Option<&str>,
    #[case] status: u16,
    #[case] from_the_network: bool,
    #[case] encoded: f64,
) {
    assert!(!cut_though_finished(
        method,
        status,
        from_the_network,
        encoded
    ));
}
