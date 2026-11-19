use goodfetch_protocol::connection::Form;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::events::PageEventName;
use rstest::rstest;
use serde_json::json;

use super::listener_fell_behind;

/// A listener that fell behind names its event as its Client listens to
/// it, and its details stay canonical.
#[rstest]
#[case::typescript(
    Form::Typescript,
    PageEventName::RequestFinished,
    "`requestfinished` events"
)]
#[case::python(
    Form::Python,
    PageEventName::RequestFinished,
    "`requestfinished` events"
)]
#[case::rust(
    Form::Rust,
    PageEventName::RequestFinished,
    "`events::RequestFinished` events"
)]
#[case::rust_error(Form::Rust, PageEventName::PageError, "`events::WebError` events")]
#[case::catalog(Form::Catalog, PageEventName::WebSocket, "`web_socket` events")]
fn a_listener_that_fell_behind_names_its_event_in_the_form(
    #[case] form: Form,
    #[case] event: PageEventName,
    #[case] expected: &str,
) {
    let error = listener_fell_behind(&PageId("p1".to_owned()), event);

    let object = error.to_error_object_in(form);

    assert!(
        object
            .message
            .starts_with(&format!("1024 {expected} of the page `p1`")),
        "{}",
        object.message
    );
    assert_eq!(
        serde_json::Value::Object(object.data.details),
        json!({"page_id": "p1"})
    );
}

/// Each event `page.on` takes is one of the catalog's, which a failure's
/// message names.
#[rstest]
fn each_listened_event_is_the_catalogs(
    #[values(
        PageEventName::Console,
        PageEventName::PageError,
        PageEventName::Request,
        PageEventName::RequestFinished,
        PageEventName::RequestFailed,
        PageEventName::Response,
        PageEventName::WebSocket,
        PageEventName::Load,
        PageEventName::DomContentLoaded,
        PageEventName::Crash,
        PageEventName::Close,
        PageEventName::Popup
    )]
    event: PageEventName,
) {
    assert!(super::listened(event).is_some());
}
