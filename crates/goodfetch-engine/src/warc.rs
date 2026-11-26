//! WARC files (ISO 28500; ADRs 0105 and 0118): [`read`], the records of a
//! file's bytes, and [`HttpRequest::parse`] and [`HttpResponse::parse`], the
//! HTTP message a `request`, `response` or `revisit` record's block holds.
//! A Response recording is a WARC file, and Foxhound's WARC output writes
//! the same records.
//!
//! The reader reads WARC 1.1 (ISO 28500:2017) and WARC 1.0 (ISO
//! 28500:2009), whose records have one structure, so a file older tools
//! such as wget write replays too; any other version is refused
//! ([`Unreadable::Version`]). A record is a version line, header lines,
//! each `Name: value` with a line that starts with a space or a tab
//! continuing the one before it, an empty line, a block of exactly
//! `Content-Length` bytes and two line ends; a line ends with CRLF, and a
//! bare LF is read as one too. A file gzipped record by record, as a
//! `.warc.gz` is, is read through all its members, whatever its name. A
//! `WARC-Block-Digest` in `sha256:`, in base32 or hex, is checked against
//! its block; another algorithm, such as the `sha1:` older tools write, is
//! not checked.
//!
//! An HTTP block's head is parsed by `httparse`; a chunked body is
//! de-chunked and its `Transfer-Encoding` dropped, and a content coding is
//! kept, as a response arrives. A body shorter than its `Content-Length`,
//! or a chunked one without its last chunk, is cut ([`HttpResponse::cut`]).

use std::io::Read as _;

use bytes::Bytes;
use flate2::read::MultiGzDecoder;
use sha2::{Digest as _, Sha256};

#[cfg(test)]
mod tests;

/// The two bytes a gzip member starts with (RFC 1952).
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// The most header lines an HTTP block's head may have.
const MAX_HTTP_HEADERS: usize = 256;

/// The most bytes of an unread version an error names.
const VERSION_SHOWN: usize = 16;

/// What a record is, its `WARC-Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordType {
    /// `warcinfo`: the file's own description, such as the software that
    /// wrote it.
    Warcinfo,
    /// `request`: the HTTP request sent.
    Request,
    /// `response`: the HTTP response received.
    Response,
    /// `revisit`: a response whose payload another record holds, such as a
    /// `304` that revalidated it.
    Revisit,
    /// Any other type, such as `resource` or `metadata`.
    Other(String),
}

/// One record of a WARC file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    headers: Vec<(String, String)>,
    block: Bytes,
}

impl Record {
    /// The value of its first header named `name`, in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Its headers, in the order written.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// What it is; `Other("")` when it names no `WARC-Type`.
    pub fn record_type(&self) -> RecordType {
        let kind = self.header("WARC-Type").unwrap_or_default();
        match kind.to_ascii_lowercase().as_str() {
            "warcinfo" => RecordType::Warcinfo,
            "request" => RecordType::Request,
            "response" => RecordType::Response,
            "revisit" => RecordType::Revisit,
            _ => RecordType::Other(kind.to_owned()),
        }
    }

    /// Its `WARC-Record-ID`.
    pub fn record_id(&self) -> Option<&str> {
        self.header("WARC-Record-ID")
    }

    /// Its `WARC-Target-URI`, without the angle brackets WARC 1.0's
    /// examples put around it.
    pub fn target_uri(&self) -> Option<&str> {
        self.header("WARC-Target-URI").map(without_brackets)
    }

    /// When it was captured, its `WARC-Date`; `None` when that is not an
    /// RFC 3339 time.
    pub fn date(&self) -> Option<jiff::Timestamp> {
        self.header("WARC-Date")?.parse().ok()
    }

    /// Whether its block holds an HTTP message: its `Content-Type` is
    /// `application/http`, whatever its parameters.
    pub fn holds_http(&self) -> bool {
        self.header("Content-Type").is_some_and(|content_type| {
            content_type
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/http"))
        })
    }

    /// Its block, the bytes after its header.
    pub fn block(&self) -> &Bytes {
        &self.block
    }
}

/// `value` without one pair of surrounding angle brackets.
fn without_brackets(value: &str) -> &str {
    value
        .strip_prefix('<')
        .and_then(|inner| inner.strip_suffix('>'))
        .unwrap_or(value)
}

