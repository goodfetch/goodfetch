//! Each content coding decoded as Chrome decodes it, whatever chunks a
//! connection hands its bytes over in: one verdict, a body, a corrupt
//! stream, a cut one or a limit, for every reader of a body (spec #2164,
//! issue 2187).
//!
//! Covers: row:SC-05, story:5-128
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::response_limits::LimitSetting;
use rstest::rstest;

use super::{MIB, URL, gzip, raw_deflate, zlib};
use crate::response_limits::{BodyFailure, Decoding, LimitExceeded, Limits};

/// `received` read as a body under `codings` and `limits`, in chunks of
/// `chunk` bytes, keeping `keep` decoded bytes.
fn read_in(
    chunk: usize,
    codings: &[&str],
    received: &[u8],
    limits: Limits,
    keep: Option<usize>,
) -> Result<Vec<u8>, BodyFailure> {
    let decoding = Decoding::new(URL, codings.iter().copied(), limits);
    let mut decoding = match keep {
        Some(keep) => decoding.keeping(keep),
        None => decoding,
    };
    for chunk in received.chunks(chunk) {
        decoding.push(chunk)?;
        if decoding.is_full() {
            break;
        }
    }
    decoding.finish()
}

/// `abc` as a Brotli stream of one uncompressed meta-block (RFC 7932),
/// checked once with `brotli --decompress`.
const BROTLI_ABC: [u8; 7] = [0x20, 0x00, 0x10, b'a', b'b', b'c', 0x03];

/// `abc` as a zstd frame of one raw block (RFC 8878), checked once with
/// `zstd --decompress`.
const ZSTD_ABC: [u8; 15] = [
    0x28, 0xb5, 0x2f, 0xfd, 0xa0, 3, 0, 0, 0, 0x19, 0, 0, b'a', b'b', b'c',
];

/// `bytes` as a Brotli stream of one uncompressed meta-block (RFC 7932),
/// written as the Proving ground's `content-coding` family writes it.
fn brotli_stored(bytes: &[u8]) -> Vec<u8> {
    let length = u32::try_from(bytes.len() - 1).unwrap();
    let mut stream = ((length << 4) | (1 << 20)).to_le_bytes()[..3].to_vec();
    stream.extend_from_slice(bytes);
    stream.push(0x03);
    stream
}

fn zstd(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(bytes, 0).unwrap()
}

/// `body` followed by `after`.
fn followed(mut body: Vec<u8>, after: &[u8]) -> Vec<u8> {
    body.extend_from_slice(after);
    body
}

#[rstest]
#[case::gzip(&["gzip"], gzip(b"abc"))]
#[case::x_gzip(&["x-gzip"], gzip(b"abc"))]
#[case::deflate_with_its_zlib_wrapper(&["deflate"], zlib(b"abc"))]
#[case::raw_deflate(&["deflate"], raw_deflate(b"abc"))]
#[case::brotli(&["br"], BROTLI_ABC.to_vec())]
#[case::brotli_as_the_proving_ground_writes_it(&["br"], brotli_stored(b"abc"))]
#[case::zstd(&["zstd"], ZSTD_ABC.to_vec())]
#[case::gzip_then_brotli_in_one_field(&["gzip, br"], brotli_stored(&gzip(b"abc")))]
#[case::gzip_then_zstd_in_two_fields(&["gzip", "zstd"], zstd(&gzip(b"abc")))]
#[case::codings_in_any_case_with_spaces(&[" GZIP ,, Br"], brotli_stored(&gzip(b"abc")))]
#[case::bytes_after_a_gzip_stream_are_ignored(&["gzip"], followed(gzip(b"abc"), b"\n\n"))]
#[case::bytes_after_a_deflate_stream_are_ignored(&["deflate"], followed(zlib(b"abc"), b"\n\n"))]
#[case::a_gzip_header_with_every_optional_part(&["gzip"], {
    let mut body = gzip(b"abc");
    // FEXTRA, FNAME, FCOMMENT and FHCRC, each part after the fixed ten
    // bytes, in RFC 1952's order.
    body[3] = 0x1e;
    let parts = [&[2, 0, b'x', b'y'][..], b"name\0", b"comment\0", &[0, 0]].concat();
    body.splice(10..10, parts);
    body
})]
#[case::identity(&["identity"], b"abc".to_vec())]
#[case::an_unknown_coding_passes_through(&["utf-8"], b"abc".to_vec())]
#[case::an_unknown_coding_beside_gzip_passes_the_body_through(&["gzip, utf-8"], b"abc".to_vec())]
#[case::no_coding(&[], b"abc".to_vec())]
fn a_body_decodes_as_chrome_decodes_its_content_codings(
    #[values(1, 7, 16 * 1024)] chunk: usize,
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
) {
    let decoded = read_in(chunk, codings, &body, Limits::default(), None);

    assert_eq!(decoded, Ok(b"abc".to_vec()));
}

#[rstest]
#[case::a_plain_body_labelled_gzip(&["gzip"], b"abc".to_vec())]
#[case::an_invalid_gzip_header(&["gzip"], {
    let mut body = gzip(b"abc");
    body[1] = 0x8c;
    body
})]
#[case::a_gzip_header_with_reserved_flags(&["gzip"], {
    let mut body = gzip(b"abc");
    body[3] = 0x20;
    body
})]
#[case::a_gzip_stream_whose_check_differs(&["gzip"], {
    let mut body = gzip(b"abc");
    let crc = body.len() - 8;
    body[crc] ^= 0xff;
    body
})]
#[case::a_plain_body_labelled_deflate(&["deflate"], b"plain text, never compressed".to_vec())]
#[case::a_plain_body_labelled_brotli(&["br"], b"<!doctype html>".to_vec())]
#[case::bytes_after_a_brotli_stream(&["br"], followed(BROTLI_ABC.to_vec(), b"\n"))]
#[case::a_plain_body_labelled_zstd(&["zstd"], b"abc".to_vec())]
fn a_body_that_does_not_decode_as_labelled_is_corrupt(
    #[values(1, 7, 16 * 1024)] chunk: usize,
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
) {
    let decoded = read_in(chunk, codings, &body, Limits::default(), None);

    assert_eq!(decoded, Err(BodyFailure::Corrupt));
}

