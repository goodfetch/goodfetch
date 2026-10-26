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
