use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use secrecy::SecretString;

use goodfetch_protocol::response_limits::DEFAULT_MAX_DECOMPRESSION_RATIO;

use super::{Answer, Miss, RecordingInvalid, Replay};
use crate::mask::{Masks, Reach, Redaction};
use crate::response_cache::{CacheRequest, ResponseHead, Served};
use crate::warc;
use crate::warc::tests::record;

/// A request record and the response record it was answered with, `id`
/// naming both, fetched from `url` at `date`.
fn exchange(id: u32, url: &str, date: &str, request: &str, response: &str) -> Vec<u8> {
    let request_id = format!("<urn:uuid:{id}-request>");
    let response_id = format!("<urn:uuid:{id}-response>");
    let mut bytes = record(
        &[
            ("WARC-Type", "request"),
            ("WARC-Record-ID", &request_id),
            ("WARC-Concurrent-To", &response_id),
            ("WARC-Target-URI", url),
            ("Content-Type", "application/http; msgtype=request"),
        ],
        request.as_bytes(),
    );
    bytes.extend(record(
        &[
            ("WARC-Type", "response"),
            ("WARC-Record-ID", &response_id),
            ("WARC-Target-URI", url),
            ("WARC-Date", date),
            ("Content-Type", "application/http; msgtype=response"),
        ],
        response.as_bytes(),
    ));
    bytes
}

/// A response record with no request record, as some writers leave out,
/// with `extra` headers, which come first and so override the others.
fn response(id: &str, url: &str, extra: &[(&str, &str)], response: &str) -> Vec<u8> {
    let mut headers = extra.to_vec();
    headers.extend([
        ("WARC-Type", "response"),
        ("WARC-Record-ID", id),
        ("WARC-Target-URI", url),
        ("Content-Type", "application/http; msgtype=response"),
    ]);
    record(&headers, response.as_bytes())
}

fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn get(url: &str) -> String {
    let url = url::Url::parse(url).expect("a URL");
    format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\n\r\n",
        &url[url::Position::BeforePath..url::Position::AfterQuery],
        url.host_str().expect("a host")
    )
}

fn replay(file: Vec<u8>) -> Replay {
    open(&Bytes::from(file), Masks::new()).expect("a Response recording")
}

/// The Response recording `file`, under the catalog's default
/// `network.max_decompression_ratio`.
fn open(file: &Bytes, masks: Masks) -> Result<Replay, RecordingInvalid> {
    Replay::open(file, masks, DEFAULT_MAX_DECOMPRESSION_RATIO)
}

fn request(method: &str, url: &str, headers: &[(&str, &str)], body: &str) -> CacheRequest {
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CacheRequest::new(
        method,
        &url::Url::parse(url).expect("a URL"),
        &headers,
        Bytes::from(body.to_owned()),
    )
}

/// The body an answer serves, or what it missed.
fn body(answer: Answer) -> Result<String, Miss> {
    match answer {
        Answer::Answered(served) => Ok(String::from_utf8_lossy(&served.body).into_owned()),
        Answer::Cut(served) => panic!("a cut response: {served:?}"),
        Answer::Missed(miss) => Err(miss),
    }
}

const A: &str = "https://example.test/a?x=1";
const DATE: &str = "2026-11-20T10:00:00Z";

#[test]
fn answers_a_request_of_the_recorded_method_and_url_whatever_its_fragment() {
    let replay = replay(exchange(1, A, DATE, &get(A), &ok("hello")));

    let answer = replay.answer(&request("GET", "https://example.test/a?x=1#top", &[], ""));

    assert_eq!(
        answer,
        Answer::Answered(Arc::new(Served {
            head: ResponseHead {
                status: 200,
                headers: vec![
                    ("content-type".to_owned(), "text/plain".to_owned()),
                    ("content-length".to_owned(), "5".to_owned()),
                ],
            },
            body: Bytes::from_static(b"hello"),
        }))
    );
    assert!(body(replay.answer(&request("HEAD", A, &[], ""))).is_err());
    assert!(
        body(replay.answer(&request("get", A, &[], ""))).is_err(),
        "a method is case-sensitive (RFC 9110 §9.1)"
    );
    assert!(body(replay.answer(&request("GET", "https://example.test/a?x=2", &[], ""))).is_err());
}

