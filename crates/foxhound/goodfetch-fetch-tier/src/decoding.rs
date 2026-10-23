//! A body's content codings, decoded as Chrome decodes them (ADR 0119;
//! pitfalls H13, H25): each coding `Content-Encoding` lists, the last
//! applied first, `gzip` (and `x-gzip`), `deflate` with its zlib wrapper or
//! raw, as Chrome falls back to raw deflate, `br` and `zstd`. As in Chrome's
//! `URLRequestHttpJob`, a list naming `identity` or a coding Chrome does not
//! know passes the whole body through undecoded, and bytes after a gzip or
//! deflate stream's end are ignored. A body that does not decode as
//! labelled fails, and so does a decoded body that still starts as a gzip
//! or zstd stream: a text body compressed once more than labelled (spec
//! #579). The charset order of a document's text is #764's; telling a cut
//! compressed stream (`response_truncated`) from a corrupt one is the cut
//! bodies ticket's.

use std::io::Read as _;

/// A body that did not decode as its codings say.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DecodingFailed;

/// A content coding the engine decodes.
#[derive(Clone, Copy, Debug)]
enum Coding {
    Gzip,
    Deflate,
    Brotli,
    Zstd,
}

impl Coding {
    /// The coding a `Content-Encoding` token names, `None` for `identity`
    /// and every coding Chrome does not know.
    fn named(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "gzip" | "x-gzip" => Some(Self::Gzip),
            "deflate" => Some(Self::Deflate),
            "br" => Some(Self::Brotli),
            "zstd" => Some(Self::Zstd),
            _ => None,
        }
    }

    fn decode(self, body: &[u8]) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::new();
        match self {
            Self::Gzip => {
                flate2::read::GzDecoder::new(body).read_to_end(&mut out)?;
            }
            Self::Deflate if has_zlib_header(body) => {
                flate2::read::ZlibDecoder::new(body).read_to_end(&mut out)?;
            }
            Self::Deflate => {
                flate2::read::DeflateDecoder::new(body).read_to_end(&mut out)?;
            }
            Self::Brotli => {
                brotli_decompressor::Decompressor::new(body, 4096).read_to_end(&mut out)?;
            }
            Self::Zstd => out = zstd::stream::decode_all(body)?,
        }
        Ok(out)
    }
}

/// Whether `body` opens with a zlib header (RFC 1950): the deflate method
/// and a check over its first two bytes. Chrome reads `deflate` without one
/// as raw deflate.
fn has_zlib_header(body: &[u8]) -> bool {
    match body {
        [method, flags, ..] => {
            method & 0x0f == 8 && (u16::from(*method) << 8 | u16::from(*flags)) % 31 == 0
        }
        _ => false,
    }
}

/// Whether `body` still opens as a gzip stream or a zstd frame.
fn still_compressed(body: &[u8]) -> bool {
    body.starts_with(&[0x1f, 0x8b]) || body.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
}

/// `body` with every coding of `codings` (each a `Content-Encoding` field's
/// value, in order) decoded, the last applied first.
pub(crate) fn decode<'a>(
    codings: impl Iterator<Item = &'a str>,
    body: Vec<u8>,
) -> Result<Vec<u8>, DecodingFailed> {
    let named: Option<Vec<Coding>> = codings
        .flat_map(|field| field.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(Coding::named)
        .collect();
    let Some(named) = named else {
        return Ok(body);
    };
    if named.is_empty() {
        return Ok(body);
    }
    let decoded = named.iter().rev().try_fold(body, |body, coding| {
        coding.decode(&body).map_err(|_| DecodingFailed)
    })?;
    if still_compressed(&decoded) {
        return Err(DecodingFailed);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests;
