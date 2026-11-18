//! Covers: story:5-120

use goodfetch_engine::browser_cache::BuildPlatform;
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::frontier::{HttpBody, HttpMethod};
use rstest::rstest;

use super::bridge::Head;
use super::{Rendered, arguments, pins, rendered, unsent};

/// What of a crawl request Lightpanda cannot send: its method when it
/// is no `GET`, else a body with bytes; an empty body is no body, as the
/// Frontier reads it (issue 2471).
#[rstest]
#[case::bodiless_get("GET", None, None)]
#[case::get_with_an_empty_body("GET", Some(""), None)]
#[case::get_with_a_body("GET", Some("q=1"), Some("crawl_request.body"))]
#[case::post("POST", Some("q=1"), Some("crawl_request.method"))]
#[case::bodiless_head("HEAD", None, Some("crawl_request.method"))]
fn lightpanda_sends_only_a_get_with_no_body(
    #[case] method: &str,
    #[case] body: Option<&str>,
    #[case] unsendable: Option<&str>,
) {
    let request = CrawlRequest {
        id: "1".to_owned(),
        url: "https://shop.test/".to_owned(),
        label: "page".to_owned(),
        payload: None,
        method: HttpMethod::parse(method).unwrap(),
        body: body.map(|body| HttpBody::new(body.as_bytes().to_vec())),
    };
    assert_eq!(unsent(&request), unsendable);
}

/// Each pinned build downloads its release's own executable for its
/// platform, from GitHub over HTTPS, never from Goodfetch's distribution;
/// its files' digest is the tree of that one file (ADR 0024's rule).
#[test]
fn each_platform_pins_its_release_executable_by_digest() {
    let pins = pins();
    let named: Vec<(Option<BuildPlatform>, &str, &str)> = pins
        .iter()
        .map(|pin| {
            (
                pin.platform,
                pin.id.build.as_str(),
                pin.url
                    .strip_prefix("https://github.com/lightpanda-io/browser/releases/download/")
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            (
                Some(BuildPlatform::LinuxArm64),
                "1.0.2",
                "1.0.2/lightpanda-aarch64-linux"
            ),
            (
                Some(BuildPlatform::LinuxX64),
                "1.0.2",
                "1.0.2/lightpanda-x86_64-linux"
            ),
            (
                Some(BuildPlatform::MacArm64),
                "1.0.2",
                "1.0.2/lightpanda-aarch64-macos"
            ),
            (
                Some(BuildPlatform::MacX64),
                "1.0.2",
                "1.0.2/lightpanda-x86_64-macos"
            ),
        ]
    );
    // The tree digest of the one file `lightpanda-x86_64-linux` whose own
    // SHA-256 is the release's, worked out by hand.
    let linux = pins
        .iter()
        .find(|pin| pin.platform == Some(BuildPlatform::LinuxX64))
        .unwrap();
    assert_eq!(
        (
            linux.archive_digest.map(|digest| digest.to_string()),
            linux.digest.to_string()
        ),
        (
            Some(
                "sha256:ae7d01d261efb9e480e18ce3424533332883863bcf70b12a4321b5733f37e19e"
                    .to_owned()
            ),
            "sha256:5aed29fca09ac92f2552587cdf7292e0d692348c0a6bc5110b112af4df99a6f5".to_owned()
        )
    );
}

/// Lightpanda's report of a page it loaded reads as the document its
/// scripts left, with the URL, status and headers of its answer.
#[test]
fn a_report_reads_as_the_document_its_scripts_left() {
    let report = br#"{"url":"http://tiers.test:8080/blog/1","http_status":200,"headers":[{"name":"Content-Type","value":"text/html"}],"dump":"html","content":"<html><body><li class=\"item\">a</li></body></html>","error":null,"page_status":{"js_errors":0}}"#;

    assert_eq!(
        rendered(report),
        Ok(Rendered {
            url: "http://tiers.test:8080/blog/1".parse().unwrap(),
            status: 200,
            headers: vec![("content-type".to_owned(), "text/html".to_owned())],
            html: "<html><body><li class=\"item\">a</li></body></html>".to_owned(),
        })
    );
}

/// A report naming an error, or one that does not read, is no document.
#[rstest::rstest]
#[case::its_error(br#"{"url":"http://a.test/","http_status":null,"headers":[],"content":null,"error":"ConnectionRefused"}"#.as_slice(), "ConnectionRefused")]
#[case::no_report(b"".as_slice(), "its report does not read")]
fn a_failed_report_names_why(#[case] report: &[u8], #[case] starts: &str) {
    let error = rendered(report).unwrap_err();
    assert!(error.starts_with(starts), "{error}");
}

/// Lightpanda reaches the network only through the bridge, and waits for
/// the page's network to go idle.
#[test]
fn lightpanda_runs_fetch_through_the_bridge() {
    assert_eq!(
        arguments("http://a.test/", "http://goodfetch:t@127.0.0.1:1").join(" "),
        "fetch --json --dump html --log-level fatal --wait-until networkidle --wait-ms 10000 --terminate-ms 30000 --http-version 1.1 --http-proxy http://goodfetch:t@127.0.0.1:1 http://a.test/"
    );
}

/// The bridge sends an absolute-form request on in origin form, without
/// the proxy's own headers, one request per connection; a `CONNECT` goes
/// to its host as `https`, and anything else nowhere.
#[test]
fn the_bridge_forwards_a_request_in_origin_form_without_its_proxy_headers() {
    let head = Head::parse(
        b"GET http://tiers.test:8080/blog/1?x=1 HTTP/1.1\r\nHost: tiers.test:8080\r\nProxy-Authorization: Basic Zw==\r\nProxy-Connection: Keep-Alive\r\nAccept: */*\r\n\r\n",
    )
    .unwrap();
    let url = head.destination().unwrap();

    assert_eq!(
        String::from_utf8(head.forwarded(&url)).unwrap(),
        "GET /blog/1?x=1 HTTP/1.1\r\nHost: tiers.test:8080\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    let tunnel =
        Head::parse(b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n")
            .unwrap();
    assert_eq!(
        tunnel.destination().map(|url| url.to_string()),
        Some("https://example.test/".to_owned())
    );
    let elsewhere = Head::parse(b"GET ftp://a.test/ HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!(elsewhere.destination(), None);
}
