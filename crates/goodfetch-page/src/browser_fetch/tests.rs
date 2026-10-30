//! How a load the browser reported finished ends (NEW-2, #1732): the pinned
//! Chromium can finish a load whose body broke off, with no bytes counted,
//! when the network's failure reached its interception only after the
//! response went on.

use std::collections::BTreeMap;

use goodfetch_protocol::page::PageResponse;
use rstest::rstest;

use super::cut_though_finished;

/// A 200 of `body` bytes declared, under `coding` when one is given.
fn response(declared: usize, coding: Option<&str>) -> PageResponse {
    let mut headers = BTreeMap::from([("content-length".to_owned(), declared.to_string())]);
    if let Some(coding) = coding {
        headers.insert("content-encoding".to_owned(), coding.to_owned());
    }
    PageResponse {
        url: "http://fetch-edge.test/cut/content-length".to_owned(),
        status: 200,
        status_text: "OK".to_owned(),
        headers,
        encoding: None,
        request_id: "1.1".to_owned(),
        from_service_worker: false,
    }
}

/// A 200 framed by the chunked transfer coding: no `Content-Length`.
fn chunked() -> PageResponse {
    let mut chunked = response(0, None);
    chunked.headers = BTreeMap::from([("transfer-encoding".to_owned(), "chunked".to_owned())]);
    chunked
}

/// A 204, which has no body.
fn no_content() -> PageResponse {
    let mut empty = response(0, None);
    empty.status = 204;
    empty
}

/// A load from the network the browser finished with no bytes counted is
/// cut, whatever its body's framing or content coding: the network
/// finishes a load it read whole only once it told the browser so, so the
/// failure of this one was lost on the way (#1732).
#[rstest]
#[case::shorter_than_its_content_length(response(410, None))]
#[case::chunked_without_its_last_chunk(chunked())]
#[case::gzip_cut_before_its_content_length(response(165, Some("gzip")))]
fn a_load_from_the_network_finished_with_no_bytes_counted_is_cut(#[case] response: PageResponse) {
    assert!(cut_though_finished(None, &response, true, 0.0));
}

#[rstest]
#[case::counted_by_the_network(None, response(410, None), true, 475.0)]
#[case::chunked_counted_by_the_network(None, chunked(), true, 494.0)]
#[case::from_the_cache_whole(None, response(410, None), false, 0.0)]
#[case::coded_from_the_cache_whole(None, response(165, Some("gzip")), false, 0.0)]
#[case::a_head_request(Some("HEAD"), response(410, None), true, 0.0)]
#[case::a_response_with_no_body(None, no_content(), true, 0.0)]
fn a_load_finished_whole_is_never_cut(
    #[case] method: Option<&str>,
    #[case] response: PageResponse,
    #[case] from_the_network: bool,
    #[case] encoded: f64,
) {
    assert!(!cut_though_finished(
        method,
        &response,
        from_the_network,
        encoded
    ));
}