/// A corrupt stream fails on the push that shows it, not once the body
/// ended: a reader stops reading a body that cannot decode.
#[test]
fn a_corrupt_stream_fails_as_soon_as_its_bytes_show_it() {
    let mut decoding = Decoding::new(URL, ["gzip"].into_iter(), Limits::default());

    let pushed = decoding.push(b"<!doctype html>");

    assert_eq!(pushed, Err(BodyFailure::Corrupt));
    assert_eq!(decoding.push(b"more"), Err(BodyFailure::Corrupt));
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
#[case::gzip_in_its_header(&["gzip"], gzip(ALPHABETS)[..5].to_vec())]
#[case::deflate_with_its_zlib_wrapper(&["deflate"], cut(zlib(ALPHABETS)))]
#[case::raw_deflate(&["deflate"], cut(raw_deflate(ALPHABETS)))]
#[case::brotli(&["br"], cut(brotli_stored(ALPHABETS)))]
#[case::zstd(&["zstd"], cut(zstd(ALPHABETS)))]
#[case::the_inner_of_two_codings(&["gzip, br"], brotli_stored(&cut(gzip(ALPHABETS))))]
fn a_compressed_stream_ending_early_is_cut(
    #[values(1, 7, 16 * 1024)] chunk: usize,
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
) {
    let decoded = read_in(chunk, codings, &body, Limits::default(), None);

    assert_eq!(decoded, Err(BodyFailure::Cut));
}

/// `count` zero bytes, gzipped: a few kilobytes per ten MiB.
fn gzipped_zeros(count: usize) -> Vec<u8> {
    gzip(&vec![0; count])
}

/// The refusal of a body for passing `setting`, whose value is `limit`.
fn refused(setting: LimitSetting, limit: u64) -> Result<Vec<u8>, BodyFailure> {
    Err(BodyFailure::Limit(LimitExceeded {
        url: URL.to_owned(),
        setting,
        limit,
    }))
}

/// A decompression bomb is refused at the ratio once it decodes past 1 MiB
/// (ADR 0067), its codings stacked or not, an inner stream's bytes counted
/// as they decode: a bomb hidden after an inner stream's end is refused
/// though what the body keeps of it is small.
///
/// Covers: setting:network.max_decompression_ratio
#[rstest]
#[case::gzip(&["gzip"], gzipped_zeros(10 * MIB))]
#[case::gzip_twice(&["gzip", "gzip"], gzip(&gzipped_zeros(10 * MIB)))]
#[case::zstd(&["zstd"], zstd(&vec![0; 10 * MIB]))]
#[case::after_an_inner_streams_end(
    &["gzip", "gzip"],
    gzip(&followed(gzip(b"abc"), &vec![0; 10 * MIB]))
)]
fn a_decompression_bomb_is_refused_at_the_ratio(
    #[values(1, 7, 16 * 1024)] chunk: usize,
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
) {
    let decoded = read_in(chunk, codings, &body, Limits::default(), None);

    assert_eq!(decoded, refused(LimitSetting::MaxDecompressionRatio, 100));
}

/// A body that decodes past `network.max_body_size` is refused, however
/// little it took on the wire under a ratio that would let it through.
///
/// Covers: setting:network.max_body_size
#[rstest]
fn a_body_decoding_past_the_size_the_run_sets_is_refused(#[values(1, 7, 16 * 1024)] chunk: usize) {
    let limits = Limits {
        max_body_size: MIB as u64,
        max_decompression_ratio: 1_000_000,
        ..Limits::default()
    };

    let decoded = read_in(chunk, &["gzip"], &gzipped_zeros(4 * MIB), limits, None);

    assert_eq!(decoded, refused(LimitSetting::MaxBodySize, MIB as u64));
}

/// A body read for its first bytes alone, a robots.txt's 500 KiB, keeps
/// those, decoding no further: a bomb's start is read, not refused, and a
/// plain body is kept whole.
#[rstest]
#[case::a_bomb(&["gzip"], gzipped_zeros(100 * MIB), 500 * 1024)]
#[case::a_plain_body(&[], vec![b'a'; 600 * 1024], 500 * 1024)]
#[case::a_body_shorter_than_the_keep(&["gzip"], gzip(b"abc"), 3)]
fn a_body_kept_for_its_first_bytes_stops_decoding_there(
    #[values(1, 7, 16 * 1024)] chunk: usize,
    #[case] codings: &[&str],
    #[case] body: Vec<u8>,
    #[case] kept: usize,
) {
    let decoded = read_in(chunk, codings, &body, Limits::default(), Some(500 * 1024));

    assert_eq!(decoded.map(|decoded| decoded.len()), Ok(kept));
}

/// What the bytes received so far decode to is there before the body
/// ends, so a reader judges a body by its first bytes.
#[test]
fn the_bytes_decoded_so_far_are_there_before_the_body_ends() {
    let text = vec![b'a'; 64 * 1024];
    let body = gzip(&text);
    let mut decoding = Decoding::new(URL, ["gzip"].into_iter(), Limits::default());

    decoding.push(&body[..body.len() - 8]).unwrap();

    assert_eq!(decoding.decoded(), &text[..]);
}
