//! WARC 1.1 writer (ISO 28500:2017; ADRs 0105 and 0118): the one writer
//! for Response recordings and Foxhound's WARC output, emitting `warcinfo`,
//! `request`, `response` and `revisit` records with `sha256:` block and
//! payload digests, `WARC-Truncated: disconnect` on cut bodies, and one
//! deterministic gzip member per record when writing `.warc.gz`.

use std::borrow::Cow;
use std::io::{self, Write as _};

use flate2::{Compression, GzBuilder};
use sha2::{Digest as _, Sha256};

/// The version of Goodfetch's WARC writer recorded in `warcinfo` (issue 800).
pub const WRITER_VERSION: &str = "0.1";

/// The `WARC-Profile` URI for a 304 `revisit` after revalidation
/// (ISO 28500:2017 §5.8).
pub const REVISIT_SERVER_NOT_MODIFIED: &str =
    "http://netpreserve.org/warc/1.1/revisit/server-not-modified";

/// The `WARC-Truncated` value for a response whose body broke off (ADR 0118).
pub const TRUNCATED_DISCONNECT: &str = "disconnect";

/// RFC 1952 "unknown OS" byte for deterministic gzip headers across platforms.
const GZIP_OS_UNKNOWN: u8 = 255;

/// `WARC-Block-Digest` / `WARC-Payload-Digest` in WARC 1.1's `sha256:<base32>`
/// form.
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        data_encoding::BASE32.encode(&Sha256::digest(bytes))
    )
}

/// The `warcinfo` record that opens a WARC file written by Goodfetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarcInfo {
    /// Its `WARC-Record-ID`, such as `urn:uuid:...` (angle brackets added
    /// when absent).
    pub record_id: String,
    /// When the file was written (`WARC-Date`).
    pub date: jiff::Timestamp,
    /// The file's own name (`WARC-Filename`), if any.
    pub filename: Option<String>,
    /// The engine's Product and version (`software` field).
    pub software: String,
    /// The Run's ID (`run-id` field), when written for a Run.
    pub run_id: Option<String>,
    /// The recording mode (`mode` field), such as `"record"`.
    pub mode: Option<String>,
    /// When the oldest recorded response was fetched (`recorded-from` field).
    pub recorded_from: Option<jiff::Timestamp>,
    /// When the newest recorded response was fetched (`recorded-to` field).
    pub recorded_to: Option<jiff::Timestamp>,
    /// The WARC writer's version (`warc-writer-version` field).
    pub writer_version: String,
}

/// A `request` record to write.
#[derive(Clone, Copy, Debug)]
pub struct RequestRecord<'a> {
    /// Its `WARC-Record-ID`.
    pub record_id: &'a str,
    /// The `WARC-Record-ID` of the response or revisit that answered it.
    pub concurrent_to: Option<&'a str>,
    /// Its `WARC-Target-URI`.
    pub target_uri: &'a str,
    /// When the request was sent (`WARC-Date`).
    pub date: jiff::Timestamp,
    /// Its HTTP method.
    pub method: &'a str,
    /// Its HTTP headers, already redacted and masked.
    pub headers: &'a [(String, String)],
    /// Its HTTP body, already masked.
    pub body: &'a [u8],
}

/// A `response` record to write.
#[derive(Clone, Copy, Debug)]
pub struct ResponseRecord<'a> {
    /// Its `WARC-Record-ID`.
    pub record_id: &'a str,
    /// The `WARC-Record-ID` of the request it answered.
    pub concurrent_to: Option<&'a str>,
    /// Its `WARC-Target-URI`.
    pub target_uri: &'a str,
    /// When the response was received (`WARC-Date`).
    pub date: jiff::Timestamp,
    /// Its HTTP status code.
    pub status: u16,
    /// Its HTTP headers, already redacted and masked.
    pub headers: &'a [(String, String)],
    /// Its HTTP body, already masked.
    pub body: &'a [u8],
    /// `WARC-Truncated` reason (such as [`TRUNCATED_DISCONNECT`]) when the
    /// body broke off.
    pub truncated: Option<&'static str>,
}

