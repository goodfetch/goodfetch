#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fmt;
use std::time::Duration;

use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::checkpoint::PathNotWritableDetails;
use goodfetch_protocol::error_code::ErrorCode;
use goodfetch_protocol::hook::HookFailedDetails;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::model::{ModelUnavailableDetails, ModelUnavailableReason};
use goodfetch_protocol::page::BlockedDetails;
use goodfetch_protocol::page::ai_work::PageNavigatedDuringCallDetails;
use goodfetch_protocol::secrets::{SecretFailure, SecretUnavailableDetails};
use goodfetch_protocol::settings::{RunOrigin, SettingLimit, SettingsLayer};
use serde_json::{Map, Value, json};

use super::{BoxError, Error};

/// The code a handler's own error fails with (ADR 0079), which this crate
/// names by value, since only Foxhound's crates name its constants.
const HANDLER_FAILED: ErrorCode = ErrorCode::from_static("handler_failed");

/// The engine's answer `code`, in `group`'s class, with `details`.
fn answer(code: &str, group: ErrorGroup, details: &Value) -> Error {
    let Value::Object(details) = details.clone() else {
        panic!("details are an object");
    };
    Error::answered(ErrorObject {
        code: group.json_rpc_class(),
        message: format!("failed with {code}"),
        data: ErrorData {
            code: code.to_owned(),
            retryable: true,
            details,
        },
    })
}

fn hook_failed_details() -> Value {
    json!({
        "stage": "before_request",
        "hook": "sign",
        "label": null,
        "exception_type": "BoxError",
        "exception_message": "bad key",
        "exception_stack": "",
        "wrapped_code": null,
        "operation": "crawl.run",
        "fix": "Fix the Hook.",
    })
}

/// A program's own handler, which `?` turns any error into a box in.
fn parse_count(text: &str) -> Result<u32, BoxError> {
    Ok(text.parse::<u32>()?)
}

/// A program's own error whose cause is `source`.
#[derive(Debug)]
struct Wrapping(Error);

impl fmt::Display for Wrapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("could not sign")
    }
}

