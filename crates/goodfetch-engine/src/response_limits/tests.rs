//! Covers: row:SC-05

use std::io::Write as _;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::response_limits::LimitSetting;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::{Value, json};

use super::{BodyFailure, Decoding, JsonFailure, Limits, check_json_depth, parse_json};
use crate::settings::{Layers, Values, resolve};

const URL: &str = "http://limits.test/";

const MIB: usize = 1024 * 1024;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn raw_deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn brotli(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut out, 4096, 1, 22);
        encoder.write_all(bytes).unwrap();
    }
    out
}

fn zstd(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(bytes, 1).unwrap()
}

/// `received` read as a body under `codings` and `limits`, in chunks of
/// 16 KiB as a connection hands them over.
fn read(codings: &[&str], received: &[u8], limits: Limits) -> Result<Vec<u8>, BodyFailure> {
    let mut decoding = Decoding::new(URL, codings.iter().copied(), limits);
    for chunk in received.chunks(16 * 1024) {
        decoding.push(chunk)?;
    }
    decoding.finish()
}

fn passed(failure: Result<Vec<u8>, BodyFailure>) -> (LimitSetting, u64) {
    match failure {
        Err(BodyFailure::Limit(exceeded)) => (exceeded.setting, exceeded.limit),
        Err(other) => panic!("expected a limit to be passed, got {other:?}"),
        Ok(bytes) => panic!("expected a limit to be passed, got {} bytes", bytes.len()),
    }
}

#[test]
fn the_limits_are_sixty_four_mebibytes_a_ratio_of_one_hundred_and_a_depth_of_one_hundred_twenty_eight()
 {
    assert_eq!(
        Limits::default(),
        Limits {
            max_body_size: 67_108_864,
            max_decompression_ratio: 100,
            max_json_depth: 128,
        }
    );
}

#[test]
fn the_limits_are_read_from_a_runs_settings() {
    let Value::Object(given) = json!({
        "network": { "max_body_size": 1000, "max_decompression_ratio": 7, "max_json_depth": 3 }
    }) else {
        unreachable!("a JSON object");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &given, Product::Collie).unwrap(),
        ..Layers::default()
    };
    let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    assert_eq!(
        Limits::of(&settings),
        Limits {
            max_body_size: 1000,
            max_decompression_ratio: 7,
            max_json_depth: 3,
        }
    );
}

/// A JSON depth past the 1024 levels the parser fits on a thread's stack is
/// refused when it is set, so `config explain` never shows a limit the
/// engine does not apply; 1024 itself is taken.
///
/// Covers: setting:network.max_json_depth, error:setting_invalid
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn a_json_depth_past_one_thousand_twenty_four_is_setting_invalid() {
    let read = |depth: u64| {
        let Value::Object(given) = json!({ "network": { "max_json_depth": depth } }) else {
            unreachable!("a JSON object");
        };
        Values::read(SettingsLayer::Run, &given, Product::Collie).map(|_| ())
    };

    let refused = read(1025).unwrap_err();

    assert_eq!(
        (refused.code(), read(1024).is_ok()),
        ("setting_invalid", true),
        "{refused}"
    );
}

#[test]
fn a_plain_body_within_the_size_is_read_whole() {
    let body = b"<html>a page</html>".repeat(100);

    let read = read(&[], &body, Limits::default());

    assert_eq!(read.unwrap(), body);
}

#[test]
fn a_plain_body_one_byte_past_the_size_is_refused_naming_the_setting() {
    let limits = Limits {
        max_body_size: 1000,
        ..Limits::default()
    };

    let read = read(&[], &[b'a'; 1001], limits);

    assert_eq!(passed(read), (LimitSetting::MaxBodySize, 1000));
}

#[test]
fn a_plain_body_of_exactly_the_size_is_read() {
    let limits = Limits {
        max_body_size: 1000,
        ..Limits::default()
    };

    let read = read(&[], &[b'a'; 1000], limits);

    assert_eq!(read.unwrap().len(), 1000);
}

#[rstest]
#[case::gzip(&["gzip"], gzip(&vec![0; 2 * MIB]))]
#[case::x_gzip(&["x-gzip"], gzip(&vec![0; 2 * MIB]))]
#[case::zlib_deflate(&["deflate"], zlib(&vec![0; 2 * MIB]))]
#[case::raw_deflate(&["deflate"], raw_deflate(&vec![0; 2 * MIB]))]
#[case::brotli(&["br"], brotli(&vec![0; 2 * MIB]))]
#[case::zstd(&["zstd"], zstd(&vec![0; 2 * MIB]))]
#[case::gzip_twice(&["gzip", "gzip"], gzip(&gzip(&vec![0; 2 * MIB])))]
#[case::gzip_twice_in_one_field(&["gzip, gzip"], gzip(&gzip(&vec![0; 2 * MIB])))]
fn a_decompression_bomb_is_refused_naming_the_ratio(
    #[case] codings: &[&str],
    #[case] received: Vec<u8>,
) {
    let read = read(codings, &received, Limits::default());

    assert_eq!(passed(read), (LimitSetting::MaxDecompressionRatio, 100));
}