/// Why a file's bytes are not WARC records the engine reads.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unreadable {
    /// The file does not start with a WARC version line.
    #[error("the file is not WARC: it does not start with a WARC version line")]
    NotWarc,
    /// A record is in a version the engine does not read.
    #[error(
        "a record is WARC/{0}, a version the engine does not read: it reads WARC/1.1 and WARC/1.0"
    )]
    Version(String),
    /// A record is not one.
    #[error("the record at byte {offset} is malformed: {reason}")]
    Malformed {
        /// Where the record starts, in the file's bytes once gunzipped.
        offset: usize,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A record's block does not match its `sha256:` block digest.
    #[error("the block of record {record} does not match its WARC-Block-Digest")]
    Digest {
        /// The record's `WARC-Record-ID`, or where it starts.
        record: String,
    },
    /// The file is gzipped and its gzip does not decode.
    #[error("the file's gzip does not decode: {0}")]
    Gzip(String),
}

/// The records of a WARC file's `contents`, in the order written, gzipped
/// or not. No bytes are copied unless the file is gzipped: each block is a
/// slice of `contents`, or of the gunzipped bytes.
pub fn read(contents: &Bytes) -> Result<Vec<Record>, Unreadable> {
    let contents = if contents.starts_with(&GZIP_MAGIC) {
        gunzip(contents)?
    } else {
        contents.clone()
    };
    let mut records = Vec::new();
    let mut at = 0;
    loop {
        at += contents[at..]
            .iter()
            .take_while(|&&byte| byte == b'\r' || byte == b'\n')
            .count();
        if at == contents.len() {
            return Ok(records);
        }
        let (record, next) = record(&contents, at, records.is_empty())?;
        records.push(record);
        at = next;
    }
}

/// Every member of a gzipped file, one after the other.
fn gunzip(contents: &[u8]) -> Result<Bytes, Unreadable> {
    let mut gunzipped = Vec::new();
    MultiGzDecoder::new(contents)
        .read_to_end(&mut gunzipped)
        .map_err(|error| Unreadable::Gzip(error.to_string()))?;
    Ok(Bytes::from(gunzipped))
}

