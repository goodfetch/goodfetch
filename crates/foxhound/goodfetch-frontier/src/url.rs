//! The one parse of every URL a crawl meets (ADR 0120; pitfall U07): seeds,
//! enqueues, `href`s, sitemap entries, `Location`s and Hook rewrites go
//! through the WHATWG URL Standard, as the `url` crate implements it, once,
//! in the Frontier, so every fetch tier receives the URL a browser would
//! request: its host lowercased and IDNA-encoded, an IPv4 host in its
//! dotted form, its default port dropped, a backslash read as a slash,
//! tabs and newlines removed, and its other characters percent-encoded as
//! the Standard says. A crawl fetches only `http` and `https`.

use goodfetch_protocol::foxhound::frontier::NotAddedReason;
use std::borrow::Cow;

use encoding_rs::{EncoderResult, Encoding};
use url::Url;

/// The schemes a crawl fetches.
pub(crate) const FETCHED: [&str; 2] = ["http", "https"];

/// Parses `input` as the WHATWG URL Standard does, against `base` when
/// given: `invalid_url` when the parser refuses it, or when it is relative
/// and has no base; `unsupported_scheme` for a URL the crawl cannot fetch.
///
/// # Errors
///
/// The reason a request to `input` is not added.
pub(crate) fn parse(input: &str, base: Option<&Url>) -> Result<Url, NotAddedReason> {
    let url = Url::options()
        .base_url(base)
        .parse(input)
        .map_err(|_| NotAddedReason::InvalidUrl)?;
    if FETCHED.contains(&url.scheme()) {
        Ok(url)
    } else {
        Err(NotAddedReason::UnsupportedScheme)
    }
}

/// Parses the link `href` of a document whose base URL is `base`, written
/// in `encoding`, as [`parse`] does, but for its query, which is
/// percent-encoded in the document's output encoding as the HTML
/// standard's URL parsing encodes it: UTF-8 for a UTF-16 document, and a
/// character the encoding cannot write as its `&#<code point>;`,
/// percent-encoded, as the WHATWG URL Standard's "percent-encode after
/// encoding" writes it (ADR 0120).
///
/// # Errors
///
/// The reason a request to `href` is not added.
pub(crate) fn parse_link(
    href: &str,
    base: &Url,
    encoding: &'static Encoding,
) -> Result<Url, NotAddedReason> {
    let url = resolve_link(href, base, encoding).ok_or(NotAddedReason::InvalidUrl)?;
    if FETCHED.contains(&url.scheme()) {
        Ok(url)
    } else {
        Err(NotAddedReason::UnsupportedScheme)
    }
}

/// The link `href` of a document whose base URL is `base`, written in
/// `encoding`, resolved as [`parse_link`] resolves it, whatever its
/// scheme: the URL a Payload function reads; `None` when it does not
/// parse.
pub fn resolve_link(href: &str, base: &Url, encoding: &'static Encoding) -> Option<Url> {
    let encoding = encoding.output_encoding();
    let encode: &dyn Fn(&str) -> Cow<'_, [u8]> = &|part| Cow::Owned(encoded(part, encoding));
    Url::options()
        .base_url(Some(base))
        .encoding_override((encoding != encoding_rs::UTF_8).then_some(encode))
        .parse(href)
        .ok()
}

/// `text` written in `encoding`, each character it cannot write as
/// `%26%23<code point in decimal>%3B`, which percent-encoding keeps.
fn encoded(text: &str, encoding: &'static Encoding) -> Vec<u8> {
    let mut encoder = encoding.new_encoder();
    let mut bytes = Vec::with_capacity(text.len() * 2);
    let mut rest = text;
    loop {
        let room = encoder
            .max_buffer_length_from_utf8_without_replacement(rest.len())
            .unwrap_or(rest.len() * 4 + 16);
        bytes.reserve(room);
        let (result, read) =
            encoder.encode_from_utf8_to_vec_without_replacement(rest, &mut bytes, true);
        rest = &rest[read..];
        match result {
            EncoderResult::InputEmpty => return bytes,
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(character) => {
                bytes.extend_from_slice(b"%26%23");
                bytes.extend_from_slice(u32::from(character).to_string().as_bytes());
                bytes.extend_from_slice(b"%3B");
            }
        }
    }
}

/// The base URL of a document reached at `document`, whose first `<base>`
/// element has the `href` `base_href`, as HTML sets it: the `href` resolved
/// against the document's URL, or the document's URL itself when there is
/// none, when it does not parse, or when it is a `data:` or `javascript:`
/// URL; `None` when the document's own URL does not parse.
pub fn document_base(document: &str, base_href: Option<&str>) -> Option<Url> {
    let document = Url::parse(document).ok()?;
    Some(
        base_href
            .and_then(|href| document.join(href).ok())
            .filter(|base| !["data", "javascript"].contains(&base.scheme()))
            .unwrap_or(document),
    )
}

#[cfg(test)]
mod tests;
