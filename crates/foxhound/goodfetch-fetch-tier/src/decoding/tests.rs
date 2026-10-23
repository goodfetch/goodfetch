//! Covers: story:5-128

use std::io::Write as _;

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use rstest::rstest;

use super::decode;

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
    let decoded = decode(codings.iter().copied(), body).unwrap();

    assert_eq!(decoded, b"abc");
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
#[case::a_cut_gzip_stream(&["gzip"], {
    let mut body = gzip(b"abcdefghijklmnopqrstuvwxyz");
    body.truncate(body.len() / 2);
    body
})]
#[case::gzip_twice_labelled_once(&["gzip"], gzip(&gzip(b"abc")))]
#[case::zstd_twice_labelled_once(&["zstd"], {
    let once = zstd::stream::encode_all(&b"abc"[..], 0).unwrap();
    zstd::stream::encode_all(&once[..], 0).unwrap()
})]
fn a_body_that_does_not_decode_as_labelled_fails(#[case] codings: &[&str], #[case] body: Vec<u8>) {
    assert!(decode(codings.iter().copied(), body).is_err());
}