#[rstest]
#[case::gzip(&["gzip"], gzip(PAGE.as_bytes()))]
#[case::zlib_deflate(&["deflate"], zlib(PAGE.as_bytes()))]
#[case::raw_deflate(&["deflate"], raw_deflate(PAGE.as_bytes()))]
#[case::brotli(&["br"], brotli(PAGE.as_bytes()))]
#[case::zstd(&["zstd"], zstd(PAGE.as_bytes()))]
#[case::gzip_twice(&["gzip", "gzip"], gzip(&gzip(PAGE.as_bytes())))]
#[case::identity(&["identity"], PAGE.as_bytes().to_vec())]
#[case::a_coding_nobody_knows(&["compress"], PAGE.as_bytes().to_vec())]
fn a_compressed_page_within_the_limits_is_decoded(
    #[case] codings: &[&str],
    #[case] received: Vec<u8>,
) {
    let read = read(codings, &received, Limits::default());

    assert_eq!(read.unwrap(), PAGE.as_bytes());
}

const PAGE: &str = "<html><body><h1>A catalog</h1><p>Prices and names.</p></body></html>";

#[test]
fn a_ratio_past_the_limit_below_one_mebibyte_is_kept() {
    let body = vec![0; MIB];
    let received = gzip(&body);

    let read = read(&["gzip"], &received, Limits::default());

    assert_eq!(read.unwrap(), body);
}

#[test]
fn a_bomb_within_the_ratio_it_is_allowed_is_decoded() {
    let body = vec![0; 2 * MIB];
    let received = gzip(&body);
    let limits = Limits {
        max_decompression_ratio: 100_000,
        ..Limits::default()
    };

    let read = read(&["gzip"], &received, limits);

    assert_eq!(read.unwrap(), body);
}

#[test]
fn a_compressed_body_past_the_size_is_refused_naming_the_size() {
    let received = gzip(&vec![b'a'; 300 * 1024]);
    let limits = Limits {
        max_body_size: 200 * 1024,
        ..Limits::default()
    };

    let read = read(&["gzip"], &received, limits);

    assert_eq!(passed(read), (LimitSetting::MaxBodySize, 200 * 1024));
}

#[rstest]
#[case::gzip(&["gzip"])]
#[case::deflate(&["deflate"])]
#[case::brotli(&["br"])]
#[case::zstd(&["zstd"])]
fn a_body_that_is_no_stream_of_its_coding_fails_to_decode(#[case] codings: &[&str]) {
    let read = read(codings, b"plain text, never compressed", Limits::default());

    assert!(matches!(read, Err(BodyFailure::Decoding)), "{read:?}");
}