/// The record that starts at `start`, the file's first when `first`, and
/// where the bytes after it start.
fn record(contents: &Bytes, start: usize, first: bool) -> Result<(Record, usize), Unreadable> {
    let malformed = |reason| Unreadable::Malformed {
        offset: start,
        reason,
    };
    if !contents[start..].starts_with(b"WARC/") {
        return Err(if first {
            Unreadable::NotWarc
        } else {
            malformed("it does not start with a WARC version line")
        });
    }
    let (version_line, mut at) =
        line(contents, start).ok_or_else(|| malformed("it ends in its version line"))?;
    let version = &version_line[b"WARC/".len()..];
    if !matches!(version, b"1.1" | b"1.0") {
        let shown = &version[..version.len().min(VERSION_SHOWN)];
        return Err(Unreadable::Version(
            String::from_utf8_lossy(shown).into_owned(),
        ));
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let (text, next) = line(contents, at).ok_or_else(|| malformed("it ends in its header"))?;
        at = next;
        match text.first() {
            None => break,
            Some(b' ' | b'\t') => {
                let (_, value) = headers
                    .last_mut()
                    .ok_or_else(|| malformed("a continuation line comes before any header"))?;
                value.push(' ');
                value.push_str(&String::from_utf8_lossy(trim(text)));
            }
            Some(_) => {
                let colon = memchr::memchr(b':', text)
                    .ok_or_else(|| malformed("a header line has no colon"))?;
                let name = &text[..colon];
                if name.is_empty() || !name.iter().all(|&byte| is_token(byte)) {
                    return Err(malformed("a header's name is not a token"));
                }
                headers.push((
                    String::from_utf8_lossy(name).into_owned(),
                    String::from_utf8_lossy(trim(&text[colon + 1..])).into_owned(),
                ));
            }
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .ok_or_else(|| malformed("its Content-Length is missing or not a number"))?;
    let end = at
        .checked_add(length)
        .filter(|&end| end <= contents.len())
        .ok_or_else(|| malformed("its block runs past the end of the file"))?;
    let record = Record {
        headers,
        block: contents.slice(at..end),
    };
    check_digest(&record, start)?;
    Ok((record, end))
}

/// The line that starts at `start`, without its CRLF or LF, and where the
/// next starts; `None` when no line end follows.
fn line(contents: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let end = start + memchr::memchr(b'\n', &contents[start..])?;
    let text = &contents[start..end];
    let text = text.strip_suffix(b"\r").unwrap_or(text);
    Some((text, end + 1))
}

/// `bytes` without its leading and trailing spaces and tabs.
fn trim(bytes: &[u8]) -> &[u8] {
    let blank = |byte: &u8| *byte == b' ' || *byte == b'\t';
    let start = bytes
        .iter()
        .position(|byte| !blank(byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !blank(byte))
        .map_or(start, |end| end + 1);
    &bytes[start..end]
}

/// Whether `byte` may be in a header's name: a `tchar` (RFC 9110 §5.6.2).
fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Fails when `record` names a `sha256:` block digest its block does not
/// have.
fn check_digest(record: &Record, start: usize) -> Result<(), Unreadable> {
    let Some((algorithm, value)) = record
        .header("WARC-Block-Digest")
        .and_then(|digest| digest.split_once(':'))
    else {
        return Ok(());
    };
    if !algorithm.trim().eq_ignore_ascii_case("sha256") {
        return Ok(());
    }
    let value = value.trim();
    let expected = if value.len() == 64 {
        data_encoding::HEXLOWER_PERMISSIVE
            .decode(value.as_bytes())
            .ok()
    } else {
        data_encoding::BASE32_NOPAD
            .decode(value.trim_end_matches('=').to_ascii_uppercase().as_bytes())
            .ok()
    };
    if expected.as_deref() == Some(Sha256::digest(&record.block).as_slice()) {
        return Ok(());
    }
    Err(Unreadable::Digest {
        record: record
            .record_id()
            .map_or_else(|| format!("at byte {start}"), str::to_owned),
    })
}

/// Why a record's block is not the HTTP message it should hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("its block is not an HTTP message: {reason}")]
pub struct NotHttp {
    /// What is wrong with it.
    pub reason: &'static str,
}

/// The HTTP request a `request` record's block holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    /// Its method.
    pub method: String,
    /// Its request target, as sent: a path and query, most often.
    pub target: String,
    /// Its headers, names in lower case, in the order sent.
    pub headers: Vec<(String, String)>,
    /// Its body, de-chunked; empty for none.
    pub body: Bytes,
}

impl HttpRequest {
    /// The request `block` holds.
    pub fn parse(block: &Bytes) -> Result<Self, NotHttp> {
        let mut slots = [httparse::EMPTY_HEADER; MAX_HTTP_HEADERS];
        let mut request = httparse::Request::new(&mut slots);
        let head = match request.parse(block) {
            Ok(httparse::Status::Complete(head)) => head,
            Ok(httparse::Status::Partial) => {
                return Err(NotHttp {
                    reason: "its head is cut",
                });
            }
            Err(_) => {
                return Err(NotHttp {
                    reason: "its head is not an HTTP request's",
                });
            }
        };
        let (Some(method), Some(target)) = (request.method, request.path) else {
            return Err(NotHttp {
                reason: "its head is not an HTTP request's",
            });
        };
        let message = Message::of(request.headers, block.slice(head..))?;
        Ok(Self {
            method: method.to_owned(),
            target: target.to_owned(),
            headers: message.headers,
            body: message.body,
        })
    }
}

/// The HTTP response a `response` or `revisit` record's block holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    /// Its status.
    pub status: u16,
    /// Its headers, names in lower case, in the order received, without
    /// `Transfer-Encoding` once de-chunked.
    pub headers: Vec<(String, String)>,
    /// Its body, de-chunked, its content coding kept; empty for none.
    pub body: Bytes,
    /// Whether its body broke off: shorter than its `Content-Length`, or
    /// chunked without its last chunk.
    pub cut: bool,
}

impl HttpResponse {
    /// The response `block` holds.
    pub fn parse(block: &Bytes) -> Result<Self, NotHttp> {
        let mut slots = [httparse::EMPTY_HEADER; MAX_HTTP_HEADERS];
        let mut response = httparse::Response::new(&mut slots);
        let head = match response.parse(block) {
            Ok(httparse::Status::Complete(head)) => head,
            Ok(httparse::Status::Partial) => {
                return Err(NotHttp {
                    reason: "its head is cut",
                });
            }
            Err(_) => {
                return Err(NotHttp {
                    reason: "its head is not an HTTP response's",
                });
            }
        };
        let Some(status) = response.code else {
            return Err(NotHttp {
                reason: "its head is not an HTTP response's",
            });
        };
        let message = Message::of(response.headers, block.slice(head..))?;
        Ok(Self {
            status,
            headers: message.headers,
            body: message.body,
            cut: message.cut,
        })
    }
}

/// What follows an HTTP message's start line.
struct Message {
    /// Its headers, names in lower case, without `Transfer-Encoding` once
    /// de-chunked.
    headers: Vec<(String, String)>,
    /// Its body, de-chunked.
    body: Bytes,
    /// Whether that body broke off.
    cut: bool,
}

impl Message {
    /// The message of the `parsed` headers and the bytes after them.
    fn of(parsed: &[httparse::Header<'_>], body: Bytes) -> Result<Self, NotHttp> {
        let mut headers: Vec<(String, String)> = parsed
            .iter()
            .map(|header| {
                (
                    header.name.to_ascii_lowercase(),
                    String::from_utf8_lossy(header.value).into_owned(),
                )
            })
            .collect();
        let chunked = headers
            .iter()
            .filter(|(name, _)| name == "transfer-encoding")
            .flat_map(|(_, value)| value.split(','))
            .last()
            .is_some_and(|coding| coding.trim().eq_ignore_ascii_case("chunked"));
        if chunked {
            headers.retain(|(name, _)| name != "transfer-encoding");
            let (body, cut) = dechunk(&body)?;
            return Ok(Self { headers, body, cut });
        }
        let cut = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .is_some_and(|length| body.len() < length);
        Ok(Self { headers, body, cut })
    }
}

/// A chunked body's data, and whether it broke off before its last chunk.
fn dechunk(body: &[u8]) -> Result<(Bytes, bool), NotHttp> {
    let mut data = Vec::with_capacity(body.len());
    let mut at = 0;
    loop {
        if at == body.len() {
            return Ok((Bytes::from(data), true));
        }
        let (used, size) = match httparse::parse_chunk_size(&body[at..]) {
            Ok(httparse::Status::Complete(chunk)) => chunk,
            Ok(httparse::Status::Partial) => return Ok((Bytes::from(data), true)),
            Err(_) => {
                return Err(NotHttp {
                    reason: "its chunked body has a malformed chunk size",
                });
            }
        };
        at += used;
        if size == 0 {
            return Ok((Bytes::from(data), false));
        }
        let available = body.len() - at;
        let size = usize::try_from(size).unwrap_or(usize::MAX);
        if size > available {
            data.extend_from_slice(&body[at..]);
            return Ok((Bytes::from(data), true));
        }
        data.extend_from_slice(&body[at..at + size]);
        at += size;
        let rest = &body[at..];
        at += if rest.starts_with(b"\r\n") {
            2
        } else if rest.starts_with(b"\n") {
            1
        } else if rest.is_empty() {
            0
        } else {
            return Err(NotHttp {
                reason: "a chunk of its chunked body has no line end after its data",
            });
        };
    }
}

/// The body of the fuzz target `warc` (ADR 0067): `data` read as a WARC
/// file, each HTTP block parsed as its record's type says. Reading and
/// parsing never panic, reading is deterministic, and each record's block
/// is as long as its `Content-Length` says.
pub fn fuzz_warc(data: &[u8]) {
    let contents = Bytes::copy_from_slice(data);
    let records = read(&contents);
    assert_eq!(records, read(&contents), "reading is not deterministic");
    for record in records.iter().flatten() {
        let length = record
            .header("Content-Length")
            .and_then(|value| value.parse::<usize>().ok());
        assert_eq!(
            length,
            Some(record.block().len()),
            "a block is not as long as its Content-Length"
        );
        let _ = (record.date(), record.target_uri(), record.holds_http());
        match record.record_type() {
            RecordType::Request => {
                let _ = HttpRequest::parse(record.block());
            }
            RecordType::Response | RecordType::Revisit => {
                let _ = HttpResponse::parse(record.block());
            }
            RecordType::Warcinfo | RecordType::Other(_) => {}
        }
    }
}
