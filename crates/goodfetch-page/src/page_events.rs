//! A page's events and the bodies of its responses (spec #577's `network`;
//! ADRs 0081, 0085, 0118 and 0121). Each open page has one reader of its
//! target's events, which follows every request through its hops, response,
//! bytes and end, and publishes an event of the page on the engine's event
//! stream while a `page.on` call listens to it, lossless, under each
//! subscription's credits (ADR 0024). A listener is armed when its call
//! arrives and removed when the call goes. The page's console and uncaught
//! exceptions come from the `Runtime` domain, a known automation signal
//! (ADR 0019): the engine enables it only once a call listens to `console`
//! or `page_error`, and passes over what the browser replays from before
//! that call.
//!
//! A body is read on demand: kept for a response a
//! `page.wait_for_request_finished` waiter answered, read and kept as the
//! request finishes; otherwise read from the browser once the request
//! ended, `body_unavailable` when the browser no longer holds it, and
//! `response_truncated` when it broke off after its response arrived, as
//! the navigation's own rule reads the pinned Chromium's signals (a network
//! error after the response, other than the browser's own cancel or a
//! coding that failed to decode).
//!
//! A response `page.request` answered from the page's Session keeps its
//! body too, whole or as cut, under its request's ID.
//!
//! A request the interception refused carries the refusal's Error code in
//! its `failure`, and one no call waited on is recorded in the Run trace as
//! `page.request_failed`, so a refusal is never a timeout.
#![expect(
    clippy::disallowed_types,
    reason = "listeners and waiters are armed as their call arrives, before any `.await`: each lock guards one page's state for one step, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use data_encoding::BASE64;
use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{network, page, runtime};
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::Events;
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::page::events::{
    BODY_UNAVAILABLE, BodyUnavailableDetails, ConsoleLocation, ConsoleMessage, PAGE_CONSOLE,
    PAGE_DOM_CONTENT_LOADED, PAGE_LOAD, PAGE_PAGE_ERROR, PAGE_REQUEST, PAGE_REQUEST_FAILED,
    PAGE_REQUEST_FINISHED, PAGE_RESPONSE, PAGE_WEB_SOCKET, PageEventName, PageExchange, PageFired,
    PageRequest, PageWebSocket, RequestFailure, WEB_SOCKET_CLOSE, WEB_SOCKET_FRAME_RECEIVED,
    WEB_SOCKET_FRAME_SENT, WebError, WebSocketClosed, WebSocketFrame,
};
use goodfetch_protocol::page::{
    BodyHandle, PageId, PageResponse, RESPONSE_TRUNCATED, ResponseTruncatedDetails,
};
use tokio::sync::{Notify, oneshot, watch};

use crate::driver::{PageDriver, response_of};
use crate::url_pattern::UrlPattern;

/// The error text of a request the browser cancelled.
const ABORTED: &str = "net::ERR_ABORTED";

/// The error text of a body whose listed content coding did not decode.
const DECODING_FAILED: &str = "net::ERR_CONTENT_DECODING_FAILED";

/// The refusals the interception made, by the browser's ID of the request,
/// which the request's failure then carries: the refusal's Error code, and
/// whether the Run trace records it, as it does when no call waits on the
/// request.
#[derive(Clone, Default)]
pub(crate) struct Refusals(Arc<Mutex<BTreeMap<String, (ErrorData, bool)>>>);

impl Refusals {
    /// Leaves `refusal` for the request `request_id`, before the browser
    /// fails it; `recorded` when no call waits on it.
    pub(crate) fn leave(&self, request_id: &str, refusal: ErrorData, recorded: bool) {
        lock(&self.0).insert(request_id.to_owned(), (refusal, recorded));
    }

    fn take(&self, request_id: &str) -> Option<(ErrorData, bool)> {
        lock(&self.0).remove(request_id)
    }
}

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The open pages' events, by page, which a call reaches as it arrives.
#[derive(Clone, Default)]
pub(crate) struct Table(Arc<Mutex<BTreeMap<PageId, PageEvents>>>);

