use encoding_rs::{Encoding, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1251, WINDOWS_1252};
use rstest::rstest;

use crate::charset::{Kind, Read, Source, content_type, decode, prescan, read, xml_declaration};

/// 日本 in `Shift_JIS`, which is no UTF-8.
const NIHON: &[u8] = &[0x93, 0xFA, 0x96, 0x7B];

/// `line` repeated past the first 1,024 bytes of a document.
fn past_1024(line: &str) -> String {
    line.repeat(1024 / line.len() + 2)
}

/// An HTML document whose head holds `head` and whose body holds `body`,
/// with the `Shift_JIS` text 日本 in its body.
fn html(head: &str, body: &str) -> Vec<u8> {
    let mut bytes = format!("<!doctype html><html><head>{head}</head><body>{body}<p>").into_bytes();
    bytes.extend_from_slice(NIHON);
    bytes.extend_from_slice(b"</p></body></html>");
    bytes
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "each case of the table names the order's answer, an `Option`, as written"
)]
const fn found(encoding: &'static Encoding, source: Source) -> Option<Read> {
    Some(Read { encoding, source })
}

/// The order: a BOM, `fetch.encoding`, the `Content-Type` charset as WHATWG
/// Encoding labels it, Chrome's `<meta>` prescan (or, for an XML type, its
/// XML declaration), then UTF-8 for a valid UTF-8 body; anything else is
/// only a guess, `None`.
///
/// Covers: story:5-129, story:6-233, setting:fetch.encoding
#[rstest]
#[case::a_utf_8_bom_beats_the_charset(
    b"\xEF\xBB\xBF<p>x".to_vec(), Kind::Html, Some("Shift_JIS"), None,
    found(UTF_8, Source::Bom)
)]
#[case::a_utf_16_bom(b"\xFF\xFE<\0p\0".to_vec(), Kind::Html, None, None, found(UTF_16LE, Source::Bom))]
#[case::a_big_endian_bom(b"\xFE\xFF\0<".to_vec(), Kind::Html, None, None, found(UTF_16BE, Source::Bom))]
#[case::a_bom_beats_the_setting(
    b"\xEF\xBB\xBFx".to_vec(), Kind::Html, None, Some(SHIFT_JIS),
    found(UTF_8, Source::Bom)
)]
#[case::the_setting_beats_the_charset(
    html("", ""),
    Kind::Html,
    Some("utf-8"),
    Some(SHIFT_JIS),
    found(SHIFT_JIS, Source::Setting)
)]
#[case::the_charset(
    html("", ""),
    Kind::Html,
    Some("Shift_JIS"),
    None,
    found(SHIFT_JIS, Source::Header)
)]
#[case::a_label_alias_as_whatwg_maps_it(
    b"\xE9".to_vec(), Kind::Html, Some("latin1"), None,
    found(WINDOWS_1252, Source::Header)
)]
#[case::the_charset_beats_the_meta(
    html("<meta charset=\"windows-1251\">", ""),
    Kind::Html,
    Some("shift_jis"),
    None,
    found(SHIFT_JIS, Source::Header)
)]
#[case::an_unknown_charset_is_none(
    html("<meta charset=\"shift_jis\">", ""),
    Kind::Html,
    Some("klingon"),
    None,
    found(SHIFT_JIS, Source::Meta)
)]
#[case::a_meta_charset(
    html("<meta charset=\"shift_jis\">", ""),
    Kind::Html,
    None,
    None,
    found(SHIFT_JIS, Source::Meta)
)]
#[case::a_meta_http_equiv(
    html(
        "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=shift_jis\">",
        ""
    ),
    Kind::Html,
    None,
    None,
    found(SHIFT_JIS, Source::Meta)
)]
#[case::the_meta_of_a_type_with_no_meta(
    html("<meta charset=\"shift_jis\">", ""),
    Kind::Other,
    None,
    None,
    None
)]
#[case::an_xhtml_xml_declaration(
    {
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"windows-1251\"?><html><body>".to_vec();
        bytes.extend_from_slice(&[0xC0, 0xE1]);
        bytes
    },
    Kind::Xml, None, None, found(WINDOWS_1251, Source::Meta)
)]
#[case::an_xml_type_reads_no_meta(
    html("<meta charset=\"shift_jis\">", ""),
    Kind::Xml,
    None,
    None,
    None
)]
#[case::undeclared_utf_8("<p>日本</p>".as_bytes().to_vec(), Kind::Html, None, None, found(UTF_8, Source::Utf8))]
#[case::undeclared_ascii(b"<p>x</p>".to_vec(), Kind::Html, None, None, found(UTF_8, Source::Utf8))]
#[case::an_empty_body(Vec::new(), Kind::Other, None, None, found(UTF_8, Source::Utf8))]
#[case::undeclared_legacy_bytes_are_only_a_guess(html("", ""), Kind::Html, None, None, None)]
fn a_documents_encoding_is_found_in_one_order(
    #[case] body: Vec<u8>,
    #[case] kind: Kind,
    #[case] charset: Option<&str>,
    #[case] forced: Option<&'static Encoding>,
    #[case] expected: Option<Read>,
) {
    assert_eq!(read(&body, kind, charset, forced), expected);
}

