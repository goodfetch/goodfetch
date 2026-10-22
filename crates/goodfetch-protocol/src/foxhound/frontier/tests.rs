use rstest::rstest;

use super::{HttpBody, HttpBodyInvalid, HttpMethod, HttpMethodInvalid};

/// Fetch's normalization: the six methods it knows uppercased, any other
/// kept as written, since methods are case-sensitive.
#[rstest]
#[case::get_in_lowercase("get", "GET")]
#[case::post_in_mixed_case("Post", "POST")]
#[case::options_in_lowercase("options", "OPTIONS")]
#[case::patch_kept("PATCH", "PATCH")]
#[case::patch_in_lowercase_kept("patch", "patch")]
#[case::an_extension_method("PROPFIND", "PROPFIND")]
fn a_method_reads_as_fetch_normalizes_it(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(
        HttpMethod::parse(text).map(|method| method.as_str().to_owned()),
        Ok(expected.to_owned())
    );
}

#[rstest]
#[case::empty("", HttpMethodInvalid::NotAToken)]
#[case::a_space("GE T", HttpMethodInvalid::NotAToken)]
#[case::a_separator("GET/1", HttpMethodInvalid::NotAToken)]
#[case::non_ascii("GÉT", HttpMethodInvalid::NotAToken)]
#[case::connect("CONNECT", HttpMethodInvalid::Forbidden)]
#[case::trace_in_lowercase("trace", HttpMethodInvalid::Forbidden)]
#[case::track("Track", HttpMethodInvalid::Forbidden)]
fn a_method_that_is_no_token_or_that_fetch_forbids_is_refused(
    #[case] text: &str,
    #[case] expected: HttpMethodInvalid,
) {
    assert_eq!(HttpMethod::parse(text), Err(expected));
}

#[rstest]
#[case::text("aGk=", b"hi".as_slice())]
#[case::empty("", b"".as_slice())]
#[case::bytes_that_are_no_utf_8("//79", [0xff, 0xfe, 0xfd].as_slice())]
fn a_body_reads_from_base64(#[case] text: &str, #[case] expected: &[u8]) {
    assert_eq!(
        HttpBody::from_base64(text).map(|body| body.bytes().to_vec()),
        Ok(expected.to_vec())
    );
}

#[rstest]
#[case::not_base64("hi!")]
#[case::without_padding("aGk")]
fn a_body_that_is_no_base64_is_refused(#[case] text: &str) {
    assert_eq!(HttpBody::from_base64(text), Err(HttpBodyInvalid));
}
