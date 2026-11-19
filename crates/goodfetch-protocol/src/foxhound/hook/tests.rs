use rstest::rstest;
use serde_json::json;

use super::{HookDuplicateDetails, HookIneffectiveDetails, HookUnknownDetails};
use crate::catalog::Product;
use crate::entries::MODULES;

#[rstest]
#[case::duplicate("hook_duplicate", Product::Foxhound)]
#[case::ineffective("hook_ineffective", Product::Foxhound)]
#[case::failed_also_by_a_collie_script_hook("hook_failed", Product::Collie)]
#[case::unknown("hook_unknown", Product::Foxhound)]
fn each_hook_code_is_declared_once_in_the_smallest_product_that_raises_it(
    #[case] name: &str,
    #[case] expected: Product,
) {
    let declared: Vec<_> = MODULES
        .into_iter()
        .flat_map(|entries| {
            entries
                .error_codes
                .iter()
                .filter(|code| code.name == name)
                .map(|_| Product::of_module(entries.defined_in))
        })
        .collect();

    assert_eq!(
        declared,
        [expected],
        "`{name}` is declared in these Products"
    );
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
