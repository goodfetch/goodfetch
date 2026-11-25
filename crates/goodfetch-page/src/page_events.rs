//! A page's events and the bodies of its responses (spec #577's `network`;
//! ADRs 0081, 0085, 0118 and 0121). Each open page has one reader of its
//! target's events, and one of each out-of-process frame's, which follow
//! every request through its hops, response, bytes and end, and send an
//! event of the page to each `page.on` call that listens to it: on that
//! call alone, so only the Connection that armed it gets the page's
//! traffic, under the credits its Client grants it (ADR 0024), lossless
//! (`LISTENER_LIMIT` unsent at most, past which the call raises
//! `page_events_lost`). A listener is armed when its call arrives and
//! removed when the call goes. The page's console and uncaught exceptions,
//! its out-of-process frames' too, come from the `Runtime` domain, a known
//! automation signal (ADR 0019): the engine enables it only once a call
//! listens to `console` or `page_error`, and passes over what the browser
//! replays from before that call. A call on the page sent after such a
//! `page.on` waits until the browser answered it, so what that call makes the
//! page log is read live, never from the 1,000 messages V8 keeps while
//! `Runtime` is off; the wait is on the browser, never on a Client, and its
//! `opening` limit bounds it.
//!
//! Each hop of a redirect chain is its own request, Playwright's
//! `redirectedFrom()`: the browser's ID names a request's first hop, and
//! [`hop_id`] each later one. A hop that a redirect ended finished with the
//! redirect as its response, which has no body.
//!
//! A request's body, in `post_data`, carries the Run's Masks as its bytes
//! do, before it is encoded, so a Secret a form sent stands as its
//! `%name%` (ADR 0092).
//! The reader ends when the page leaves the engine or its target goes, and
//! so do the page's waiters and body reads. A reader that fell too far
//! behind the browser's events has lost some: the page's listeners,
//! waiters and body reads then raise `page_events_lost`, rather than
//! reading the live page as gone, and the engine closes the page, whose
//! every call raises it too (ADR 0146).
//!
//! What the page's events keep is bounded: every request in flight, and of
//! those that ended the latest `ENDED_LIMIT`, the main frame's document's
//! always; and the latest `KEPT_LIMIT` bodies its waiters kept.
//!
//! A body is read on demand, on the session of the frame that loaded it,
//! within its call's own `timeout`, else the Run's `timeout.wait`: kept for
//! a response a
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
//! A body the browser no longer holds, or one past the Run's limits, is
//! passed as unavailable.
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
    reason = "listeners and waiters are armed as their call arrives, before any `.await`: each lock guards one page's state or its refusals for one step, never across an `.await` or another of them (a request's refusal and its relay's error are taken before `state`), and the Tokio `watch` values updated under `state` never take one, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::Events;
use goodfetch_engine::events::{Envelope, EventData};
use goodfetch_engine::response_limits::{LimitExceeded, Limits};
use goodfetch_engine::rpc::send_events;
use goodfetch_engine::run_files::RunFiles;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_trace::{RequestLog, RunTrace};
use goodfetch_engine::runs::outcome::record_outcome;
use goodfetch_engine::secrets::RunSecrets;
use goodfetch_engine::tasks;
use goodfetch_engine::{CallError, CallStream, named};
use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::connection::{Form, WaitedFor};
use goodfetch_protocol::events::Position;
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::naming;
use goodfetch_protocol::page::events::{
    BODY_UNAVAILABLE, BodyUnavailableDetails, PAGE_CONSOLE, PAGE_DOM_CONTENT_LOADED,
    PAGE_EVENTS_LOST, PAGE_LOAD, PAGE_ON, PAGE_PAGE_ERROR, PAGE_REQUEST, PAGE_REQUEST_FAILED,
    PAGE_REQUEST_FINISHED, PAGE_RESPONSE, PAGE_WAIT_FOR_REQUEST_FINISHED, PAGE_WEB_SOCKET,
    PageEventName, PageExchange, PageFired, PageRequest, PageWebSocket, RequestFailure,
    WEB_SOCKET_CLOSE, WEB_SOCKET_FRAME_RECEIVED, WEB_SOCKET_FRAME_SENT, WebSocketClosed,
    WebSocketFrame,
};
use goodfetch_protocol::page::{
    BodyHandle, PageId, PageResponse, RESPONSE_TRUNCATED, ResponseTruncatedDetails,
};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

mod document;
mod latest;
mod requests;
mod waiters;

pub(crate) use self::document::WaitDeadline;
use self::document::{Document, InFlight, UrlWaiter};
use self::latest::Latest;
use self::requests::Requests;
use self::waiters::{Armed, answer_armed};
use crate::cdp::{
    CdpPage, FollowedEvent, FollowedEvents, LoadingFailed, NetError, RequestSent, SocketFrame,
    TargetSession,
};
use crate::driver::cdp_response::{cut_by, declared_length};
use crate::driver::{Child, ChildKind, OnChild, PageDriver, browser_id, hop_id};
use crate::errors::events_lost;
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

/// What a page's events use of a live Run (ADR 0145): its identity, whose
/// tasks they start, whose clock dates them and whose settings bound the
/// bodies they read, and whose outcome a lost event or a refused request
/// records; its files, which store those bodies; its Secrets, which a
/// request's posted data is masked by; and its trace, which logs refused
/// requests. Cloning it is four reference counts.
#[derive(Clone)]
pub(crate) struct RunPowers {
    pub(crate) identity: RunIdentity,
    pub(crate) files: RunFiles,
    pub(crate) secrets: RunSecrets,
    pub(crate) trace: RunTrace,
}

