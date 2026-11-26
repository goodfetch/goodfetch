use std::io::Write as _;

use bytes::Bytes;
use proptest::prelude::*;
use sha2::{Digest as _, Sha256};

use super::{HttpRequest, HttpResponse, NotHttp, RecordType, Unreadable, read};

/// A WARC/1.1 record with `headers` and `block`, lines ended with CRLF, as
/// a writer writes it.
fn record(headers: &[(&str, &str)], block: &[u8]) -> Vec<u8> {
    let mut bytes = b"WARC/1.1\r\n".to_vec();
    for (name, value) in headers {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(format!("Content-Length: {}\r\n\r\n", block.len()).as_bytes());
    bytes.extend_from_slice(block);
    bytes.extend_from_slice(b"\r\n\r\n");
    bytes
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).expect("writing to a Vec");
    encoder.finish().expect("writing to a Vec")
}

fn sha256_base32(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        data_encoding::BASE32.encode(&Sha256::digest(bytes))
    )
}

const RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello";

fn file() -> Vec<u8> {
    let mut file = record(
        &[
            ("WARC-Type", "warcinfo"),
            ("WARC-Record-ID", "<urn:uuid:1>"),
            ("Content-Type", "application/warc-fields"),
        ],
        b"software: Goodfetch\r\n",
    );
    file.extend(record(
        &[
            ("WARC-Type", "response"),
            ("WARC-Record-ID", "<urn:uuid:2>"),
            ("WARC-Target-URI", "https://example.test/a"),
            ("WARC-Date", "2026-11-20T10:00:02Z"),
            ("Content-Type", "application/http; msgtype=response"),
            ("WARC-Block-Digest", &sha256_base32(RESPONSE)),
        ],
        RESPONSE,
    ));
    file
}

#[test]
fn reads_each_record_of_a_file_in_order_with_its_headers_and_block() {
    let records = read(&Bytes::from(file())).expect("a WARC file");

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].record_type(), RecordType::Warcinfo);
    assert_eq!(records[0].block().as_ref(), b"software: Goodfetch\r\n");
    let response = &records[1];
    assert_eq!(response.record_type(), RecordType::Response);
    assert_eq!(response.record_id(), Some("<urn:uuid:2>"));
    assert_eq!(response.target_uri(), Some("https://example.test/a"));
    assert_eq!(
        response.date(),
        Some("2026-11-20T10:00:02Z".parse().expect("a timestamp"))
    );
    assert!(response.holds_http());
    assert_eq!(response.header("warc-type"), Some("response"));
    assert_eq!(response.block().as_ref(), RESPONSE);
}

#[test]
fn reads_a_warc_1_0_file_with_bare_line_feeds_as_wget_writes_it() {
    let contents = Bytes::from_static(include_bytes!("fixtures/wget-1.0.warc"));

    let records = read(&contents).expect("a WARC/1.0 file");

    let kinds: Vec<RecordType> = records.iter().map(super::Record::record_type).collect();
    assert_eq!(
        kinds,
        [
            RecordType::Warcinfo,
            RecordType::Request,
            RecordType::Response
        ]
    );
    assert_eq!(
        records[2].target_uri(),
        Some("http://example.test/page?a=1")
    );
    let request = HttpRequest::parse(records[1].block()).expect("an HTTP request");
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("GET", "/page?a=1")
    );
    let response = HttpResponse::parse(records[2].block()).expect("an HTTP response");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.as_ref(), b"<p>hello</p>\n");
    assert!(!response.cut);
}

#[test]
fn reads_a_file_gzipped_record_by_record_as_its_plain_records() {
    let plain = file();
    let mut members = Vec::new();
    for part in [&plain[..], &record(&[("WARC-Type", "metadata")], b"x")[..]] {
        members.extend(gzip(part));
    }
    let mut both = plain.clone();
    both.extend(record(&[("WARC-Type", "metadata")], b"x"));

    assert_eq!(
        read(&Bytes::from(gzip(&plain))),
        read(&Bytes::from(plain)),
        "one member"
    );
    let records = read(&Bytes::from(members)).expect("two gzip members");
    assert_eq!(Ok(records), read(&Bytes::from(both)), "two members");
}

#[test]
fn reads_an_empty_file_as_no_record() {
    assert_eq!(read(&Bytes::new()), Ok(Vec::new()));
}

#[test]
fn refuses_a_file_that_is_not_warc() {
    let contents = Bytes::from_static(include_bytes!("fixtures/not-warc.txt"));

    assert_eq!(read(&contents), Err(Unreadable::NotWarc));
}

#[test]
fn refuses_a_version_it_does_not_read() {
    let contents = Bytes::from_static(include_bytes!("fixtures/warc-2.0.warc"));

    assert_eq!(read(&contents), Err(Unreadable::Version("2.0".to_owned())));
}

#[test]
fn refuses_a_record_whose_block_runs_past_the_end() {
    let mut contents = record(&[("WARC-Type", "resource")], b"0123456789");
    contents.truncate(contents.len() - 8);

    assert!(matches!(
        read(&Bytes::from(contents)),
        Err(Unreadable::Malformed { offset: 0, .. })
    ));
}

#[test]
fn refuses_a_record_without_a_content_length() {
    let contents = b"WARC/1.1\r\nWARC-Type: resource\r\n\r\n".to_vec();

    assert!(matches!(
        read(&Bytes::from(contents)),
        Err(Unreadable::Malformed { .. })
    ));
}

