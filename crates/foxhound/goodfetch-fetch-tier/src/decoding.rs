//! A body's content codings, decoded as Chrome decodes them (ADR 0119;
//! pitfalls H13, H25): each coding `Content-Encoding` lists, the last
//! applied first, `gzip` (and `x-gzip`), `deflate` with its zlib wrapper or
//! raw, as Chrome falls back to raw deflate, `br` and `zstd`. As in Chrome's
//! `URLRequestHttpJob`, a list naming `identity` or a coding Chrome does not
//! know passes the whole body through undecoded, and bytes after a gzip or
//! deflate stream's end are ignored. A body that does not decode as
//! labelled fails, and so does a text body that still starts as a gzip or
//! zstd stream once decoded: one compressed once more than labelled (ADR
//! 0119). A stream that ends before its end is told from a corrupt one: the
//! body was cut, which the driver fails with `response_truncated` (ADR
//! 0118). Which codings a body names, and how `deflate` tells its zlib
//! wrapper, are the engine's rule ([`ContentCoding`]), which its own
//! readers decode by too. The charset order of a document's text is #764's.
//!
//! **Limits (ADR 0067, SC-05; pitfall H14):** every decoded byte counts
//! against `network.max_body_size`, and past 1 MiB against
//! `network.max_decompression_ratio` over the bytes received, judged as
//! each decoder gives its bytes: a decompression bomb, stacked codings
//! included, stops within one decoder's buffer of the limit. A zstd frame
//! may ask for a window of 8 MiB, which RFC 9659 has every HTTP decoder
//! support, or up to `network.max_body_size` rounded up to a power of two,
//! as the engine's own readers allow; a larger one does not decode.

use std::io::Read;

use bytes::Bytes;
use goodfetch_engine::response_limits::{
    ContentCoding, GZIP_MAGIC, LimitExceeded, Limits, ZSTD_MAGIC, has_zlib_header,
};

/// Why a body did not decode as its codings say.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DecodingFailed {
    /// It is no stream of its coding.
    Corrupt,
    /// Its stream stops before its end: the body was cut (ADR 0118).
    Cut,
    /// It decodes past one of the limits (ADR 0067).
    Limit(LimitExceeded),
}

/// What a body may decode to: the limits of the Run it is read for, judged
/// against the bytes it took on the wire, and how much of it is kept.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bound<'a> {
    /// The URL the body answered, which a refusal names.
    pub(crate) url: &'a str,
    /// `network.max_body_size` and `network.max_decompression_ratio`.
    pub(crate) limits: &'a Limits,
    /// The bytes the body took on the wire.
    pub(crate) received: usize,
    /// The most decoded bytes kept, past which the decoding stops: a
    /// robots.txt's first 500 KiB (RFC 9309); `None` keeps the whole body.
    pub(crate) keep: Option<usize>,
}

impl Bound<'_> {
    /// Adds `decoded` to `out`: `true` once `out` holds all it keeps.
    ///
    /// # Errors
    ///
    /// [`DecodingFailed::Limit`] when `out` would pass a limit.
    fn take(&self, out: &mut Vec<u8>, decoded: &[u8]) -> Result<bool, DecodingFailed> {
        let (decoded, full) = match self.keep {
            Some(keep) if out.len() + decoded.len() >= keep => {
                (&decoded[..keep.saturating_sub(out.len())], true)
            }
            _ => (decoded, false),
        };
        let total = u64::try_from(out.len() + decoded.len()).unwrap_or(u64::MAX);
        let received = u64::try_from(self.received).unwrap_or(u64::MAX);
        self.limits
            .judge_decoded(self.url, total, Some(received))
            .map_err(DecodingFailed::Limit)?;
        out.extend_from_slice(decoded);
        Ok(full)
    }

    /// The bound of an inner coding of stacked ones, whose bytes another
    /// coding still decodes: the limits apply, and it is kept whole.
    fn inner(&self) -> Self {
        Self {
            keep: None,
            ..*self
        }
    }
}

/// The bytes a decoder gives at once.
const DECODER_BUFFER: usize = 64 * 1024;

/// The window, as a power of two, a zstd frame may always ask for: 8 MiB
/// (RFC 9659).
const ZSTD_WINDOW_LOG_FLOOR: u32 = 23;

/// The largest window, as a power of two, zstd's decoder takes on 64-bit
/// hosts.
const ZSTD_WINDOW_LOG_CEILING: u32 = 31;

/// The largest window, as a power of two, a zstd frame may ask for under
/// `limits`: `network.max_body_size` rounded up to a power of two, between
/// 8 MiB and zstd's own ceiling, the engine's own readers' rule
/// (`goodfetch_engine::response_limits`), so a frame gets one verdict on
/// every HTTP path.
fn zstd_window_log(limits: &Limits) -> u32 {
    limits
        .max_body_size
        .saturating_sub(1)
        .bit_width()
        .clamp(ZSTD_WINDOW_LOG_FLOOR, ZSTD_WINDOW_LOG_CEILING)
}

