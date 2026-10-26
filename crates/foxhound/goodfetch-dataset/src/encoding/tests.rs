use goodfetch_protocol::foxhound::export::ExportFormat;
use rstest::rstest;

use super::ExportEncoder;

fn encoded(format: ExportFormat, values: &[&str]) -> String {
    let mut encoder = ExportEncoder::new(format);
    let mut out = Vec::new();
    encoder.begin(&mut out);
    for value in values {
        encoder.value(value, &mut out);
    }
    encoder.end(&mut out);
    String::from_utf8(out).unwrap()
}

#[rstest]
#[case::json_of_none(ExportFormat::Json, &[], "[]")]
#[case::jsonl_of_none(ExportFormat::Jsonl, &[], "")]
#[case::json_of_two(
    ExportFormat::Json,
    &[r#"{"title":"Café"}"#, r#"{"title":"Ünïcode"}"#],
    r#"[{"title":"Café"},{"title":"Ünïcode"}]"#
)]
#[case::jsonl_of_two(
    ExportFormat::Jsonl,
    &[r#"{"title":"Café"}"#, r#"{"title":"Ünïcode"}"#],
    "{\"title\":\"Café\"}\n{\"title\":\"Ünïcode\"}\n"
)]
fn an_export_writes_each_value_as_committed_in_its_format(
    #[case] format: ExportFormat,
    #[case] values: &[&str],
    #[case] expected: &str,
) {
    assert_eq!(encoded(format, values), expected);
}

/// The Export profile's corpus files (ADR 0066), as `xtask compat write`
/// wrote them.
const HEAD_JSON: &str = include_str!("../../../../../compat/export/head/books.json");
const HEAD_JSONL: &str = include_str!("../../../../../compat/export/head/books.jsonl");

/// The corpus's Exports read back as a consumer reads them, each line of
/// the JSONL file one value, and written again, give the same bytes in
/// both formats: the profile holds each value as JSON writes it, in order,
/// with no BOM.
///
/// Covers: format:export
#[test]
fn the_export_profiles_head_set_reads_back_and_writes_the_same_bytes() {
    let values: Vec<String> = HEAD_JSONL
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap()
                .to_string()
        })
        .collect();
    let read: Vec<&str> = values.iter().map(String::as_str).collect();
    let array: Vec<serde_json::Value> = serde_json::from_str(HEAD_JSON).unwrap();

    assert_eq!(
        (
            encoded(ExportFormat::Json, &read),
            encoded(ExportFormat::Jsonl, &read),
            array.len(),
        ),
        (HEAD_JSON.to_owned(), HEAD_JSONL.to_owned(), 3)
    );
}