#[test]
fn refuses_a_second_record_that_is_not_one_at_its_offset() {
    let mut contents = record(&[("WARC-Type", "resource")], b"x");
    let offset = contents.len();
    contents.extend_from_slice(b"garbage\r\n");

    assert!(matches!(
        read(&Bytes::from(contents)),
        Err(Unreadable::Malformed { offset: at, .. }) if at == offset
    ));
}

#[test]
fn folds_a_continuation_line_into_the_header_before_it() {
    let contents = b"WARC/1.1\r\nWARC-Type: resource\r\nWARC-Filename: a\r\n  b\r\nContent-Length: 0\r\n\r\n\r\n\r\n";

    let records = read(&Bytes::from_static(contents)).expect("a WARC file");

    assert_eq!(records[0].header("WARC-Filename"), Some("a b"));
}

#[test]
fn checks_a_sha256_block_digest_in_base32_or_hex() {
    let hex = format!(
        "sha256:{}",
        data_encoding::HEXLOWER.encode(&Sha256::digest(b"x"))
    );
    for digest in [sha256_base32(b"x"), hex] {
        let contents = record(&[("WARC-Block-Digest", &digest)], b"x");

        assert!(read(&Bytes::from(contents)).is_ok(), "{digest}");
    }
}

#[test]
fn refuses_a_block_its_sha256_digest_does_not_match_and_skips_other_algorithms() {
    let wrong = record(
        &[
            ("WARC-Record-ID", "<urn:uuid:7>"),
            ("WARC-Block-Digest", &sha256_base32(b"y")),
        ],
        b"x",
    );
    let sha1 = record(&[("WARC-Block-Digest", "sha1:AAAA")], b"x");

    assert_eq!(
        read(&Bytes::from(wrong)),
        Err(Unreadable::Digest {
            record: "<urn:uuid:7>".to_owned()
        })
    );
    assert!(read(&Bytes::from(sha1)).is_ok());
}

#[test]
fn dechunks_a_chunked_body_and_drops_its_transfer_encoding() {
    let block = Bytes::from_static(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nVary: Accept\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n",
    );

    let response = HttpResponse::parse(&block).expect("an HTTP response");

    assert_eq!(response.body.as_ref(), b"hello world");
    assert_eq!(response.headers, [("vary".to_owned(), "Accept".to_owned())]);
    assert!(!response.cut);
}

#[test]
fn says_a_body_that_broke_off_is_cut() {
    let short = Bytes::from_static(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhello");
    let unfinished =
        Bytes::from_static(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n");

    for block in [short, unfinished] {
        let response = HttpResponse::parse(&block).expect("an HTTP response");

        assert!(response.cut);
        assert_eq!(response.body.as_ref(), b"hello");
    }
}

#[test]
fn reads_a_requests_method_target_headers_and_body() {
    let block = Bytes::from_static(
        b"POST /login HTTP/1.1\r\nHost: example.test\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 6\r\n\r\nuser=a",
    );

    let request = HttpRequest::parse(&block).expect("an HTTP request");

    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/login");
    assert_eq!(
        request.headers[0],
        ("host".to_owned(), "example.test".to_owned())
    );
    assert_eq!(request.body.as_ref(), b"user=a");
}

#[test]
fn refuses_a_block_that_is_not_an_http_message() {
    let cut = Bytes::from_static(b"HTTP/1.1 200 OK\r\nContent-");
    let not_http = Bytes::from_static(b"\x00\x01 not HTTP\r\n\r\n");

    assert_eq!(
        HttpResponse::parse(&cut),
        Err(NotHttp {
            reason: "its head is cut"
        })
    );
    assert!(HttpResponse::parse(&not_http).is_err());
    assert!(HttpRequest::parse(&not_http).is_err());
}

proptest! {
    #[test]
    fn reads_back_the_records_written(
        blocks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..64), 1..5),
        filename in "[ -~]{0,24}",
        digest in any::<bool>(),
        gzipped in any::<bool>(),
    ) {
        let mut contents = Vec::new();
        for block in &blocks {
            let mut headers = vec![("WARC-Type", "resource"), ("WARC-Filename", filename.as_str())];
            let sha256 = sha256_base32(block);
            if digest {
                headers.push(("WARC-Block-Digest", &sha256));
            }
            contents.extend(record(&headers, block));
        }
        let contents = if gzipped { gzip(&contents) } else { contents };

        let records = read(&Bytes::from(contents)).expect("the records written");

        let read_blocks: Vec<&[u8]> = records.iter().map(|record| record.block().as_ref()).collect();
        prop_assert_eq!(read_blocks, blocks.iter().map(Vec::as_slice).collect::<Vec<_>>());
        for record in &records {
            prop_assert_eq!(record.header("WARC-Filename"), Some(filename.trim()));
        }
    }

    #[test]
    fn reads_back_a_body_however_it_is_chunked(
        body in proptest::collection::vec(any::<u8>(), 0..256),
        sizes in proptest::collection::vec(1_usize..40, 1..10),
    ) {
        let mut block = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        let mut rest = &body[..];
        for size in sizes.iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (chunk, after) = rest.split_at((*size).min(rest.len()));
            block.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            block.extend_from_slice(chunk);
            block.extend_from_slice(b"\r\n");
            rest = after;
        }
        block.extend_from_slice(b"0\r\n\r\n");

        let response = HttpResponse::parse(&Bytes::from(block)).expect("an HTTP response");

        prop_assert_eq!(response.body.as_ref(), &body[..]);
        prop_assert!(!response.cut);
    }
}

mod warc_reader {
    goodfetch_test_support::fuzz_replay!(crate::warc::fuzz_warc, "fuzz/warc");
}