impl std::error::Error for Wrapping {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Covers: story:11-30, story:11-36, story:11-187, error:hook_failed
#[test]
fn an_answer_keeps_its_code_group_flag_and_fields_and_reads_its_details_by_type() {
    let error = answer("hook_failed", ErrorGroup::Program, &hook_failed_details());

    assert_eq!(error.code(), ErrorCode::HOOK_FAILED);
    assert_eq!(error.group(), ErrorGroup::Program);
    assert!(error.retryable());
    assert_eq!(error.to_string(), "failed with hook_failed");
    assert_eq!(error.operation(), Some("crawl.run"));
    assert_eq!(error.fix(), Some("Fix the Hook."));
    let details: HookFailedDetails = error.details().unwrap();
    assert_eq!(details.hook, "sign");
    assert_eq!(error.details::<PathNotWritableDetails>(), None);
}

/// Covers: story:11-36, block-cause:decoy
#[test]
fn a_blocked_answer_with_foxhounds_decoy_cause_reads_it_as_unknown() {
    let error = answer(
        "blocked",
        ErrorGroup::Page,
        &json!({
            "cause": "decoy",
            "url": "https://example.com/trap",
            "status": null,
            "retry_after": null,
        }),
    );

    let details: BlockedDetails = error.details().unwrap();
    let BlockCause::Unknown(cause) = details.cause else {
        panic!("{:?} is a cause Collie knows", details.cause);
    };
    assert_eq!(cause.as_str(), "decoy");
}

/// ADRs 0080 and 0101: how long a Model provider asks to wait reads as a
/// `Duration`, never a point in time, and as none when it asks for no wait.
///
/// Covers: error:model_unavailable
#[test]
fn a_model_unavailable_answer_reads_its_retry_after_as_a_duration() {
    let waiting = answer(
        "model_unavailable",
        ErrorGroup::Unavailable,
        &json!({ "reason": "rate_limited", "retry_after": 1500 }),
    );
    let not_waiting = answer(
        "model_unavailable",
        ErrorGroup::Unavailable,
        &json!({ "reason": "unreachable", "retry_after": null }),
    );

    assert_eq!(waiting.code(), ErrorCode::MODEL_UNAVAILABLE);
    assert_eq!(
        [waiting, not_waiting].map(|error| error
            .details::<ModelUnavailableDetails>()
            .map(|details| (details.reason, details.retry_after))),
        [
            Some((
                ModelUnavailableReason::RateLimited,
                Some(Duration::from_millis(1500))
            )),
            Some((ModelUnavailableReason::Unreachable, None)),
        ]
    );
}

/// Covers: story:11-36, error:page_navigated_during_call
#[test]
fn a_navigation_during_an_ai_verb_reads_both_documents_by_type() {
    let error = answer(
        "page_navigated_during_call",
        ErrorGroup::Page,
        &json!({
            "url": "https://travel.test/books",
            "navigated_to": "https://travel.test/books/1",
            "fix": "Await the Collie call before your driver acts on the page.",
        }),
    );

    assert_eq!(error.code(), ErrorCode::PAGE_NAVIGATED_DURING_CALL);
    assert_eq!(
        error.details::<PageNavigatedDuringCallDetails>(),
        Some(PageNavigatedDuringCallDetails {
            url: "https://travel.test/books".to_owned(),
            navigated_to: "https://travel.test/books/1".to_owned(),
        })
    );
}

/// Covers: story:11-36, error:secret_unavailable
#[test]
fn a_secret_provider_that_made_no_code_reads_as_no_code() {
    let error = answer(
        "secret_unavailable",
        ErrorGroup::Unavailable,
        &json!({
            "placeholder": "shop_totp",
            "reference": "bw:shop/totp",
            "provider": "bw",
            "failure": "no_code",
            "fix": "Check the seed in the vault.",
        }),
    );

    let details: SecretUnavailableDetails = error.details().unwrap();
    assert_eq!(details.failure, SecretFailure::NoCode);
}

/// Covers: story:11-36, story:11-187
#[test]
fn a_code_this_program_does_not_know_stays_a_value_of_its_group() {
    let error = answer("brand_new", ErrorGroup::Unavailable, &json!({}));

    assert_eq!(error.code().as_str(), "brand_new");
    assert_eq!(error.group(), ErrorGroup::Unavailable);
}

/// Covers: story:11-36, error:handler_failed
#[test]
fn a_programs_own_error_fails_with_the_places_code_and_its_source_chain() {
    let own = parse_count("ten").unwrap_err();
    let error = Error::of_program(own, HANDLER_FAILED);

    assert_eq!(error.code(), HANDLER_FAILED);
    assert_eq!(error.group(), ErrorGroup::Program);
    let mut data = Map::new();
    data.insert("exception_type".to_owned(), "BoxError".into());
    data.insert(
        "exception_message".to_owned(),
        "invalid digit found in string".into(),
    );
    data.insert("exception_stack".to_owned(), "".into());
    assert_eq!(error.to_object().data.details, data);
    assert!(std::error::Error::source(&error).is_some());
}

/// Covers: story:11-36, error:handler_failed
#[test]
fn an_error_that_wraps_the_sdks_own_is_the_programs_own() {
    let wrapping: BoxError = Box::new(Wrapping(answer(
        "hook_failed",
        ErrorGroup::Program,
        &hook_failed_details(),
    )));

    let error = Error::of_program(wrapping, HANDLER_FAILED);

    assert_eq!(error.code(), HANDLER_FAILED);
    assert_eq!(
        error.to_object().data.details["exception_message"],
        "could not sign: failed with hook_failed"
    );
}

/// Covers: error:hook_failed
#[test]
fn a_box_holding_the_sdks_own_error_keeps_its_code_and_flag() {
    let sdk: BoxError = answer("hook_failed", ErrorGroup::Program, &hook_failed_details()).into();

    let error = Error::of_program(sdk, HANDLER_FAILED);

    assert_eq!(error.code(), ErrorCode::HOOK_FAILED);
    assert!(error.retryable());
    assert_eq!(error.to_object().data.code, "hook_failed");
}

/// Covers: story:11-30
#[test]
fn an_answers_limit_names_the_setting_its_value_layer_and_origin() {
    let error = answer(
        "setting_invalid",
        ErrorGroup::Program,
        &json!({
            "limit": { "key": "timeout", "value": 0, "layer": "run", "origin": "argument" },
        }),
    );

    assert_eq!(
        error.limit(),
        Some(SettingLimit {
            key: "timeout".to_owned(),
            value: json!(0),
            layer: SettingsLayer::Run,
            origin: Some(RunOrigin::Argument),
        })
    );
}
