use rstest::rstest;

use super::{Kind, content_type};

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

/// HTML reads the `<meta>` prescan, an XML type its XML declaration, any
/// other type nothing.
///
/// Covers: story:5-129
#[rstest]
#[case::html("text/html", Kind::Html)]
#[case::xhtml("application/xhtml+xml", Kind::Xml)]
#[case::text_xml("text/xml", Kind::Xml)]
#[case::a_plus_xml_type("image/svg+xml", Kind::Xml)]
#[case::json("application/json", Kind::Other)]
fn a_type_reads_its_own_declaration(#[case] essence: &str, #[case] kind: Kind) {
    assert_eq!(Kind::of(essence), kind);
}
