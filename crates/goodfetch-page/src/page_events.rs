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
//! body too, whole or as cut, under its request's ID: the latest
//! `REQUESTED_LIMIT` such bodies, an older one then `body_unavailable`.
//!
//! The page's response observer, when its Session registered one (ADRs
//! 0110 and 0121), receives each response the page's events report whose
//! URL its patterns match, with its body once it finished, cut at its byte
//! cap: one a service worker answered too, marked as the browser marks it.
//!
//! A request the interception refused carries the refusal's Error code in
//! its `failure`, and one no call waited on is recorded in the Run trace as
//! `page.request_failed`, so a refusal is never a timeout.
//!
//! The app-state waits on a page's requests, its URL and its load states
//! are armed here when their call arrives (ADR 0080): a request waiter is
//! answered by the next matching request sent, a response waiter by the
//! next matching response, a redirect's included, and the page's document
//! (`document`) answers the URL and load state waits. Each waits at most
//! its call's own `timeout`, else the Run's `timeout.wait`.
//!
//! Size: one page's event reader and the state its calls read, kept whole
//! so each event changes that state in one place, under one lock.
#![expect(
    clippy::disallowed_types,
    reason = "listeners and waiters are armed as their call arrives, before any `.await`: each lock guards one page's state for one step, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use data_encoding::BASE64;