/// Chrome's `<meta>` prescan (Blink's `HTMLMetaCharsetParser`): the first
/// 1,024 bytes always, then on while only tags allowed in the head appear,
/// with the HTML standard's labels: UTF-16 reads as UTF-8 and
/// `x-user-defined` as windows-1252; a declaration in a comment, a title or
/// a script is no declaration, and an unknown label is skipped.
///
/// Covers: story:5-129
#[rstest]
#[case::a_charset(html("<meta charset=shift_jis>", ""), Some(SHIFT_JIS))]
#[case::a_quoted_charset_in_any_case(html("<META CharSet='Shift_JIS'>", ""), Some(SHIFT_JIS))]
#[case::a_pragma(
    html(
        "<meta content=\"text/html;charset='shift_jis'\" http-equiv=content-type>",
        ""
    ),
    Some(SHIFT_JIS)
)]
#[case::a_content_without_its_pragma(
    html("<meta content=\"text/html; charset=shift_jis\">", ""),
    None
)]
#[case::a_utf_16_label(html("<meta charset=utf-16le>", ""), Some(UTF_8))]
#[case::x_user_defined(html("<meta charset=x-user-defined>", ""), Some(WINDOWS_1252))]
#[case::an_unknown_label_then_a_known_one(
    html("<meta charset=klingon><meta charset=windows-1251>", ""),
    Some(WINDOWS_1251)
)]
#[case::in_a_comment(html("<!-- <meta charset=shift_jis> -->", ""), None)]
#[case::in_a_title(html("<title><meta charset=shift_jis></title>", ""), None)]
#[case::in_a_script(html("<script>'<meta charset=shift_jis>'</script>", ""), None)]
#[case::late_among_head_tags(
    html(&format!("{}<meta charset=windows-1251>", past_1024("<meta name=k content=v>")), ""),
    Some(WINDOWS_1251)
)]
#[case::after_the_body_in_the_first_1024_bytes(
    html("", "<meta charset=windows-1251>"),
    Some(WINDOWS_1251)
)]
#[case::after_the_body_past_1024_bytes(
    html("", &format!("{}<meta charset=windows-1251>", past_1024("<p>filler</p>"))),
    None
)]
#[case::an_unclosed_tag(b"<meta charset=shift_jis".to_vec(), None)]
fn the_prescan_reads_a_meta_as_chrome_does(
    #[case] body: Vec<u8>,
    #[case] expected: Option<&'static Encoding>,
) {
    assert_eq!(prescan(&body), expected);
}

/// The XML declaration, as Blink's `TextResourceDecoder` reads it: at the
/// very start, its `encoding` quoted, a UTF-16 label read as UTF-8.
///
/// Covers: story:5-129
#[rstest]
#[case::double_quoted(b"<?xml version=\"1.0\" encoding=\"Shift_JIS\"?><a/>".as_slice(), Some(SHIFT_JIS))]
#[case::single_quoted_with_spaces(b"<?xml version='1.0' encoding = 'windows-1251' ?>".as_slice(), Some(WINDOWS_1251))]
#[case::utf_16(b"<?xml version=\"1.0\" encoding=\"UTF-16\"?>".as_slice(), Some(UTF_8))]
#[case::no_encoding(b"<?xml version=\"1.0\"?><a/>".as_slice(), None)]
#[case::unquoted(b"<?xml version=\"1.0\" encoding=Shift_JIS?>".as_slice(), None)]
#[case::not_at_the_start(b" <?xml version=\"1.0\" encoding=\"Shift_JIS\"?>".as_slice(), None)]
#[case::unclosed(b"<?xml version=\"1.0\" encoding=\"Shift_JIS\"".as_slice(), None)]
fn the_xml_declaration_names_an_encoding(
    #[case] body: &[u8],
    #[case] expected: Option<&'static Encoding>,
) {
    assert_eq!(xml_declaration(body), expected);
}

/// A `Content-Type` value's essence, lowercase, and its `charset`
/// parameter, in any case and unquoted.
///
/// Covers: story:6-233
#[rstest]
#[case::a_charset("text/html; charset=Shift_JIS", "text/html", Some("Shift_JIS"))]
#[case::quoted_in_another_case("Text/HTML;Charset=\"shift_jis\"", "text/html", Some("shift_jis"))]
#[case::another_parameter_first(
    "application/xhtml+xml; q=1 ; charset = utf-8",
    "application/xhtml+xml",
    Some("utf-8")
)]
#[case::none(" text/plain ", "text/plain", None)]
fn a_content_type_names_its_essence_and_charset(
    #[case] value: &str,
    #[case] essence: &str,
    #[case] charset: Option<&str>,
) {
    assert_eq!(content_type(value), (essence.to_owned(), charset));
}

/// A document is decoded in the encoding the order found, its BOM left
/// out, a byte that does not decode replaced by U+FFFD.
///
/// Covers: story:5-129
#[rstest]
#[case::without_its_bom(b"\xEF\xBB\xBFx\xFF".as_slice(), found(UTF_8, Source::Bom), "x\u{FFFD}")]
#[case::shift_jis(NIHON, found(SHIFT_JIS, Source::Setting), "日本")]
fn a_document_is_decoded_in_the_encoding_found(
    #[case] body: &[u8],
    #[case] found: Option<Read>,
    #[case] text: &str,
) {
    assert_eq!(decode(body, found.unwrap()), text);
}

#[path = "tests/stream.rs"]
mod stream;

mod prescan_target {
    goodfetch_test_support::fuzz_replay!(crate::charset::fuzz_prescan, "fuzz/prescan");
}