impl Table {
    pub(crate) fn insert(&self, page_id: PageId, events: PageEvents) {
        lock(&self.0).insert(page_id, events);
    }

    /// The events of the open page `page_id`, or `closed`.
    pub(crate) fn get(&self, page_id: &PageId) -> Result<PageEvents, CallError> {
        lock(&self.0).get(page_id).cloned().ok_or_else(|| {
            CallError::new(
                &goodfetch_protocol::connection::CLOSED,
                format!("the page `{}` is closed, or its Run ended", page_id.0),
                &serde_json::json!({ "page_id": page_id }),
            )
        })
    }

    /// Takes the pages `pages` out, as they go: their calls answer.
    pub(crate) fn close(&self, pages: &[PageId]) {
        let mut table = lock(&self.0);
        for page in pages {
            if let Some(events) = table.remove(page) {
                events.close();
            }
        }
    }
}

/// One open page's events and bodies; clones share them.
#[derive(Clone)]
pub(crate) struct PageEvents {
    shared: Arc<Shared>,
}

struct Shared {
    page_id: PageId,
    driver: PageDriver,
    run: Run,
    events: Events,
    refusals: Refusals,
    /// The Run's `timeout.wait`, on its clock.
    wait: Deadline,
    clock: Clock,
    log: Arc<RequestLog>,
    state: Mutex<State>,
    /// Woken whenever a request ends or the page goes.
    changed: Notify,
    /// Whether its target's events ended: the page left, or its browser's
    /// connection did; its waiters and body reads answer then.
    gone: watch::Sender<bool>,
    /// Whether the page left the engine, closed or with its Run: its
    /// listeners' calls answer then, and a crash leaves them listening.
    closed: watch::Sender<bool>,
}

#[derive(Default)]
struct State {
    /// How many calls listen to each event.
    listening: BTreeMap<PageEventName, usize>,
    /// When the `Runtime` domain was asked for, in milliseconds since the
    /// epoch: what the browser replays from before is passed over.
    runtime_since: Option<f64>,
    requests: BTreeMap<String, Tracked>,
    /// The bodies kept for the responses a waiter answered.
    kept: BTreeMap<String, BodyHandle>,
    /// The bodies kept for the responses `page.request` answered from the
    /// page's Session, whole or cut.
    requested: BTreeMap<String, Requested>,
    waiters: Vec<Waiter>,
    /// The sockets the page opened, by the browser's ID.
    sockets: BTreeMap<String, String>,
}

/// One request, at its latest hop.
struct Tracked {
    request: PageRequest,
    response: Option<PageResponse>,
    /// The bytes of its body that arrived, decoded.
    received: u64,
    ended: Option<Ended>,
}

/// The body of a response `page.request` answered, as it kept it.
#[derive(Clone)]
pub(crate) enum Requested {
    /// Whole, in the Run's folder.
    Whole(BodyHandle),
    /// Broken off after `received` bytes of the `expected`, from `url`.
    Cut {
        url: String,
        received: u64,
        expected: Option<u64>,
    },
}

/// How a request ended.
#[derive(Clone)]
enum Ended {
    Finished,
    Failed(String),
}

/// A `page.wait_for_request_finished` call's waiter.
struct Waiter {
    pattern: Option<UrlPattern>,
    /// The surface of the waiting call's Client, whose spelling its errors
    /// take.
    surface: Surface,
    answer: oneshot::Sender<Result<PageExchange, CallError>>,
}

/// A listener, which listens until dropped.
pub(crate) struct Listening {
    shared: Arc<Shared>,
    event: PageEventName,
}

impl Drop for Listening {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        if let Some(count) = state.listening.get_mut(&self.event) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.listening.remove(&self.event);
            }
        }
    }
}

impl Listening {
    /// Waits until the page goes, once `Runtime` is on when the event
    /// needs it.
    ///
    /// # Errors
    ///
    /// The code of the page's failure when the browser refused `Runtime`.
    pub(crate) async fn until_gone(self, runtime: bool) -> Result<(), CallError> {
        if runtime {
            self.shared
                .driver
                .opening_command(&runtime::EnableParams {})
                .await?;
        }
        let mut closed = self.shared.closed.subscribe();
        let _ = closed.wait_for(|closed| *closed).await;
        Ok(())
    }
}

