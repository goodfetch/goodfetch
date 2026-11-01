//! The `body-limits` family on `body-limits.test` (ADR 0067, SC-05): bodies
//! past the engine's limits on what it reads, beside the `bombs` family's
//! decompression bombs. A body one byte past 64 MiB with its
//! `Content-Length`, a chunked body that never sends its last chunk and a
//! body with neither that never closes each expect `response_limit_exceeded`
//! naming `network.max_body_size`; a discovered API whose JSON nests arrays
//! 129 deep expects it naming `network.max_json_depth`. One task is the
//! positive control: JSON nested exactly 128 deep, whose innermost object a
//! correct tool keeps as one Record. The sixth is a gzip body within the
//! engine's `network.max_decompression_ratio` that decodes to 272 MiB, past
//! the 256 MiB a browser's pipe frames in one message, which expects
//! `response_limit_exceeded` naming `network.max_body_size` from every
//! reader, a browser's included; the named host [`ROBOTS_HOST`], of no
//! task, serves the same body as its robots.txt and a small page at any
//! other path, for a navigation's robots.txt read.
//!
//! The raw server writes each long uncoded body as filler while it sends it
//! ([`Answer::Filler`]), so no page holds what it sends; the coded body is
//! built once, about 8 MiB, at the first request for it. It sends them over
//! HTTP/1.1 only, so the family's hosts offer nothing else.

use std::io::Write as _;
use std::sync::OnceLock;

use flate2::Compression;
use flate2::write::DeflateEncoder;

use crate::raw::{Answer, Framing, Page, Request};
use crate::task::{Expected, ExpectedError, Kind, Record, Task};

/// The host name the family is served on.
pub(super) const HOST: &str = "body-limits.test";

const FAMILY: &str = "body-limits";

/// The named host, of no task, whose robots.txt is the coded body.
pub(super) const ROBOTS_HOST: &str = "robots.body-limits.test";

/// The family's host names that belong to no task.
pub(super) const NAMED_HOSTS: [&str; 1] = [ROBOTS_HOST];

/// The small page [`ROBOTS_HOST`] answers at every path but its robots.txt.
const ROBOTS_HOST_PAGE: &str = "A page whose site's robots.txt is past the size limit.";

/// The code a refusal at a limit reports.
const LIMIT_CODE: &str = "response_limit_exceeded";

/// The bytes past which `network.max_body_size` refuses a body, by default:
/// 64 MiB.
const MAX_BODY_SIZE: u64 = 64 * 1024 * 1024;

/// The deepest nesting `network.max_json_depth` keeps, by default.
const MAX_JSON_DEPTH: usize = 128;

/// One task's page.
#[derive(Clone, Copy)]
enum Limit {
    /// A body of `network.max_body_size` and one byte, with its
    /// `Content-Length`.
    PastTheSize,
    /// A chunked body that never sends its last chunk.
    EndlessChunked,
    /// A body with no length that never closes.
    EndlessUntilClosed,
    /// JSON whose arrays nest one deeper than `network.max_json_depth`.
    JsonPastTheDepth,
    /// JSON nested exactly as deep as `network.max_json_depth` keeps, the
    /// positive control.
    JsonAtTheDepth,
    /// A gzip body within the decompression ratio that decodes past the
    /// browser pipe's 256 MiB frame.
    CodedPastTheFrame,
}

