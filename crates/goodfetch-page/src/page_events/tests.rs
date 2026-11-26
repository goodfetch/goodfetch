use goodfetch_engine::CallError;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::EventData;
use goodfetch_engine::settings::RunSettings;
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::events::PageEventName;
use goodfetch_protocol::page::waits::LoadState;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_WAIT};
use goodfetch_test_support::{TestEnv, runtime};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::sync::mpsc::error::TryRecvError;

use super::{Listening, PageEvents, Refusals, Requested, listener_fell_behind};
use crate::access_mode::runtime_under_stealth;
use crate::cdp::driver::PageDriver;
use crate::cdp::driver::replay::{answer, event};
use crate::live_run::{LiveRun, played_page};
use crate::url_pattern::UrlPattern;

/// The events of the page `page_id`, which `driver` drives for the test's
/// Run, read as its Session reads them.
pub(crate) fn started(page_id: &PageId, driver: &PageDriver, live: &LiveRun) -> PageEvents {
    PageEvents::start(
        page_id.clone(),
        driver.clone(),
        crate::sessions::events_run(&live.run),
        live.events.clone(),
        Refusals::default(),
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

/// The mark that `Runtime` was asked for belongs to the request (ADR
/// 0146): once the page's request part was replaced, the earlier request's
/// listener has ended, and the next request's first `page.on("console")`
/// asks for `Runtime` again; its listener hears the console calls made
/// since, each naming the next request's Page ID.
#[test]
fn a_replaced_request_asks_for_runtime_again_and_its_events_name_its_page_id() {
    let (first, next) = (PageId("page-1".to_owned()), PageId("page-2".to_owned()));

    let (earlier_ended, asked_again, heard) = runtime().block_on(async {
        let env = TestEnv::new();
        let (live, mut engine) = LiveRun::start(&env).await;
        let (driver, mut played, mut connection) = played_page();
        let events = started(&first, &driver, &live);
        let replaced = async {
            let mut earlier = events.listen(PageEventName::Console);
            let enable = played.command().await;
            assert_eq!(enable["method"], "Runtime.enable", "{enable}");
            assert!(played.send(&[answer(&enable, json!({}))]).await);
            events.replace_request(next.clone());
            let (_, earlier_ended) = earlier.taken();
            let mut listening = events.listen(PageEventName::Console);
            let again = played.command().await;
            assert!(played.send(&[answer(&again, json!({}))]).await);
            let called = event(
                "Runtime.consoleAPICalled",
                json!({
                    "type": "log",
                    "args": [{ "type": "string", "value": "the next request's" }],
                    "executionContextId": 1,
                    // Past any clock the Run reads, so after the ask.
                    "timestamp": 1.0e15,
                }),
            );
            assert!(played.send(&[called]).await);
            let heard = listening
                .heard()
                .await
                .expect("the next request's listener ended");
            (earlier_ended, again["method"].clone(), heard)
        };
        let ran = tokio::select! {
            biased;
            ran = replaced => ran,
            () = &mut engine => panic!("the engine ended before the test"),
            () = &mut connection => panic!("the page's connection ended before the test"),
        };
        drop((events, live));
        // The Run ends once the page's reads did, at the connection's end.
        drop((driver, played));
        tokio::join!(engine, connection);
        ran
    });

    assert!(
        earlier_ended,
        "the earlier request's listener still listens"
    );
    assert_eq!(asked_again, "Runtime.enable");
    assert_eq!(heard["page_id"], "page-2", "{heard}");
}

/// A `stealth` Session's page refuses each `page.on` that needs `Runtime`
/// with its refusal, and never asks for `Runtime`; its other events are
/// listened to (#599's Q2).
#[rstest]
#[case::console(PageEventName::Console, Some("capability_unavailable"))]
#[case::page_error(PageEventName::PageError, Some("capability_unavailable"))]
#[case::request(PageEventName::Request, None)]
fn a_page_that_refuses_runtime_refuses_what_needs_it(
    #[case] event: PageEventName,
    #[case] expected: Option<&str>,
) {
    let (refused, asked) = runtime().block_on(async {
        let env = TestEnv::new();
        let (live, engine) = LiveRun::start(&env).await;
        let (driver, played, connection) = played_page();
        let events = started(&PageId("page-1".to_owned()), &driver, &live);
        events.refuse_runtime(runtime_under_stealth(SettingsLayer::Run));
        let mut listening = events.listen(event);
        let refused = match listening.armed.take() {
            Some(armed) => armed
                .await
                .expect("an answer")
                .err()
                .map(|error| error.code()),
            None => None,
        };
        let asked = *events.shared.arming.borrow();
        drop((listening, events, live));
        drop((driver, played));
        tokio::join!(engine, connection);
        (refused, asked)
    });

    assert_eq!((refused, asked), (expected, false));
}

/// What a request armed or kept belongs to it (ADR 0146): once the page's
/// request part was replaced, each wait the earlier request armed raises
/// `closed` at once, never answered by the next request's traffic, and the
/// bodies it kept are gone.
#[test]
fn a_replaced_request_answers_its_waits_closed_and_keeps_none_of_its_bodies() {
    let first = PageId("page-1".to_owned());

    let (codes, kept) = runtime().block_on(async {
        let env = TestEnv::new();
        let (live, mut engine) = LiveRun::start(&env).await;
        let (driver, played, mut connection) = played_page();
        let events = started(&first, &driver, &live);
        let deadline = || {
            events
                .wait_deadline(Some(Some(30_000)))
                .expect("invariant: thirty seconds is a valid wait")
        };
        events.keep_requested(
            "request-1".to_owned(),
            Requested::Cut {
                url: "https://first.test/".to_owned(),
                received: 1,
                expected: Some(2),
            },
        );
        let pattern = || UrlPattern::glob("https://next.test/**").expect("invariant: a valid glob");
        let request = events.wait_for_request(Some(pattern()), deadline());
        let response = events.wait_for_response(Some(pattern()), deadline());
        let finished =
            events.wait_for_request_finished(Some(pattern()), Form::Typescript, deadline());
        let url = events.wait_for_url(pattern(), LoadState::Load, deadline());
        events.replace_request(PageId("page-2".to_owned()));
        let replaced = async {
            let (request, response, finished, url) = tokio::join!(request, response, finished, url);
            let code = |error: Option<CallError>| error.map(|error| error.code().to_owned());
            (
                [
                    code(request.err()),
                    code(response.err()),
                    code(finished.err()),
                    code(url.err()),
                ],
                events.kept("request-1").is_some(),
            )
        };
        let ran = tokio::select! {
            biased;
            ran = replaced => ran,
            () = &mut engine => panic!("the engine ended before the test"),
            () = &mut connection => panic!("the page's connection ended before the test"),
        };
        drop((events, live));
        drop((driver, played));
        tokio::join!(engine, connection);
        ran
    });

    let closed = Some(goodfetch_protocol::connection::CLOSED.name.to_owned());
    assert_eq!(
        codes,
        [closed.clone(), closed.clone(), closed.clone(), closed]
    );
    assert!(!kept, "the earlier request's body is still kept");
}
