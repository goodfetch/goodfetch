//! The `body-limits` family on `body-limits.test` (ADR 0067, SC-05): bodies
//! past the engine's limits on what it reads, beside the `bombs` family's
//! decompression bombs. A body one byte past 64 MiB with its
//! `Content-Length`, a chunked body that never sends its last chunk and a
//! body with neither that never closes each expect `response_limit_exceeded`
//! naming `network.max_body_size`; a discovered API whose JSON nests arrays
//! 129 deep expects it naming `network.max_json_depth`. One task is the
//! positive control: JSON nested exactly 128 deep, whose innermost object a
//! correct tool keeps as one Record.
//!
//! The raw server writes each long body as filler while it sends it
//! ([`Answer::Filler`]), so no page holds what it sends. It sends them over
//! HTTP/1.1 only, so the family's hosts offer nothing else.

use crate::raw::{Answer, Framing, Page, Request};
use crate::task::{Expected, ExpectedError, Kind, Record, Task};

/// The host name the family is served on.
pub(super) const HOST: &str = "body-limits.test";

const FAMILY: &str = "body-limits";

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
}

const LIMITS: [(Limit, &str); 5] = [
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
];

/// The path of each task's page.
fn path(limit: Limit) -> &'static str {
    match limit {
        Limit::JsonPastTheDepth | Limit::JsonAtTheDepth => "/api",
        Limit::PastTheSize | Limit::EndlessChunked | Limit::EndlessUntilClosed => "/",
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
                Limit::PastTheSize | Limit::EndlessChunked | Limit::EndlessUntilClosed => {
                    Some("network.max_body_size")
                }
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
    }
}
