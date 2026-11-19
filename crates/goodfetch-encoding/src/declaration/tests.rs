use encoding_rs::{Encoding, SHIFT_JIS, UTF_8, WINDOWS_1251, WINDOWS_1252};
use rstest::rstest;

use super::{prescan, xml_declaration};

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