impl PageEvents {
    /// Starts reading the events of the page `page_id`, which `driver`
    /// drives for `run`, from `subscription`, publishing on `events`; its
    /// waiters wait at most `wait`.
    pub(crate) fn start(
        page_id: PageId,
        driver: PageDriver,
        run: &Run,
        events: Events,
        refusals: Refusals,
        subscription: Subscription,
        wait: Deadline,
    ) -> Self {
        let shared = Arc::new(Shared {
            page_id,
            driver,
            run: run.clone(),
            events,
            refusals,
            wait,
            clock: run.clock(),
            log: Arc::new(RequestLog::new(run.owner().child(), run.folder())),
            state: Mutex::new(State::default()),
            changed: Notify::new(),
            gone: watch::Sender::new(false),
            closed: watch::Sender::new(false),
        });
        run.spawn(read(subscription, Arc::clone(&shared)));
        Self { shared }
    }

    /// Arms a listener for `event` now: the event is published while it
    /// listens. The second value says the listener must first turn
    /// `Runtime` on, which `wall` dates.
    pub(crate) fn listen(&self, event: PageEventName, wall: SystemTime) -> (Listening, bool) {
        let mut state = lock(&self.shared.state);
        *state.listening.entry(event).or_default() += 1;
        let runtime = matches!(event, PageEventName::Console | PageEventName::PageError)
            && state.runtime_since.is_none();
        if runtime {
            state.runtime_since = Some(millis_since_epoch(wall));
        }
        drop(state);
        (
            Listening {
                shared: Arc::clone(&self.shared),
                event,
            },
            runtime,
        )
    }

    /// Publishes `event`, the page's `page.crash` or `page.close`, when a
    /// call listens to `name`.
    pub(crate) async fn publish_end(&self, name: PageEventName, event: &'static Event) {
        let listening = lock(&self.shared.state).listening.contains_key(&name);
        if listening {
            let fired = PageFired {
                page_id: self.shared.page_id.clone(),
            };
            self.shared
                .events
                .publish(event, Some(&self.shared.page_id.0), &fired)
                .await;
        }
    }

    /// Publishes `event` with `data`, an event of the page the engine
    /// itself sees, such as `page.popup`, when a call listens to `name`.
    pub(crate) async fn publish(
        &self,
        name: PageEventName,
        event: &'static Event,
        data: &impl serde::Serialize,
    ) {
        let listening = lock(&self.shared.state).listening.contains_key(&name);
        if listening {
            self.shared
                .events
                .publish(event, Some(&self.shared.page_id.0), data)
                .await;
        }
    }

    /// The engine's event stream the page publishes on.
    pub(crate) fn stream(&self) -> &Events {
        &self.shared.events
    }

    /// The page went: every listener's and waiter's call answers.
    pub(crate) fn close(&self) {
        lock(&self.shared.state).waiters.clear();
        self.shared.gone.send_replace(true);
        self.shared.closed.send_replace(true);
        self.shared.changed.notify_waiters();
    }

    /// Arms a waiter for the next request whose URL `pattern` matches, any
    /// when none, to finish; it answers the request and its response,
    /// whose body it kept, unless the Run's `timeout.wait` passes first;
    /// its errors are spelled for `surface`.
    pub(crate) fn wait_for_request_finished(
        &self,
        pattern: Option<UrlPattern>,
        surface: Surface,
    ) -> impl Future<Output = Result<PageExchange, CallError>> + use<> {
        let (answer, answered) = oneshot::channel();
        lock(&self.shared.state).waiters.push(Waiter {
            pattern,
            surface,
            answer,
        });
        let (deadline, clock) = (self.shared.wait.clone(), self.shared.clock);
        async move {
            match deadline
                .within(&clock, "finished request", answered)
                .await?
            {
                Ok(answer) => answer,
                Err(_) => Err(gone()),
            }
        }
    }

