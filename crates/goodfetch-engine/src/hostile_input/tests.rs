//! The hostile-input tables (ADR 0067) through the checks' interface: every
//! header value Goodfetch would send, and every name or path it would write
//! under, from every OS's cases.
//! Covers: error:header_value_invalid, error:path_unsafe
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::hostile_input::{HeaderCharacter, PathHazard};
use goodfetch_test_support::{HeaderSource, HostileTables, PathTarget};
use rstest::rstest;
use serde_json::json;

use super::{PathShape, check_header_value, check_headers, check_path};

/// The shape of name each target takes: one name, or a path under its
/// folder.
fn shape(target: PathTarget) -> Option<PathShape> {
    match target {
        PathTarget::ExportName | PathTarget::KeyValueStoreKey | PathTarget::PlanName => {
            Some(PathShape::Name)
        }
        PathTarget::ScreenshotPath | PathTarget::RunTracePath => Some(PathShape::RelativePath),
        // A Plugin bundle's entries are judged by the bundle's reader.
        PathTarget::PluginBundleEntry => None,
    }
}

/// Every header case of the committed table whose value Goodfetch would
/// send, from every source, is refused naming the header: the table runs
/// at this layer, every miss collected before the test fails. The cases
/// come from the table alone, so a case it gains runs here too; the test
/// only checks that every source Goodfetch sends from is among them.
#[test]
fn every_header_value_the_table_names_is_refused() {
    let tables = HostileTables::committed().unwrap();
    let sent: Vec<_> = tables
        .header_values
        .cases
        .iter()
        .filter(|case| case.input.source != HeaderSource::Response)
        .collect();

    let misses: Vec<String> = sent
        .iter()
        .filter(|case| check_header_value(&case.input.name, &case.input.value).is_ok())
        .map(|case| case.name.clone())
        .collect();

    for source in [
        HeaderSource::Recipe,
        HeaderSource::Sink,
        HeaderSource::Webhook,
        HeaderSource::PageRequest,
        HeaderSource::Route,
    ] {
        assert!(
            sent.iter().any(|case| case.input.source == source),
            "the table has no header case sent from {source:?}"
        );
    }
    assert_eq!(misses, Vec::<String>::new());
}

#[rstest]
#[case::crlf("a\r\nX-Injected: yes", HeaderCharacter::Cr)]
#[case::lf("a\nX-Injected: yes", HeaderCharacter::Lf)]
#[case::cr("a\rX-Injected: yes", HeaderCharacter::Cr)]
#[case::nul("a\u{0}b", HeaderCharacter::Nul)]
fn a_header_value_with_a_line_break_or_a_nul_names_the_header_and_the_character(
    #[case] value: &str,
    #[case] character: HeaderCharacter,
) {
    let refused = check_header_value("X-Note", value).unwrap_err();

    assert_eq!(
        serde_json::to_value(refused.error().details()).unwrap(),
        json!({ "name": "X-Note", "character": character })
    );
    assert_eq!(refused.error().code(), "header_value_invalid");
}

#[rstest]
#[case::plain("a note")]
#[case::a_tab_and_spaces("a b\tc")]
#[case::utf8("café ☕")]
#[case::empty("")]
fn a_plain_header_value_is_sent(#[case] value: &str) {
    assert!(check_header_value("X-Note", value).is_ok());
}

#[test]
fn headers_are_judged_one_by_one_until_one_is_refused() {
    let headers = [
        ("Accept", "text/html"),
        ("X-Note", "a\nb"),
        ("X-Other", "\0"),
    ];

    let refused = check_headers(headers).unwrap_err();

    assert_eq!(refused.name, "X-Note");
}

/// Every path case of the committed table for a target Goodfetch names
/// from outside input is refused with the hazard the table names, on every
/// OS alike, as the refusal rule is portable: each OS's cases (Windows'
/// reserved names, streams, UNC and verbatim paths, macOS's decomposed
/// names) on all three. The cases come from the table alone, so a case it
/// gains runs here too; the test only checks that every target judged
/// here is among them.
#[test]
fn every_path_case_the_table_names_is_refused_with_its_hazard_on_every_os() {
    let tables = HostileTables::committed().unwrap();
    let mut judged = Vec::new();
    let mut misses = Vec::new();

    for case in &tables.paths.cases {
        for target in &case.input.targets {
            let Some(shape) = shape(*target) else {
                continue;
            };
            judged.push(*target);
            let expected = serde_json::to_value(case.input.hazard).unwrap();
            match check_path(&case.input.name, shape) {
                Ok(()) => misses.push(format!("{} as {target:?}: kept", case.name)),
                Err(refused) => {
                    let hazard = serde_json::to_value(refused.hazard).unwrap();
                    if hazard != expected {
                        misses.push(format!(
                            "{} as {target:?}: {hazard} for {expected}",
                            case.name
                        ));
                    }
                }
            }
        }
    }

    for target in [
        PathTarget::ExportName,
        PathTarget::KeyValueStoreKey,
        PathTarget::PlanName,
        PathTarget::ScreenshotPath,
        PathTarget::RunTracePath,
    ] {
        assert!(
            judged.contains(&target),
            "the table has no path case for {target:?}"
        );
    }
    assert_eq!(misses, Vec::<String>::new());
}

#[rstest]
#[case::a_plain_name("report.csv", PathShape::Name)]
#[case::a_composed_name("caf\u{e9}.csv", PathShape::Name)]
#[case::a_dotted_name(".hidden.json", PathShape::Name)]
#[case::a_name_with_spaces_inside("my report.csv", PathShape::Name)]
#[case::a_name_255_bytes_long(&"a".repeat(255), PathShape::Name)]
#[case::a_reserved_word_inside_a_name("console.log", PathShape::Name)]
#[case::a_path_to_a_subfolder("shots/home.png", PathShape::RelativePath)]
#[case::a_deeper_path("traces/2026/run.zip", PathShape::RelativePath)]
fn a_safe_name_or_path_is_kept(#[case] name: &str, #[case] shape: PathShape) {
    let checked = check_path(name, shape);

    assert!(checked.is_ok(), "{checked:?}");
}

#[rstest]
#[case::a_separator_in_a_name("shots/home.png", PathHazard::Separator)]
#[case::a_backslash_in_a_name("shots\\home.png", PathHazard::Separator)]
#[case::a_trailing_separator("shots/", PathHazard::Separator)]
fn a_path_where_one_name_is_taken_is_refused(#[case] name: &str, #[case] hazard: PathHazard) {
    let refused = check_path(name, PathShape::Name).unwrap_err();

    assert_eq!(refused.hazard, hazard);
}

#[test]
fn a_refused_path_is_path_unsafe_naming_it_and_its_hazard() {
    let refused = check_path("../../report.csv", PathShape::RelativePath).unwrap_err();

    let error = refused.error();

    assert_eq!(error.code(), "path_unsafe");
    assert_eq!(
        serde_json::to_value(error.details()).unwrap(),
        json!({ "name": "../../report.csv", "hazard": "parent_traversal" })
    );
}
