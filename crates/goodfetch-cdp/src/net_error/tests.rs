use rstest::rstest;

use super::NetError;

#[rstest]
#[case::aborted("net::ERR_ABORTED", NetError::Aborted)]
#[case::too_many_redirects("net::ERR_TOO_MANY_REDIRECTS", NetError::TooManyRedirects)]
#[case::decoding_failed("net::ERR_CONTENT_DECODING_FAILED", NetError::ContentDecodingFailed)]
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
