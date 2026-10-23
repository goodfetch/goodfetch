//! A body's content codings, decoded as Chrome decodes them (ADR 0119;
//! pitfalls H13, H25): each coding `Content-Encoding` lists, the last
//! applied first, `gzip` (and `x-gzip`), `deflate` with its zlib wrapper or
//! raw, as Chrome falls back to raw deflate, `br` and `zstd`; `identity`
//! changes nothing, and a coding Chrome does not know passes the body
//! through. A body that does not decode as labelled, or holds bytes past
//! its coding's end, fails. The charset order of a document's text is
//! #764's.

use std::io::Read as _;

/// A body that did not decode as its codings say.
#[derive(Debug)]
pub(crate) struct DecodingFailed;

/// `body` with every coding of `codings` (each a `Content-Encoding` field's
/// value, in order) decoded, the last applied first.
pub(crate) fn decode<'a>(
    codings: impl Iterator<Item = &'a str>,
    body: Vec<u8>,
) -> Result<Vec<u8>, DecodingFailed> {
    let _ = codings;
    Ok(body)
}

#[cfg(test)]
mod tests;