/// One open page's events and bodies, which its owner holds (ADR 0146);
/// clones share them.
#[derive(Clone)]
pub(crate) struct PageEvents {
    shared: Arc<Shared>,
}

struct Shared {
    page_id: PageId,
    driver: PageDriver,
    /// The primitives its events turn on and read bodies by.
    cdp: CdpPage,
    run: RunPowers,
    /// The engine's event stream the page's Session publishes on, which
    /// the page's own events never take.
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
    /// Whether a `page.on` asked for `Runtime` and the browser has not
    /// answered yet: the page's calls wait while it is.
    arming: watch::Sender<bool>,
    /// Whether its reader fell too far behind its target's events and
    /// stopped while the page was open: its calls on events then raise
    /// `page_events_lost`.
    lost: watch::Sender<bool>,
    /// The main frame's document and the load states it reached.
    document: watch::Sender<Document>,
    /// The response observer its Session registered, and what it keeps,
    /// read once when the page opened.
    observing: Option<(Observer, Kept)>,
}

impl Shared {
    /// Starts `future` as one of the Run's tasks, in its span.
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        self.run.identity.spawn(future);
    }

    /// Records that the Run's driver recorded `error_code`, which no reply
    /// carries.
    fn record_outcome(&self, error_code: &str) {
        record_outcome(&self.run.identity, error_code);
    }
}

/// How many `page.request` bodies a page keeps for `response.body`: the
/// latest, each a handle to its file in the Run's folder; an older one
/// answers `body_unavailable`.
const REQUESTED_LIMIT: usize = 256;

/// How many requests that ended a page keeps, besides its main frame's
/// document's: the latest; an older one's body answers `body_unavailable`.
const ENDED_LIMIT: usize = 1_024;

/// How many bodies its `page.wait_for_request_finished` waiters kept a
/// page keeps for `response.body`: the latest, each a handle to its file in
/// the Run's folder; an older one answers `body_unavailable`.
const KEPT_LIMIT: usize = 256;

/// How many events wait unsent for one `page.on` call, its Client granting
/// too few credits, before the call raises `page_events_lost`.
const LISTENER_LIMIT: usize = 1_024;

/// The most events one `$/events` batch of a `page.on` call carries.
const BATCH_LIMIT: usize = 256;

/// The `tracing` target of the event a body read logs at `debug` once it
/// waits for its response to end (ADR 0132: Experimental): a Client's
/// `$/log` then shows the read in flight, as a page function's console
/// line shows its call.
const BODY_READ_TARGET: &str = "goodfetch::page::body_read";

#[derive(Default)]
struct State {
    /// The calls that listen to each event.
    listeners: BTreeMap<PageEventName, Vec<Listener>>,
    /// Where each event's one encode counts (ADR 0140's E1).
    work: CountedWork,
    /// The number the next listener takes.
    next_listener: u64,
    /// When the `Runtime` domain was asked for, in milliseconds since the
    /// epoch: what the browser replays from before is passed over.
    runtime_since: Option<f64>,
    /// The requests, by the browser's ID, each at its latest hop.
    requests: Requests<Tracked, ENDED_LIMIT>,
    /// The URLs of the latest hops a redirect ended, by the engine's ID
    /// for each.
    redirected: Latest<String, ENDED_LIMIT>,
    /// The sessions of the page's out-of-process frames while they live,
    /// with what cancels once each goes.
    frames: Vec<(TargetSession, CancellationToken)>,
    /// The bodies kept for the latest responses a waiter answered, or the
    /// `response_limit_exceeded` a body past the Run's limits raises.
    kept: Latest<Result<BodyHandle, CallError>, KEPT_LIMIT>,
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
    /// The session its events came on: an out-of-process frame's, where
    /// the browser holds its body; none for the page's own.
    session: Option<TargetSession>,
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
    /// The form of the waiting call's Client, whose spelling its errors
    /// take.
    form: Form,
    answer: oneshot::Sender<Result<PageExchange, CallError>>,
}

/// One `page.on` call's listener, as the page's state holds it: the queue
/// its events wait in for the call to send them.
struct Listener {
    id: u64,
    queue: mpsc::Sender<Published>,
    /// Told when its queue was full and it was removed: its call raises
    /// `page_events_lost`.
    overflowed: Option<oneshot::Sender<()>>,
}

/// A listener, which listens until dropped.
pub(crate) struct Listening {
    shared: Arc<Shared>,
    event: PageEventName,
    id: u64,
    /// Its events, until the page goes or its queue overflowed.
    events: mpsc::Receiver<Published>,
    overflowed: oneshot::Receiver<()>,
    /// The browser's answer to `Runtime.enable`, when this listener asked
    /// for it.
    armed: Option<oneshot::Receiver<Result<(), CallError>>>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        if let Some(listeners) = state.listeners.get_mut(&self.event) {
            listeners.retain(|listener| listener.id != self.id);
            if listeners.is_empty() {
                state.listeners.remove(&self.event);
            }
        }
    }
}