fn nested(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

#[rstest]
#[case::arrays_at_the_depth(nested(128))]
#[case::objects_at_the_depth(format!("{}1{}", r#"{"a":"#.repeat(128), "}".repeat(128)))]
#[case::brackets_inside_a_string(format!(r#"["{}"]"#, "[".repeat(1000)))]
#[case::an_escaped_quote_inside_a_string(format!(r#"["\"{}"]"#, "[".repeat(1000)))]
#[case::a_flat_value(r#"{"name":"a","price":1}"#.to_owned())]
fn json_within_the_depth_is_kept(#[case] json: String) {
    let checked = check_json_depth(URL, json.as_bytes(), &Limits::default());

    assert!(checked.is_ok(), "{checked:?}");
}

#[rstest]
#[case::arrays_one_past_the_depth(nested(129))]
#[case::objects_one_past_the_depth(format!("{}1{}", r#"{"a":"#.repeat(129), "}".repeat(129)))]
#[case::unclosed_arrays("[".repeat(100_000))]
#[case::after_a_string_ending_in_an_escaped_backslash(format!(r#"["\\",{}]"#, nested(128)))]
fn json_past_the_depth_is_refused_naming_the_setting(#[case] json: String) {
    let checked = check_json_depth(URL, json.as_bytes(), &Limits::default()).unwrap_err();

    assert_eq!(
        (checked.setting, checked.limit),
        (LimitSetting::MaxJsonDepth, 128)
    );
}

#[test]
fn json_nested_as_deep_as_the_depth_parses() {
    let json = nested(128);

    let parsed = parse_json(URL, json.as_bytes(), &Limits::default());

    assert!(parsed.is_ok(), "{parsed:?}");
}

#[test]
fn json_past_the_ceiling_is_refused_whatever_the_setting_says() {
    let limits = Limits {
        max_json_depth: 100_000,
        ..Limits::default()
    };
    let within = nested(1024);
    let past = nested(1025);

    let within = parse_json(URL, within.as_bytes(), &limits);
    let past = parse_json(URL, past.as_bytes(), &limits);

    assert!(within.is_ok(), "{within:?}");
    let Err(JsonFailure::Limit(exceeded)) = past else {
        panic!("expected a limit, got {past:?}");
    };
    assert_eq!(
        (exceeded.setting, exceeded.limit),
        (LimitSetting::MaxJsonDepth, 1024)
    );
}

#[test]
fn a_body_that_is_no_json_is_malformed() {
    let parsed = parse_json(URL, b"[1, 2", &Limits::default());

    assert!(
        matches!(parsed, Err(JsonFailure::Malformed(_))),
        "{parsed:?}"
    );
}

/// A zstd frame (RFC 8878) that regenerates `size` zero bytes from RLE
/// blocks, single-segment, so its window is its whole content.
fn zstd_rle(size: u32) -> Vec<u8> {
    let mut frame = 0xFD2F_B528_u32.to_le_bytes().to_vec();
    frame.push(0b1010_0000);
    frame.extend_from_slice(&size.to_le_bytes());
    rle_blocks(&mut frame, size);
    frame
}

/// A zstd frame (RFC 8878) that asks for a window of `2^window_log` bytes,
/// not single-segment, and regenerates `size` zero bytes from RLE blocks.
fn zstd_rle_windowed(window_log: u8, size: u32) -> Vec<u8> {
    let mut frame = 0xFD2F_B528_u32.to_le_bytes().to_vec();
    // No content size and no single segment; the Window_Descriptor's
    // exponent is the window's log less 10, its mantissa 0.
    frame.push(0);
    frame.push((window_log - 10) << 3);
    rle_blocks(&mut frame, size);
    frame
}

/// Appends RLE blocks of zero bytes, 128 KiB each at most, that regenerate
/// `size` bytes.
fn rle_blocks(frame: &mut Vec<u8>, size: u32) {
    let mut remaining = size;
    while remaining > 0 {
        let block = remaining.min(128 * 1024);
        remaining -= block;
        let header = (block << 3) | (1 << 1) | u32::from(remaining == 0);
        frame.extend_from_slice(&header.to_le_bytes()[..3]);
        frame.push(0);
    }
}

/// RFC 9659's window, 8 MiB, decodes.
#[test]
fn a_zstd_frame_whose_window_is_eight_mebibytes_is_decoded() {
    let received = zstd_rle_windowed(23, 10 * 1024 * 1024);
    let limits = Limits {
        max_decompression_ratio: 100_000,
        ..Limits::default()
    };

    let read = read(&["zstd"], &received, limits);

    assert_eq!(read.unwrap().len(), 10 * MIB);
}

/// A window past RFC 9659's 8 MiB fails to decode whatever
/// `network.max_body_size` allows, as Chromium refuses it
/// (`ERR_ZSTD_WINDOW_SIZE_TOO_BIG`), so every path gives one frame one
/// verdict (owner, 2026-10-28).
#[rstest]
#[case::single_segment_within_the_default_size(zstd_rle(10 * 1024 * 1024), Limits::default())]
#[case::sixteen_mebibytes_under_a_larger_size(
    zstd_rle_windowed(24, 1024 * 1024),
    Limits { max_body_size: 1 << 30, ..Limits::default() }
)]
#[case::past_a_small_size(
    zstd_rle(10 * 1024 * 1024),
    Limits { max_body_size: MIB as u64, ..Limits::default() }
)]
fn a_zstd_frame_whose_window_is_past_eight_mebibytes_fails_to_decode(
    #[case] received: Vec<u8>,
    #[case] limits: Limits,
) {
    let limits = Limits {
        max_decompression_ratio: 100_000,
        ..limits
    };

    let read = read(&["zstd"], &received, limits).map(|body| body.len());

    assert!(matches!(read, Err(BodyFailure::Decoding)), "{read:?}");
}

#[test]
fn a_refusal_is_response_limit_exceeded_with_the_url_the_setting_and_its_value() {
    let limits = Limits {
        max_body_size: 10,
        ..Limits::default()
    };
    let Err(BodyFailure::Limit(exceeded)) = read(&[], &[0; 11], limits) else {
        panic!("expected a limit to be passed");
    };

    let error = exceeded.error();

    assert_eq!(error.code(), "response_limit_exceeded");
    assert_eq!(
        serde_json::to_value(error.details()).unwrap(),
        json!({ "url": URL, "setting": "network.max_body_size", "limit": 10 })
    );
}
