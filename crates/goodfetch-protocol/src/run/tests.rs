use rstest::rstest;

use super::{RunId, RunIdInvalid, Tags};

#[rstest]
#[case::one_letter("a")]
#[case::one_digit("7")]
#[case::every_allowed_character("my-run.v2_1")]
#[case::dots_inside("a..b")]
#[case::a_ulid("01jbz3kq8m5v2c7x9d4e6f0g1h")]
#[case::reserved_name_inside_another("console")]
#[case::longest(&"a".repeat(128))]
fn a_run_id_of_safe_characters_reads(#[case] text: &str) {
    assert_eq!(
        RunId::parse(text).map(|id| id.as_str().to_owned()),
        Ok(text.to_owned())
    );
}

/// TESTING.md's path traversal cases, each OS's own included (ADR 0067):
/// a Run ID names a folder, so none reaches outside the run folder.
#[rstest]
#[case::empty("", RunIdInvalid::Characters)]
#[case::current_folder(".", RunIdInvalid::Characters)]
#[case::parent_folder("..", RunIdInvalid::Characters)]
#[case::parent_then_name("../escape", RunIdInvalid::Characters)]
#[case::slash("a/b", RunIdInvalid::Characters)]
#[case::backslash("a\\b", RunIdInvalid::Characters)]
#[case::absolute_unix("/etc", RunIdInvalid::Characters)]
#[case::windows_drive("c:", RunIdInvalid::Characters)]
#[case::windows_absolute("c:\\windows", RunIdInvalid::Characters)]
#[case::windows_unc("\\\\server\\share", RunIdInvalid::Characters)]
#[case::alternate_data_stream("a:stream", RunIdInvalid::Characters)]
#[case::trailing_dot("a.", RunIdInvalid::Characters)]
#[case::trailing_space("a ", RunIdInvalid::Characters)]
#[case::leading_dot(".hidden", RunIdInvalid::Characters)]
#[case::leading_hyphen("-a", RunIdInvalid::Characters)]
#[case::uppercase("Run", RunIdInvalid::Characters)]
#[case::non_ascii("été", RunIdInvalid::Characters)]
#[case::nul_byte("a\0b", RunIdInvalid::Characters)]
#[case::too_long(&"a".repeat(129), RunIdInvalid::Characters)]
#[case::windows_device("con", RunIdInvalid::Reserved)]
#[case::windows_device_with_extension("nul.txt", RunIdInvalid::Reserved)]
#[case::windows_serial_port("com1", RunIdInvalid::Reserved)]
#[case::windows_printer_port_with_extension("lpt9.log", RunIdInvalid::Reserved)]
fn a_run_id_that_could_name_another_folder_is_refused(
    #[case] text: &str,
    #[case] expected: RunIdInvalid,
) {
    assert_eq!(RunId::parse(text), Err(expected));
}

#[rstest]
#[case::sixteen_tags_of_sixty_four_characters(
    serde_json::json!(vec!["é".repeat(64); 16]),
    true
)]
#[case::seventeen_tags(serde_json::json!(vec!["t"; 17]), false)]
#[case::a_tag_of_sixty_five_characters(serde_json::json!(["t".repeat(65)]), false)]
fn tags_hold_at_most_sixteen_of_sixty_four_characters(
    #[case] tags: serde_json::Value,
    #[case] read: bool,
) {
    assert_eq!(serde_json::from_value::<Tags>(tags).is_ok(), read);
}