const LIMITS: [(Limit, &str); 6] = [
    (
        Limit::PastTheSize,
        "Read the start page as one Record with `id` and `text`. Its body is one byte past \
         64 MiB, the engine's `network.max_body_size`, and says so in its `Content-Length`. A \
         correct tool refuses it at the limit rather than reading it whole.",
    ),
    (
        Limit::EndlessChunked,
        "Read the start page as one Record with `id` and `text`. Its body is chunked and never \
         sends its last chunk. A correct tool stops it at the engine's `network.max_body_size` \
         rather than reading it without end.",
    ),
    (
        Limit::EndlessUntilClosed,
        "Read the start page as one Record with `id` and `text`. Its body has no length and \
         never closes. A correct tool stops it at the engine's `network.max_body_size` rather \
         than reading it without end.",
    ),
    (
        Limit::JsonPastTheDepth,
        "Read the API's answer as one Record with `id` and `text`. Its JSON nests arrays 129 \
         deep, past the engine's `network.max_json_depth`. A correct tool refuses it before it \
         parses it.",
    ),
    (
        Limit::JsonAtTheDepth,
        "Read the API's answer as one Record with `id` and `text`: the object nested 128 deep, \
         as deep as the engine's `network.max_json_depth` keeps. A correct tool keeps it.",
    ),
    (
        Limit::CodedPastTheFrame,
        "Read the start page as one Record with `id` and `text`. Its body is gzip within the \
         engine's decompression ratio and decodes to 272 MiB, past its `network.max_body_size` \
         and past the 256 MiB a browser's pipe frames in one message. A correct tool stops it \
         at the limit rather than decoding it whole.",
    ),
];

/// The path of each task's page.
fn path(limit: Limit) -> &'static str {
    match limit {
        Limit::JsonPastTheDepth | Limit::JsonAtTheDepth => "/api",
        Limit::PastTheSize
        | Limit::EndlessChunked
        | Limit::EndlessUntilClosed
        | Limit::CodedPastTheFrame => "/",
    }
}

/// The Record the positive control keeps.
fn kept(ordinal: u64) -> Record {
    Record::from([
        ("id", format!("B-{ordinal}")),
        ("text", "Nested as deep as the engine keeps.".to_owned()),
    ])
}

/// JSON of `depth` levels: arrays around one innermost object, which is the
/// last level, holding `record`.
fn nested_json(depth: usize, record: &Record) -> Vec<u8> {
    let object = serde_json::to_string(record).expect("invariant: a Record serializes");
    let arrays = depth - 1;
    format!("{}{object}{}", "[".repeat(arrays), "]".repeat(arrays)).into_bytes()
}

/// The family's tasks.
pub(super) fn tasks(port: Option<u16>) -> Vec<Task> {
    (1..)
        .zip(LIMITS)
        .map(|(ordinal, (limit, description))| {
            let setting = match limit {
                Limit::JsonAtTheDepth => None,
                Limit::JsonPastTheDepth => Some("network.max_json_depth"),
                Limit::PastTheSize
                | Limit::EndlessChunked
                | Limit::EndlessUntilClosed
                | Limit::CodedPastTheFrame => Some("network.max_body_size"),
            };
            let expected = match setting {
                None => Expected::records(vec![kept(ordinal)]),
                Some(setting) => Expected {
                    refusal: true,
                    error: Some(ExpectedError {
                        code: LIMIT_CODE,
                        setting: Some(setting),
                        ..ExpectedError::default()
                    }),
                    ..Expected::default()
                },
            };
            super::task(
                (FAMILY, HOST),
                ordinal,
                Kind::Extract,
                path(limit),
                description,
                expected,
                port,
            )
        })
        .collect()
}