/// A `revisit` record for a 304 response after revalidation.
#[derive(Clone, Copy, Debug)]
pub struct RevisitRecord<'a> {
    /// Its `WARC-Record-ID`.
    pub record_id: &'a str,
    /// The `WARC-Record-ID` of the conditional request it answered.
    pub concurrent_to: Option<&'a str>,
    /// Its `WARC-Target-URI`.
    pub target_uri: &'a str,
    /// When the 304 response was received (`WARC-Date`).
    pub date: jiff::Timestamp,
    /// The `WARC-Record-ID` of the original response whose payload it reuses.
    pub refers_to: &'a str,
    /// The original response's `WARC-Target-URI`, if known.
    pub refers_to_target_uri: Option<&'a str>,
    /// The original response's `WARC-Date`, if known.
    pub refers_to_date: Option<jiff::Timestamp>,
    /// The original payload's `WARC-Payload-Digest` (`sha256:...`).
    pub payload_digest: &'a str,
    /// Its HTTP status code (typically `304`).
    pub status: u16,
    /// Its HTTP response headers, already redacted and masked.
    pub headers: &'a [(String, String)],
}

/// A WARC 1.1 writer: writes records sequentially into `W`, optionally
/// compressing each record as its own gzip member (RFC 1952 / ISO 28500).
pub struct Writer<W: io::Write> {
    inner: W,
    gzip_per_record: bool,
}

impl<W: io::Write> Writer<W> {
    /// A new writer over `inner`; when `gzip_per_record` is true, each record
    /// is written as its own standalone gzip member.
    pub fn new(inner: W, gzip_per_record: bool) -> Self {
        Self {
            inner,
            gzip_per_record,
        }
    }

    /// Consumes the writer and returns the underlying sink.
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Writes a `warcinfo` record.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from the underlying writer.
    pub fn write_warcinfo(&mut self, info: &WarcInfo) -> io::Result<()> {
        let mut fields = String::new();
        push_field(&mut fields, "software", &info.software);
        if let Some(run_id) = &info.run_id {
            push_field(&mut fields, "run-id", run_id);
        }
        if let Some(mode) = &info.mode {
            push_field(&mut fields, "mode", mode);
        }
        if let Some(from) = info.recorded_from {
            push_field(&mut fields, "recorded-from", &warc_date(from));
        }
        if let Some(to) = info.recorded_to {
            push_field(&mut fields, "recorded-to", &warc_date(to));
        }
        push_field(&mut fields, "warc-writer-version", &info.writer_version);
        let block = fields.as_bytes();
        let record_id = bracketed(&info.record_id);
        let date = warc_date(info.date);
        let block_digest = sha256_digest(block);
        let mut headers: Vec<(&str, Cow<'_, str>)> = vec![
            ("WARC-Type", Cow::Borrowed("warcinfo")),
            ("WARC-Record-ID", record_id),
            ("WARC-Date", Cow::Owned(date)),
            ("WARC-Block-Digest", Cow::Owned(block_digest)),
            ("Content-Type", Cow::Borrowed("application/warc-fields")),
        ];
        if let Some(filename) = &info.filename {
            headers.push(("WARC-Filename", Cow::Owned(sanitize_value(filename))));
        }
        self.write_record(&headers, block)
    }

