//! The `content-coding` family on `content-coding.test` (ADR 0119): one
//! hand-made page per task, each sent with a `Content-Encoding` that says
//! more or less than its body holds. The oracle expects the page's text where
//! the browser decodes it (`gzip, br` in reverse order, raw deflate, and an
//! unknown coding passed through), and `navigation_failed` with
//! `net::ERR_CONTENT_DECODING_FAILED` where it cannot (a plain body labelled
//! gzip, an invalid gzip header, and a body compressed twice but labelled
//! once). Four more pages carry one coding each, decoded: `gzip`, `deflate`
//! with its zlib wrapper, `br` and `zstd`, the transport conformance gate's
//! codings (pitfall H13). Brotli and zstd bodies are written here as their formats' stored
//! blocks (RFC 7932's uncompressed meta-blocks, RFC 8878's raw blocks), so no
//! encoder's update can change a page.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};

use crate::raw::{Answer, Page, Request};
use crate::task::{Expected, ExpectedError, Kind, Record, Task};

/// The host name the family is served on.
pub(super) const HOST: &str = "content-coding.test";

const FAMILY: &str = "content-coding";

/// What a decoding failure reports.
const DECODING_FAILED: ExpectedError = ExpectedError {
    code: "navigation_failed",
    network_error: Some("net::ERR_CONTENT_DECODING_FAILED"),
    rule: None,
    setting: None,
};

#[derive(Clone, Copy)]
enum Coding {
    /// A plain body labelled `gzip`.
    PlainLabelledGzip,
    /// A gzip body whose header's magic number is wrong.
    InvalidGzipHeader,
    /// `gzip`, then `br`, labelled `gzip, br`.
    GzipThenBrotli,
    /// Raw deflate with no zlib wrapper, labelled `deflate`.
    RawDeflate,
    /// A plain body labelled with the unknown coding `utf-8`.
    Unknown,
    /// Compressed twice with gzip, labelled `gzip` once.
    GzipTwice,
    /// Compressed twice with zstd, labelled `zstd` once.
    ZstdTwice,
    /// `gzip`, labelled `gzip`.
    Gzip,
    /// Deflate with its zlib wrapper, labelled `deflate`.
    ZlibDeflate,
    /// `br`, labelled `br`.
    Brotli,
    /// `zstd`, labelled `zstd`.
    Zstd,
}

const CODINGS: [(Coding, &str); 11] = [
    (
        Coding::PlainLabelledGzip,
        "Read this page's text, with the `id` and `text` fields. It is labelled \
         `Content-Encoding: gzip` but plain, so a browser fails to decode it; report the \
         failure.",
    ),
    (
        Coding::InvalidGzipHeader,
        "Read this page's text, with the `id` and `text` fields. Its gzip header is invalid, \
         so a browser fails to decode it; report the failure.",
    ),
    (
        Coding::GzipThenBrotli,
        "Read this page's text, with the `id` and `text` fields. It is labelled \
         `Content-Encoding: gzip, br`.",
    ),
    (
        Coding::RawDeflate,
        "Read this page's text, with the `id` and `text` fields. It is raw deflate, with no \
         zlib wrapper, labelled `Content-Encoding: deflate`.",
    ),
    (
        Coding::Unknown,
        "Read this page's text, with the `id` and `text` fields. It is labelled with the \
         unknown coding `utf-8`.",
    ),
    (
        Coding::GzipTwice,
        "Read this page's text, with the `id` and `text` fields. It is compressed twice with \
         gzip but labelled once, so a browser fails to decode it; report the failure.",
    ),
    (
        Coding::ZstdTwice,
        "Read this page's text, with the `id` and `text` fields. It is compressed twice with \
         zstd but labelled once, so a browser fails to decode it; report the failure.",
    ),
    (
        Coding::Gzip,
        "Read this page's text, with the `id` and `text` fields. It is labelled \
         `Content-Encoding: gzip`.",
    ),
    (
        Coding::ZlibDeflate,
        "Read this page's text, with the `id` and `text` fields. It is deflate with its zlib \
         wrapper, labelled `Content-Encoding: deflate`.",
    ),
    (
        Coding::Brotli,
        "Read this page's text, with the `id` and `text` fields. It is labelled \
         `Content-Encoding: br`.",
    ),
    (
        Coding::Zstd,
        "Read this page's text, with the `id` and `text` fields. It is labelled \
         `Content-Encoding: zstd`.",
    ),
];

/// The page's text, the same for every seed.
fn text(ordinal: u64) -> String {
    format!("Content coding page {ordinal}, decoded once and as labelled.")
}

fn html(ordinal: u64) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Content coding</title></head><body><main><article data-id=\"C-{ordinal}\"><h1 class=\"id\">C-{ordinal}</h1><p class=\"text\">{}</p></article></main></body></html>",
        text(ordinal)
    )
}