    /// Keeps `body`, the body of the response `page.request` answered from
    /// the page's Session to the request `request_id`, for
    /// [`body`](Self::body).
    pub(crate) fn keep_requested(&self, request_id: String, body: Requested) {
        lock(&self.shared.state).requested.insert(request_id, body);
    }

    /// The body of the response to the request `request_id`, once it
    /// ended, as a page body's handle.
    ///
    /// # Errors
    ///
    /// `body_unavailable` when the browser no longer holds it, or the page
    /// never sent the request, its fix spelled for `surface`, and
    /// `response_truncated` when it broke off.
    pub(crate) async fn body(
        &self,
        request_id: &str,
        surface: Surface,
    ) -> Result<BodyHandle, CallError> {
        let shared = &self.shared;
        let (url, ended, received, expected) = loop {
            let changed = shared.changed.notified();
            {
                let state = lock(&shared.state);
                if let Some(kept) = state.kept.get(request_id) {
                    return Ok(kept.clone());
                }
                match state.requested.get(request_id) {
                    Some(Requested::Whole(kept)) => return Ok(kept.clone()),
                    Some(Requested::Cut {
                        url,
                        received,
                        expected,
                    }) => return Err(cut(url, *received, *expected)),
                    None => {}
                }
                let Some(tracked) = state.requests.get(request_id) else {
                    return Err(unavailable("", request_id, surface));
                };
                if let Some(ended) = &tracked.ended {
                    break (
                        tracked.request.url.clone(),
                        ended.clone(),
                        tracked.received,
                        tracked.response.as_ref().and_then(declared_length),
                    );
                }
            }
            if *shared.gone.borrow() {
                return Err(gone());
            }
            changed.await;
        };
        let responded = lock(&shared.state)
            .requests
            .get(request_id)
            .is_some_and(|tracked| tracked.response.is_some());
        match ended {
            Ended::Failed(error) if responded && error != ABORTED && error != DECODING_FAILED => {
                Err(cut(&url, received, expected))
            }
            Ended::Failed(_) => Err(unavailable(&url, request_id, surface)),
            Ended::Finished => match read_body(shared, request_id).await {
                Some(handle) => Ok(handle),
                None => Err(unavailable(&url, request_id, surface)),
            },
        }
    }
}

/// The `Content-Length` a response declared.
fn declared_length(response: &PageResponse) -> Option<u64> {
    response
        .headers
        .get("content-length")
        .and_then(|length| length.trim().parse().ok())
}

/// Reads the body of the finished request `request_id` from the browser
/// and stores it in the Run's folder; none when the browser no longer holds
/// it.
async fn read_body(shared: &Shared, request_id: &str) -> Option<BodyHandle> {
    let read = shared
        .driver
        .command(&network::GetResponseBodyParams {
            request_id: request_id.to_owned(),
        })
        .await
        .ok()?;
    let bytes = if read.base64_encoded {
        BASE64.decode(read.body.as_bytes()).ok()?
    } else {
        read.body.into_bytes()
    };
    match shared.run.store_body(bytes).await {
        Ok(handle) => Some(handle),
        Err(error) => {
            tracing::warn!(%error, "a response's body was not stored");
            None
        }
    }
}

/// `body_unavailable` for the request `request_id` to `url`, its fix
/// spelled for `surface`.
fn unavailable(url: &str, request_id: &str, surface: Surface) -> CallError {
    let waiter = match surface {
        Surface::SdkTypescript => "page.waitForRequestFinished",
        Surface::SdkPython
        | Surface::Rust
        | Surface::Cli
        | Surface::Mcp
        | Surface::Console
        | Surface::Cloud
        | Surface::BrowserHost
        | Surface::Worker => "page.wait_for_request_finished",
    };
    let fix = format!(
        "obtain the response through `{waiter}`, `route.fetch` or `page.request`, whose bodies the engine keeps"
    );
    CallError::new(
        &BODY_UNAVAILABLE,
        format!("the browser no longer holds the body of the response to {url}: {fix}"),
        &BodyUnavailableDetails {
            url: url.to_owned(),
            request_id: request_id.to_owned(),
            fix,
        },
    )
}

