//! A decoded body judged as Chrome judges it (ADR 0119; pitfalls H13,
//! H25): a text body that still starts as a gzip or zstd stream once its
//! codings are decoded is one compressed once more than labelled, and
//! fails.
//!
//! Decoding is the engine's ([`goodfetch_engine::response_limits::Decoding`]):
//! the driver decodes each body as its bytes arrive with the one decoder
//! every reader of a body uses, so a body gets one verdict, a corrupt
//! stream, a cut one (ADR 0118), a limit (ADR 0067) or a zstd window past
//! 8 MiB, on every HTTP path (spec #2164, issue 2187).

use goodfetch_engine::response_limits::{GZIP_MAGIC, ZSTD_MAGIC};

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

#[cfg(test)]
mod tests;
