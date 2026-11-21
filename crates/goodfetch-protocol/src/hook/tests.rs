use serde_json::json;

use super::HookFailedDetails;

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
