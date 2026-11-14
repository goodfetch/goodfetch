//! The limits on what the engine reads from outside (ADR 0067, SC-05,
//! pitfall H14), so a hostile site cannot exhaust the engine's memory:
//! `network.max_body_size`, the decoded bytes of one body the engine itself
//! reads, never a download, which the disk governor bounds (ADR 0122);
//! `network.max_decompression_ratio`, the decoded bytes per received byte
//! once a decoded body passes 1 MiB; and `network.max_json_depth`, the
//! nesting of JSON the engine parses from outside. Past any of them the call
//! fails with `response_limit_exceeded` naming the setting.
//!
//! A body is decoded as it arrives ([`Decoding`]), by every reader of a body
//! from outside, the fetch tier's included: each content coding
//! `Content-Encoding` lists, as Chrome decodes them. Every decoded byte is
//! counted as the decoders give it, so a limit stops the decoding within one
//! decoder's buffer of where it was passed, and the engine never holds more
//! than the limit and that buffer. A zstd frame may ask for a window of at
//! most 8 MiB, RFC 9659's, and one that asks for more fails to decode, as
//! Chromium's decoder refuses it (`ERR_ZSTD_WINDOW_SIZE_TOO_BIG`), so every
//! tier and the browser give one frame one verdict.
//!
//! JSON is judged before anything parses it ([`check_json_depth`]), then
//! parsed with no recursion limit of the parser's own, which refuses the
//! 128th level (`serde_json` 1.0.154, run): the scan bounds the depth instead, to
//! `network.max_json_depth` and at most [`JSON_DEPTH_CEILING`], where the
//! parser still fits a thread's stack.
//!
//! Every network read of JSON goes through [`parse_json`]: a discovered API's
//! answers, captured and replayed, a site's `TDMRep` file, and the daily
//! channel's manifest and token. A capture is told to no call, so it keeps a
//! call past the depth without its answer, and the replay of that call refuses
//! it. These reads stay outside, each on the parser's own limit of 127 levels:
//!
//! - a Secret provider's or an OTLP collector's reply: an Operator's endpoint,
//!   which no page call waits on (ADR 0067);
//! - a page's JSON-LD and a consent message: text of a document already read
//!   under the limits, where one block that does not parse costs that block;
//! - a Clean-up rules manifest: read from files its pinned digest vouches for,
//!   or from the Browser cache;
//! - the browser's CDP messages and the JSON-RPC channel, which holds its own
//!   `message_too_deep`.

use goodfetch_protocol::response_limits::{
    DEFAULT_MAX_BODY_SIZE, DEFAULT_MAX_DECOMPRESSION_RATIO, DEFAULT_MAX_JSON_DEPTH, LimitSetting,
    MAX_JSON_DEPTH_CEILING, NETWORK_MAX_BODY_SIZE, NETWORK_MAX_DECOMPRESSION_RATIO,
    NETWORK_MAX_JSON_DEPTH, RESPONSE_LIMIT_EXCEEDED, ResponseLimitExceededDetails,
};

use crate::call_error::CallError;
use crate::settings::{Resolved, RunSettings};

/// The decoded size past which `network.max_decompression_ratio` applies:
/// 1 MiB.
const RATIO_APPLIES_PAST: u64 = 1024 * 1024;

/// The deepest JSON the engine parses, whatever a [`Limits`] built in code
/// says: the most `network.max_json_depth` takes, past which the parser,
/// which recurses, might not fit a thread's stack.
pub const JSON_DEPTH_CEILING: u64 = MAX_JSON_DEPTH_CEILING;

/// The limits a Run's settings set on what the engine reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// `network.max_body_size`: the most decoded bytes of one body.
    pub max_body_size: u64,
    /// `network.max_decompression_ratio`: the most decoded bytes per
    /// received byte once a decoded body passes 1 MiB.
    pub max_decompression_ratio: u64,
    /// `network.max_json_depth`: the deepest nesting of arrays and objects.
    pub max_json_depth: u64,
}

impl Default for Limits {
    /// The catalog's defaults: 64 MiB, a ratio of 100 and a depth of 128.
    fn default() -> Self {
        Self {
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            max_decompression_ratio: DEFAULT_MAX_DECOMPRESSION_RATIO,
            max_json_depth: DEFAULT_MAX_JSON_DEPTH,
        }
    }
}

impl Limits {
    /// The limits `settings` set.
    pub fn of(settings: &Resolved) -> Self {
        Self {
            max_body_size: settings.get(&NETWORK_MAX_BODY_SIZE),
            max_decompression_ratio: settings.get(&NETWORK_MAX_DECOMPRESSION_RATIO),
            max_json_depth: settings.get(&NETWORK_MAX_JSON_DEPTH),
        }
    }

    /// The limits of a Run under `settings`; the catalog's defaults for a
    /// Run with none.
    pub fn of_run(settings: Option<&RunSettings>) -> Self {
        settings.map_or_else(Self::default, |settings| Self::of(settings.resolved()))
    }