/// `response_truncated` for the body of the response to `url`, which broke
/// off after `received` of its `expected` bytes.
fn cut(url: &str, received: u64, expected: Option<u64>) -> CallError {
    let of = expected.map_or_else(String::new, |expected| format!(" of {expected}"));
    CallError::new(
        &RESPONSE_TRUNCATED,
        format!("the body of the response to {url} broke off after {received} bytes{of}"),
        &ResponseTruncatedDetails {
            url: url.to_owned(),
            received,
            expected,
        },
    )
}

/// `closed` for a call on a page that went before it answered.
fn gone() -> CallError {
    CallError::new(
        &goodfetch_protocol::connection::CLOSED,
        "the page closed, or its Run ended, before the call answered",
        &serde_json::json!({}),
    )
}

fn millis_since_epoch(wall: SystemTime) -> f64 {
    wall.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
        * 1000.0
}

/// Reads the page's events until its target goes.
async fn read(mut subscription: Subscription, shared: Arc<Shared>) {
    while let Some(event) = subscription.next().await {
        let published = take_in(&shared, &event);
        for (event, data) in published {
            shared
                .events
                .publish(event, Some(&shared.page_id.0), &data)
                .await;
        }
    }
    shared.gone.send_replace(true);
    shared.changed.notify_waiters();
}

/// One event a page's event publishes: its catalog entry and its data.
type Published = (&'static Event, serde_json::Value);

/// `data` as `event`'s, when a call listens to `name`.
fn emit(
    state: &State,
    out: &mut Vec<Published>,
    name: PageEventName,
    event: &'static Event,
    data: &impl serde::Serialize,
) {
    if state.listening.contains_key(&name) {
        out.push((
            event,
            serde_json::to_value(data).expect("invariant: an event's payload encodes as JSON"),
        ));
    }
}

/// What one event of the page's target publishes, as it changes the page's
/// state.
fn take_in(shared: &Arc<Shared>, event: &goodfetch_cdp::RawEvent) -> Vec<Published> {
    let mut out = Vec::new();
    let mut guard = lock(&shared.state);
    let state = &mut *guard;
    if let Some(sent) = event.read::<network::RequestWillBeSentEvent>() {
        sent_request(shared, state, &mut out, sent);
    } else if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
        let response = response_of(&received.request_id, &received.response);
        if let Some(tracked) = state.requests.get_mut(&received.request_id) {
            tracked.response = Some(response.clone());
            let exchange = PageExchange {
                request: tracked.request.clone(),
                response,
            };
            emit(
                state,
                &mut out,
                PageEventName::Response,
                &PAGE_RESPONSE,
                &exchange,
            );
        }
    } else if let Some(data) = event.read::<network::DataReceivedEvent>() {
        if let Some(tracked) = state.requests.get_mut(&data.request_id) {
            tracked.received += u64::try_from(data.data_length).unwrap_or_default();
        }
    } else if let Some(finished) = event.read::<network::LoadingFinishedEvent>() {
        if let Some(tracked) = state.requests.get_mut(&finished.request_id) {
            tracked.ended = Some(Ended::Finished);
            let (request, response) = (tracked.request.clone(), tracked.response.clone());
            emit(
                state,
                &mut out,
                PageEventName::RequestFinished,
                &PAGE_REQUEST_FINISHED,
                &request,
            );
            if let Some(response) = response {
                answer_waiters(shared, state, request, response);
            }
        }
        shared.changed.notify_waiters();
    } else if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
        failed_request(shared, state, &mut out, &failed);
        shared.changed.notify_waiters();
    } else {
        take_in_other(shared, state, &mut out, event);
    }
    out
}

