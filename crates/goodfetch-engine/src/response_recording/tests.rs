#![expect(
    clippy::disallowed_methods,
    reason = "unit tests use tokio::test, paused Clock and std::fs::read"
)]

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
        Answer::Answered { served, .. } => Ok(String::from_utf8_lossy(&served.body).into_owned()),
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

    let expected_at = SystemTime::from(DATE.parse::<jiff::Timestamp>().expect("a date"));
    assert_eq!(
        answer,
        Answer::Answered {
            served: Arc::new(Served {
                head: ResponseHead {
                    status: 200,
                    headers: vec![
                        ("content-type".to_owned(), "text/plain".to_owned()),
                        ("content-length".to_owned(), "5".to_owned()),
                    ],
                },
                body: Bytes::from_static(b"hello"),
            }),
            recorded_at: Some(expected_at),
        }
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
        Answer::Answered {
            served: Arc::new(Served {
                head: ResponseHead {
                    status: 200,
                    headers: vec![
                        ("content-type".to_owned(), "text/plain".to_owned()),
                        ("content-length".to_owned(), "5".to_owned()),
                        ("etag".to_owned(), "\"2\"".to_owned()),
                    ],
                },
                body: Bytes::from_static(b"hello"),
            }),
            recorded_at: None,
        }
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
        Answer::Answered {
            served: Arc::new(Served {
                head: ResponseHead {
                    status: 200,
                    headers: vec![("x-copy".to_owned(), "yes".to_owned())],
                },
                body: Bytes::from_static(b"hello"),
            }),
            recorded_at: None,
        }
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

#[tokio::test]
async fn recording_redacts_credentials_masks_secrets_and_replays_both_halves_and_cut() {
    let dir = tempfile::tempdir().unwrap();
    let warc_path = dir.path().join("out.warc.gz");
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    let secret = "gf-secret-token-9876543210";
    run_mask.join("api_key", &SecretString::from(secret), Reach::Value);

    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for statement in super::TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }

    let first_at: SystemTime = "2026-01-05T09:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .into();
    let second_at: SystemTime = "2026-01-05T09:05:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .into();

    // First half before a simulated kill: redacts credentials & masks secret before SQLite insert.
    let first = super::SanitizedExchange::from_raw(
        &masks,
        request(
            "POST",
            A,
            &[
                ("cookie", "sid=live-cookie-secret"),
                ("authorization", "Bearer live-bearer-secret"),
                ("proxy-authorization", "Basic cHJveHk6c2VjcmV0"),
                ("x-custom", &format!("val-{secret}")),
                ("content-type", "application/json"),
            ],
            &format!(r#"{{"token":"{secret}"}}"#),
        ),
        ResponseHead {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), "text/plain".to_owned()),
                (
                    "set-cookie".to_owned(),
                    "sid=top-secret-session-cookie; Path=/; HttpOnly".to_owned(),
                ),
            ],
        },
        Bytes::from(format!("hello {secret}")),
        false,
        first_at,
    );
    assert!(!first.request_headers.iter().any(|(n, _)| matches!(
        n.as_str(),
        "cookie" | "authorization" | "proxy-authorization"
    )));
    assert_eq!(
        first.response_headers[1],
        (
            "set-cookie".to_owned(),
            "sid=[redacted]; Path=/; HttpOnly".to_owned()
        )
    );
    assert!(!String::from_utf8_lossy(&first.request_body).contains(secret));
    assert!(!String::from_utf8_lossy(&first.response_body).contains(secret));
    super::record::insert_exchange(&connection, &first).unwrap();

    // Second half after resume: records a cut response into the same SQLite table.
    let recorder = super::Recorder::new(
        warc_path.display().to_string(),
        goodfetch_protocol::run::RunId::parse("run-resume-1").unwrap(),
        "Collie 0.1.0".to_owned(),
        masks.clone(),
        crate::clock::Clock::paused(second_at),
        42,
        super::RecorderPolicy {
            stage_for_attempt: true,
            write_on_end: true,
        },
    );
    let cut_exchange = super::SanitizedExchange::from_raw(
        &masks,
        request("GET", "https://example.test/cut", &[], ""),
        ResponseHead {
            status: 200,
            headers: vec![("content-length".to_owned(), "100".to_owned())],
        },
        Bytes::from_static(b"partial"),
        true,
        second_at,
    );
    super::record::insert_exchange(&connection, &cut_exchange).unwrap();

    let report = recorder
        .finish_and_write(&connection)
        .unwrap()
        .expect("writes WARC");
    assert_eq!(report.mode, "record");
    assert_eq!(report.responses_used, 2);
    assert_eq!(report.recorded_from, Some(first_at));
    assert_eq!(report.recorded_to, Some(second_at));
    assert_eq!(report.written_by.as_deref(), Some("Collie 0.1.0"));
    assert_recorded_warc_redacted_and_replays(&warc_path, secret, masks);
}

