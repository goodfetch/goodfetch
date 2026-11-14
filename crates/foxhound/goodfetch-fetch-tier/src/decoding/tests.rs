//! Covers: story:5-128

use std::io::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_engine::response_limits::{Decoding, Limits};
use rstest::rstest;

use super::still_compressed;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// `body` decoded as `codings` say, as the driver decodes it.
fn decoded(codings: &[&str], body: &[u8]) -> Vec<u8> {
    let mut decoding = Decoding::new(
        "http://site.test/",
        codings.iter().copied(),
        Limits::default(),
    );
    decoding.push(body).unwrap();
    decoding.finish().unwrap()
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
    let decoded = decoded(codings, &body);

    assert_eq!(still_compressed(content_type, &decoded), expected);
}