    /// Judges a body another reader decoded, such as a browser: `decoded`
    /// bytes of it, from `received` bytes on the wire when that is known.
    ///
    /// # Errors
    ///
    /// The limit it passed: `network.max_body_size` first, then
    /// `network.max_decompression_ratio`.
    pub fn judge_decoded(
        &self,
        url: &str,
        decoded: u64,
        received: Option<u64>,
    ) -> Result<(), LimitExceeded> {
        self.passed(decoded, received)
            .map_or(Ok(()), |setting| Err(self.exceeded(url, setting)))
    }

    /// The refusal of `url`'s body as past `network.max_body_size`, for a
    /// body known to be past it before its bytes were counted, such as one
    /// a browser answers in a message longer than the connection reads.
    pub fn past_the_size(&self, url: &str) -> LimitExceeded {
        self.exceeded(url, LimitSetting::MaxBodySize)
    }

    /// The setting `decoded` bytes from `received` pass, if any.
    fn passed(&self, decoded: u64, received: Option<u64>) -> Option<LimitSetting> {
        if decoded > self.max_body_size {
            return Some(LimitSetting::MaxBodySize);
        }
        let past_ratio = received.is_some_and(|received| {
            decoded > RATIO_APPLIES_PAST
                && u128::from(decoded)
                    > u128::from(received) * u128::from(self.max_decompression_ratio)
        });
        past_ratio.then_some(LimitSetting::MaxDecompressionRatio)
    }

    /// The refusal of `url`'s body for passing `setting`.
    fn exceeded(&self, url: &str, setting: LimitSetting) -> LimitExceeded {
        let limit = match setting {
            LimitSetting::MaxBodySize => self.max_body_size,
            LimitSetting::MaxDecompressionRatio => self.max_decompression_ratio,
            LimitSetting::MaxJsonDepth => self.max_json_depth.min(JSON_DEPTH_CEILING),
        };
        LimitExceeded {
            url: url.to_owned(),
            setting,
            limit,
        }
    }
}

/// A body or JSON that passed one of the limits: `response_limit_exceeded`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitExceeded {
    /// The URL whose body passed it.
    pub url: String,
    /// The setting whose limit it passed.
    pub setting: LimitSetting,
    /// The setting's value.
    pub limit: u64,
}

impl LimitExceeded {
    /// The refusal as the call that read the body fails with:
    /// `response_limit_exceeded`, naming the URL, the setting and its value.
    pub fn error(&self) -> CallError {
        CallError::new(
            &RESPONSE_LIMIT_EXCEEDED,
            self.to_string(),
            &ResponseLimitExceededDetails {
                url: self.url.clone(),
                setting: self.setting,
                limit: self.limit,
            },
        )
    }
}

impl std::fmt::Display for LimitExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.setting {
            LimitSetting::MaxBodySize => "decodes to more bytes than",
            LimitSetting::MaxDecompressionRatio => "decodes to more bytes per received byte than",
            LimitSetting::MaxJsonDepth => "holds JSON nested deeper than",
        };
        write!(
            formatter,
            "the body of {} {what} `{}` allows ({})",
            self.url,
            self.setting.setting().name,
            self.limit
        )
    }
}

mod decoding;

pub use decoding::{BodyFailure, ContentCoding, Decoding, GZIP_MAGIC, ZSTD_MAGIC, has_zlib_header};

/// `json`, a body of `url`, parsed once [`check_json_depth`] found it within
/// `network.max_json_depth` and [`JSON_DEPTH_CEILING`].
///
/// # Errors
///
/// [`JsonFailure::Limit`] past the depth; [`JsonFailure::Malformed`] for a
/// body that is no JSON.
pub fn parse_json(
    url: &str,
    json: &[u8],
    limits: &Limits,
) -> Result<serde_json::Value, JsonFailure> {
    check_json_depth(url, json, limits).map_err(JsonFailure::Limit)?;
    let mut deserializer = serde_json::Deserializer::from_slice(json);
    // The scan above bounds the depth, below the ceiling.
    deserializer.disable_recursion_limit();
    let value =
        serde::Deserialize::deserialize(&mut deserializer).map_err(JsonFailure::Malformed)?;
    deserializer.end().map_err(JsonFailure::Malformed)?;
    Ok(value)
}

/// Why JSON from outside was not parsed.
#[derive(Debug)]
pub enum JsonFailure {
    /// It nests past `network.max_json_depth`.
    Limit(LimitExceeded),
    /// It is no JSON, as the parser says.
    Malformed(serde_json::Error),
}

/// Judges how deep `json`, a body of `url`, nests its arrays and objects,
/// before anything parses it: a scan that holds no state but its depth, so
/// JSON past `network.max_json_depth` is refused at the bracket that passes
/// it. Malformed JSON is left for the parser to refuse.
///
/// # Errors
///
/// `response_limit_exceeded` naming `network.max_json_depth`.
pub fn check_json_depth(url: &str, json: &[u8], limits: &Limits) -> Result<(), LimitExceeded> {
    let mut depth = 0_u64;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in json {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > limits.max_json_depth.min(JSON_DEPTH_CEILING) {
                    return Err(limits.exceeded(url, LimitSetting::MaxJsonDepth));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
