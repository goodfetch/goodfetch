//! Covers: story:5-128
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::Write as _;

use bytes::Bytes;
use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use goodfetch_engine::response_limits::{LimitExceeded, Limits};
use goodfetch_protocol::response_limits::LimitSetting;
use rstest::rstest;

use super::{Bound, DecodingFailed, decode, still_compressed};

/// The URL the bodies answered.
const URL: &str = "http://site.test/";

/// `body` decoded as `codings` say under `limits`, keeping `keep` bytes.
fn decoded_under(
    codings: &[&str],
    body: Vec<u8>,
    limits: &Limits,
    keep: Option<usize>,
) -> Result<Bytes, DecodingFailed> {
    let bound = Bound {
        url: URL,
        limits,
        received: body.len(),
        keep,
    };
    decode(codings.iter().copied(), body.into(), &bound)
}

/// `body` decoded as `codings` say under the default limits.
fn decoded(codings: &[&str], body: Vec<u8>) -> Result<Bytes, DecodingFailed> {
    decoded_under(codings, body, &Limits::default(), None)
}

/// `abc` as a Brotli stream of one uncompressed meta-block (RFC 7932),
/// checked once with `brotli --decompress`.
const BROTLI_ABC: [u8; 7] = [0x20, 0x00, 0x10, b'a', b'b', b'c', 0x03];

/// `abc` as a zstd frame of one raw block (RFC 8878), checked once with
/// `zstd --decompress`.
const ZSTD_ABC: [u8; 15] = [
    0x28, 0xb5, 0x2f, 0xfd, 0xa0, 3, 0, 0, 0, 0x19, 0, 0, b'a', b'b', b'c',
];

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// `bytes` as a Brotli stream of one uncompressed meta-block (RFC 7932),
/// written as the Proving ground's `content-coding` family writes it.
fn brotli(bytes: &[u8]) -> Vec<u8> {
    let length = u32::try_from(bytes.len() - 1).unwrap();
    let mut stream = ((length << 4) | (1 << 20)).to_le_bytes()[..3].to_vec();
    stream.extend_from_slice(bytes);
    stream.push(0x03);
    stream
}

fn raw_deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[rstest]
#[case::gzip(&["gzip"], gzip(b"abc"))]
#[case::x_gzip(&["x-gzip"], gzip(b"abc"))]
#[case::deflate_with_its_zlib_wrapper(&["deflate"], zlib(b"abc"))]
#[case::raw_deflate(&["deflate"], raw_deflate(b"abc"))]
#[case::brotli(&["br"], BROTLI_ABC.to_vec())]
#[case::brotli_as_written_here(&["br"], brotli(b"abc"))]
#[case::zstd(&["zstd"], ZSTD_ABC.to_vec())]
#[case::gzip_then_brotli_in_one_field(&["gzip, br"], brotli(&gzip(b"abc")))]
#[case::gzip_then_zstd_in_two_fields(&["gzip", "zstd"], zstd::stream::encode_all(&gzip(b"abc")[..], 0).unwrap())]
#[case::codings_in_any_case_with_spaces(&[" GZIP ,, Br"], brotli(&gzip(b"abc")))]
#[case::bytes_after_a_gzip_stream_are_ignored(&["gzip"], {
    let mut body = gzip(b"abc");
    body.extend_from_slice(b"\n\n");
    body
})]
#[case::identity(&["identity"], b"abc".to_vec())]
#[case::an_unknown_coding_passes_through(&["utf-8"], b"abc".to_vec())]
#[case::an_unknown_coding_beside_gzip_passes_the_body_through(&["gzip, utf-8"], b"abc".to_vec())]
#[case::no_coding(&[], b"abc".to_vec())]
fn a_body_decodes_as_chrome_decodes_its_content_codings(
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
) {
    let decoded = decoded(codings, body).unwrap();

    assert_eq!(decoded, &b"abc"[..]);
}

#[rstest]
#[case::a_plain_body_labelled_gzip(&["gzip"], b"abc".to_vec())]
#[case::an_invalid_gzip_header(&["gzip"], {
    let mut body = gzip(b"abc");
    body[1] = 0x8c;
    body
})]
#[case::a_plain_body_labelled_brotli(&["br"], b"<!doctype html>".to_vec())]
#[case::a_plain_body_labelled_zstd(&["zstd"], b"abc".to_vec())]
fn a_body_that_does_not_decode_as_labelled_fails(#[case] codings: &[&str], #[case] body: Vec<u8>) {
    assert_eq!(decoded(codings, body), Err(DecodingFailed::Corrupt));
}

/// The first half of `stream`.
fn cut(mut stream: Vec<u8>) -> Vec<u8> {
    stream.truncate(stream.len() / 2);
    stream
}

/// A text long enough that half of its compressed stream stops inside it.
const ALPHABETS: &[u8] = b"abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz 0123456789";

#[rstest]
#[case::gzip(&["gzip"], cut(gzip(ALPHABETS)))]
#[case::deflate_with_its_zlib_wrapper(&["deflate"], cut(zlib(ALPHABETS)))]
#[case::raw_deflate(&["deflate"], cut(raw_deflate(ALPHABETS)))]
#[case::brotli(&["br"], cut(brotli(ALPHABETS)))]
#[case::zstd(&["zstd"], cut(zstd::stream::encode_all(ALPHABETS, 0).unwrap()))]
#[case::the_inner_of_two_codings(&["gzip, br"], brotli(&cut(gzip(ALPHABETS))))]
fn a_compressed_stream_ending_early_is_cut(#[case] codings: &[&str], #[case] body: Vec<u8>) {
    assert_eq!(decoded(codings, body), Err(DecodingFailed::Cut));
}

