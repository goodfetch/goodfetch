use rstest::rstest;

use crate::sniffing::sniff;

/// A response with no `Content-Type` is typed as Chromium's MIME sniffer
/// (`net/base/mime_sniffer.cc`) types it: HTML by its first tag after
/// leading spaces within 512 bytes, `<?xml` as XML, a PDF, an image or an
/// archive by its magic number, an Office document by its magic number and
/// its path's extension, and otherwise plain text unless a control byte in
/// its first 1,024 bytes makes it binary.
///
/// Covers: story:5-129
#[rstest]
#[case::a_doctype(b"<!DOCTYPE html><p>x".as_slice(), "/", "text/html")]
#[case::spaces_then_a_tag_in_any_case(b" \t\r\n\x0b\x0c<HtMl><body>x".as_slice(), "/", "text/html")]
#[case::a_tag_prefix(b"<bdi>x</bdi>".as_slice(), "/", "text/html")]
#[case::a_comment(b"<!-- x -->".as_slice(), "/", "text/html")]
#[case::an_xml_declaration(b"<?xml version=\"1.0\"?><a/>".as_slice(), "/", "text/xml")]
#[case::an_xml_declaration_in_another_case(b"<?XML version=\"1.0\"?>".as_slice(), "/", "text/plain")]
#[case::a_tag_chromium_does_not_sniff(b"<section>x</section>".as_slice(), "/", "text/plain")]
#[case::plain_text(b"just words\n".as_slice(), "/", "text/plain")]
#[case::empty(b"".as_slice(), "/", "text/plain")]
#[case::a_control_byte(b"words\x00more".as_slice(), "/", "application/octet-stream")]
#[case::a_bom_before_a_control_byte(b"\xEF\xBB\xBFwords\x01".as_slice(), "/", "text/plain")]
#[case::a_pdf(b"%PDF-1.7\n".as_slice(), "/", "application/pdf")]
#[case::a_png(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice(), "/", "image/png")]
#[case::a_webp(b"RIFF\x10\0\0\0WEBPVP8 ".as_slice(), "/", "image/webp")]
#[case::a_gzip_stream(b"\x1f\x8b\x08\0".as_slice(), "/", "application/x-gzip")]
#[case::a_zip(b"PK\x03\x04\x14\0".as_slice(), "/archive.zip", "application/zip")]
#[case::a_docx(
    b"PK\x03\x04\x14\0".as_slice(),
    "/report.DOCX",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
)]
#[case::a_script_line(b"#!/bin/sh\necho".as_slice(), "/", "text/plain")]
#[case::an_executable(b"MZ\x90\0".as_slice(), "/", "application/octet-stream")]
#[case::html_past_512_bytes(&[b' '; 600].iter().chain(b"<html>").copied().collect::<Vec<u8>>(), "/", "text/plain")]
#[case::a_control_byte_past_1024_bytes(
    &[b'a'; 1024].iter().chain(b"\x00").copied().collect::<Vec<u8>>(),
    "/",
    "text/plain"
)]
fn a_response_with_no_type_is_typed_as_chromium_sniffs_it(
    #[case] body: &[u8],
    #[case] path: &str,
    #[case] expected: &str,
) {
    assert_eq!(sniff(body, path), expected);
}
