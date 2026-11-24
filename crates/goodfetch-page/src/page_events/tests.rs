use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::EventData;
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::events::PageEventName;
use goodfetch_protocol::settings::TIMEOUT_WAIT;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::sync::mpsc::error::TryRecvError;

use super::{Listening, PageEvents, Refusals, listener_fell_behind};
use crate::driver::PageDriver;
use crate::live_run::LiveRun;

/// The events of the page `page_id`, which `driver` drives for the test's
/// Run, read as its Session reads them.
pub(crate) fn started(page_id: &PageId, driver: &PageDriver, live: &LiveRun) -> PageEvents {
    PageEvents::start(
        page_id.clone(),
        driver.clone(),
        crate::sessions::events_run(&live.run),
        live.events.clone(),
        Refusals::default(),
        driver.events(),
        Deadline::of_resolved(
            live.run.settings().map(RunSettings::resolved),
            &TIMEOUT_WAIT,
        ),
        None,
    )
}

/// An event's data, decoded.
fn decoded(data: &EventData) -> Value {
    serde_json::from_str(data.json()).expect("invariant: an event's data is JSON")
}

impl Listening {
    /// The events published to this listener so far, each its data, and
    /// whether its listening ended.
    pub(crate) fn taken(&mut self) -> (Vec<Value>, bool) {
        let mut taken = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok((_, data)) => taken.push(decoded(&data)),
                Err(TryRecvError::Empty) => return (taken, false),
                Err(TryRecvError::Disconnected) => return (taken, true),
            }
        }
    }

    /// The data of the next event published to this listener, once it is;
    /// none once its listening ended.
    pub(crate) async fn heard(&mut self) -> Option<Value> {
        self.events.recv().await.map(|(_, data)| decoded(&data))
    }
}

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
    let listened = super::listened(event);

    assert_eq!(
        listened.name.strip_prefix("page."),
        Some(goodfetch_protocol::wire_name::of(&event))
    );
}