#[test]
fn answers_a_key_recorded_several_times_in_the_order_recorded_then_by_its_last() {
    let mut file = exchange(1, A, DATE, &get(A), &ok("first"));
    file.extend(exchange(2, A, DATE, &get(A), &ok("second")));
    let replay = replay(file);

    let bodies: Vec<String> = (0..3)
        .map(|_| body(replay.answer(&request("GET", A, &[], ""))).expect("an answer"))
        .collect();

    assert_eq!(bodies, ["first", "second", "second"]);
}

#[test]
fn answers_a_form_body_by_its_canonical_form() {
    let url = "https://example.test/search";
    let post = "POST /search HTTP/1.1\r\nHost: example.test\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 7\r\n\r\nb=2&a=1";
    let replay = replay(exchange(1, url, DATE, post, &ok("found")));
    let form = [("Content-Type", "application/x-www-form-urlencoded")];

    assert_eq!(
        body(replay.answer(&request("POST", url, &form, "a=1&b=2"))),
        Ok("found".to_owned())
    );
    assert!(body(replay.answer(&request("POST", url, &form, "a=1&b=3"))).is_err());
}

#[test]
fn answers_by_the_headers_vary_names_but_credentials() {
    let get = "GET /a?x=1 HTTP/1.1\r\nHost: example.test\r\nAccept-Language: fr\r\nCookie: session=recorded\r\n\r\n";
    let vary =
        "HTTP/1.1 200 OK\r\nVary: Accept-Language, Cookie\r\nContent-Length: 7\r\n\r\nbonjour";
    let replay = replay(exchange(1, A, DATE, get, vary));

    let french = replay.answer(&request(
        "GET",
        A,
        &[("Accept-Language", "fr"), ("Cookie", "session=live")],
        "",
    ));
    let english = replay.answer(&request("GET", A, &[("Accept-Language", "en")], ""));

    assert_eq!(body(french), Ok("bonjour".to_owned()));
    assert!(body(english).is_err());
}

#[test]
fn answers_a_response_recorded_without_its_request_to_a_get_without_a_body() {
    let replay = replay(response(
        "<urn:uuid:1>",
        A,
        &[],
        "HTTP/1.1 200 OK\r\nVary: Accept\r\nContent-Length: 2\r\n\r\nok",
    ));

    assert_eq!(
        body(replay.answer(&request("GET", A, &[("Accept", "text/html")], ""))),
        Ok("ok".to_owned())
    );
    assert!(body(replay.answer(&request("POST", A, &[], "a=1"))).is_err());
}

#[test]
fn compares_a_live_body_holding_a_secrets_value_with_its_placeholder() {
    let url = "https://example.test/login";
    let recorded = "user=ada&password=%password%";
    let post = format!(
        "POST /login HTTP/1.1\r\nHost: example.test\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{recorded}",
        recorded.len()
    );
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    mask.join(
        "password",
        &SecretString::from("correct horse"),
        Reach::Value,
    );
    let replay = open(
        &Bytes::from(exchange(1, url, DATE, &post, &ok("welcome"))),
        masks,
    )
    .expect("a Response recording");
    let form = [("Content-Type", "application/x-www-form-urlencoded")];

    let live = replay.answer(&request(
        "POST",
        url,
        &form,
        "password=correct horse&user=ada",
    ));
    let wrong = replay.answer(&request("POST", url, &form, "password=wrong&user=ada"));

    assert_eq!(body(live), Ok("welcome".to_owned()));
    assert!(body(wrong).is_err());
}