#[rstest]
#[case::gzip_twice_labelled_once_as_html(Some("text/html; charset=utf-8"), &["gzip"], gzip(&gzip(b"abc")), true)]
#[case::zstd_twice_labelled_once_as_json(Some("application/ld+json"), &["zstd"], {
    let once = zstd::stream::encode_all(&b"abc"[..], 0).unwrap();
    zstd::stream::encode_all(&once[..], 0).unwrap()
}, true)]
#[case::a_gzip_file_served_as_itself(Some("application/gzip"), &["gzip"], gzip(&gzip(b"abc")), false)]
#[case::an_untyped_body(None, &["gzip"], gzip(&gzip(b"abc")), false)]
#[case::a_text_body_decoded_once(Some("text/html"), &["gzip"], gzip(b"abc"), false)]
fn a_text_body_still_compressed_once_decoded_is_one_compressed_more_than_labelled(
    #[case] content_type: Option<&str>,
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
    #[case] expected: bool,
) {
    let decoded = decoded(codings, body).unwrap();

    assert_eq!(still_compressed(content_type, &decoded), expected);
}

/// `count` zero bytes, gzipped: a few kilobytes per ten MiB.
fn gzipped_zeros(count: usize) -> Vec<u8> {
    gzip(&vec![0; count])
}

/// The refusal of a body for passing `setting`, whose value is `limit`.
fn refused(setting: LimitSetting, limit: u64) -> Result<Bytes, DecodingFailed> {
    Err(DecodingFailed::Limit(LimitExceeded {
        url: URL.to_owned(),
        setting,
        limit,
    }))
}

/// A decompression bomb is refused at the ratio once it decodes past 1 MiB
/// (ADR 0067), its codings stacked or not: each decoded byte counts against
/// the bytes the body took on the wire.
///
/// Covers: setting:network.max_decompression_ratio
#[rstest]
#[case::gzip(&["gzip"], gzipped_zeros(10 * 1024 * 1024))]
#[case::gzip_twice(&["gzip", "gzip"], gzip(&gzipped_zeros(10 * 1024 * 1024)))]
#[case::zstd(&["zstd"], zstd::stream::encode_all(&vec![0; 10 * 1024 * 1024][..], 3).unwrap())]
fn a_decompression_bomb_is_refused_at_the_ratio(#[case] codings: &[&str], #[case] body: Vec<u8>) {
    assert_eq!(
        decoded(codings, body),
        refused(LimitSetting::MaxDecompressionRatio, 100)
    );
}

/// A body that decodes past `network.max_body_size` is refused, however
/// little it took on the wire under a ratio that would let it through.
///
/// Covers: setting:network.max_body_size
#[test]
fn a_body_decoding_past_the_size_the_run_sets_is_refused() {
    let limits = Limits {
        max_body_size: 1024 * 1024,
        max_decompression_ratio: 1_000_000,
        ..Limits::default()
    };

    assert_eq!(
        decoded_under(&["gzip"], gzipped_zeros(4 * 1024 * 1024), &limits, None),
        refused(LimitSetting::MaxBodySize, 1024 * 1024)
    );
}

/// A body read for its first bytes alone, a robots.txt's 500 KiB, keeps
/// those, decoding no further: a bomb's start is read, not refused.
#[test]
fn a_body_kept_for_its_first_bytes_stops_decoding_there() {
    let kept = decoded_under(
        &["gzip"],
        gzipped_zeros(100 * 1024 * 1024),
        &Limits::default(),
        Some(500 * 1024),
    );

    assert_eq!(kept.map(|kept| kept.len()), Ok(500 * 1024));
}

/// A zstd frame may ask for a window up to `network.max_body_size`
/// rounded up to a power of two, 16 MiB here, as the engine's own readers
/// allow; one asking for more does not decode, so its decoder never holds
/// a window larger than the body it may decode.
#[rstest]
#[case::a_window_of_16_mib(24, Ok(9 * 1024 * 1024))]
#[case::a_window_of_32_mib(25, Err(DecodingFailed::Corrupt))]
fn a_zstd_frame_may_ask_for_a_window_up_to_the_body_size(
    #[case] window_log: u32,
    #[case] expected: Result<usize, DecodingFailed>,
) {
    let text: Vec<u8> = (0..9 * 1024 * 1024_u32)
        .map(|index| u8::try_from(index.wrapping_mul(2_654_435_761) >> 24).unwrap())
        .collect();
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
    encoder.window_log(window_log).unwrap();
    encoder.include_contentsize(false).unwrap();
    encoder.write_all(&text).unwrap();
    let frame = encoder.finish().unwrap();

    let limits = Limits {
        max_body_size: 16 * 1024 * 1024,
        max_decompression_ratio: 1_000_000,
        ..Limits::default()
    };

    let decoded = decoded_under(&["zstd"], frame, &limits, None).map(|decoded| decoded.len());

    assert_eq!(decoded, expected);
}
