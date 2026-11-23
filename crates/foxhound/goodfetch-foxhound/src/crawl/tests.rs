#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::any::Any;

use goodfetch_protocol::foxhound::crawl::HandlerFailedDetails;
use serde_json::Value;

use super::{Crawl, CrawlContext, panicked};
use crate::error_code;

/// What a handler's panic with `payload` fails its attempt with.
fn details_of(payload: &(dyn Any + Send)) -> HandlerFailedDetails {
    let error = panicked("page", payload);
    assert_eq!(error.code(), error_code::HANDLER_FAILED);
    error
        .details()
        .expect("invariant: a panic's error details its exception")
}

#[test]
fn a_handlers_panic_is_handler_failed_of_the_exception_type_panic() {
    let details = details_of(&"the shelf is gone");

    assert_eq!(
        details,
        HandlerFailedDetails {
            exception_type: "panic".to_owned(),
            exception_message: "the shelf is gone".to_owned(),
            exception_stack: String::new(),
        }
    );
}

/// A crawl that attaches a handler to one Label and none to another is
/// refused before its start is sent, with `label_unhandled`. The SDK
/// raises it itself, so no exercise record of a test that connects can show it (ADR
/// 0072): this test, which never reaches `rpc`, declares it.
///
/// Covers: error:label_unhandled
#[test]
fn a_label_left_unhandled_beside_a_handled_one_refuses_the_start_with_label_unhandled() {
    let mut crawl = Crawl::new();
    let page = crawl.label::<Value>("page").unwrap();
    let _detail = crawl.label::<Value>("detail").unwrap();
    crawl
        .handle(&page, |_ctx: CrawlContext<Value>| async { Ok(()) })
        .unwrap();

    let refused = crawl.start_params(Vec::new()).unwrap_err();

    assert_eq!(refused.code(), error_code::LABEL_UNHANDLED, "{refused}");
}

#[test]
fn a_panic_formatted_or_of_no_text_keeps_its_message_or_says_it_has_none() {
    let formatted = details_of(&format!("shelf {}", 3));
    let opaque = details_of(&7_u8);

    assert_eq!(
        [formatted.exception_message, opaque.exception_message],
        ["shelf 3", "a panic of no message"]
    );
}
