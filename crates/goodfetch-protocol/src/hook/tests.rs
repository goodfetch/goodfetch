use serde_json::json;

use super::{HookFailedDetails, ReportedHookStage};

#[test]
fn hook_failed_names_the_hook_its_label_the_exception_and_the_wrapped_code() {
    let details = HookFailedDetails {
        stage: ReportedHookStage::AfterResponse,
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
fn hook_failed_at_a_lifecycle_stage_for_the_programs_own_exception_keeps_the_stage_and_both_nulls()
{
    let written = json!({
        "stage": "run_started",
        "hook": "openLog",
        "label": null,
        "exception_type": "BoxError",
        "exception_message": "log folder missing",
        "exception_stack": "",
        "wrapped_code": null,
    });

    let details = serde_json::from_value::<HookFailedDetails>(written.clone()).unwrap();

    assert!(matches!(details.stage, ReportedHookStage::Unknown(_)));
    assert_eq!(details.stage.as_str(), "run_started");
    assert_eq!(details.label, None);
    assert_eq!(details.wrapped_code, None);
    assert_eq!(serde_json::to_value(&details).unwrap(), written);
}
