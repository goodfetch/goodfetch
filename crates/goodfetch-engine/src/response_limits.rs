//! The limits on what the engine reads from outside (ADR 0067, SC-05,
//! pitfall H14), so a hostile site cannot exhaust the engine's memory:
//! `network.max_body_size`, the decoded bytes of one body the engine itself
//! reads, never a download, which the disk governor bounds (ADR 0122);
//! `network.max_decompression_ratio`, the decoded bytes per received byte
//! once a decoded body passes 1 MiB; and `network.max_json_depth`, the
//! nesting of JSON the engine parses from outside. Past any of them the call
//! fails with `response_limit_exceeded` naming the setting.
//!
//! A body is decoded as it arrives ([`Decoding`]): each content coding
//! `Content-Encoding` lists, the last applied first, `gzip` (and `x-gzip`),
//! `deflate` with its zlib wrapper or raw, `br` and `zstd`, as Chrome
//! decodes them; `identity` or a coding Chrome does not know passes the body
//! through undecoded, as Chrome does. Every decoded byte is counted as the
//! decoders give it, so a limit stops the decoding within one decoder's
//! buffer of where it was passed, and the engine never holds more than the
//! limit and that buffer. A zstd frame may ask for a window of at most 8 MiB,
//! RFC 9659's, and one that asks for more fails to decode, as Chromium's
//! decoder refuses it (`ERR_ZSTD_WINDOW_SIZE_TOO_BIG`), so the HTTP tiers and
//! the browser give one frame one verdict.
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

use std::io::{self, Write};

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

/// The largest window, as a power of two, a zstd frame may ask for: 8 MiB
/// (RFC 9659, and Chromium's `windowLogMax`).
const ZSTD_WINDOW_LOG_MAX: u32 = 23;

/// The deepest JSON the engine parses, whatever a [`Limits`] built in code
/// says: the most `network.max_json_depth` takes, past which the parser,
/// which recurses, might not fit a thread's stack.
pub const JSON_DEPTH_CEILING: u64 = MAX_JSON_DEPTH_CEILING;

/// The bytes a Brotli decoder writes out at once.
const BROTLI_BUFFER: usize = 32 * 1024;

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

/// Why a body was not read whole.
#[derive(Debug)]
pub enum BodyFailure {
    /// It passed one of the limits.
    Limit(LimitExceeded),
    /// It is no stream of the codings it names.
    Decoding,
}

/// A body being decoded as it arrives, under the limits.
pub struct Decoding {
    /// The first stage the received bytes go through; the last is the
    /// [`Sink`].
    stage: Box<dyn Stage>,
}

impl std::fmt::Debug for Decoding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Decoding").finish_non_exhaustive()
    }
}

impl Decoding {
    /// A body of `url` under `codings`, each a `Content-Encoding` field's
    /// value in order, read under `limits`.
    pub fn new<'a>(url: &str, codings: impl Iterator<Item = &'a str>, limits: Limits) -> Self {
        let sink = Sink {
            url: url.to_owned(),
            limits,
            received: 0,
            bytes: Vec::new(),
            passed: None,
        };
        // The last coding applied is the first decoded, so it is the
        // outermost stage.
        let stage = ContentCoding::listed(codings)
            .unwrap_or_default()
            .into_iter()
            .fold(Box::new(sink) as Box<dyn Stage>, |inner, coding| {
                coding.stage(inner)
            });
        Self { stage }
    }

    /// Decodes the body's next `received` bytes.
    ///
    /// # Errors
    ///
    /// [`BodyFailure::Limit`] once the decoded bytes pass a limit;
    /// [`BodyFailure::Decoding`] when they are no stream of their coding.
    pub fn push(&mut self, received: &[u8]) -> Result<(), BodyFailure> {
        self.stage.sink().received +=
            u64::try_from(received.len()).expect("invariant: a chunk's length fits in u64");
        let written = self.stage.write_all(received);
        self.failed(written)
    }

    /// Every byte the body received so far decodes to, the decoders'
    /// buffers flushed: enough to judge a body by its first bytes before
    /// the rest arrives. The decoding goes on with the next
    /// [`Self::push`].
    ///
    /// # Errors
    ///
    /// As [`Self::push`] says, for what the decoders held.
    pub fn decoded(&mut self) -> Result<&[u8], BodyFailure> {
        let flushed = self.stage.flush();
        self.failed(flushed)?;
        Ok(&self.stage.sink().bytes)
    }

    /// The whole decoded body, once its last bytes arrived.
    ///
    /// # Errors
    ///
    /// As [`Self::push`] says, for what the decoders still held.
    pub fn finish(mut self) -> Result<Vec<u8>, BodyFailure> {
        let ended = self.stage.end();
        self.failed(ended)?;
        Ok(std::mem::take(&mut self.stage.sink().bytes))
    }

    /// The failure `result` stands for: a limit the sink saw passed, or a
    /// stream that did not decode.
    fn failed(&mut self, result: io::Result<()>) -> Result<(), BodyFailure> {
        if let Some(exceeded) = self.stage.sink().passed.clone() {
            return Err(BodyFailure::Limit(exceeded));
        }
        result.map_err(|_| BodyFailure::Decoding)
    }
}