impl Listening {
    /// Sends its events on `stream`, its call's, until the page goes, once
    /// the `Runtime` it asked for, if any, is on.
    ///
    /// # Errors
    ///
    /// The code of the page's failure when the browser refused `Runtime`,
    /// and `page_events_lost` once the page's reader lost its events or the
    /// call's Client took too few of them.
    pub(crate) async fn serve(mut self, stream: &CallStream) -> Result<(), CallError> {
        if let Some(armed) = self.armed.take() {
            // None only as the Run stops, which stops this call too.
            armed.await.unwrap_or(Ok(()))?;
        }
        let mut position = 0;
        while let Some(first) = self.events.recv().await {
            let credits = stream.spend(BATCH_LIMIT).await;
            let mut batch = vec![first];
            while batch.len() < credits
                && let Ok(next) = self.events.try_recv()
            {
                batch.push(next);
            }
            stream.refund(credits - batch.len());
            let events = batch
                .into_iter()
                .map(|(event, data)| {
                    position += 1;
                    Envelope {
                        position: Position(position),
                        event: event.name,
                        data,
                        dropped: 0,
                    }
                })
                .collect();
            if !send_events(stream, events).await {
                return Ok(());
            }
        }
        if self.overflowed.try_recv().is_ok() {
            return Err(listener_fell_behind(&self.shared.page_id, self.event));
        }
        if *self.shared.lost.borrow() {
            return Err(events_lost(&self.shared.page_id));
        }
        Ok(())
    }
}

/// Turns `Runtime` on, on the page and then its out-of-process frames, as a
/// task of the Run's, so a `page.on` cancelled meanwhile neither leaves it off
/// nor keeps the page's calls waiting: the answer goes to the call that asked,
/// if it is still there, and the page's calls wait no more.
fn arm_runtime(shared: &Arc<Shared>) -> oneshot::Receiver<Result<(), CallError>> {
    /// Ends the arming however the task ends.
    struct ArmingEnds(Arc<Shared>);
    impl Drop for ArmingEnds {
        fn drop(&mut self) {
            self.0.arming.send_replace(false);
        }
    }
    let (answer, answered) = oneshot::channel();
    let ending = ArmingEnds(Arc::clone(shared));
    shared.spawn(async move {
        let shared = &ending.0;
        let enabled = shared.cdp.follow_console_opening().await;
        if enabled.is_ok() {
            enable_runtime_on_frames(shared).await;
        }
        drop(ending);
        let _ = answer.send(enabled);
    });
    answered
}

/// Turns `Runtime` on in each of the page's out-of-process frames, once a
/// call listens to `console` or `page_error`; a frame gone meanwhile has
/// nothing more to report, and one that does not answer holds the page's
/// calls no longer than a command of the page's opening.
async fn enable_runtime_on_frames(shared: &Shared) {
    let frames: Vec<TargetSession> = {
        let mut state = lock(&shared.state);
        state.frames.retain(|(_, gone)| !gone.is_cancelled());
        state
            .frames
            .iter()
            .map(|(session, _)| session.clone())
            .collect()
    };
    for session in frames {
        if let Err(error) = shared.cdp.follow_frame_console_opening(&session).await {
            tracing::debug!(%error, "an out-of-process frame went before its console was read");
        }
    }
}

impl PageEvents {
    /// Starts reading the events of the page `page_id`, which `driver`
    /// drives for `run`, from now, publishing on `events`; its waiters wait
    /// at most `wait`, and each response `observer` keeps is passed to it.
    pub(crate) fn start(
        page_id: PageId,
        driver: PageDriver,
        run: RunPowers,
        events: Events,
        refusals: Refusals,
        wait: Deadline,
        observer: Option<Observer>,
    ) -> Self {
        let cdp = CdpPage::of(&driver);
        let followed = cdp.followed_events();
        let shared = Arc::new(Shared {
            page_id,
            cdp,
            driver,
            events,
            refusals,
            wait,
            clock: run.identity.clock(),
            log: Arc::new(RequestLog::new(&run.trace)),
            state: Mutex::new(State {
                url: "about:blank".to_owned(),
                work: run.identity.metrics().counted_work().clone(),
                ..State::default()
            }),
            changed: Notify::new(),
            gone: watch::Sender::new(false),
            closed: watch::Sender::new(false),
            arming: watch::Sender::new(false),
            lost: watch::Sender::new(false),
            document: watch::Sender::new(Document::blank()),
            observing: observer.map(|observer| {
                let kept = observer.kept();
                (observer, kept)
            }),
            run,
        });
        shared.spawn(read(followed, Arc::clone(&shared)));
        shared.spawn(document::watch_idle(Arc::clone(&shared)));
        Self { shared }
    }

    /// What the page's events do with each of its out-of-process frames,
    /// which `driver::follow_children` hands over: each one's network is
    /// read from before it runs, so its requests count in the page's
    /// `networkidle` and reach its waiters and events, their bodies read on
    /// the frame's session, and a document request the page started ends
    /// when the frame finishes it, on the frame's own session; its console
    /// and uncaught exceptions too, once a call listens to them.
    pub(crate) fn frames(&self) -> OnChild {
        let shared = Arc::clone(&self.shared);
        Arc::new(move |child: &Child| {
            let shared = Arc::clone(&shared);
            let (kind, session, gone) = (child.kind, child.session.clone(), child.gone.clone());
            Box::pin(async move {
                if kind != ChildKind::Frame {
                    return true;
                }
                let events = shared.cdp.frame_events(&session);
                // Listed before its console is asked for, so a call that
                // listens from now turns it on here, or finds it on.
                let console = {
                    let mut state = lock(&shared.state);
                    state.frames.retain(|(_, gone)| !gone.is_cancelled());
                    state.frames.push((session.clone(), gone.clone()));
                    state.runtime_since.is_some()
                };
                match shared.cdp.follow_frame_network(&session).await {
                    Ok(()) => {
                        if console
                            && let Err(failure) = shared.cdp.follow_frame_console(&session).await
                        {
                            tracing::debug!(%failure, "an out-of-process frame went before its console was read");
                        }
                        shared.spawn(read_frame(events, gone, session, Arc::clone(&shared)));
                    }
                    Err(failure) => {
                        tracing::debug!(%failure, "an out-of-process frame went before its network was read");
                    }
                }
                true
            })
        })
    }

