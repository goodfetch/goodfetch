//! Fetch's redirect rules as a pure function of a hop and its answer's
//! head (ADR 0118): each status and `Location`, the method, body and
//! request-body headers each status keeps, and `Authorization` across
//! origins.

use bytes::Bytes;
use rstest::rstest;
use url::Url;

use super::{Next, next};
use crate::driver::Hop;

/// A hop of `method` to `http://site.test/from`, carrying a body and its
/// request-body headers, `Authorization` and an `Accept` header.
fn hop(method: &str) -> Hop {
    Hop {
        url: Url::parse("http://site.test/from").unwrap(),
        method: http::Method::from_bytes(method.as_bytes()).unwrap(),
        headers: vec![
            ("accept".to_owned(), "text/html".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("content-length".to_owned(), "2".to_owned()),
            (
                "authorization".to_owned(),
                "Bearer {{secret:api}}".to_owned(),
            ),
        ],
        body: Some(Bytes::from_static(b"{}")),
        read: true,
    }
}

/// The method, whether the body is kept and the header names, in order,
/// of the hop `next` follows to.
fn followed(next: Next) -> (String, String, bool, Vec<String>) {
    let Next::Follow(hop) = next else {
        panic!("not followed: {next:?}");
    };
    (
        hop.url.to_string(),
        hop.method.to_string(),
        hop.body.is_some(),
        hop.headers.into_iter().map(|(name, _)| name).collect(),
    )
}

const ALL: [&str; 4] = ["accept", "Content-Type", "content-length", "authorization"];
const NO_BODY_HEADERS: [&str; 2] = ["accept", "authorization"];

/// Covers: story:5-124
#[rstest]
#[case::a_301_turns_a_post_into_a_get(301, "POST", "GET", false)]
#[case::a_302_turns_a_post_into_a_get(302, "POST", "GET", false)]
#[case::a_301_keeps_a_put(301, "PUT", "PUT", true)]
#[case::a_302_keeps_a_get(302, "GET", "GET", true)]
#[case::a_303_turns_a_put_into_a_get(303, "PUT", "GET", false)]
#[case::a_303_turns_a_post_into_a_get(303, "POST", "GET", false)]
#[case::a_303_keeps_a_head(303, "HEAD", "HEAD", true)]
#[case::a_307_keeps_a_post_and_its_body(307, "POST", "POST", true)]
#[case::a_308_keeps_a_put_and_its_body(308, "PUT", "PUT", true)]
fn a_redirect_status_keeps_or_turns_the_method_as_fetch_does(
    #[case] status: u16,
    #[case] method: &str,
    #[case] expected: &str,
    #[case] body_kept: bool,
) {
    let headers: Vec<String> = if body_kept {
        ALL.to_vec()
    } else {
        NO_BODY_HEADERS.to_vec()
    }
    .into_iter()
    .map(str::to_owned)
    .collect();

    assert_eq!(
        followed(next(&hop(method), status, Some("/to?x=1"))),
        (
            "http://site.test/to?x=1".to_owned(),
            expected.to_owned(),
            body_kept,
            headers
        )
    );
}

/// Covers: story:5-124
#[rstest]
#[case::to_another_host("https://other.test/to", false)]
#[case::to_another_scheme("https://site.test/to", false)]
#[case::to_another_port("http://site.test:8080/to", false)]
#[case::on_the_same_origin("/to", true)]
#[case::on_the_same_origin_spelt_in_full("HTTP://SITE.test:80/to", true)]
fn authorization_is_removed_once_a_hop_reaches_another_origin(
    #[case] location: &str,
    #[case] kept: bool,
) {
    let (_, _, _, headers) = followed(next(&hop("GET"), 307, Some(location)));

    assert_eq!(headers.iter().any(|name| name == "authorization"), kept);
}

/// Covers: story:5-124
#[rstest]
#[case::a_300(300, Some("/to"))]
#[case::a_304(304, Some("/to"))]
#[case::a_305(305, Some("/to"))]
#[case::a_306(306, Some("/to"))]
#[case::a_redirect_with_no_location(302, None)]
#[case::a_location_that_does_not_parse(301, Some("http://[::1/"))]
#[case::a_location_naming_ftp(302, Some("ftp://site.test/to"))]
#[case::a_location_naming_data(308, Some("data:text/html,hi"))]
#[case::a_location_naming_file(307, Some("file:///etc/passwd"))]
fn any_other_3xx_is_not_ok(#[case] status: u16, #[case] location: Option<&str>) {
    assert!(
        matches!(next(&hop("GET"), status, location), Next::NotOk(_)),
        "{status} {location:?}"
    );
}

/// Covers: story:5-124
#[rstest]
#[case::a_200(200)]
#[case::a_204(204)]
#[case::a_404(404)]
#[case::a_503(503)]
fn an_answer_outside_3xx_is_the_document(#[case] status: u16) {
    assert_eq!(next(&hop("GET"), status, Some("/to")), Next::Answer);
}
