use goodfetch_protocol::foxhound::crawl::{CrawlRequest, PushedRecord};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::foxhound::hook::{CrawlHookHandleParams, CrawlHookStage};
use goodfetch_protocol::run::RunId;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{Calling, raised};
use crate::delivery::failure::Why;

/// What a call sends, borrowed from the attempt, is written as
/// `crawl_hook.handle`'s params are, so the program reads it as them: with
/// its Record at the Record stage, and with none at a request's stage.
#[test]
fn a_hook_call_sends_crawl_hook_handles_params() {
    let at_record_stage = CrawlHookHandleParams {
        run_id: RunId::parse("run-1").unwrap(),
        hook: "stamp".to_owned(),
        stage: CrawlHookStage::BeforeRecordCommit,
        crawl_request: CrawlRequest {
            id: "r1".to_owned(),
            url: "https://example.com/".to_owned(),
            label: "detail".to_owned(),
            payload: Some(json!({ "page": 2 })),
            method: HttpMethod::get(),
            body: None,
        },
        attempt: 2,
        lease_epoch: 7,
        record: Some(PushedRecord {
            dataset: "products".to_owned(),
            data: json!({ "name": "Kettle" }),
        }),
    };
    let at_request_stage = CrawlHookHandleParams {
        stage: CrawlHookStage::BeforeRequest,
        record: None,
        ..at_record_stage.clone()
    };
    let calling = |params: &CrawlHookHandleParams| {
        serde_json::to_value(Calling {
            run_id: &params.run_id,
            hook: &params.hook,
            stage: &params.stage,
            crawl_request: &params.crawl_request,
            attempt: params.attempt,
            lease_epoch: params.lease_epoch,
            record: params.record.as_ref(),
        })
        .unwrap()
    };

    assert_eq!(
        (calling(&at_record_stage), calling(&at_request_stage)),
        (
            serde_json::to_value(&at_record_stage).unwrap(),
            serde_json::to_value(&at_request_stage).unwrap()
        )
    );
}

fn details(value: Value) -> Map<String, Value> {
    let Value::Object(details) = value else {
        panic!("details are an object");
    };
    details
}

/// A Hook's failure becomes the attempt's `hook_failed`, naming the stage,
/// the Hook and the Label: the program's own exception keeps what it says
/// of itself and of the code it wraps, and is retried as the handler's
/// failures are; any other code is wrapped and retried by its own flag.
#[rstest]
#[case::own_exception(
    ("hook_failed", true),
    json!({ "exception_type": "TypeError", "exception_message": "no price", "exception_stack": "at stamp" }),
    json!({ "exception_type": "TypeError", "exception_message": "no price", "exception_stack": "at stamp", "wrapped_code": null }),
    Why::Hook
)]
#[case::own_exception_as_a_handler_answers_it(
    ("handler_failed", true),
    json!({ "exception_type": "TypeError", "exception_message": "no price", "wrapped_code": "kv_conflict" }),
    json!({ "exception_type": "TypeError", "exception_message": "no price", "exception_stack": "", "wrapped_code": null }),
    Why::Hook
)]
#[case::wrapped_code_carried(
    ("hook_failed", false),
    json!({ "exception_type": "GoodfetchError", "exception_message": "gone", "wrapped_code": "proxy_unavailable" }),
    json!({ "exception_type": "GoodfetchError", "exception_message": "gone", "exception_stack": "", "wrapped_code": "proxy_unavailable" }),
    Why::Other
)]
#[case::another_code_wrapped(
    ("kv_conflict", false),
    json!({}),
    json!({ "exception_type": "kv_conflict", "exception_message": "the write lost", "exception_stack": "", "wrapped_code": "kv_conflict" }),
    Why::Other
)]
fn a_hooks_failure_is_the_attempts_hook_failed(
    #[case] (code, retryable): (&str, bool),
    #[case] given: Value,
    #[case] expected: Value,
    #[case] why: Why,
) {
    let failure = raised(
        &CrawlHookStage::BeforeRecordCommit,
        ("stamp", "detail"),
        (code.to_owned(), retryable),
        "the write lost",
        &details(given),
    );

    let mut expected = details(expected);
    expected.insert("stage".to_owned(), json!("before_record_commit"));
    expected.insert("hook".to_owned(), json!("stamp"));
    expected.insert("label".to_owned(), json!("detail"));
    assert_eq!(
        (
            failure.why() == &why,
            failure.code.as_str(),
            failure.retryable,
            failure.details,
        ),
        (true, "hook_failed", retryable, expected)
    );
}