    /// Marks the document whose request's hop the engine names
    /// `request_id` as judged, by the call that waited for its navigation.
    pub(crate) fn judged(&self, request_id: &str) {
        lock(&self.shared.state).judged = Some(browser_id(request_id).to_owned());
    }

    /// When the response to the request's hop `request_id` arrived, on the
    /// Run's clock, once it did.
    pub(crate) fn answered_at(&self, request_id: &str) -> Option<SystemTime> {
        lock(&self.shared.state)
            .requests
            .get(browser_id(request_id))
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
    /// listens. When the event needs `Runtime` and no listener asked for it
    /// yet, it is asked for now, which the Run's clock dates.
    pub(crate) fn listen(&self, event: PageEventName) -> Listening {
        let (queue, events) = mpsc::channel(LISTENER_LIMIT);
        let (overflowed, overflow) = oneshot::channel();
        let mut state = lock(&self.shared.state);
        let id = state.next_listener;
        state.next_listener += 1;
        // A page that went arms nothing: the call answers at once.
        if !*self.shared.closed.borrow() {
            state.listeners.entry(event).or_default().push(Listener {
                id,
                queue,
                overflowed: Some(overflowed),
            });
        }
        let runtime = matches!(event, PageEventName::Console | PageEventName::PageError)
            && state.runtime_since.is_none();
        if runtime {
            state.runtime_since = Some(millis_since_epoch(self.shared.clock.wall()));
            self.shared.arming.send_replace(true);
        }
        drop(state);
        Listening {
            shared: Arc::clone(&self.shared),
            event,
            id,
            events,
            overflowed: overflow,
            armed: runtime.then(|| arm_runtime(&self.shared)),
        }
    }

    /// Sends `event`, the page's `page.crash` or `page.close`, to each call
    /// that listens to `name`.
    pub(crate) fn publish_end(&self, name: PageEventName, event: &'static Event) {
        let fired = PageFired {
            page_id: self.shared.page_id.clone(),
        };
        emit(&mut lock(&self.shared.state), name, event, &fired);
    }

    /// Sends `event` with `data`, an event of the page the engine itself
    /// sees, such as `page.popup`, to each call that listens to `name`.
    pub(crate) fn publish(
        &self,
        name: PageEventName,
        event: &'static Event,
        data: &impl serde::Serialize,
    ) {
        emit(&mut lock(&self.shared.state), name, event, data);
    }

    /// The engine's event stream the page publishes on.
    pub(crate) fn stream(&self) -> &Events {
        &self.shared.events
    }

    /// The page went: every listener's call answers once it sent the
    /// events it holds, and every waiter's call answers.
    pub(crate) fn close(&self) {
        // Closed first, so a listener armed from now arms nothing.
        self.shared.closed.send_replace(true);
        answer_every_waiter(&self.shared);
    }

    /// The wait until no `page.on` of the page sent before now is still
    /// turning `Runtime` on; none while none is.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) fn arming(&self) -> Option<impl Future<Output = ()> + Send + use<>> {
        let mut arming = self.shared.arming.subscribe();
        let on = *arming.borrow_and_update();
        on.then_some(async move {
            let _ = arming.wait_for(|arming| !*arming).await;
        })
    }

    /// Arms a waiter for the next request whose URL `pattern` matches, any
    /// when none, to finish; it answers the request and its response,
    /// whose body it kept, unless `deadline` passes first;
    /// its errors are spelled in `form`.
    pub(crate) fn wait_for_request_finished(
        &self,
        pattern: Option<UrlPattern>,
        form: Form,
        deadline: WaitDeadline,
    ) -> impl Future<Output = Result<PageExchange, CallError>> + use<> {
        let (answer, answered) = oneshot::channel();
        let mut state = lock(&self.shared.state);
        // Those whose call went go now, not at the next finished request.
        state.waiters.retain(|waiter| !waiter.answer.is_closed());
        state.waiters.push(Waiter {
            pattern,
            form,
            answer,
        });
        drop(state);
        let shared = Arc::clone(&self.shared);
        async move {
            match deadline
                .within(WaitedFor::FINISHED_REQUEST, answered)
                .await?
            {
                Ok(answer) => answer,
                Err(_) => Err(gone(&shared)),
            }
        }
    }

    /// Keeps `body`, the body of the response `page.request` answered from
    /// the page's Session to the request `request_id`, for
    /// [`body`](Self::body).
    pub(crate) fn keep_requested(&self, request_id: String, body: Requested) {
        lock(&self.shared.state).requested.keep(request_id, body);
    }

    /// The body the page keeps for the request `request_id`, a file in the
    /// Run's folder that reading needs no browser for: one a waiter or
    /// `page.request` kept, or the error it keeps instead.
    pub(crate) fn kept(&self, request_id: &str) -> Option<Result<BodyHandle, CallError>> {
        kept_in(&lock(&self.shared.state), request_id)
    }

    /// The body of the response to the request `request_id`, once it
    /// ended, as a page body's handle, unless `deadline` passes first.
    ///
    /// # Errors
    ///
    /// `body_unavailable` when the browser no longer holds it, or the page
    /// never sent the request, its fix spelled in `form`,
    /// `response_truncated` when it broke off, and `deadline_exceeded` when
    /// it had not ended by the deadline.
    pub(crate) async fn body(
        &self,
        request_id: &str,
        form: Form,
        deadline: WaitDeadline,
    ) -> Result<BodyHandle, CallError> {
        deadline
            .within(WaitedFor::RESPONSE_BODY, self.ended_body(request_id, form))
            .await?
    }

