use hyper::header::{HeaderMap, HeaderValue};

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{Form, ProductVersion, Surface};
use goodfetch_protocol::http_listener::{HEADER_FORM, HEADER_PRODUCT, HEADER_PROTOCOL};

use super::{extract_trace_headers, parse_hello_headers, query_carries_token};

#[test]
fn query_carries_token_detects_token_parameters() {
    assert!(!query_carries_token(None));
    assert!(!query_carries_token(Some("run=123&after=4")));
    assert!(query_carries_token(Some("access_token=abc")));
    assert!(query_carries_token(Some("run=1&TOKEN=abc")));
    assert!(query_carries_token(Some("foo=bar&Access_Token=secret")));
}

#[test]
fn parse_hello_headers_defaults_form_to_catalog_and_rejects_missing_or_invalid() {
    let version = ProductVersion::parse("0.1.0").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(HEADER_PROTOCOL, HeaderValue::from_static("0.1"));
    headers.insert(HEADER_PRODUCT, HeaderValue::from_static("collie"));

    let hello = parse_hello_headers(&headers, &version).unwrap();
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    assert_eq!(hello.needs, Some(Product::Collie));
    assert_eq!(hello.surface, Surface::HttpListener);
    assert_eq!(hello.form, Some(Form::Catalog));

    headers.insert(HEADER_FORM, HeaderValue::from_static("tool"));
    let hello = parse_hello_headers(&headers, &version).unwrap();
    assert_eq!(hello.form, Some(Form::Tool));

    headers.insert(HEADER_FORM, HeaderValue::from_static("unknown"));
    assert!(parse_hello_headers(&headers, &version).is_err());

    let mut missing_product = HeaderMap::new();
    missing_product.insert(HEADER_PROTOCOL, HeaderValue::from_static("0.1"));
    assert!(parse_hello_headers(&missing_product, &version).is_err());

    let mut bad_protocol = HeaderMap::new();
    bad_protocol.insert(HEADER_PROTOCOL, HeaderValue::from_static("nope"));
    bad_protocol.insert(HEADER_PRODUCT, HeaderValue::from_static("collie"));
    assert!(parse_hello_headers(&bad_protocol, &version).is_err());
}

#[test]
fn extract_trace_headers_drops_non_utf8_and_keeps_valid_headers() {
    let mut headers = HeaderMap::new();
    assert_eq!(extract_trace_headers(&headers), None);

    headers.insert(
        "traceparent",
        HeaderValue::from_static("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
    );
    headers.insert(
        "tracestate",
        HeaderValue::from_static("rojo=00f067aa010902b7"),
    );
    let trace = extract_trace_headers(&headers).unwrap();
    assert_eq!(
        trace.traceparent.as_deref(),
        Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
    );
    assert_eq!(trace.tracestate.as_deref(), Some("rojo=00f067aa010902b7"));

    let mut non_utf8 = HeaderMap::new();
    non_utf8.insert(
        "traceparent",
        HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
    );
    assert_eq!(extract_trace_headers(&non_utf8), None);
}