/// A request sent, or its next hop after a redirect: the hop before got the
/// redirect as its response, and finished.
fn sent_request(
    shared: &Shared,
    state: &mut State,
    out: &mut Vec<Published>,
    sent: network::RequestWillBeSentEvent,
) {
    let mut chain = Vec::new();
    if let Some(redirect) = &sent.redirect_response
        && let Some(before) = state.requests.get(&sent.request_id)
    {
        let exchange = PageExchange {
            request: before.request.clone(),
            response: response_of(&sent.request_id, redirect),
        };
        chain.clone_from(&exchange.request.redirect_chain);
        chain.push(exchange.request.url.clone());
        emit(
            state,
            out,
            PageEventName::Response,
            &PAGE_RESPONSE,
            &exchange,
        );
        emit(
            state,
            out,
            PageEventName::RequestFinished,
            &PAGE_REQUEST_FINISHED,
            &exchange.request,
        );
    }
    let request = PageRequest {
        page_id: shared.page_id.clone(),
        request_id: sent.request_id.clone(),
        url: sent.request.url.clone(),
        method: sent.request.method.clone(),
        headers: headers(&sent.request.headers),
        post_data: post_data(&sent.request),
        resource_type: resource_type(sent.r#type.unwrap_or(network::ResourceType::Other)),
        is_navigation_request: sent.r#type == Some(network::ResourceType::Document)
            && sent.frame_id.as_ref() == Some(shared.driver.main_frame()),
        redirect_chain: chain,
        failure: None,
    };
    emit(state, out, PageEventName::Request, &PAGE_REQUEST, &request);
    state.requests.insert(
        sent.request_id,
        Tracked {
            request,
            response: None,
            received: 0,
            ended: None,
        },
    );
}

/// A request failed: its failure carries the interception's refusal, which
/// the Run trace records when no call waited on the request, as the Run's
/// outcome record names its Error code.
fn failed_request(
    shared: &Shared,
    state: &mut State,
    out: &mut Vec<Published>,
    failed: &network::LoadingFailedEvent,
) {
    let refusal = shared.refusals.take(&failed.request_id);
    let Some(tracked) = state.requests.get_mut(&failed.request_id) else {
        return;
    };
    tracked.ended = Some(Ended::Failed(error_text(&failed.error_text).to_owned()));
    let mut request = tracked.request.clone();
    request.failure = Some(RequestFailure {
        error_text: error_text(&failed.error_text).to_owned(),
        error: refusal.as_ref().map(|(error, _)| error.clone()),
    });
    if let Some((refused, true)) = &refusal {
        // The Run trace holds the Error code no reply carries (ADR 0072).
        shared.run.record_outcome(&refused.code);
        let (log, request) = (Arc::clone(&shared.log), request.clone());
        shared.run.spawn(async move {
            if let Err(error) = log.record(&PAGE_REQUEST_FAILED, &request).await {
                tracing::warn!(%error, "the Run trace did not take a refused request");
            }
        });
    }
    emit(
        state,
        out,
        PageEventName::RequestFailed,
        &PAGE_REQUEST_FAILED,
        &request,
    );
}

/// A failed request's network error as the page would see it: the
/// browser marks a request its debugging client failed with a suffix,
/// `net::ERR_BLOCKED_BY_CLIENT.Inspector` for the interception's block,
/// which no blocker in a person's browser gives.
fn error_text(error: &str) -> &str {
    error.strip_suffix(INSPECTOR).unwrap_or(error)
}

/// The suffix of a network error the browser's debugging client caused.
const INSPECTOR: &str = ".Inspector";

/// The events of sockets, of the main frame's document and of the
/// `Runtime` domain.
fn take_in_other(
    shared: &Shared,
    state: &mut State,
    out: &mut Vec<Published>,
    event: &goodfetch_cdp::RawEvent,
) {
    let page_id = &shared.page_id;
    if let Some(created) = event.read::<network::WebSocketCreatedEvent>() {
        state
            .sockets
            .insert(created.request_id.clone(), created.url.clone());
        let socket = PageWebSocket {
            page_id: page_id.clone(),
            web_socket_id: created.request_id,
            url: created.url,
        };
        emit(
            state,
            out,
            PageEventName::WebSocket,
            &PAGE_WEB_SOCKET,
            &socket,
        );
    } else if let Some(sent) = event.read::<network::WebSocketFrameSentEvent>() {
        let frame = frame_of(page_id, sent.request_id, &sent.response);
        emit(
            state,
            out,
            PageEventName::WebSocket,
            &WEB_SOCKET_FRAME_SENT,
            &frame,
        );
    } else if let Some(received) = event.read::<network::WebSocketFrameReceivedEvent>() {
        let frame = frame_of(page_id, received.request_id, &received.response);
        emit(
            state,
            out,
            PageEventName::WebSocket,
            &WEB_SOCKET_FRAME_RECEIVED,
            &frame,
        );
    } else if let Some(closed) = event.read::<network::WebSocketClosedEvent>() {
        if state.sockets.remove(&closed.request_id).is_some() {
            let closed = WebSocketClosed {
                page_id: page_id.clone(),
                web_socket_id: closed.request_id,
            };
            emit(
                state,
                out,
                PageEventName::WebSocket,
                &WEB_SOCKET_CLOSE,
                &closed,
            );
        }
    } else if event.read::<page::DomContentEventFiredEvent>().is_some() {
        let fired = PageFired {
            page_id: page_id.clone(),
        };
        emit(
            state,
            out,
            PageEventName::DomContentLoaded,
            &PAGE_DOM_CONTENT_LOADED,
            &fired,
        );
    } else if event.read::<page::LoadEventFiredEvent>().is_some() {
        let fired = PageFired {
            page_id: page_id.clone(),
        };
        emit(state, out, PageEventName::Load, &PAGE_LOAD, &fired);
    } else if let Some(called) = event.read::<runtime::ConsoleAPICalledEvent>() {
        if state
            .runtime_since
            .is_some_and(|since| called.timestamp >= since)
        {
            let message = console_message(page_id, &called);
            emit(state, out, PageEventName::Console, &PAGE_CONSOLE, &message);
        }
    } else if let Some(thrown) = event.read::<runtime::ExceptionThrownEvent>()
        && state
            .runtime_since
            .is_some_and(|since| thrown.timestamp >= since)
    {
        let error = web_error(page_id, &thrown.exception_details);
        emit(
            state,
            out,
            PageEventName::PageError,
            &PAGE_PAGE_ERROR,
            &error,
        );
    }
}

/// Answers each waiter whose pattern matches the finished `request`, once
/// its body is read and kept.
fn answer_waiters(
    shared: &Arc<Shared>,
    state: &mut State,
    request: PageRequest,
    response: PageResponse,
) {
    let (matched, waiting): (Vec<Waiter>, Vec<Waiter>) = std::mem::take(&mut state.waiters)
        .into_iter()
        .partition(|waiter| {
            !waiter.answer.is_closed()
                && waiter
                    .pattern
                    .as_ref()
                    .is_none_or(|pattern| pattern.matches(&request.url))
        });
    state.waiters = waiting
        .into_iter()
        .filter(|waiter| !waiter.answer.is_closed())
        .collect();
    if matched.is_empty() {
        return;
    }
    let shared = Arc::clone(shared);
    let run = shared.run.clone();
    run.spawn(async move {
        let request_id = request.request_id.clone();
        let kept = read_body(&shared, &request_id).await;
        if let Some(handle) = &kept {
            lock(&shared.state)
                .kept
                .insert(request_id.clone(), handle.clone());
        }
        for waiter in matched {
            let answer = match kept {
                Some(_) => Ok(PageExchange {
                    request: request.clone(),
                    response: response.clone(),
                }),
                None => Err(unavailable(&request.url, &request_id, waiter.surface)),
            };
            let _ = waiter.answer.send(answer);
        }
    });
}

/// A request's headers as strings.
fn headers(headers: &network::Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned),
            )
        })
        .collect()
}