    /// The body of the response to the request `request_id`, once it
    /// ended, as [`body`](Self::body) answers it, with no deadline.
    async fn ended_body(&self, request_id: &str, form: Form) -> Result<BodyHandle, CallError> {
        let shared = &self.shared;
        let mut waits = false;
        let (url, ended, received, expected) = loop {
            let changed = shared.changed.notified();
            {
                let state = lock(&shared.state);
                if let Some(kept) = kept_in(&state, request_id) {
                    return kept;
                }
                if let Some(url) = state.redirected.get(request_id) {
                    return Err(redirect_has_no_body(url, request_id));
                }
                let Some(tracked) = state
                    .requests
                    .get(browser_id(request_id))
                    .filter(|tracked| tracked.request.request_id == request_id)
                else {
                    return Err(unavailable("", request_id, form));
                };
                if let Some(ended) = &tracked.ended {
                    break (
                        tracked.request.url.clone(),
                        ended.clone(),
                        tracked.received,
                        tracked
                            .response
                            .as_ref()
                            .and_then(|response| declared_length(&response.headers)),
                    );
                }
            }
            if *shared.gone.borrow() {
                return Err(gone(shared));
            }
            if !waits {
                // Once the read waits, the page's end wakes it: what a
                // Client's `$/log` at `debug` shows of a read in flight.
                tracing::debug!(
                    target: BODY_READ_TARGET,
                    request_id,
                    "the body read waits for its response to end"
                );
                waits = true;
            }
            changed.await;
        };
        let responded = lock(&shared.state)
            .requests
            .get(browser_id(request_id))
            .is_some_and(|tracked| tracked.response.is_some());
        match ended {
            Ended::Failed(error) if responded && cut_by(&error) => {
                Err(cut(&url, received, expected))
            }
            Ended::Failed(_) => Err(unavailable(&url, request_id, form)),
            Ended::Finished => {
                match read_body(shared, request_id, &url, (received, expected)).await {
                    Ok(handle) => Ok(handle),
                    Err(Unread::PastLimit(exceeded)) => Err(exceeded),
                    Err(Unread::Gone) => Err(unavailable(&url, request_id, form)),
                }
            }
        }
    }
}

/// The body the page keeps for the request `request_id`, as
/// [`PageEvents::body`] answers it, when it keeps one.
fn kept_in(state: &State, request_id: &str) -> Option<Result<BodyHandle, CallError>> {
    if let Some(kept) = state.kept.get(request_id) {
        return Some(kept.clone());
    }
    match state.requested.get(request_id)? {
        Requested::Whole(kept) => Some(Ok(kept.clone())),
        Requested::Cut {
            url,
            received,
            expected,
        } => Some(Err(cut(url, *received, *expected))),
    }
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
/// it (ticket #644).
async fn read_body(
    shared: &Shared,
    request_id: &str,
    url: &str,
    counted: (u64, Option<u64>),
) -> Result<BodyHandle, Unread> {
    let bytes = judged_body(shared, request_id, url, counted).await?;
    match shared.run.files.store_body(bytes.into()).await {
        Ok(handle) => Ok(handle),
        Err(error) => {
            tracing::warn!(%error, "a response's body was not stored");
            Err(Unread::Gone)
        }
    }
}

/// The body of the finished request `request_id` to `url`, read from the
/// browser once the Run's limits allow it (ticket #644): `received`
/// decoded bytes the browser counted, from the `Content-Length` it
/// declared, if any, are judged before the body is read, and the bytes
/// read after.
async fn judged_body(
    shared: &Shared,
    request_id: &str,
    url: &str,
    (received, declared): (u64, Option<u64>),
) -> Result<Vec<u8>, Unread> {
    let limits = Limits::of_run(shared.run.identity.settings());
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
    Ok(bytes)
}

/// The body of the finished request's hop `request_id`, after any content
/// coding, as the browser holds it, read on the session the request's
/// events came on; none when it no longer does.
async fn body_bytes(shared: &Shared, request_id: &str) -> Option<Vec<u8>> {
    let browser = browser_id(request_id);
    let session = lock(&shared.state)
        .requests
        .get(browser)
        .and_then(|tracked| tracked.session.clone());
    shared.cdp.response_body(browser, session.as_ref()).await
}

/// Passes `response` to the page's response observer when it keeps its URL
/// (ADR 0110): once the request finished, with its body read from the
/// browser within the Run's limits from the `received` decoded bytes the
/// browser counted, and cut at the observer's byte cap, or as unavailable;
/// and with an empty one for a redirect's, which has no body (`received`
/// none).
fn observe(shared: &Arc<Shared>, response: &PageResponse, received: Option<u64>) {
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
    let identity = shared.run.identity.clone();
    identity.spawn(async move {
        let body = match received {
            Some(received) => {
                let counted = (received, declared_length(&response.headers));
                judged_body(&shared, &response.request_id, &response.url, counted)
                    .await
                    .ok()
            }
            None => Some(Vec::new()),
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
            body_length: body.as_ref().map_or(0, Vec::len),
            status: response.status,
            headers: &headers,
            from_service_worker: response.from_service_worker,
        };
        let cut = body
            .as_deref()
            .map(|body| &body[..body.len().min(kept.byte_cap)]);
        observer.on_response(&passed, cut);
    });
}

/// `body_unavailable` for the request `request_id` to `url`, its fix
/// spelled in `form`.
fn unavailable(url: &str, request_id: &str, form: Form) -> CallError {
    let waiter = naming::operation(form, PAGE_WAIT_FOR_REQUEST_FINISHED.name);
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

/// `closed` for a call on a page that went before it answered, or
/// `page_events_lost` once its reader lost its events while it was open.
fn gone(shared: &Shared) -> CallError {
    if *shared.lost.borrow() {
        return events_lost(&shared.page_id);
    }
    CallError::new(
        &goodfetch_protocol::connection::CLOSED,
        "the page closed, or its Run ended, before the call answered",
        &serde_json::json!({}),
    )
}

/// `body_unavailable` for the hop `request_id` to `url`, which a redirect
/// ended: its response has no body, as Playwright's `body()` says.
fn redirect_has_no_body(url: &str, request_id: &str) -> CallError {
    let fix = "read the body of the hop the redirect led to, its `redirected_from` naming this one"
        .to_owned();
    CallError::new(
        &BODY_UNAVAILABLE,
        format!("the response to {url} is a redirect, which has no body: {fix}"),
        &BodyUnavailableDetails {
            url: url.to_owned(),
            request_id: request_id.to_owned(),
            fix,
        },
    )
}

/// `page_events_lost` for a `page.on` call listening to `event` on the page
/// `page_id` whose Client took too few of its events, the event named as
/// that Client listens to it (ADR 0099).
fn listener_fell_behind(page_id: &PageId, event: PageEventName) -> CallError {
    let event = named::event(listened(event));
    CallError::new(
        &PAGE_EVENTS_LOST,
        format!(
            "{LISTENER_LIMIT} `{event}` events of the page `{}` waited unsent for this listener, whose Client granted too few credits: listen again, and grant credits as events are read",
            page_id.0
        ),
        &serde_json::json!({ "page_id": page_id }),
    )
}

/// The catalog's event `page.on` sends for `event`.
fn listened(event: PageEventName) -> &'static Event {
    let name = goodfetch_protocol::wire_name::of(&event);
    PAGE_ON
        .events
        .iter()
        .copied()
        .find(|listened| listened.name.strip_prefix("page.") == Some(name))
        .expect("invariant: `page.on` sends each event it listens to")
}

/// Answers every waiter of the page, which raises what [`gone`] says, every
/// body read waiting for its request to end, and every listener's call once
/// it sent the events it holds.
fn answer_every_waiter(shared: &Shared) {
    {
        let mut state = lock(&shared.state);
        state.listeners.clear();
        state.waiters.clear();
        state.request_waiters.clear();
        state.response_waiters.clear();
        state.url_waiters.clear();
    }
    shared.gone.send_replace(true);
    shared.changed.notify_waiters();
}

fn millis_since_epoch(wall: SystemTime) -> f64 {
    wall.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
        * 1000.0
}

/// Reads the page's events until the page leaves the engine or its target
/// goes. A subscription that fell too far behind while the page was open
/// lost events, its reader's having fallen behind: its waiters, body reads
/// and listeners raise `page_events_lost`, as the page's close, which the
/// Run trace records, makes every call on it do.
async fn read(mut followed: FollowedEvents, shared: Arc<Shared>) {
    let mut closed = shared.closed.subscribe();
    loop {
        let event = tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => break,
            event = followed.next() => event,
        };
        let Some(event) = event else {
            break;
        };
        take_in(&shared, event, None);
    }
    let lost = followed.fell_behind() && !*closed.borrow();
    if lost {
        tracing::warn!(
            page_id = %shared.page_id.0,
            "the page's events fell too far behind and some are lost: its calls on events raise `page_events_lost`"
        );
        shared.lost.send_replace(true);
        answer_every_waiter(&shared);
    } else {
        shared.gone.send_replace(true);
        shared.changed.notify_waiters();
    }
}