/// The answer to `request` on `host`.
pub(super) fn answer(host: &str, request: &Request) -> Answer {
    if host == ROBOTS_HOST {
        return match request.path() {
            "/robots.txt" => coded(),
            _ => Answer::Page(Page::text(200, ROBOTS_HOST_PAGE)),
        };
    }
    let found = super::ordinal(host).and_then(|ordinal| {
        let at = usize::try_from(ordinal.checked_sub(1)?).ok()?;
        LIMITS.get(at).map(|(limit, _)| (ordinal, *limit))
    });
    let Some((ordinal, limit)) = found.filter(|(_, limit)| request.path() == path(*limit)) else {
        return Answer::Page(Page::text(404, "not found"));
    };
    let filler = |framing, length| Answer::Filler {
        page: Page::text(200, Vec::new()).framed(framing),
        length,
    };
    match limit {
        Limit::PastTheSize => filler(Framing::Length, Some(MAX_BODY_SIZE + 1)),
        Limit::EndlessChunked => filler(Framing::Chunked, None),
        Limit::EndlessUntilClosed => filler(Framing::Close, None),
        Limit::JsonPastTheDepth => Answer::Page(Page::new(
            200,
            "application/json",
            nested_json(MAX_JSON_DEPTH + 1, &kept(ordinal)),
        )),
        Limit::JsonAtTheDepth => Answer::Page(Page::new(
            200,
            "application/json",
            nested_json(MAX_JSON_DEPTH, &kept(ordinal)),
        )),
        Limit::CodedPastTheFrame => coded(),
    }
}

/// The coded body's answer.
fn coded() -> Answer {
    Answer::Page(Page::text(200, coded_body()).with("content-encoding", "gzip"))
}

/// The decoded bytes of [`Limit::CodedPastTheFrame`]'s body: 272 MiB, past
/// the 256 MiB a browser's pipe frames in one message, so a reader that
/// takes the decoded body whole over CDP meets the framing limit before
/// the engine's `network.max_body_size`.
const CODED_DECODED_BYTES: u64 = 272 * SEGMENT_BYTES;

/// The decoded bytes of one of the coded body's deflate segments.
const SEGMENT_BYTES: u64 = 1024 * 1024;

/// In each KiB of a segment, the bytes that do not repeat: enough that the
/// body compresses about 34 to 1, within the engine's ratio of 100.
const NOISE_PER_KIB: usize = 24;

/// The coded body, built once at the first request for it.
static CODED: OnceLock<Vec<u8>> = OnceLock::new();

/// The gzip body that decodes to [`CODED_DECODED_BYTES`] within the
/// engine's decompression ratio. One segment is compressed once, by a fresh
/// raw-deflate compressor that refers to nothing before it and ends on a
/// sync flush, and is repeated: the members' blocks then follow one another
/// as one deflate stream, closed by an empty final block, so building it
/// costs one segment's compression.
fn coded_body() -> &'static [u8] {
    CODED.get_or_init(|| {
        let segment = segment();
        let mut compressor = DeflateEncoder::new(Vec::new(), Compression::default());
        compressor
            .write_all(&segment)
            .expect("invariant: writing to a Vec cannot fail");
        compressor
            .flush()
            .expect("invariant: writing to a Vec cannot fail");
        let compressed = compressor.get_ref().clone();
        let repeats = CODED_DECODED_BYTES / SEGMENT_BYTES;
        let mut crc = crc32fast::Hasher::new();
        // A gzip header: deflate, no flags, no time, unknown system.
        let mut body = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
        for _ in 0..repeats {
            body.extend_from_slice(&compressed);
            crc.update(&segment);
        }
        // An empty final block of fixed codes.
        body.extend_from_slice(&[0x03, 0x00]);
        body.extend_from_slice(&crc.finalize().to_le_bytes());
        let size = u32::try_from(CODED_DECODED_BYTES % (1 << 32))
            .expect("invariant: a size modulo 2^32 fits in 32 bits");
        body.extend_from_slice(&size.to_le_bytes());
        body
    })
}

/// One segment's decoded bytes: each KiB is [`NOISE_PER_KIB`] bytes of a
/// fixed pseudo-random sequence, then zeros.
fn segment() -> Vec<u8> {
    let mut state: u32 = 0x9e37_79b9;
    let mut segment = vec![0; usize::try_from(SEGMENT_BYTES).expect("invariant: 1 MiB fits")];
    for kib in segment.chunks_mut(1024) {
        for byte in &mut kib[..NOISE_PER_KIB] {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state.to_le_bytes()[0];
        }
    }
    segment
}