    /// Writes a `request` record holding an HTTP/1.1 request block.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from the underlying writer.
    pub fn write_request(&mut self, request: &RequestRecord<'_>) -> io::Result<()> {
        let block = http_request_block(
            request.method,
            request.target_uri,
            request.headers,
            request.body,
        );
        let record_id = bracketed(request.record_id);
        let date = warc_date(request.date);
        let block_digest = sha256_digest(&block);
        let payload_digest = sha256_digest(request.body);
        let mut headers: Vec<(&str, Cow<'_, str>)> = vec![
            ("WARC-Type", Cow::Borrowed("request")),
            ("WARC-Record-ID", record_id),
            ("WARC-Date", Cow::Owned(date)),
            (
                "WARC-Target-URI",
                Cow::Owned(sanitize_value(request.target_uri)),
            ),
            ("WARC-Block-Digest", Cow::Owned(block_digest)),
            ("WARC-Payload-Digest", Cow::Owned(payload_digest)),
            (
                "Content-Type",
                Cow::Borrowed("application/http; msgtype=request"),
            ),
        ];
        if let Some(concurrent_to) = request.concurrent_to {
            headers.push(("WARC-Concurrent-To", bracketed(concurrent_to)));
        }
        self.write_record(&headers, &block)
    }

    /// Writes a `response` record holding an HTTP/1.1 response block.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from the underlying writer.
    pub fn write_response(&mut self, response: &ResponseRecord<'_>) -> io::Result<()> {
        let block = http_response_block(
            response.status,
            response.headers,
            response.body,
            response.truncated.is_some(),
        );
        let record_id = bracketed(response.record_id);
        let date = warc_date(response.date);
        let block_digest = sha256_digest(&block);
        let payload_digest = sha256_digest(response.body);
        let mut headers: Vec<(&str, Cow<'_, str>)> = vec![
            ("WARC-Type", Cow::Borrowed("response")),
            ("WARC-Record-ID", record_id),
            ("WARC-Date", Cow::Owned(date)),
            (
                "WARC-Target-URI",
                Cow::Owned(sanitize_value(response.target_uri)),
            ),
            ("WARC-Block-Digest", Cow::Owned(block_digest)),
            ("WARC-Payload-Digest", Cow::Owned(payload_digest)),
            (
                "Content-Type",
                Cow::Borrowed("application/http; msgtype=response"),
            ),
        ];
        if let Some(concurrent_to) = response.concurrent_to {
            headers.push(("WARC-Concurrent-To", bracketed(concurrent_to)));
        }
        if let Some(truncated) = response.truncated {
            headers.push(("WARC-Truncated", Cow::Borrowed(truncated)));
        }
        self.write_record(&headers, &block)
    }

    /// Writes a `revisit` record for a 304 response after revalidation.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from the underlying writer.
    pub fn write_revisit(&mut self, revisit: &RevisitRecord<'_>) -> io::Result<()> {
        let block = http_response_block(revisit.status, revisit.headers, &[], false);
        let record_id = bracketed(revisit.record_id);
        let date = warc_date(revisit.date);
        let block_digest = sha256_digest(&block);
        let mut headers: Vec<(&str, Cow<'_, str>)> = vec![
            ("WARC-Type", Cow::Borrowed("revisit")),
            ("WARC-Record-ID", record_id),
            ("WARC-Date", Cow::Owned(date)),
            (
                "WARC-Target-URI",
                Cow::Owned(sanitize_value(revisit.target_uri)),
            ),
            ("WARC-Profile", Cow::Borrowed(REVISIT_SERVER_NOT_MODIFIED)),
            ("WARC-Refers-To", bracketed(revisit.refers_to)),
            ("WARC-Block-Digest", Cow::Owned(block_digest)),
            (
                "WARC-Payload-Digest",
                Cow::Owned(sanitize_value(revisit.payload_digest)),
            ),
            (
                "Content-Type",
                Cow::Borrowed("application/http; msgtype=response"),
            ),
        ];
        if let Some(target_uri) = revisit.refers_to_target_uri {
            headers.push((
                "WARC-Refers-To-Target-URI",
                Cow::Owned(sanitize_value(target_uri)),
            ));
        }
        if let Some(refers_date) = revisit.refers_to_date {
            headers.push(("WARC-Refers-To-Date", Cow::Owned(warc_date(refers_date))));
        }
        if let Some(concurrent_to) = revisit.concurrent_to {
            headers.push(("WARC-Concurrent-To", bracketed(concurrent_to)));
        }
        self.write_record(&headers, &block)
    }