/// A request's body in Base64, from its entries' bytes or else its text.
fn post_data(request: &network::Request) -> Option<String> {
    if let Some(entries) = &request.post_data_entries {
        let mut bytes = Vec::new();
        for entry in entries {
            bytes.extend(BASE64.decode(entry.bytes.as_deref()?.as_bytes()).ok()?);
        }
        return Some(BASE64.encode(&bytes));
    }
    request
        .post_data
        .as_ref()
        .map(|text| BASE64.encode(text.as_bytes()))
}

/// The name the browser gives `resource_type`, such as `Document`.
fn resource_type(resource_type: network::ResourceType) -> String {
    serde_json::to_value(resource_type)
        .ok()
        .and_then(|name| name.as_str().map(str::to_owned))
        .unwrap_or_else(|| "Other".to_owned())
}

/// One frame of the socket `web_socket_id`: text, or bytes in Base64,
/// which the browser already gives.
fn frame_of(
    page_id: &PageId,
    web_socket_id: String,
    frame: &network::WebSocketFrame,
) -> WebSocketFrame {
    WebSocketFrame {
        page_id: page_id.clone(),
        web_socket_id,
        payload: frame.payload_data.clone(),
        #[expect(
            clippy::float_cmp,
            reason = "the browser gives an opcode, a small integer, as a JSON number"
        )]
        binary: frame.opcode == 2.0,
    }
}

