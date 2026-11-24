//! The codec samples on `codecs.test` (ADR 0074): the Goodfetch Chromium's
//! codec test reads that every codec `canPlayType` or Media Capabilities
//! claims decodes its sample. No task, no seed: the samples are fixed
//! files, each at most 256 KiB in `fixtures/codecs/` with a `.source.toml`
//! naming the command that generated it (ADR 0062).
//!
//! Every video sample is one second of 160×120 frames at 10 frames a second
//! in one color, [`COLOR`]; every audio sample is one second of a 440 Hz
//! sine on one channel at 48 kHz.
//!
//! - `/samples.json`: `{tolerance, samples}`, each sample's `name`, `file`,
//!   `media_type`, the `type` a page asks `canPlayType` and Media
//!   Capabilities about, its `kind` (`video` or `audio`) and its `expected`
//!   decode result: a video's `width`, `height`, `duration` and first
//!   frame's `color`, an audio's `channels`, `duration` and `frequency`.
//!   `tolerance` bounds how far a decoded `duration`, `color` channel and
//!   `frequency` may stray from the expected one.
//! - `/samples/<file>`: a sample, with its media type.
//! - `/`: the page that asks about and decodes each sample. Once done,
//!   `#results` shows each sample's `can_play_type` answer,
//!   `media_capabilities` answer, `observed` decode result and whether it
//!   `decoded` to the expected one as JSON, by name, and the root element
//!   has `data-codecs="done"`.
//! - `/codecs.js`: the page's script.

use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use crate::exchange::{Reply, Request};

/// The host name of the pages.
pub(crate) const HOST: &str = "codecs.test";

/// The color of every video sample's frames, as red, green and blue.
const COLOR: [u8; 3] = [0x20, 0x60, 0xc0];

/// What a sample holds.
#[derive(Clone, Copy)]
enum Kind {
    Video,
    Audio,
}

/// One codec's sample.
struct Sample {
    name: &'static str,
    file: &'static str,
    media_type: &'static str,
    /// The type with its `codecs` parameter, as a page asks about it.
    kind_type: &'static str,
    kind: Kind,
    bytes: &'static [u8],
}

/// A sample of `$name`, in `fixtures/codecs/$file`.
macro_rules! sample {
    ($name:literal, $file:literal, $media_type:literal, $kind_type:literal, $kind:ident) => {
        Sample {
            name: $name,
            file: $file,
            media_type: $media_type,
            kind_type: $kind_type,
            kind: Kind::$kind,
            bytes: include_bytes!(concat!("../fixtures/codecs/", $file)),
        }
    };
}

/// One sample per codec a Chromium's `canPlayType` or Media Capabilities
/// can claim.
const SAMPLES: [Sample; 13] = [
    sample!(
        "h264",
        "h264.mp4",
        "video/mp4",
        "video/mp4; codecs=\"avc1.42E01E\"",
        Video
    ),
    sample!(
        "hevc",
        "hevc.mp4",
        "video/mp4",
        "video/mp4; codecs=\"hvc1.1.6.L30.90\"",
        Video
    ),
    sample!(
        "vp8",
        "vp8.webm",
        "video/webm",
        "video/webm; codecs=\"vp8\"",
        Video
    ),
    sample!(
        "vp9",
        "vp9.webm",
        "video/webm",
        "video/webm; codecs=\"vp09.00.10.08\"",
        Video
    ),
    sample!(
        "av1",
        "av1.mp4",
        "video/mp4",
        "video/mp4; codecs=\"av01.0.00M.08\"",
        Video
    ),
    sample!(
        "aac",
        "aac.m4a",
        "audio/mp4",
        "audio/mp4; codecs=\"mp4a.40.2\"",
        Audio
    ),
    sample!("mp3", "mp3.mp3", "audio/mpeg", "audio/mpeg", Audio),
    sample!(
        "opus",
        "opus.webm",
        "audio/webm",
        "audio/webm; codecs=\"opus\"",
        Audio
    ),
    sample!(
        "vorbis",
        "vorbis.ogg",
        "audio/ogg",
        "audio/ogg; codecs=\"vorbis\"",
        Audio
    ),
    sample!("flac", "flac.flac", "audio/flac", "audio/flac", Audio),
    sample!(
        "pcm",
        "pcm.wav",
        "audio/wav",
        "audio/wav; codecs=\"1\"",
        Audio
    ),
    sample!(
        "ac3",
        "ac3.mp4",
        "audio/mp4",
        "audio/mp4; codecs=\"ac-3\"",
        Audio
    ),
    sample!(
        "eac3",
        "eac3.mp4",
        "audio/mp4",
        "audio/mp4; codecs=\"ec-3\"",
        Audio
    ),
];

/// The page.
const PAGE: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
    <title>Codecs</title>\n</head>\n<body>\n<h1>Codecs</h1>\n<pre id=\"results\"></pre>\n\
    <script src=\"/codecs.js\"></script>\n</body>\n</html>\n";

/// The page's script.
const SCRIPT: &str = include_str!("../pages/codecs/codecs.js");

/// Answers a request to the codec samples; `None` on another host.
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<Reply> {
    if name != HOST {
        return None;
    }
    if request.method != Method::GET {
        return Some(Reply::not_found());
    }
    Some(match request.path {
        "/" => Reply::html(PAGE.to_owned()),
        "/codecs.js" => file("application/javascript", SCRIPT.as_bytes()),
        "/samples.json" => Reply::json(StatusCode::OK, &listing()),
        path => path
            .strip_prefix("/samples/")
            .and_then(|file_name| SAMPLES.iter().find(|sample| sample.file == file_name))
            .map_or_else(Reply::not_found, |sample| {
                file(sample.media_type, sample.bytes)
            }),
    })
}

/// A `200` of `body` as `content_type`.
fn file(content_type: &'static str, body: &'static [u8]) -> Reply {
    Reply {
        status: StatusCode::OK,
        content_type,
        headers: Vec::new(),
        body: body.into(),
    }
}

/// The samples and their expected decode results.
fn listing() -> Value {
    let samples: Vec<Value> = SAMPLES
        .iter()
        .map(|sample| {
            let (kind, expected) = match sample.kind {
                Kind::Video => (
                    "video",
                    json!({ "width": 160, "height": 120, "duration": 1.0, "color": COLOR }),
                ),
                Kind::Audio => (
                    "audio",
                    json!({ "channels": 1, "duration": 1.0, "frequency": 440 }),
                ),
            };
            json!({
                "name": sample.name,
                "file": sample.file,
                "media_type": sample.media_type,
                "type": sample.kind_type,
                "kind": kind,
                "expected": expected,
            })
        })
        .collect();
    json!({
        "tolerance": { "duration": 0.1, "color": 16, "frequency": 10 },
        "samples": samples,
    })
}