    fn write_record(&mut self, headers: &[(&str, Cow<'_, str>)], block: &[u8]) -> io::Result<()> {
        if self.gzip_per_record {
            let mut encoder = GzBuilder::new()
                .mtime(0)
                .operating_system(GZIP_OS_UNKNOWN)
                .write(&mut self.inner, Compression::default());
            write_plain_record(&mut encoder, headers, block)?;
            encoder.finish()?;
            Ok(())
        } else {
            write_plain_record(&mut self.inner, headers, block)
        }
    }
}

fn write_plain_record(
    sink: &mut impl io::Write,
    headers: &[(&str, Cow<'_, str>)],
    block: &[u8],
) -> io::Result<()> {
    sink.write_all(b"WARC/1.1\r\n")?;
    for (name, value) in headers {
        write!(sink, "{name}: {value}\r\n")?;
    }
    write!(sink, "Content-Length: {}\r\n\r\n", block.len())?;
    sink.write_all(block)?;
    sink.write_all(b"\r\n\r\n")
}

/// Formats `timestamp` as UTC RFC 3339 seconds (`YYYY-MM-DDTHH:MM:SSZ`) for
/// `WARC-Date` (ISO 28500:2017).
pub fn warc_date(timestamp: jiff::Timestamp) -> String {
    jiff::Timestamp::from_second(timestamp.as_second())
        .unwrap_or(timestamp)
        .to_string()
}

fn bracketed(id: &str) -> Cow<'_, str> {
    let clean = sanitize_value(id);
    if clean.starts_with('<') && clean.ends_with('>') {
        Cow::Owned(clean)
    } else {
        Cow::Owned(format!("<{clean}>"))
    }
}

fn push_field(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(": ");
    out.push_str(&sanitize_value(value));
    out.push_str("\r\n");
}

fn sanitize_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| if matches!(ch, '\r' | '\n') { ' ' } else { ch })
        .collect()
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(super::is_token)
}

fn http_request_block(
    method: &str,
    target_uri: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Vec<u8> {
    let parsed = url::Url::parse(target_uri).ok();
    let target = parsed.as_ref().map_or_else(
        || "/".to_owned(),
        |url| {
            let path = if url.path().is_empty() {
                "/"
            } else {
                url.path()
            };
            match url.query() {
                Some(query) => format!("{path}?{query}"),
                None => path.to_owned(),
            }
        },
    );
    let method = sanitize_value(method);
    let mut block = Vec::with_capacity(128 + body.len());
    let _ = write!(block, "{method} {target} HTTP/1.1\r\n");
    let has_host = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"));
    if !has_host
        && let Some(url) = &parsed
        && let Some(host) = url.host_str()
    {
        match url.port() {
            Some(port) => {
                let _ = write!(block, "Host: {host}:{port}\r\n");
            }
            None => {
                let _ = write!(block, "Host: {host}\r\n");
            }
        }
    }
    for (name, value) in headers {
        if !valid_header_name(name)
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        let _ = write!(block, "{name}: {}\r\n", sanitize_value(value));
    }
    if !body.is_empty() {
        let _ = write!(block, "Content-Length: {}\r\n", body.len());
    }
    block.extend_from_slice(b"\r\n");
    block.extend_from_slice(body);
    block
}

fn http_response_block(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    truncated: bool,
) -> Vec<u8> {
    let reason = http::StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("");
    let mut block = Vec::with_capacity(128 + body.len());
    let _ = write!(block, "HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        if !valid_header_name(name) || name.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        if !truncated && name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        let _ = write!(block, "{name}: {}\r\n", sanitize_value(value));
    }
    if !truncated && !matches!(status, 100..=199 | 204 | 304) {
        let _ = write!(block, "Content-Length: {}\r\n", body.len());
    }
    block.extend_from_slice(b"\r\n");
    block.extend_from_slice(body);
    block
}