/// The two bytes a gzip stream opens with (RFC 1952).
pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// The four bytes a zstd frame opens with (RFC 8878).
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// A content coding the engine decodes, as Chrome decodes it: every tier
/// that reads a body names its codings with this one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentCoding {
    /// `gzip`, and `x-gzip`.
    Gzip,
    /// `deflate`, with its zlib wrapper or raw ([`has_zlib_header`]).
    Deflate,
    /// `br`.
    Brotli,
    /// `zstd`.
    Zstd,
}

impl ContentCoding {
    /// The coding a `Content-Encoding` token names, `None` for `identity`
    /// and every coding Chrome does not know, which leaves the body as it
    /// is.
    pub fn named(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "gzip" | "x-gzip" => Some(Self::Gzip),
            "deflate" => Some(Self::Deflate),
            "br" => Some(Self::Brotli),
            "zstd" => Some(Self::Zstd),
            _ => None,
        }
    }

    /// The codings `fields` list, each a `Content-Encoding` field's value
    /// in order, in the order they were applied; `None` when one names
    /// `identity` or a coding Chrome does not know, which, as in Chrome's
    /// `URLRequestHttpJob`, leaves the whole body undecoded.
    pub fn listed<'a>(fields: impl Iterator<Item = &'a str>) -> Option<Vec<Self>> {
        fields
            .flat_map(|field| field.split(','))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(Self::named)
            .collect()
    }

    /// The stage that decodes this coding into `inner`.
    fn stage(self, inner: Box<dyn Stage>) -> Box<dyn Stage> {
        match self {
            Self::Gzip => Box::new(flate2::write::GzDecoder::new(inner)),
            Self::Deflate => Box::new(Deflate::Opening(Vec::new(), Some(inner))),
            Self::Brotli => Box::new(brotli_decompressor::DecompressorWriter::new(
                inner,
                BROTLI_BUFFER,
            )),
            Self::Zstd => Box::new(Zstd::new(inner)),
        }
    }
}

/// One step of a body's decoding: it writes what it decodes into the next
/// step, the last of which is the [`Sink`].
trait Stage: Write + Send {
    /// Writes out what it still holds once the body ended, and fails when
    /// its stream is not whole.
    fn end(&mut self) -> io::Result<()>;

    /// The last step.
    fn sink(&mut self) -> &mut Sink;
}

/// The last step of a body's decoding: it keeps the decoded bytes and
/// judges the limits on each write.
struct Sink {
    url: String,
    limits: Limits,
    /// The bytes received on the wire so far.
    received: u64,
    /// The decoded bytes so far.
    bytes: Vec<u8>,
    /// The limit the body passed, once it did.
    passed: Option<LimitExceeded>,
}