#[test]
fn a_miss_names_the_recorded_urls_of_its_host_and_path_and_the_report_lists_it() {
    let mut file = exchange(1, A, DATE, &get(A), &ok("1"));
    let second = "https://example.test/a?x=2";
    file.extend(exchange(2, second, DATE, &get(second), &ok("2")));
    let other = "https://example.test/b";
    file.extend(exchange(3, other, DATE, &get(other), &ok("3")));
    let replay = replay(file);

    let answer = replay.answer(&request("GET", "https://example.test/a?x=3", &[], ""));

    let miss = Miss {
        method: "GET".to_owned(),
        url: "https://example.test/a?x=3".to_owned(),
        nearest: vec![A.to_owned(), second.to_owned()],
    };
    assert_eq!(miss.to_string(), "GET https://example.test/a?x=3");
    assert_eq!(answer, Answer::Missed(miss.clone()));
    assert_eq!(replay.report().misses, [miss]);
}

#[test]
fn reports_the_responses_used_when_they_were_fetched_and_the_writer() {
    let mut file = record(
        &[
            ("WARC-Type", "warcinfo"),
            ("Content-Type", "application/warc-fields"),
        ],
        b"format: WARC File Format 1.1\r\nsoftware: Goodfetch 0.1\r\n",
    );
    let (b, c) = ("https://example.test/b", "https://example.test/c");
    file.extend(exchange(1, A, "2026-11-20T10:00:00Z", &get(A), &ok("1")));
    file.extend(exchange(2, b, "2026-11-21T10:00:00Z", &get(b), &ok("2")));
    file.extend(exchange(3, c, "2026-11-22T10:00:00Z", &get(c), &ok("3")));
    let replay = replay(file);

    for url in [A, c, c] {
        assert!(body(replay.answer(&request("GET", url, &[], ""))).is_ok());
    }
    let report = replay.report();

    let at = |date: &str| SystemTime::from(date.parse::<jiff::Timestamp>().expect("a date"));
    assert_eq!(report.responses_used, 2);
    assert_eq!(report.recorded_from, Some(at("2026-11-20T10:00:00Z")));
    assert_eq!(report.recorded_to, Some(at("2026-11-22T10:00:00Z")));
    assert_eq!(report.written_by.as_deref(), Some("Goodfetch 0.1"));
    assert_eq!(report.misses, []);
}

#[test]
fn answers_a_server_not_modified_revisit_with_the_originals_payload_and_updated_headers() {
    let mut file = response(
        "<urn:uuid:1>",
        A,
        &[],
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nETag: \"1\"\r\nContent-Length: 5\r\n\r\nhello",
    );
    file.extend(response(
        "<urn:uuid:2>",
        "https://example.test/a?x=1#later",
        &[
            ("WARC-Type", "revisit"),
            ("WARC-Refers-To", "<urn:uuid:1>"),
            (
                "WARC-Profile",
                "http://netpreserve.org/warc/1.1/revisit/server-not-modified",
            ),
        ],
        "HTTP/1.1 304 Not Modified\r\nETag: \"2\"\r\n\r\n",
    ));
    let replay = replay(file);

    replay.answer(&request("GET", A, &[], ""));
    let revisited = replay.answer(&request("GET", A, &[], ""));

    assert_eq!(
        revisited,
        Answer::Answered(Arc::new(Served {
            head: ResponseHead {
                status: 200,
                headers: vec![
                    ("content-type".to_owned(), "text/plain".to_owned()),
                    ("content-length".to_owned(), "5".to_owned()),
                    ("etag".to_owned(), "\"2\"".to_owned()),
                ],
            },
            body: Bytes::from_static(b"hello"),
        }))
    );
}