/// The family's tasks.
pub(super) fn tasks(port: Option<u16>) -> Vec<Task> {
    (1..)
        .zip(CODINGS)
        .map(|(ordinal, (coding, description))| {
            let decodable = !matches!(
                coding,
                Coding::PlainLabelledGzip
                    | Coding::InvalidGzipHeader
                    | Coding::GzipTwice
                    | Coding::ZstdTwice
            );
            let expected = if decodable {
                Expected::records(vec![Record::from([
                    ("id", format!("C-{ordinal}")),
                    ("text", text(ordinal)),
                ])])
            } else {
                Expected {
                    refusal: true,
                    error: Some(DECODING_FAILED),
                    ..Expected::default()
                }
            };
            super::task(
                (FAMILY, HOST),
                ordinal,
                Kind::Extract,
                "/page",
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
        CODINGS.get(at).map(|(coding, _)| (ordinal, *coding))
    });
    let (Some((ordinal, coding)), "/page") = (found, request.path()) else {
        return Answer::Page(Page::text(404, "not found"));
    };
    let plain = html(ordinal).into_bytes();
    let (label, body) = match coding {
        Coding::PlainLabelledGzip => ("gzip", plain),
        Coding::InvalidGzipHeader => {
            let mut body = gzip(&plain);
            body[1] = 0x8c;
            ("gzip", body)
        }
        Coding::GzipThenBrotli => ("gzip, br", brotli(&gzip(&plain))),
        Coding::RawDeflate => ("deflate", deflate(&plain)),
        Coding::Unknown => ("utf-8", plain),
        Coding::GzipTwice => ("gzip", gzip(&gzip(&plain))),
        Coding::ZstdTwice => ("zstd", zstd(&zstd(&plain))),
        Coding::Gzip => ("gzip", gzip(&plain)),
        Coding::ZlibDeflate => ("deflate", zlib(&plain)),
        Coding::Brotli => ("br", brotli(&plain)),
        Coding::Zstd => ("zstd", zstd(&plain)),
    };
    Answer::Page(Page::html(200, body).with("content-encoding", label))
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .expect("invariant: writing to a Vec succeeds");
    encoder
        .finish()
        .expect("invariant: writing to a Vec succeeds")
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .expect("invariant: writing to a Vec succeeds");
    encoder
        .finish()
        .expect("invariant: writing to a Vec succeeds")
}

fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .expect("invariant: writing to a Vec succeeds");
    encoder
        .finish()
        .expect("invariant: writing to a Vec succeeds")
}

/// A Brotli stream (RFC 7932) holding `bytes` in uncompressed meta-blocks of
/// at most 65,536 bytes: a window of 16 bits, each meta-block's header
/// (not last, four nibbles of length, uncompressed) padded to a byte, its
/// bytes, then an empty last meta-block.
fn brotli(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut first = true;
    for block in bytes.chunks(65_536) {
        let length = u32::try_from(block.len() - 1).expect("invariant: a block is short");
        // Bits, least significant first: [WBITS = 0, only before the first
        // meta-block] ISLAST = 0, MNIBBLES = 0 (four), MLEN - 1 in 16 bits,
        // ISUNCOMPRESSED = 1.
        let (bits, count) = if first {
            ((length << 4) | (1 << 20), 21)
        } else {
            ((length << 3) | (1 << 19), 20)
        };
        first = false;
        let bytes_needed = usize::div_ceil(count, 8);
        out.extend_from_slice(&bits.to_le_bytes()[..bytes_needed]);
        out.extend_from_slice(block);
    }
    // ISLAST = 1, ISLASTEMPTY = 1 (and WBITS = 0 first for an empty body).
    out.push(if first { 0b110 } else { 0b11 });
    out
}

/// A zstd frame (RFC 8878) holding `bytes` in raw blocks of at most
/// 128 KiB, its content size in four bytes, with no checksum.
fn zstd(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0x28, 0xb5, 0x2f, 0xfd];
    // Frame_Content_Size_flag 2 (four bytes), Single_Segment_flag set.
    out.push(0b1010_0000);
    out.extend_from_slice(
        &u32::try_from(bytes.len())
            .expect("invariant: a page is short")
            .to_le_bytes(),
    );
    let mut blocks = bytes.chunks(128 * 1024).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0]);
    }
    while let Some(block) = blocks.next() {
        let last = u32::from(blocks.peek().is_none());
        let header = last | (u32::try_from(block.len()).expect("invariant: a block is short") << 3);
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both literals were checked once with the reference tools:
    // `brotli --decompress` and `zstd --decompress` give back `abc`.

    #[test]
    fn brotli_writes_uncompressed_meta_blocks() {
        assert_eq!(brotli(b"abc"), [0x20, 0x00, 0x10, b'a', b'b', b'c', 0x03]);
    }

    #[test]
    fn zstd_writes_raw_blocks() {
        assert_eq!(
            zstd(b"abc"),
            [
                0x28, 0xb5, 0x2f, 0xfd, 0xa0, 3, 0, 0, 0, 0x19, 0, 0, b'a', b'b', b'c'
            ]
        );
    }
}