/// Reads the network events, console and uncaught exceptions of one of the
/// page's out-of-process frames, on its `session`, until its target goes
/// (`gone`), as the page's own: a document request the page started and
/// the frame finished ends there.
async fn read_frame(
    mut followed: FollowedEvents,
    gone: CancellationToken,
    session: TargetSession,
    shared: Arc<Shared>,
) {
    while let Some(event) = gone.run_until_cancelled(followed.next()).await.flatten() {
        take_in(&shared, event, Some(&session));
    }
    lock(&shared.state)
        .frames
        .retain(|(frame, _)| *frame != session);
}

/// One event a page's events send a listener: its catalog entry and its
/// data.
type Published = (&'static Event, EventData);

/// Sends `data` as `event`'s to each call that listens to `name`, in the
/// order the page's state changes, never waiting: a listener whose queue
/// is full is removed, and its call raises `page_events_lost`.
fn emit(
    state: &mut State,
    name: PageEventName,
    event: &'static Event,
    data: &impl serde::Serialize,
) {
    let Some(listeners) = state.listeners.get_mut(&name) else {
        return;
    };
    // Encoded once, every listener sharing it (ADR 0140's E1).
    let data = EventData::encode(data, &state.work);
    listeners.retain_mut(
        |listener| match listener.queue.try_send((event, data.clone())) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                if let Some(overflowed) = listener.overflowed.take() {
                    let _ = overflowed.send(());
                }
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        },
    );
    if listeners.is_empty() {
        state.listeners.remove(&name);
    }
}

/// Takes in one event of the page's target, or of one of its
/// out-of-process frames on `session`, as it changes the page's state and
/// sends the page's events it makes.
fn take_in(shared: &Arc<Shared>, event: FollowedEvent, session: Option<&TargetSession>) {
    let FollowedEvent::LoadingFailed(failed) = event else {
        take_in_locked(shared, &mut lock(&shared.state), event, session);
        return;
    };
    // A failed request's refusal and its relay's error are taken before the
    // page's state, so its lock never holds another.
    let how = failure_of(shared, &failed);
    let mut guard = lock(&shared.state);
    let state = &mut *guard;
    failed_request(shared, state, &failed, how);
    ended(shared, state, failed.network_id());
    document::ended(shared, state, failed.network_id());
    shared.changed.notify_waiters();
}