/// What `reader`, a decoder, gives, under `bound`.
fn bounded(mut reader: impl Read, bound: &Bound<'_>) -> Result<Vec<u8>, DecodingFailed> {
    let mut out = Vec::new();
    let mut buffer = vec![0; DECODER_BUFFER];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => return Ok(out),
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(failure(&error)),
        };
        if bound.take(&mut out, &buffer[..count])? {
            return Ok(out);
        }
    }
}

/// `body` decoded as `coding` under `bound`; a stream of it that ends
/// early is [`DecodingFailed::Cut`], anything else that does not decode
/// [`DecodingFailed::Corrupt`].
fn decoded(
    coding: ContentCoding,
    body: &[u8],
    bound: &Bound<'_>,
) -> Result<Vec<u8>, DecodingFailed> {
    match coding {
        // A body too short for gzip's header is cut only once it opens as
        // a gzip stream.
        ContentCoding::Gzip if !body.starts_with(&GZIP_MAGIC) => Err(DecodingFailed::Corrupt),
        ContentCoding::Gzip => bounded(flate2::read::GzDecoder::new(body), bound),
        ContentCoding::Deflate if has_zlib_header(body) => {
            bounded(flate2::read::ZlibDecoder::new(body), bound)
        }
        ContentCoding::Deflate => bounded(flate2::read::DeflateDecoder::new(body), bound),
        ContentCoding::Brotli => brotli(body, bound),
        ContentCoding::Zstd => {
            let mut decoder =
                zstd::stream::read::Decoder::with_buffer(body).map_err(|error| failure(&error))?;
            decoder
                .window_log_max(zstd_window_log(bound.limits))
                .map_err(|error| failure(&error))?;
            bounded(decoder, bound)
        }
    }
}

/// What a decoder's `error` means: a stream that ended early is cut.
fn failure(error: &std::io::Error) -> DecodingFailed {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => DecodingFailed::Cut,
        _ => DecodingFailed::Corrupt,
    }
}

/// `body` decoded as Brotli (RFC 7932) under `bound`, through the
/// decoder's stream interface, which tells a stream that wants more input,
/// a cut one, from an invalid one; its reader reports both alike. Bytes
/// after the stream's end are invalid, as the reader holds them.
fn brotli(body: &[u8], bound: &Bound<'_>) -> Result<Vec<u8>, DecodingFailed> {
    use brotli_decompressor::{BrotliDecompressStream, BrotliResult, BrotliState, StandardAlloc};

    let mut state = BrotliState::new(
        StandardAlloc::default(),
        StandardAlloc::default(),
        StandardAlloc::default(),
    );
    let mut out = Vec::new();
    let mut buffer = vec![0; DECODER_BUFFER];
    let mut available_in = body.len();
    let mut input_offset = 0;
    let mut written = 0;
    loop {
        let mut available_out = buffer.len();
        let mut output_offset = 0;
        let result = BrotliDecompressStream(
            &mut available_in,
            &mut input_offset,
            body,
            &mut available_out,
            &mut output_offset,
            &mut buffer,
            &mut written,
            &mut state,
        );
        if bound.take(&mut out, &buffer[..output_offset])? {
            return Ok(out);
        }
        match result {
            BrotliResult::NeedsMoreOutput => {}
            BrotliResult::ResultSuccess if available_in == 0 => return Ok(out),
            BrotliResult::NeedsMoreInput => return Err(DecodingFailed::Cut),
            BrotliResult::ResultSuccess | BrotliResult::ResultFailure => {
                return Err(DecodingFailed::Corrupt);
            }
        }
    }
}

/// Whether `body`, decoded, of the type `content_type` names, is a text
/// body that still opens as a gzip stream or a zstd frame (ADR 0119): the
/// text types are `text/*`, `application/json`, `application/xml`,
/// `application/xhtml+xml`, `application/javascript` and any `+json` or
/// `+xml` type.
pub(crate) fn still_compressed(content_type: Option<&str>, body: &[u8]) -> bool {
    let compressed = body.starts_with(&GZIP_MAGIC) || body.starts_with(&ZSTD_MAGIC);
    compressed && content_type.is_some_and(is_text)
}

/// Whether the `Content-Type` value `content_type` names a text type.
fn is_text(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    essence.starts_with("text/")
        || essence.ends_with("+json")
        || essence.ends_with("+xml")
        || [
            "application/json",
            "application/xml",
            "application/xhtml+xml",
            "application/javascript",
        ]
        .contains(&essence.as_str())
}

/// `body` with every coding of `codings` (each a `Content-Encoding` field's
/// value, in order) decoded under `bound`, the last applied first, only the
/// first-applied coding's bytes kept as `bound` says; a body no coding
/// applies to is answered as it is, never copied.
pub(crate) fn decode<'a>(
    codings: impl Iterator<Item = &'a str>,
    body: Bytes,
    bound: &Bound<'_>,
) -> Result<Bytes, DecodingFailed> {
    let Some(listed) = ContentCoding::listed(codings) else {
        return Ok(body);
    };
    listed
        .iter()
        .enumerate()
        .rev()
        .try_fold(body, |body, (index, coding)| {
            let bound = if index == 0 { *bound } else { bound.inner() };
            decoded(*coding, &body, &bound).map(Bytes::from)
        })
}

#[cfg(test)]
mod tests;
