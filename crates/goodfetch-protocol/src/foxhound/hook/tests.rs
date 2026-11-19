use rstest::rstest;
use serde_json::json;

use super::{HookDuplicateDetails, HookFailedDetails, HookIneffectiveDetails, HookUnknownDetails};
use crate::catalog::{ErrorGroup, Product};
use crate::entries::MODULES;

#[rstest]
#[case::duplicate("hook_duplicate", false)]
#[case::ineffective("hook_ineffective", false)]
#[case::failed("hook_failed", true)]
#[case::unknown("hook_unknown", false)]
fn foxhound_declares_each_hook_code_once_as_a_program_error(
    #[case] name: &str,
    #[case] retryable: bool,
) {
    let declared: Vec<_> = MODULES
        .into_iter()
        .flat_map(|entries| {
            entries
                .error_codes
                .iter()
                .filter(|code| code.name == name)
                .map(|code| (Product::of_module(entries.defined_in), code))
        })
        .collect();

    let [(product, code)] = declared.as_slice() else {
        panic!("`{name}` is declared {} times", declared.len());
    };
    assert_eq!(*product, Product::Foxhound);
    assert_eq!(code.group, ErrorGroup::Program);
    assert_eq!(code.retryable, retryable);
    assert_eq!(code.exit_code, None);
}

#[test]
fn hook_duplicate_names_the_hook_and_its_stage() {
    let details = HookDuplicateDetails {
        hook: "addToken".to_owned(),
        stage: "before_request".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({ "hook": "addToken", "stage": "before_request" })
    );
}

#[test]
fn hook_ineffective_names_the_hook_and_its_stage() {
    let details = HookIneffectiveDetails {
        hook: "before_navigation#1".to_owned(),
        stage: "before_navigation".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({ "hook": "before_navigation#1", "stage": "before_navigation" })
    );
}

#[test]
fn hook_failed_names_the_hook_its_label_the_exception_and_the_wrapped_code() {
    let details = HookFailedDetails {
        stage: "after_response".to_owned(),
        hook: "checkPrice".to_owned(),
        label: Some("product".to_owned()),
        exception_type: "TypeError".to_owned(),
        exception_message: "price is undefined".to_owned(),
        exception_stack: "at checkPrice (crawl.ts:12:5)".to_owned(),
        wrapped_code: Some("header_forbidden".to_owned()),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({
            "stage": "after_response",
            "hook": "checkPrice",
            "label": "product",
            "exception_type": "TypeError",
            "exception_message": "price is undefined",
            "exception_stack": "at checkPrice (crawl.ts:12:5)",
            "wrapped_code": "header_forbidden",
        })
    );
}

#[test]
fn hook_failed_at_a_lifecycle_stage_for_the_programs_own_exception_writes_both_nulls() {
    let details = HookFailedDetails {
        stage: "run_started".to_owned(),
        hook: "openLog".to_owned(),
        label: None,
        exception_type: "BoxError".to_owned(),
        exception_message: "log folder missing".to_owned(),
        exception_stack: String::new(),
        wrapped_code: None,
    };

    let written = serde_json::to_value(&details).unwrap();

    assert_eq!(written["label"], json!(null));
    assert_eq!(written["wrapped_code"], json!(null));
    assert_eq!(
        serde_json::from_value::<HookFailedDetails>(written).unwrap(),
        details
    );
}

#[rstest]
#[case::closest(Some("addToken"), json!({ "hook": "addTokn", "closest": "addToken" }))]
#[case::none_held(None, json!({ "hook": "addTokn", "closest": null }))]
fn hook_unknown_names_the_hook_asked_for_and_the_closest(
    #[case] closest: Option<&str>,
    #[case] expected: serde_json::Value,
) {
    let details = HookUnknownDetails {
        hook: "addTokn".to_owned(),
        closest: closest.map(str::to_owned),
    };

    assert_eq!(serde_json::to_value(&details).unwrap(), expected);
}