/// A console call as Playwright's `ConsoleMessage`.
fn console_message(page_id: &PageId, called: &runtime::ConsoleAPICalledEvent) -> ConsoleMessage {
    let kind = serde_json::to_value(called.r#type)
        .ok()
        .and_then(|name| name.as_str().map(str::to_owned))
        .unwrap_or_else(|| "log".to_owned());
    let text = called
        .args
        .iter()
        .map(described)
        .collect::<Vec<_>>()
        .join(" ");
    let location = called
        .stack_trace
        .as_ref()
        .and_then(|stack| stack.call_frames.first())
        .map_or_else(
            || ConsoleLocation {
                url: String::new(),
                line_number: 0,
                column_number: 0,
            },
            |frame| ConsoleLocation {
                url: frame.url.clone(),
                line_number: u64::try_from(frame.line_number).unwrap_or_default(),
                column_number: u64::try_from(frame.column_number).unwrap_or_default(),
            },
        );
    ConsoleMessage {
        page_id: page_id.clone(),
        kind,
        text,
        location,
    }
}

/// An argument of a console call as its text: a primitive's value, any
/// other value as the browser describes it.
fn described(argument: &runtime::RemoteObject) -> String {
    match &argument.value {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(value) if !value.is_object() && !value.is_array() => value.to_string(),
        _ => argument
            .unserializable_value
            .clone()
            .or_else(|| argument.description.clone())
            .unwrap_or_else(|| {
                serde_json::to_value(argument.r#type)
                    .ok()
                    .and_then(|kind| kind.as_str().map(str::to_owned))
                    .unwrap_or_default()
            }),
    }
}

/// An uncaught exception as Playwright's `WebError`: an error's name and
/// message from its description's first line, a thrown value that is no
/// error as `Error` with its value as the message.
fn web_error(page_id: &PageId, details: &runtime::ExceptionDetails) -> WebError {
    let thrown = details.exception.as_ref();
    let is_error =
        thrown.is_some_and(|thrown| thrown.subtype == Some(runtime::RemoteObjectSubtype::Error));
    let description = thrown.and_then(|thrown| thrown.description.clone());
    if is_error && let Some(description) = description {
        let name = thrown
            .and_then(|thrown| thrown.class_name.clone())
            .unwrap_or_else(|| "Error".to_owned());
        let first = description.lines().next().unwrap_or_default();
        let message = first
            .strip_prefix(&format!("{name}: "))
            .or_else(|| (first == name).then_some(""))
            .unwrap_or(first)
            .to_owned();
        return WebError {
            page_id: page_id.clone(),
            name,
            message,
            stack: Some(description),
        };
    }
    WebError {
        page_id: page_id.clone(),
        name: "Error".to_owned(),
        message: thrown.map_or_else(|| details.text.clone(), described),
        stack: None,
    }
}
