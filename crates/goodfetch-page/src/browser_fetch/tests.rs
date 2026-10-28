//! How a load the browser reported finished ends (NEW-2): the pinned
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

#[test]
fn a_load_finished_with_no_bytes_counted_and_fewer_bytes_than_its_content_length_is_cut() {
    assert!(cut_though_finished(None, &response(410, None), 205, 0.0));
}

#[rstest]
#[case::counted_by_the_network(None, response(410, None), 410, 475.0)]
#[case::from_the_cache_whole(None, response(410, None), 410, 0.0)]
#[case::a_head_request(Some("HEAD"), response(410, None), 0, 0.0)]
#[case::decoded_from_a_content_coding(None, response(165, Some("gzip")), 120, 0.0)]
fn a_load_finished_whole_is_never_cut(
    #[case] method: Option<&str>,
    #[case] response: PageResponse,
    #[case] bytes: usize,
    #[case] encoded: f64,
) {
    assert!(!cut_though_finished(method, &response, bytes, encoded));
}