/// Takes in an event other than a request's failure, under the page's
/// state's lock.
fn take_in_locked(
    shared: &Arc<Shared>,
    state: &mut State,
    event: FollowedEvent,
    session: Option<&TargetSession>,
) {
    match event {
        FollowedEvent::RequestSent(sent) => sent_request(shared, state, sent, session),
        FollowedEvent::ResponseReceived(received) => {
            let Some(tracked) = state.requests.get_mut(received.network_id()) else {
                return;
            };
            let response = received.response().of(&tracked.request.request_id);
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
            emit(state, PageEventName::Response, &PAGE_RESPONSE, &exchange);
        }
        FollowedEvent::DataReceived { network_id, length } => {
            if let Some(tracked) = state.requests.get_mut(&network_id) {
                tracked.received += length;
            }
        }
        FollowedEvent::LoadingFinished { network_id } => {
            if let Some(tracked) = state.requests.get_mut(&network_id) {
                tracked.ended = Some(Ended::Finished);
                let (request, response, received) = (
                    tracked.request.clone(),
                    tracked.response.clone(),
                    tracked.received,
                );
                emit(
                    state,
                    PageEventName::RequestFinished,
                    &PAGE_REQUEST_FINISHED,
                    &request,
                );
                if let Some(response) = response {
                    observe(shared, &response, Some(received));
                    answer_waiters(shared, state, request, response, Some(received));
                }
            }
            ended(shared, state, &network_id);
            document::ended(shared, state, &network_id);
            shared.changed.notify_waiters();
        }
        event @ (FollowedEvent::FrameNavigated(_)
        | FollowedEvent::NavigatedWithinDocument { .. }
        | FollowedEvent::Lifecycle { .. }) => {
            // An out-of-process frame's own document is no page's document.
            if session.is_none() {
                document::take_in(shared, state, event);
            }
        }
        event => take_in_other(shared, state, event),
    }
}

/// A request sent on `session`, the page's when none, or its next hop
/// after a redirect: the hop before got the redirect as its response, and
/// finished, which its finished waiters hear.
fn sent_request(
    shared: &Arc<Shared>,
    state: &mut State,
    sent: RequestSent,
    session: Option<&TargetSession>,
) {
    let mut chain = Vec::new();
    let mut from = None;
    if let Some(redirect) = sent.redirect()
        && let Some(before) = state.requests.get(sent.network_id())
    {
        let exchange = PageExchange {
            request: before.request.clone(),
            response: redirect.of(&before.request.request_id),
        };
        chain.clone_from(&exchange.request.redirect_chain);
        chain.push(exchange.request.url.clone());
        from = Some(exchange.request.request_id.clone());
        observe(shared, &exchange.response, None);
        answer_armed(
            &mut state.response_waiters,
            &exchange.request.url,
            &exchange,
        );
        emit(state, PageEventName::Response, &PAGE_RESPONSE, &exchange);
        emit(
            state,
            PageEventName::RequestFinished,
            &PAGE_REQUEST_FINISHED,
            &exchange.request,
        );
        state.redirected.keep(
            exchange.request.request_id.clone(),
            exchange.request.url.clone(),
        );
        let (request, response) = (exchange.request, exchange.response);
        answer_waiters(shared, state, request, response, None);
    }
    let request = PageRequest {
        page_id: shared.page_id.clone(),
        request_id: hop_id(sent.network_id(), chain.len()),
        url: sent.url().to_owned(),
        method: sent.method().to_owned(),
        headers: sent.headers(),
        post_data: sent.post_data(shared.run.secrets.mask().masks()),
        resource_type: sent.resource_type(),
        is_navigation_request: sent.is_navigation(),
        redirect_chain: chain,
        redirected_from: from,
        failure: None,
    };
    answer_armed(&mut state.request_waiters, &request.url, &request);
    document::sent(shared, state, &sent);
    emit(state, PageEventName::Request, &PAGE_REQUEST, &request);
    state.requests.insert(
        sent.into_network_id(),
        Tracked {
            request,
            session: session.cloned(),
            response: None,
            answered_at: None,
            received: 0,
            ended: None,
        },
    );
}

/// The request `request_id` ended: the page keeps it among the latest that
/// ended, and its main frame's document's whatever ends after it.
fn ended(shared: &Shared, state: &mut State, request_id: &str) {
    let document = shared.document.borrow().loader().cloned();
    state.requests.end(request_id, document.as_deref());
}

/// What the page's other parts say of a failed request, read before its
/// state is locked.
struct HowItFailed {
    /// The interception's refusal of it, and whether the Run trace records
    /// it.
    refusal: Option<(ErrorData, bool)>,
    /// The network error: the site's own failure when the Session's egress
    /// relay met it for the browser, which sees only the relay's answer,
    /// else the browser's.
    error: NetError,
}

/// Takes the interception's refusal of the request `failed` names and the
/// failure its relay met, if any.
fn failure_of(shared: &Shared, failed: &LoadingFailed) -> HowItFailed {
    let url = lock(&shared.state)
        .requests
        .get(failed.network_id())
        .map(|tracked| tracked.request.url.clone());
    let error = failed.error();
    HowItFailed {
        refusal: shared.refusals.take(failed.network_id()),
        error: url
            .and_then(|url| {
                shared
                    .driver
                    .relay()
                    .and_then(|relay| relay.net_error(&url, &error, None))
            })
            .unwrap_or(error),
    }
}