fn assert_recorded_warc_redacted_and_replays(
    warc_path: &std::path::Path,
    secret: &str,
    masks: Masks,
) {
    // Raw file bytes are gzipped per record and hold zero secrets or cookies.
    let raw = std::fs::read(warc_path).unwrap();
    assert!(raw.starts_with(&[0x1f, 0x8b]));
    let records = warc::read(&Bytes::from(raw.clone()), DEFAULT_MAX_DECOMPRESSION_RATIO).unwrap();
    assert_eq!(records.len(), 5); // warcinfo + 2*(request + response)
    let warcinfo_text = String::from_utf8_lossy(records[0].block());
    assert!(warcinfo_text.contains("software: Collie 0.1.0"));
    assert!(warcinfo_text.contains("run-id: run-resume-1"));
    assert!(warcinfo_text.contains("mode: record"));
    assert!(warcinfo_text.contains("warc-writer-version: "));
    for record in &records {
        let block = String::from_utf8_lossy(record.block());
        assert!(!block.contains(secret));
        assert!(!block.contains("live-cookie-secret"));
        assert!(!block.contains("live-bearer-secret"));
        assert!(!block.contains("top-secret-session-cookie"));
    }

    // Replaying the written file answers the POST (matching masked secret) and cuts `/cut`.
    let replayed = open(&Bytes::from(raw), masks).unwrap();
    let answered = replayed.answer(&request(
        "POST",
        A,
        &[("content-type", "application/json")],
        &format!(r#"{{"token":"{secret}"}}"#),
    ));
    assert!(
        matches!(answered, Answer::Answered { served, .. } if served.body.as_ref() == b"hello %api_key%")
    );
    let cut_answer = replayed.answer(&request("GET", "https://example.test/cut", &[], ""));
    assert!(matches!(cut_answer, Answer::Cut(_)));
}

#[tokio::test]
async fn recording_to_directory_returns_path_not_writable_and_leaves_existing_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for statement in super::TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let now: SystemTime = "2026-01-05T09:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .into();
    let recorder = super::Recorder::new(
        dir.path().display().to_string(),
        goodfetch_protocol::run::RunId::parse("run-bad-path").unwrap(),
        "Collie 0.1.0".to_owned(),
        Masks::new(),
        crate::clock::Clock::paused(now),
        7,
        super::RecorderPolicy {
            stage_for_attempt: true,
            write_on_end: true,
        },
    );
    let err = recorder
        .finish_and_write(&connection)
        .unwrap()
        .expect_err("directory path must fail");
    assert_eq!(
        err.code,
        goodfetch_protocol::checkpoint::PATH_NOT_WRITABLE.name
    );
}

#[tokio::test(start_paused = true)]
async fn failed_run_keeps_its_own_error_and_records_write_failure_as_warning() {
    let dir = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for statement in super::TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let now: SystemTime = "2026-01-05T09:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .into();
    let run_id = goodfetch_protocol::run::RunId::parse("run-failed-write").unwrap();
    let recorder = super::Recorder::new(
        dir.path().display().to_string(),
        run_id.clone(),
        "Collie 0.1.0".to_owned(),
        Masks::new(),
        crate::clock::Clock::paused(now),
        9,
        super::RecorderPolicy {
            stage_for_attempt: true,
            write_on_end: true,
        },
    );
    let recordings = super::Recordings::default();
    recordings.attach_recorder(run_id.clone(), recorder);

    // Exercise the real RecordingReportPart production path (`Recordings::apply_to_report`):
    // a Run that already failed with its own error keeps `report.error` and records
    // the recording write failure in `report.warnings`.
    let mut report = goodfetch_protocol::run::RunReport {
        run_id: run_id.clone(),
        lifecycle: goodfetch_protocol::run::Lifecycle::Failed,
        stop_reason: None,
        verdict: goodfetch_protocol::run::Verdict::NeedsReview,
        verdict_reasons: Vec::new(),
        completeness: goodfetch_protocol::run::Completeness::Incomplete,
        completeness_reasons: Vec::new(),
        started_at: now,
        ended_at: now,
        usage: goodfetch_protocol::run::RunUsage::default(),
        weakened: Vec::new(),
        plan: None,
        policy_version: None,
        settings: None,
        plugins: None,
        warnings: Vec::new(),
        recording: None,
        cache_mode: None,
        checkpoint: None,
        nothing_done: None,
        redone: None,
        uncertain_effects: None,
        error: Some(goodfetch_protocol::run::EndingError {
            code: "limit_reached".to_owned(),
            message: "the Run reached its limit".to_owned(),
            details: serde_json::Map::new(),
        }),
        kind_parts: serde_json::Map::new(),
    };
    recordings
        .apply_to_report(&connection, &mut report)
        .unwrap();
    assert_eq!(
        report.error.as_ref().map(|e| e.code.as_str()),
        Some("limit_reached")
    );
    assert!(report.recording.is_none());
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(
        report.warnings[0].code,
        goodfetch_protocol::checkpoint::PATH_NOT_WRITABLE.name
    );

    // On a clean Run (no prior error), the write failure becomes `report.error`.
    let clean_id = goodfetch_protocol::run::RunId::parse("run-clean-write-fail").unwrap();
    let clean_recorder = super::Recorder::new(
        dir.path().display().to_string(),
        clean_id.clone(),
        "Collie 0.1.0".to_owned(),
        Masks::new(),
        crate::clock::Clock::paused(now),
        10,
        super::RecorderPolicy {
            stage_for_attempt: true,
            write_on_end: true,
        },
    );
    recordings.attach_recorder(clean_id.clone(), clean_recorder);
    report.run_id = clean_id;
    report.lifecycle = goodfetch_protocol::run::Lifecycle::Finished;
    report.error = None;
    report.warnings.clear();
    recordings
        .apply_to_report(&connection, &mut report)
        .unwrap();
    assert_eq!(
        report.error.as_ref().map(|e| e.code.as_str()),
        Some(goodfetch_protocol::checkpoint::PATH_NOT_WRITABLE.name)
    );
    assert_eq!(report.warnings, []);
}