#[test]
fn answers_an_identical_payload_revisit_with_its_own_head_and_the_originals_body() {
    let digest = ("WARC-Payload-Digest", "sha1:AAAA");
    let mut file = response("<urn:uuid:1>", A, &[digest], &ok("hello"));
    let b = "https://example.test/b";
    file.extend(response(
        "<urn:uuid:2>",
        b,
        &[
            ("WARC-Type", "revisit"),
            digest,
            (
                "WARC-Profile",
                "http://netpreserve.org/warc/1.1/revisit/identical-payload-digest",
            ),
        ],
        "HTTP/1.1 200 OK\r\nX-Copy: yes\r\n\r\n",
    ));
    let replay = replay(file);

    let answer = replay.answer(&request("GET", b, &[], ""));

    assert_eq!(
        answer,
        Answer::Answered(Arc::new(Served {
            head: ResponseHead {
                status: 200,
                headers: vec![("x-copy".to_owned(), "yes".to_owned())],
            },
            body: Bytes::from_static(b"hello"),
        }))
    );
}

#[test]
fn answers_a_revisit_of_an_original_recorded_truncated_as_cut() {
    let mut file = response(
        "<urn:uuid:1>",
        A,
        &[("WARC-Truncated", "disconnect")],
        &ok("hello"),
    );
    let b = "https://example.test/b";
    file.extend(response(
        "<urn:uuid:2>",
        b,
        &[("WARC-Type", "revisit"), ("WARC-Refers-To", "<urn:uuid:1>")],
        "HTTP/1.1 200 OK\r\n\r\n",
    ));
    let replay = replay(file);

    let answer = replay.answer(&request("GET", b, &[], ""));

    assert!(
        matches!(&answer, Answer::Cut(served) if served.body.as_ref() == b"hello"),
        "{answer:?}"
    );
}

#[test]
fn leaves_out_a_revisit_whose_original_is_not_in_the_file() {
    let replay = replay(response(
        "<urn:uuid:2>",
        A,
        &[("WARC-Type", "revisit"), ("WARC-Refers-To", "<urn:uuid:1>")],
        "HTTP/1.1 304 Not Modified\r\n\r\n",
    ));

    assert!(body(replay.answer(&request("GET", A, &[], ""))).is_err());
}

#[test]
fn answers_a_response_recorded_truncated_or_broken_off_as_cut() {
    let mut file = response(
        "<urn:uuid:1>",
        A,
        &[("WARC-Truncated", "length")],
        &ok("hello"),
    );
    let b = "https://example.test/b";
    file.extend(response(
        "<urn:uuid:2>",
        b,
        &[],
        "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhello",
    ));
    let replay = replay(file);

    for url in [A, b] {
        let answer = replay.answer(&request("GET", url, &[], ""));

        assert!(
            matches!(&answer, Answer::Cut(served) if served.body.as_ref() == b"hello"),
            "{url}: {answer:?}"
        );
    }
}

#[test]
fn refuses_a_file_that_is_not_a_response_recording() {
    let not_warc = open(&Bytes::from_static(b"<html></html>"), Masks::new());
    let not_http = open(
        &Bytes::from(response(
            "<urn:uuid:9>",
            A,
            &[],
            "not an HTTP message\r\n\r\n",
        )),
        Masks::new(),
    );

    assert!(matches!(
        not_warc,
        Err(RecordingInvalid::Unreadable(warc::Unreadable::NotWarc))
    ));
    assert!(matches!(
        not_http,
        Err(RecordingInvalid::NotHttp { record, .. }) if record == "<urn:uuid:9>"
    ));
}

#[test]
fn skips_records_that_answer_no_request() {
    let mut file = record(&[("WARC-Type", "resource"), ("WARC-Target-URI", A)], b"x");
    file.extend(record(
        &[
            ("WARC-Type", "response"),
            ("WARC-Target-URI", A),
            ("Content-Type", "text/dns"),
        ],
        b"x",
    ));
    file.extend(response("<urn:uuid:1>", "not a url", &[], &ok("x")));
    let replay = replay(file);

    let miss = body(replay.answer(&request("GET", A, &[], ""))).expect_err("a miss");

    assert_eq!(miss.nearest, Vec::<String>::new());
}
