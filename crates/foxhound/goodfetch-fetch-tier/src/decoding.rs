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

use std::io::Read as _;

use bytes::Bytes;
use goodfetch_engine::response_limits::{ContentCoding, has_zlib_header};

/// Why a body did not decode as its codings say.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DecodingFailed {
    /// It is no stream of its coding.
    Corrupt,
    /// Its stream stops before its end: the body was cut (ADR 0118).
    Cut,
}

/// `body` decoded as `coding`; a stream of it that ends early is
/// [`DecodingFailed::Cut`], anything else that does not decode
/// [`DecodingFailed::Corrupt`].
fn decoded(coding: ContentCoding, body: &[u8]) -> Result<Vec<u8>, DecodingFailed> {
    let mut out = Vec::new();
    let read = match coding {
        // A body too short for gzip's header is cut only once it opens as
        // a gzip stream.
        ContentCoding::Gzip if !body.starts_with(&GZIP_MAGIC) => Err(DecodingFailed::Corrupt),
        ContentCoding::Gzip => flate2::read::GzDecoder::new(body)
            .read_to_end(&mut out)
            .map_err(|error| failure(&error)),
        ContentCoding::Deflate if has_zlib_header(body) => flate2::read::ZlibDecoder::new(body)
            .read_to_end(&mut out)
            .map_err(|error| failure(&error)),
        ContentCoding::Deflate => flate2::read::DeflateDecoder::new(body)
            .read_to_end(&mut out)
            .map_err(|error| failure(&error)),
        ContentCoding::Brotli => return brotli(body),
        ContentCoding::Zstd => {
            return zstd::stream::decode_all(body).map_err(|error| failure(&error));
        }
    };
    read.map(|_| out)
}

/// The two bytes a gzip stream opens with (RFC 1952).
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// What a decoder's `error` means: a stream that ended early is cut.
fn failure(error: &std::io::Error) -> DecodingFailed {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => DecodingFailed::Cut,
        _ => DecodingFailed::Corrupt,
    }
}

/// `body` decoded as Brotli (RFC 7932), through the decoder's stream
/// interface, which tells a stream that wants more input, a cut one, from
/// an invalid one; its reader reports both alike. Bytes after the stream's
/// end are invalid, as the reader holds them.
fn brotli(body: &[u8]) -> Result<Vec<u8>, DecodingFailed> {
    use brotli_decompressor::{BrotliDecompressStream, BrotliResult, BrotliState, StandardAlloc};

    let mut state = BrotliState::new(
        StandardAlloc::default(),
        StandardAlloc::default(),
        StandardAlloc::default(),
    );
    let mut out = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
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
        out.extend_from_slice(&buffer[..output_offset]);
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
    let compressed = body.starts_with(&[0x1f, 0x8b]) || body.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]);
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
/// value, in order) decoded, the last applied first; a body no coding
/// applies to is answered as it is, never copied.
pub(crate) fn decode<'a>(
    codings: impl Iterator<Item = &'a str>,
    body: Bytes,
) -> Result<Bytes, DecodingFailed> {
    let Some(listed) = ContentCoding::listed(codings) else {
        return Ok(body);
    };
    listed.iter().rev().try_fold(body, |body, coding| {
        decoded(*coding, &body).map(Bytes::from)
    })
}

#[cfg(test)]
mod tests;
