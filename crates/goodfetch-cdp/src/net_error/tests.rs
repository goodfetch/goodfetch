use rstest::rstest;

use super::{CertificateError, NetError};

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
#[case::failed("net::ERR_FAILED", NetError::Failed)]
#[case::failed_by_the_debugging_client("net::ERR_FAILED.Inspector", NetError::Failed)]
#[case::empty_response("net::ERR_EMPTY_RESPONSE", NetError::EmptyResponse)]
#[case::name_not_resolved("net::ERR_NAME_NOT_RESOLVED", NetError::NameNotResolved)]
#[case::connection_refused("net::ERR_CONNECTION_REFUSED", NetError::ConnectionRefused)]
#[case::connection_timed_out("net::ERR_CONNECTION_TIMED_OUT", NetError::ConnectionTimedOut)]
#[case::connection_failed("net::ERR_CONNECTION_FAILED", NetError::ConnectionFailed)]
#[case::tunnel_refused("net::ERR_TUNNEL_CONNECTION_FAILED", NetError::TunnelRefused)]
#[case::name_mismatch(
    "net::ERR_CERT_COMMON_NAME_INVALID",
    NetError::Certificate(CertificateError::NameMismatch)
)]
#[case::certificate(
    "net::ERR_CERT_AUTHORITY_INVALID",
    NetError::Certificate(CertificateError::UnknownIssuer)
)]
#[case::revoked(
    "net::ERR_CERT_REVOKED",
    NetError::Certificate(CertificateError::Revoked)
)]
fn each_network_error_the_engine_tells_apart_becomes_its_variant(
    #[case] text: &str,
    #[case] expected: NetError,
) {
    assert_eq!(NetError::of(text), expected);
}

#[rstest]
#[case::http2_stream("net::ERR_HTTP2_PROTOCOL_ERROR")]
#[case::connection_reset("net::ERR_CONNECTION_RESET")]
fn any_other_named_network_error_keeps_its_name(#[case] text: &str) {
    let error = NetError::of(text);

    assert_eq!(
        (matches!(error, NetError::Other(_)), error.name()),
        (true, text)
    );
}

/// Chromium gives these only for the proxy: one it cannot reach, whose TLS
/// failed, or that refused the tunnel or the SOCKS handshake (its
/// `net_error_list.h`).
#[rstest]
#[case::tunnel_refused("net::ERR_TUNNEL_CONNECTION_FAILED")]
#[case::proxy_down("net::ERR_PROXY_CONNECTION_FAILED")]
#[case::proxy_certificate("net::ERR_PROXY_CERTIFICATE_INVALID")]
#[case::socks_refused("net::ERR_SOCKS_CONNECTION_FAILED")]
#[case::socks_host_unreachable("net::ERR_SOCKS_CONNECTION_HOST_UNREACHABLE")]
#[case::proxy_auth_unsupported("net::ERR_PROXY_AUTH_UNSUPPORTED")]
#[case::https_proxy_redirect("net::ERR_HTTPS_PROXY_TUNNEL_RESPONSE_REDIRECT")]
#[case::mandatory_proxy("net::ERR_MANDATORY_PROXY_CONFIGURATION_FAILED")]
#[case::no_supported_proxies("net::ERR_NO_SUPPORTED_PROXIES")]
fn a_failure_chromium_gives_only_for_the_proxy_is_the_proxys(#[case] text: &str) {
    let error = NetError::of(text);

    assert_eq!((error.is_the_proxys(), error.name()), (true, text));
}

#[rstest]
#[case::site_refused("net::ERR_CONNECTION_REFUSED")]
#[case::name_not_resolved("net::ERR_NAME_NOT_RESOLVED")]
#[case::site_certificate("net::ERR_CERT_AUTHORITY_INVALID")]
#[case::aborted("net::ERR_ABORTED")]
#[case::unrecognized("the request failed for some reason")]
fn a_failure_the_site_can_cause_is_never_the_proxys(#[case] text: &str) {
    assert!(!NetError::of(text).is_the_proxys());
}

/// A certificate the browser refused for a reason the engine does not tell
/// apart keeps its name, and its code is that name's rest.
#[rstest]
#[case::expired_or_not_yet_valid("net::ERR_CERT_DATE_INVALID", "date_invalid")]
#[case::weak_key("net::ERR_CERT_WEAK_KEY", "weak_key")]
#[case::transparency(
    "net::ERR_CERTIFICATE_TRANSPARENCY_REQUIRED",
    "certificate_transparency_required"
)]
fn any_other_certificate_refusal_keeps_its_name_and_code(#[case] text: &str, #[case] code: &str) {
    let error = NetError::of(text);
    let NetError::Certificate(certificate) = &error else {
        panic!("{text} is no certificate's refusal: {error:?}");
    };

    assert_eq!((certificate.code(), error.name()), (code.to_owned(), text));
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
