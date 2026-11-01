use rstest::rstest;

use super::NetError;

#[rstest]
#[case::aborted("net::ERR_ABORTED", NetError::Aborted)]
#[case::too_many_redirects("net::ERR_TOO_MANY_REDIRECTS", NetError::TooManyRedirects)]
#[case::decoding_failed("net::ERR_CONTENT_DECODING_FAILED", NetError::ContentDecodingFailed)]
#[case::zstd_window_too_big("net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG", NetError::ZstdWindowSizeTooBig)]
#[case::response_code_failure(
    "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
    NetError::HttpResponseCodeFailure
)]
#[case::blocked_by_client("net::ERR_BLOCKED_BY_CLIENT", NetError::BlockedByClient)]
#[case::blocked_by_the_debugging_client(
    "net::ERR_BLOCKED_BY_CLIENT.Inspector",
    NetError::BlockedByClient
)]
#[case::aborted_by_the_debugging_client("net::ERR_ABORTED.Inspector", NetError::Aborted)]
fn each_network_error_the_engine_tells_apart_becomes_its_variant(
    #[case] text: &str,
    #[case] expected: NetError,
) {
    assert_eq!(NetError::of(text), expected);
}

#[rstest]
#[case::name_not_resolved("net::ERR_NAME_NOT_RESOLVED")]
#[case::connection_refused("net::ERR_CONNECTION_REFUSED")]
#[case::tunnel_refused("net::ERR_TUNNEL_CONNECTION_FAILED")]
#[case::certificate("net::ERR_CERT_AUTHORITY_INVALID")]
#[case::http2_stream("net::ERR_HTTP2_PROTOCOL_ERROR")]
fn any_other_named_network_error_keeps_its_name(#[case] text: &str) {
    let error = NetError::of(text);

    assert_eq!(
        (matches!(error, NetError::Other(_)), error.name()),
        (true, text)
    );
}

#[rstest]
#[case::a_variants_name(NetError::Aborted, "net::ERR_ABORTED")]
#[case::without_the_debugging_clients_mark(
    NetError::of("net::ERR_CONNECTION_RESET.Inspector"),
    "net::ERR_CONNECTION_RESET"
)]
fn a_network_error_is_named_as_the_browser_names_it(#[case] error: NetError, #[case] name: &str) {
    assert_eq!(error.name(), name);
}

#[rstest]
#[case::prose("the request failed for some reason")]
#[case::empty("")]
#[case::lower_case("net::err_aborted")]
#[case::no_name_after_the_prefix("net::ERR_")]
fn a_text_that_names_no_network_error_is_unrecognized_and_named_failed(#[case] text: &str) {
    let error = NetError::of(text);

    assert_eq!(
        (matches!(error, NetError::Unrecognized(_)), error.name()),
        (true, "net::ERR_FAILED")
    );
}

/// A content coding that did not decode is a failure of its own, never a
/// cut body (ADR 0118's #699 note): a zstd window past RFC 9659's 8 MB is
/// one (finding V152-a read it as a cut body), and keeps its own name.
#[rstest]
#[case::decoding_failed("net::ERR_CONTENT_DECODING_FAILED", true)]
#[case::zstd_window_too_big("net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG", true)]
#[case::length_mismatch("net::ERR_CONTENT_LENGTH_MISMATCH", false)]
#[case::incomplete_chunked("net::ERR_INCOMPLETE_CHUNKED_ENCODING", false)]
#[case::aborted("net::ERR_ABORTED", false)]
fn a_content_coding_that_did_not_decode_is_told_from_a_cut_body(
    #[case] text: &str,
    #[case] fails_to_decode: bool,
) {
    let error = NetError::of(text);

    assert_eq!(
        (error.fails_to_decode(), error.name()),
        (fails_to_decode, text)
    );
}