/// A request failed: its failure carries the interception's refusal, which
/// the Run trace records when no call waited on the request, as the Run's
/// outcome record names its Error code.
fn failed_request(shared: &Shared, state: &mut State, failed: &LoadingFailed, how: HowItFailed) {
    let HowItFailed { refusal, error } = how;
    let Some(tracked) = state.requests.get_mut(failed.network_id()) else {
        return;
    };
    let mut request = tracked.request.clone();
    request.failure = Some(RequestFailure {
        error_text: error.name().to_owned(),
        error: refusal.as_ref().map(|(error, _)| error.clone()),
    });
    tracked.ended = Some(Ended::Failed(error));
    if let Some((refused, true)) = &refusal {
        // The Run trace holds the Error code no reply carries (ADR 0072).
        shared.record_outcome(&refused.code);
        // Its entry goes to the Run trace's writer with no task started,
        // unless it is the Run's first (`C5.spawns`).
        let counted = shared.run.identity.metrics().counted_work().clone();
        tasks::counting_starts_now(counted, Row::InterceptedSpawns, || {
            shared.log.record_now(&PAGE_REQUEST_FAILED, &request);
        });
    }
    emit(
        state,
        PageEventName::RequestFailed,
        &PAGE_REQUEST_FAILED,
        &request,
    );
}

/// The events of sockets, of the main frame's document and of the
/// `Runtime` domain.
fn take_in_other(shared: &Shared, state: &mut State, event: FollowedEvent) {
    let page_id = &shared.page_id;
    match event {
        FollowedEvent::SocketCreated { id, url } => {
            state.sockets.insert(id.clone(), url.clone());
            let socket = PageWebSocket {
                page_id: page_id.clone(),
                web_socket_id: id,
                url,
            };
            emit(state, PageEventName::WebSocket, &PAGE_WEB_SOCKET, &socket);
        }
        FollowedEvent::SocketFrameSent { id, frame } => {
            let frame = frame_of(page_id, id, frame);
            emit(
                state,
                PageEventName::WebSocket,
                &WEB_SOCKET_FRAME_SENT,
                &frame,
            );
        }
        FollowedEvent::SocketFrameReceived { id, frame } => {
            let frame = frame_of(page_id, id, frame);
            emit(
                state,
                PageEventName::WebSocket,
                &WEB_SOCKET_FRAME_RECEIVED,
                &frame,
            );
        }
        FollowedEvent::SocketClosed { id } => {
            if state.sockets.remove(&id).is_some() {
                let closed = WebSocketClosed {
                    page_id: page_id.clone(),
                    web_socket_id: id,
                };
                emit(state, PageEventName::WebSocket, &WEB_SOCKET_CLOSE, &closed);
            }
        }
        FollowedEvent::DomContentLoaded => {
            let fired = PageFired {
                page_id: page_id.clone(),
            };
            emit(
                state,
                PageEventName::DomContentLoaded,
                &PAGE_DOM_CONTENT_LOADED,
                &fired,
            );
        }
        FollowedEvent::Loaded => {
            let fired = PageFired {
                page_id: page_id.clone(),
            };
            emit(state, PageEventName::Load, &PAGE_LOAD, &fired);
        }
        FollowedEvent::ConsoleCalled(called)
            if state
                .runtime_since
                .is_some_and(|since| called.timestamp() >= since) =>
        {
            let message = called.message(page_id);
            emit(state, PageEventName::Console, &PAGE_CONSOLE, &message);
        }
        FollowedEvent::ExceptionThrown(thrown)
            if state
                .runtime_since
                .is_some_and(|since| thrown.timestamp() >= since) =>
        {
            let error = thrown.error(page_id);
            emit(state, PageEventName::PageError, &PAGE_PAGE_ERROR, &error);
        }
        // A request's events and its document's are taken in before, and a
        // console call or exception from before the console was followed
        // is not the page's to pass on.
        _ => {}
    }
}

/// Answers each waiter whose pattern matches the finished `request`, once
/// its body is read and kept, from the `received` decoded bytes the browser
/// counted; at once for a hop a redirect ended (`received` none), whose
/// response has no body.
fn answer_waiters(
    shared: &Arc<Shared>,
    state: &mut State,
    request: PageRequest,
    response: PageResponse,
    received: Option<u64>,
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
    let Some(received) = received else {
        for waiter in matched {
            let _ = waiter.answer.send(Ok(PageExchange {
                request: request.clone(),
                response: response.clone(),
            }));
        }
        return;
    };
    let declared = declared_length(&response.headers);
    let shared = Arc::clone(shared);
    let identity = shared.run.identity.clone();
    identity.spawn(async move {
        let request_id = request.request_id.clone();
        let kept = read_body(&shared, &request_id, &request.url, (received, declared)).await;
        let gone = matches!(kept, Err(Unread::Gone));
        match kept {
            Ok(handle) => {
                lock(&shared.state)
                    .kept
                    .keep(request_id.clone(), Ok(handle));
            }
            Err(Unread::PastLimit(exceeded)) => {
                lock(&shared.state)
                    .kept
                    .keep(request_id.clone(), Err(exceeded));
            }
            Err(Unread::Gone) => {}
        }
        for waiter in matched {
            // A body past the limits is no body the browser dropped: the
            // request finished, and `response.body` raises the limit.
            let answer = if gone {
                Err(unavailable(&request.url, &request_id, waiter.form))
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

/// One frame of the socket `web_socket_id`: text, or bytes in Base64,
/// which the browser already gives.
fn frame_of(page_id: &PageId, web_socket_id: String, frame: SocketFrame) -> WebSocketFrame {
    WebSocketFrame {
        page_id: page_id.clone(),
        web_socket_id,
        payload: frame.payload,
        binary: frame.binary,
    }
}

#[cfg(test)]
pub(crate) mod tests;