impl Write for Sink {
    fn write(&mut self, decoded: &[u8]) -> io::Result<usize> {
        if self.passed.is_some() {
            return Err(io::Error::other("a limit was passed"));
        }
        let total = u64::try_from(self.bytes.len() + decoded.len())
            .expect("invariant: a length fits in u64");
        if let Some(setting) = self.limits.passed(total, Some(self.received)) {
            self.passed = Some(self.limits.exceeded(&self.url, setting));
            return Err(io::Error::other("a limit was passed"));
        }
        self.bytes.extend_from_slice(decoded);
        Ok(decoded.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Stage for Sink {
    fn end(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn sink(&mut self) -> &mut Sink {
        self
    }
}

impl Stage for flate2::write::GzDecoder<Box<dyn Stage>> {
    fn end(&mut self) -> io::Result<()> {
        self.try_finish()?;
        self.get_mut().end()
    }

    fn sink(&mut self) -> &mut Sink {
        self.get_mut().sink()
    }
}

impl Stage for brotli_decompressor::DecompressorWriter<Box<dyn Stage>> {
    fn end(&mut self) -> io::Result<()> {
        self.close()?;
        self.get_mut().end()
    }

    fn sink(&mut self) -> &mut Sink {
        self.get_mut().sink()
    }
}

/// A `deflate` stage: with its zlib wrapper or raw, as Chrome falls back to
/// raw deflate, which its first two bytes tell.
enum Deflate {
    /// Its first bytes, until there are two, and the next stage.
    Opening(Vec<u8>, Option<Box<dyn Stage>>),
    /// A zlib stream (RFC 1950).
    Zlib(flate2::write::ZlibDecoder<Box<dyn Stage>>),
    /// A raw deflate stream (RFC 1951).
    Raw(flate2::write::DeflateDecoder<Box<dyn Stage>>),
}

impl Deflate {
    /// Opens the stream on its first bytes, then writes them.
    fn open(&mut self) -> io::Result<()> {
        let Self::Opening(first, inner) = self else {
            return Ok(());
        };
        let first = std::mem::take(first);
        let inner = inner
            .take()
            .expect("invariant: an opening deflate stage holds its next stage");
        *self = if has_zlib_header(&first) {
            Self::Zlib(flate2::write::ZlibDecoder::new(inner))
        } else {
            Self::Raw(flate2::write::DeflateDecoder::new(inner))
        };
        self.write_all(&first)
    }
}

impl Write for Deflate {
    fn write(&mut self, received: &[u8]) -> io::Result<usize> {
        match self {
            Self::Opening(first, _) => {
                first.extend_from_slice(received);
                if first.len() >= 2 {
                    self.open()?;
                }
                Ok(received.len())
            }
            Self::Zlib(decoder) => decoder.write(received),
            Self::Raw(decoder) => decoder.write(received),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Opening(..) => Ok(()),
            Self::Zlib(decoder) => decoder.flush(),
            Self::Raw(decoder) => decoder.flush(),
        }
    }
}

impl Stage for Deflate {
    fn end(&mut self) -> io::Result<()> {
        self.open()?;
        match self {
            Self::Opening(..) => Ok(()),
            Self::Zlib(decoder) => {
                decoder.try_finish()?;
                decoder.get_mut().end()
            }
            Self::Raw(decoder) => {
                decoder.try_finish()?;
                decoder.get_mut().end()
            }
        }
    }

    fn sink(&mut self) -> &mut Sink {
        match self {
            Self::Opening(_, inner) => inner
                .as_mut()
                .expect("invariant: an opening deflate stage holds its next stage")
                .sink(),
            Self::Zlib(decoder) => decoder.get_mut().sink(),
            Self::Raw(decoder) => decoder.get_mut().sink(),
        }
    }
}

/// Whether `body` opens with a zlib header (RFC 1950): the deflate method
/// and a check over its first two bytes. Chrome reads `deflate` without one
/// as raw deflate.
pub fn has_zlib_header(body: &[u8]) -> bool {
    match body {
        [method, flags, ..] => {
            method & 0x0f == 8 && (u16::from(*method) << 8 | u16::from(*flags)) % 31 == 0
        }
        _ => false,
    }
}

/// A `zstd` stage, whose frame may ask for a window of at most 8 MiB; one
/// whose decoder cannot start fails on its first write.
enum Zstd {
    /// The decoder.
    Decoding(Box<zstd::stream::write::Decoder<'static, Box<dyn Stage>>>),
    /// No decoder could start; the next stage is kept for its sink.
    Unopened(Box<dyn Stage>),
}

impl Zstd {
    fn new(inner: Box<dyn Stage>) -> Self {
        // The decoder's context is made before the inner stage is moved in,
        // so a context that cannot be made keeps the stage.
        match zstd::stream::raw::Decoder::new() {
            Ok(mut raw) => {
                if raw
                    .set_parameter(zstd::stream::raw::DParameter::WindowLogMax(
                        ZSTD_WINDOW_LOG_MAX,
                    ))
                    .is_err()
                {
                    return Self::Unopened(inner);
                }
                Self::Decoding(Box::new(zstd::stream::write::Decoder::with_decoder(
                    inner, raw,
                )))
            }
            Err(_) => Self::Unopened(inner),
        }
    }
}

impl Write for Zstd {
    fn write(&mut self, received: &[u8]) -> io::Result<usize> {
        match self {
            Self::Decoding(decoder) => decoder.write(received),
            Self::Unopened(_) => Err(io::Error::other("no zstd decoder")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Decoding(decoder) => decoder.flush(),
            Self::Unopened(_) => Ok(()),
        }
    }
}

impl Stage for Zstd {
    fn end(&mut self) -> io::Result<()> {
        match self {
            Self::Decoding(decoder) => {
                decoder.flush()?;
                decoder.get_mut().end()
            }
            Self::Unopened(_) => Err(io::Error::other("no zstd decoder")),
        }
    }

    fn sink(&mut self) -> &mut Sink {
        match self {
            Self::Decoding(decoder) => decoder.get_mut().sink(),
            Self::Unopened(inner) => inner.sink(),
        }
    }
}

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