use goodfetch_cdp::generated::{network, page, runtime};
use goodfetch_cdp::{NetError, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::Events;
use goodfetch_engine::response_limits::{LimitExceeded, Limits};
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::page::events::{
    BODY_UNAVAILABLE, BodyUnavailableDetails, PAGE_CONSOLE, PAGE_DOM_CONTENT_LOADED, PAGE_LOAD,
    PAGE_PAGE_ERROR, PAGE_REQUEST, PAGE_REQUEST_FAILED, PAGE_REQUEST_FINISHED, PAGE_RESPONSE,
    PAGE_WEB_SOCKET, PageEventName, PageExchange, PageFired, PageRequest, PageWebSocket,
    RequestFailure, WEB_SOCKET_CLOSE, WEB_SOCKET_FRAME_RECEIVED, WEB_SOCKET_FRAME_SENT,
    WebSocketClosed, WebSocketFrame,
};
use goodfetch_protocol::page::{
    BodyHandle, PageId, PageResponse, RESPONSE_TRUNCATED, ResponseTruncatedDetails,
};
use tokio::sync::{Notify, oneshot, watch};
use tokio_util::sync::CancellationToken;

mod document;
mod latest;
mod runtime_values;
mod waiters;

pub(crate) use self::document::WaitDeadline;
use self::document::{Document, InFlight, UrlWaiter};
use self::latest::Latest;
use self::runtime_values::{console_message, web_error};
use self::waiters::{Armed, answer_armed};
use crate::driver::{Child, ChildKind, OnChild, PageDriver, response_of};
use crate::page_value::{Kept, ObservedResponse, Observer};
use crate::url_pattern::UrlPattern;

/// The refusals the interception made, by the browser's ID of the request,
/// which the request's failure then carries: the refusal's Error code, and
/// whether the Run trace records it, as it does when no call waits on the
/// request.
/// It also hands each main-frame navigation the navigation gate refused,
/// with no navigation call waiting on it, to the waiters armed for it.
#[derive(Clone, Default)]
pub(crate) struct Refusals(Arc<RefusalsShared>);

#[derive(Default)]
struct RefusalsShared {
    requests: Mutex<BTreeMap<String, (ErrorData, bool)>>,
    /// The latest main-frame navigation refused that no call waited on.
    navigations: watch::Sender<Option<Arc<RefusedNavigation>>>,
}

/// A main-frame navigation refused with no navigation call waiting on it:
/// its URL, and the error it raises from the waiters armed for it.
pub(crate) struct RefusedNavigation {
    url: String,
    error: CallError,
}

impl Refusals {
    /// Leaves `refusal` for the request `request_id`, before the browser
    /// fails it; `recorded` when no call waits on it.
    pub(crate) fn leave(&self, request_id: &str, refusal: ErrorData, recorded: bool) {
        lock(&self.0.requests).insert(request_id.to_owned(), (refusal, recorded));
    }

    fn take(&self, request_id: &str) -> Option<(ErrorData, bool)> {
        lock(&self.0.requests).remove(request_id)
    }

    /// The main-frame navigation to `url` was refused with `error`, and no
    /// navigation call waits on it: each waiter armed for it raises it.
    pub(crate) fn refuse_navigation(&self, url: &str, error: CallError) {
        self.0
            .navigations
            .send_replace(Some(Arc::new(RefusedNavigation {
                url: url.to_owned(),
                error,
            })));
    }

    /// The error of the first main-frame navigation refused from now on
    /// whose URL `matches`; never, while none is.
    fn refused<F: Fn(&str) -> bool>(&self, matches: F) -> impl Future<Output = CallError> + use<F> {
        let mut navigations = self.0.navigations.subscribe();
        async move {
            loop {
                if navigations.changed().await.is_err() {
                    return std::future::pending().await;
                }
                let refused = navigations.borrow_and_update().clone();
                if let Some(refused) = refused
                    && matches(&refused.url)
                {
                    return refused.error.clone();
                }
            }
        }
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
    /// The main frame's document and the load states it reached.
    document: watch::Sender<Document>,
    /// The response observer its Session registered, and what it keeps,
    /// read once when the page opened.
    observing: Option<(Observer, Kept)>,
}

/// How many `page.request` bodies a page keeps for `response.body`: the
/// latest, each a handle to its file in the Run's folder; an older one
/// answers `body_unavailable`.
const REQUESTED_LIMIT: usize = 256;

#[derive(Default)]
struct State {
    /// How many calls listen to each event.
    listening: BTreeMap<PageEventName, usize>,
    /// When the `Runtime` domain was asked for, in milliseconds since the
    /// epoch: what the browser replays from before is passed over.
    runtime_since: Option<f64>,
    requests: BTreeMap<String, Tracked>,
    /// The bodies kept for the responses a waiter answered, or the
    /// `response_limit_exceeded` a body past the Run's limits raises.
    kept: BTreeMap<String, Result<BodyHandle, CallError>>,
    /// The bodies kept for the latest responses `page.request` answered
    /// from the page's Session, whole or cut.
    requested: Latest<Requested, REQUESTED_LIMIT>,
    waiters: Vec<Waiter>,
    /// The `page.wait_for_request` waiters.
    request_waiters: Vec<Armed<PageRequest>>,
    /// The `page.wait_for_response` waiters.
    response_waiters: Vec<Armed<PageExchange>>,
    /// The `page.wait_for_url` waiters.
    url_waiters: Vec<UrlWaiter>,
    /// The main frame's URL.
    url: String,
    /// The requests `networkidle` counts while they are in flight.
    in_flight: InFlight,
    /// The sockets the page opened, by the browser's ID.
    sockets: BTreeMap<String, String>,
    /// The browser's ID of the request of the main frame's document last
    /// judged for its Block cause: by the call that waited for its
    /// navigation, or else by the page's next call.
    judged: Option<String>,
}

/// One request, at its latest hop.
struct Tracked {
    request: PageRequest,
    response: Option<PageResponse>,
    /// When its response arrived, on the Run's clock.
    answered_at: Option<SystemTime>,
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
    Failed(NetError),
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
    /// waiters wait at most `wait`, and each response `observer` keeps is
    /// passed to it.
    #[expect(
        clippy::too_many_arguments,
        reason = "one value per part of the page its events follow, each its own owner's"
    )]
    pub(crate) fn start(
        page_id: PageId,
        driver: PageDriver,
        run: &Run,
        events: Events,
        refusals: Refusals,
        subscription: Subscription,
        wait: Deadline,
        observer: Option<Observer>,
    ) -> Self {
        let shared = Arc::new(Shared {
            page_id,
            driver,
            run: run.clone(),
            events,
            refusals,
            wait,
            clock: run.clock(),
            log: Arc::new(RequestLog::new(
                run.owner().child(),
                run.folder(),
                run.secrets().mask().clone(),
            )),
            state: Mutex::new(State {
                url: "about:blank".to_owned(),
                ..State::default()
            }),
            changed: Notify::new(),
            gone: watch::Sender::new(false),
            closed: watch::Sender::new(false),
            document: watch::Sender::new(Document::blank()),
            observing: observer.map(|observer| {
                let kept = observer.kept();
                (observer, kept)
            }),
        });
        run.spawn(read(subscription, Arc::clone(&shared)));
        run.spawn(document::watch_idle(Arc::clone(&shared)));
        Self { shared }
    }

    /// What the page's events do with each of its out-of-process frames,
    /// which `driver::follow_children` hands over: each one's network is
    /// read from before it runs, so its requests count in the page's
    /// `networkidle` and reach its waiters and events, and a document
    /// request the page started ends when the frame finishes it, on the
    /// frame's own session.
    pub(crate) fn frames(&self) -> OnChild {
        let shared = Arc::clone(&self.shared);
        Arc::new(move |child: &Child| {
            let shared = Arc::clone(&shared);
            let (kind, session, gone) = (child.kind, child.session.clone(), child.gone.clone());
            Box::pin(async move {
                if kind != ChildKind::Frame {
                    return true;
                }
                let events = shared.driver.events_on(&session);
                match shared
                    .driver
                    .command_on(&session, &network::EnableParams::default())
                    .await
                {
                    Ok(_) => shared
                        .run
                        .spawn(read_frame(events, gone, Arc::clone(&shared))),
                    Err(failure) => {
                        tracing::debug!(%failure, "an out-of-process frame went before its network was read");
                    }
                }
                true
            })
        })
    }

    /// Marks the document whose request the browser names `request_id` as
    /// judged, by the call that waited for its navigation.
    pub(crate) fn judged(&self, request_id: &str) {
        lock(&self.shared.state).judged = Some(request_id.to_owned());
    }

    /// When the response to the request `request_id` arrived, on the Run's
    /// clock, once it did.
    pub(crate) fn answered_at(&self, request_id: &str) -> Option<SystemTime> {
        lock(&self.shared.state)
            .requests
            .get(request_id)
            .and_then(|tracked| tracked.answered_at)
    }

    /// The load of the document the main frame committed last, its
    /// response and when it arrived, judged or not; none for the page's
    /// first, blank document, or before its response arrived.
    pub(crate) fn document(&self) -> Option<(String, PageResponse, SystemTime)> {
        let loader = self.shared.document.borrow().loader()?.clone();
        let state = lock(&self.shared.state);
        let tracked = state.requests.get(&loader)?;
        let (response, received) = tracked.response.clone().zip(tracked.answered_at)?;
        drop(state);
        Some((loader, response, received))
    }

    /// The response of the document the main frame committed last, when it
    /// arrived, and the URL its navigation was first sent to, before any
    /// redirect, unless it was judged already, as a navigation no call
    /// waited for leaves it: it is judged from now. None for the page's
    /// first, blank document.
    pub(crate) fn take_unjudged(&self) -> Option<(PageResponse, SystemTime, String)> {
        let loader = self.shared.document.borrow().loader()?.clone();
        let mut state = lock(&self.shared.state);
        if state.judged.as_ref() == Some(&loader) {
            return None;
        }
        let tracked = state.requests.get(&loader)?;
        let (response, received) = tracked.response.clone().zip(tracked.answered_at)?;
        let requested = tracked
            .request
            .redirect_chain
            .first()
            .unwrap_or(&tracked.request.url)
            .clone();
        state.judged = Some(loader);
        Some((response, received, requested))
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
        {
            let mut state = lock(&self.shared.state);
            state.waiters.clear();
            state.request_waiters.clear();
            state.response_waiters.clear();
            state.url_waiters.clear();
        }
        self.shared.gone.send_replace(true);
        self.shared.closed.send_replace(true);
        self.shared.changed.notify_waiters();
    }

    /// Arms a waiter for the next request whose URL `pattern` matches, any
    /// when none, to finish; it answers the request and its response,
    /// whose body it kept, unless `deadline` passes first;
    /// its errors are spelled for `surface`.
    pub(crate) fn wait_for_request_finished(
        &self,
        pattern: Option<UrlPattern>,
        surface: Surface,
        deadline: WaitDeadline,
    ) -> impl Future<Output = Result<PageExchange, CallError>> + use<> {
        let (answer, answered) = oneshot::channel();
        let mut state = lock(&self.shared.state);
        // Those whose call went go now, not at the next finished request.
        state.waiters.retain(|waiter| !waiter.answer.is_closed());
        state.waiters.push(Waiter {
            pattern,
            surface,
            answer,
        });
        drop(state);
        async move {
            match deadline.within("finished request", answered).await? {
                Ok(answer) => answer,
                Err(_) => Err(gone()),
            }
        }
    }

    /// Keeps `body`, the body of the response `page.request` answered from
    /// the page's Session to the request `request_id`, for
    /// [`body`](Self::body).
    pub(crate) fn keep_requested(&self, request_id: String, body: Requested) {
        lock(&self.shared.state).requested.keep(request_id, body);
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
                    return kept.clone();
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
            Ended::Failed(error)
                if responded
                    && !matches!(error, NetError::Aborted | NetError::ContentDecodingFailed) =>
            {
                Err(cut(&url, received, expected))
            }
            Ended::Failed(_) => Err(unavailable(&url, request_id, surface)),
            Ended::Finished => {
                match read_body(shared, request_id, &url, (received, expected)).await {
                    Ok(handle) => Ok(handle),
                    Err(Unread::PastLimit(exceeded)) => Err(exceeded),
                    Err(Unread::Gone) => Err(unavailable(&url, request_id, surface)),
                }
            }
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

/// Why a finished request's body was not read.
enum Unread {
    /// The browser no longer holds it, or it was not stored.
    Gone,
    /// It is past the Run's `network.max_body_size` or
    /// `network.max_decompression_ratio`: `response_limit_exceeded`.
    PastLimit(CallError),
}

/// Reads the body of the finished request `request_id` to `url` from the
/// browser and stores it in the Run's folder, once the Run's limits allow
/// it (ticket #644): `received` decoded bytes the browser counted, from
/// the `Content-Length` it declared, if any, are judged before the body is
/// read, and the bytes read after.
async fn read_body(
    shared: &Shared,
    request_id: &str,
    url: &str,
    (received, declared): (u64, Option<u64>),
) -> Result<BodyHandle, Unread> {
    let limits = shared
        .run
        .settings()
        .map_or_else(Limits::default, |settings| Limits::of(settings.resolved()));
    let past = |exceeded: LimitExceeded| Unread::PastLimit(exceeded.error());
    limits
        .judge_decoded(url, received, declared)
        .map_err(past)?;
    let bytes = body_bytes(shared, request_id).await.ok_or(Unread::Gone)?;
    limits
        .judge_decoded(
            url,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            declared,
        )
        .map_err(past)?;
    match shared.run.store_body(bytes).await {
        Ok(handle) => Ok(handle),
        Err(error) => {
            tracing::warn!(%error, "a response's body was not stored");
            Err(Unread::Gone)
        }
    }
}

/// The body of the finished request `request_id`, after any content
/// coding, as the browser holds it; none when it no longer does.
async fn body_bytes(shared: &Shared, request_id: &str) -> Option<Vec<u8>> {
    let read = shared
        .driver
        .command(&network::GetResponseBodyParams {
            request_id: request_id.to_owned(),
        })
        .await
        .ok()?;
    if read.base64_encoded {
        BASE64.decode(read.body.as_bytes()).ok()
    } else {
        Some(read.body.into_bytes())
    }
}

/// Passes `response` to the page's response observer when it keeps its URL
/// (ADR 0110): with its body once the request `finished`, read from the
/// browser and cut at the observer's byte cap, and with none for a
/// redirect's, which has no body.
fn observe(shared: &Arc<Shared>, response: &PageResponse, finished: bool) {
    let Some((_, kept)) = &shared.observing else {
        return;
    };
    if !kept
        .urls
        .iter()
        .any(|pattern| pattern.matches(&response.url))
    {
        return;
    }
    let (shared, response) = (Arc::clone(shared), response.clone());
    let run = shared.run.clone();
    run.spawn(async move {
        let body = if finished {
            body_bytes(&shared, &response.request_id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let Some((observer, kept)) = &shared.observing else {
            return;
        };
        let headers: Vec<(String, String)> = response
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let passed = ObservedResponse {
            page_id: &shared.page_id,
            url: &response.url,
            body_length: body.len(),
            status: response.status,
            headers: &headers,
            from_service_worker: response.from_service_worker,
        };
        let cut = body.len().min(kept.byte_cap);
        observer.on_response(&passed, &body[..cut]);
    });
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
/// off after `received` decoded bytes, its `Content-Length` declaring
/// `expected` as sent.
fn cut(url: &str, received: u64, expected: Option<u64>) -> CallError {
    // `received` counts decoded bytes and `expected` the bytes as sent, so
    // a compressed body's count can pass its `Content-Length`: the two are
    // never read as a fraction.
    let of = expected.map_or_else(String::new, |expected| {
        format!("; its `Content-Length` declared {expected} as sent")
    });
    CallError::new(
        &RESPONSE_TRUNCATED,
        format!(
            "the body of the response to {url} broke off after {received} bytes once decoded{of}"
        ),
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

/// Reads the network events of one of the page's out-of-process frames
/// until its target goes (`gone`), as the page's own: a document request
/// the page started and the frame finished ends there.
async fn read_frame(mut subscription: Subscription, gone: CancellationToken, shared: Arc<Shared>) {
    while let Some(event) = gone
        .run_until_cancelled(subscription.next())
        .await
        .flatten()
    {
        if !event.method.starts_with(NETWORK) {
            continue;
        }
        let published = take_in(&shared, &event);
        for (event, data) in published {
            shared
                .events
                .publish(event, Some(&shared.page_id.0), &data)
                .await;
        }
    }
}

/// The prefix of the `Network` domain's events.
const NETWORK: &str = "Network.";

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
            tracked.answered_at = Some(shared.clock.wall());
            let exchange = PageExchange {
                request: tracked.request.clone(),
                response,
            };
            answer_armed(
                &mut state.response_waiters,
                &exchange.request.url,
                &exchange,
            );
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
                observe(shared, &response, true);
                answer_waiters(shared, state, request, response);
            }
        }
        document::ended(shared, state, &finished.request_id);
        shared.changed.notify_waiters();
    } else if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
        failed_request(shared, state, &mut out, &failed);
        document::ended(shared, state, &failed.request_id);
        shared.changed.notify_waiters();
    } else {
        document::take_in(shared, state, event);
        take_in_other(shared, state, &mut out, event);
    }
    out
}

/// A request sent, or its next hop after a redirect: the hop before got the
/// redirect as its response, and finished.
fn sent_request(
    shared: &Arc<Shared>,
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
        observe(shared, &exchange.response, false);
        answer_armed(
            &mut state.response_waiters,
            &exchange.request.url,
            &exchange,
        );
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
    answer_armed(&mut state.request_waiters, &request.url, &request);
    document::sent(shared, state, &sent);
    emit(state, out, PageEventName::Request, &PAGE_REQUEST, &request);
    state.requests.insert(
        sent.request_id,
        Tracked {
            request,
            response: None,
            answered_at: None,
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
    // The site's own failure, when the Session's egress relay met it for
    // the browser, which sees only the relay's answer.
    let text = shared
        .driver
        .relay()
        .and_then(|relay| relay.net_error(&tracked.request.url, &failed.error_text, None))
        .unwrap_or(&failed.error_text);
    let error = NetError::of(text);
    let mut request = tracked.request.clone();
    request.failure = Some(RequestFailure {
        error_text: error.name().to_owned(),
        error: refusal.as_ref().map(|(error, _)| error.clone()),
    });
    tracked.ended = Some(Ended::Failed(error));
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
    let received = state
        .requests
        .get(&request.request_id)
        .map_or(0, |tracked| tracked.received);
    let declared = declared_length(&response);
    let shared = Arc::clone(shared);
    let run = shared.run.clone();
    run.spawn(async move {
        let request_id = request.request_id.clone();
        let kept = read_body(&shared, &request_id, &request.url, (received, declared)).await;
        let gone = matches!(kept, Err(Unread::Gone));
        match kept {
            Ok(handle) => {
                lock(&shared.state)
                    .kept
                    .insert(request_id.clone(), Ok(handle));
            }
            Err(Unread::PastLimit(exceeded)) => {
                lock(&shared.state)
                    .kept
                    .insert(request_id.clone(), Err(exceeded));
            }
            Err(Unread::Gone) => {}
        }
        for waiter in matched {
            // A body past the limits is no body the browser dropped: the
            // request finished, and `response.body` raises the limit.
            let answer = if gone {
                Err(unavailable(&request.url, &request_id, waiter.surface))
            } else {
                Ok(PageExchange {
                    request: request.clone(),
                    response: response.clone(),
                })
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
pub(crate) fn post_data(request: &network::Request) -> Option<String> {
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
