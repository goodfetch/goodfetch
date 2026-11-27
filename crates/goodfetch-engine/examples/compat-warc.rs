//! The WARC 1.1 profile's writer for the Compatibility corpus (ADR 0066,
//! issue 800): writes `recording.warc` and `recording.warc.gz` (gzipped record
//! by record) through `goodfetch_engine::warc::Writer` at ADR 0059's fixed
//! test `Clock` start, holding every record kind the writer emits:
//! `warcinfo`, `request`, `response` (GET HTML, POST JSON with masked secret
//! and redacted `Set-Cookie`, and a cut response marked
//! `WARC-Truncated: disconnect`), and `revisit` (304 after revalidation).
//! `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::warc::{
    RequestRecord, ResponseRecord, RevisitRecord, TRUNCATED_DISCONNECT, WRITER_VERSION, WarcInfo,
    Writer, sha256_digest,
};

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";
const LATER: &str = "2026-01-05T09:01:00Z";
const GET_URL: &str = "https://shop.test/book";
const GET_RESPONSE_ID: &str = "urn:uuid:00000000-0000-4000-8000-000000000003";
const GET_BODY: &[u8] = b"<!doctype html><h1>Book</h1>";

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    std::fs::create_dir_all(&output).expect("invariant: output folder is created");
    std::fs::write(
        output.join("recording.warc"),
        build("recording.warc", false),
    )
    .expect("invariant: recording.warc writes");
    std::fs::write(
        output.join("recording.warc.gz"),
        build("recording.warc.gz", true),
    )
    .expect("invariant: recording.warc.gz writes");
}

fn build(filename: &str, gzip_per_record: bool) -> Vec<u8> {
    let start: jiff::Timestamp = START.parse().expect("invariant: valid timestamp");
    let later: jiff::Timestamp = LATER.parse().expect("invariant: valid timestamp");
    let mut writer = Writer::new(Vec::new(), gzip_per_record);
    writer
        .write_warcinfo(&WarcInfo {
            record_id: "urn:uuid:00000000-0000-4000-8000-000000000001".to_owned(),
            date: start,
            filename: Some(filename.to_owned()),
            software: "Collie 0.0.0".to_owned(),
            run_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned()),
            mode: Some("record".to_owned()),
            recorded_from: Some(start),
            recorded_to: Some(later),
            writer_version: WRITER_VERSION.to_owned(),
        })
        .expect("invariant: writing to Vec succeeds");
    write_get(&mut writer, start);
    write_post(&mut writer, start);
    write_cut(&mut writer, later);
    write_revisit(&mut writer, start, later);
    writer.into_inner()
}

fn write_get(writer: &mut Writer<Vec<u8>>, date: jiff::Timestamp) {
    let request_id = "urn:uuid:00000000-0000-4000-8000-000000000002";
    writer
        .write_request(&RequestRecord {
            record_id: request_id,
            concurrent_to: Some(GET_RESPONSE_ID),
            target_uri: GET_URL,
            date,
            method: "GET",
            headers: &[("accept".to_owned(), "text/html".to_owned())],
            body: b"",
        })
        .expect("invariant: writing to Vec succeeds");
    writer
        .write_response(&ResponseRecord {
            record_id: GET_RESPONSE_ID,
            concurrent_to: Some(request_id),
            target_uri: GET_URL,
            date,
            status: 200,
            headers: &[
                (
                    "content-type".to_owned(),
                    "text/html; charset=utf-8".to_owned(),
                ),
                ("etag".to_owned(), "\"book-v1\"".to_owned()),
                (
                    "set-cookie".to_owned(),
                    "sid=[redacted]; Path=/; HttpOnly".to_owned(),
                ),
            ],
            body: GET_BODY,
            truncated: None,
        })
        .expect("invariant: writing to Vec succeeds");
}

fn write_post(writer: &mut Writer<Vec<u8>>, date: jiff::Timestamp) {
    let url = "https://shop.test/api/order";
    let request_id = "urn:uuid:00000000-0000-4000-8000-000000000004";
    let response_id = "urn:uuid:00000000-0000-4000-8000-000000000005";
    writer
        .write_request(&RequestRecord {
            record_id: request_id,
            concurrent_to: Some(response_id),
            target_uri: url,
            date,
            method: "POST",
            headers: &[("content-type".to_owned(), "application/json".to_owned())],
            body: br#"{"token":"%api_key%"}"#,
        })
        .expect("invariant: writing to Vec succeeds");
    writer
        .write_response(&ResponseRecord {
            record_id: response_id,
            concurrent_to: Some(request_id),
            target_uri: url,
            date,
            status: 201,
            headers: &[("content-type".to_owned(), "application/json".to_owned())],
            body: br#"{"ok":true}"#,
            truncated: None,
        })
        .expect("invariant: writing to Vec succeeds");
}

fn write_cut(writer: &mut Writer<Vec<u8>>, date: jiff::Timestamp) {
    let url = "https://shop.test/cut";
    let request_id = "urn:uuid:00000000-0000-4000-8000-000000000006";
    let response_id = "urn:uuid:00000000-0000-4000-8000-000000000007";
    writer
        .write_request(&RequestRecord {
            record_id: request_id,
            concurrent_to: Some(response_id),
            target_uri: url,
            date,
            method: "GET",
            headers: &[],
            body: b"",
        })
        .expect("invariant: writing to Vec succeeds");
    writer
        .write_response(&ResponseRecord {
            record_id: response_id,
            concurrent_to: Some(request_id),
            target_uri: url,
            date,
            status: 200,
            headers: &[("content-length".to_owned(), "100".to_owned())],
            body: b"partial",
            truncated: Some(TRUNCATED_DISCONNECT),
        })
        .expect("invariant: writing to Vec succeeds");
}

fn write_revisit(writer: &mut Writer<Vec<u8>>, start: jiff::Timestamp, later: jiff::Timestamp) {
    let request_id = "urn:uuid:00000000-0000-4000-8000-000000000008";
    let response_id = "urn:uuid:00000000-0000-4000-8000-000000000009";
    let payload_digest = sha256_digest(GET_BODY);
    writer
        .write_request(&RequestRecord {
            record_id: request_id,
            concurrent_to: Some(response_id),
            target_uri: GET_URL,
            date: later,
            method: "GET",
            headers: &[("if-none-match".to_owned(), "\"book-v1\"".to_owned())],
            body: b"",
        })
        .expect("invariant: writing to Vec succeeds");
    writer
        .write_revisit(&RevisitRecord {
            record_id: response_id,
            concurrent_to: Some(request_id),
            target_uri: GET_URL,
            date: later,
            refers_to: GET_RESPONSE_ID,
            refers_to_target_uri: Some(GET_URL),
            refers_to_date: Some(start),
            payload_digest: &payload_digest,
            status: 304,
            headers: &[("etag".to_owned(), "\"book-v1\"".to_owned())],
        })
        .expect("invariant: writing to Vec succeeds");
}
