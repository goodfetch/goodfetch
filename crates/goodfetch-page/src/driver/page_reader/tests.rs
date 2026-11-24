//! The model-based test of a Page's owner (spec #2256, seam 3; TESTING.md,
//! Model-based tests), at the parts it holds so far: its one reader and its
//! events. Parts of the page subscribe on the page's session or on a
//! frame's, early or late, and leave; `page.on("request")` listeners start
//! and end, and waits for a request arm; the browser sends events or
//! requests on any session still attached, detaches a frame, or detaches
//! the page; and the page closes, in any order, against a browser played in
//! process and a live Run. After each step, synced by a command whose
//! answer the browser sends after its events ([`super::PageReader::after_events`])
//! and by a listener of the test's own that heard the step's requests:
//!
//! - each part holds, in order, exactly the events of its session the
//!   browser sent after it subscribed, a frame's up to its detach and the
//!   page's up to the page's, and has ended `Detached` once its session's
//!   target, or the page's, detached;
//! - each listener heard, in order, the URL of each request the page sent
//!   after it started, until the page closed, which ended it; one started
//!   once the page closed raises `closed`;
//! - each wait is answered by the first request the page sent after it
//!   armed, or raises `closed` once the page closed first, or at once when
//!   it closed before; a page whose target detached leaves its waits
//!   waiting until it closes, so no call is left unanswered;
//! - each event the browser sent visited one reader of the connection, the
//!   page's, when it is listed under the event's session
//!   (`C1.event_visits`).

use std::pin::Pin;
use std::time::Duration;

use futures_util::FutureExt as _;
use goodfetch_cdp::{Connection, SessionId as TargetSession, Subscription, SubscriptionEnd};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::events::{PageEventName, PageRequest};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_test_support::{TestEnv, runtime};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::driver::PageDriver;
use crate::driver::replay::{self, Played, SESSION};
use crate::interception::Interception;
use crate::interception::tests::hold_page;
use crate::live_run::LiveRun;
use crate::page_events::Listening;
use crate::page_events::tests::started;
use crate::page_owner::{PageOwner, PageOwners, PageParts};

/// How many out-of-process frames the page has.
const FRAMES: usize = 2;

/// A session of the page's: its own or a frame's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum On {
    Page,
    Frame(usize),
}

impl On {
    fn session(self) -> TargetSession {
        TargetSession(match self {
            Self::Page => SESSION.to_owned(),
            Self::Frame(frame) => format!("frame-{frame}"),
        })
    }
}

#[derive(Clone, Debug)]
enum Transition {
    /// A part subscribes on the session.
    Subscribe(On),
    /// A part leaves, its subscription dropped.
    Leave(Index),
    /// The browser sends `count` events on the session.
    Send(On, usize),
    /// The page sends `count` requests, which the browser tells on its
    /// session.
    Request(usize),
    /// A call starts listening to the page's requests.
    Listen,
    /// A listener's call ends, its listener dropped.
    Unlisten(Index),
    /// A call waits for the page's next request.
    Wait,
    /// The browser detaches the frame, which it tells on the page's session.
    DetachFrame(usize),
    /// The browser detaches the page, which it tells on its own session.
    DetachPage,
    /// The page closes, as its Session takes it out.
    Close,
}

/// What the model expects of one part.
#[derive(Clone, Debug)]
struct Part {
    on: On,
    methods: Vec<String>,
    ended: Option<SubscriptionEnd>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    /// Whether a part asked for its events since it attached: the page's
    /// reader is then listed under its session.
    listed: bool,
    detached: bool,
}

/// How a listener's listening stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listened {
    Going,
    /// The page closed while it listened.
    Ended,
    /// The page had closed when it started: its call raised `closed`.
    Refused,
}

/// What the model expects of one listener: the URLs of the requests it
/// heard, in order, and how its listening stands.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Heard {
    urls: Vec<String>,
    listened: Listened,
}

/// How a wait for a request stands.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Waited {
    Waiting,
    /// Answered by the request to the URL.
    Answered(String),
    /// The page closed while it waited: its call raised `closed`.
    Ended,
    /// The page had closed when it was armed: its call raised `closed`.
    Refused,
}

/// What the reference model knows.
#[derive(Clone, Debug, Default)]
struct Reference {
    parts: Vec<Part>,
    listeners: Vec<Heard>,
    waits: Vec<Waited>,
    frames: [Frame; FRAMES],
    page_detached: bool,
    closed: bool,
    /// How many events the browser sent, which names the next one.
    sent: usize,
    /// How many readers the connection visited.
    visits: u64,
}

impl Reference {
    fn attached(&self, on: On) -> bool {
        !self.page_detached
            && match on {
                On::Page => true,
                On::Frame(frame) => !self.frames[frame].detached,
            }
    }

    /// Hands the event `method` on `on` to each part of `on` still going.
    fn hand(&mut self, on: On, method: &str) {
        for part in &mut self.parts {
            if part.on == on && part.ended.is_none() {
                part.methods.push(method.to_owned());
            }
        }
    }

    /// Ends each part `ends` picks still going, saying it detached.
    fn detach(&mut self, ends: impl Fn(On) -> bool) {
        for part in &mut self.parts {
            if ends(part.on) && part.ended.is_none() {
                part.ended = Some(SubscriptionEnd::Detached);
            }
        }
    }

    /// The page sent a request to `url`: each listener still going hears
    /// it and each wait still waiting takes it, while the page is open.
    fn requested(&mut self, url: &str) {
        if self.closed {
            return;
        }
        for heard in &mut self.listeners {
            if heard.listened == Listened::Going {
                heard.urls.push(url.to_owned());
            }
        }
        for waited in &mut self.waits {
            if *waited == Waited::Waiting {
                *waited = Waited::Answered(url.to_owned());
            }
        }
    }
}

fn on() -> impl Strategy<Value = On> {
    prop_oneof![Just(On::Page), (0..FRAMES).prop_map(On::Frame)]
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self::default()).boxed()
    }

    fn transitions(state: &Self) -> BoxedStrategy<Transition> {
        let mut options = vec![
            on().prop_map(Transition::Subscribe).boxed(),
            Just(Transition::Listen).boxed(),
            Just(Transition::Wait).boxed(),
        ];
        if !state.parts.is_empty() {
            options.push(any::<Index>().prop_map(Transition::Leave).boxed());
        }
        if !state.listeners.is_empty() {
            options.push(any::<Index>().prop_map(Transition::Unlisten).boxed());
        }
        let attached: Vec<On> = [On::Page, On::Frame(0), On::Frame(1)]
            .into_iter()
            .filter(|on| state.attached(*on))
            .collect();
        if !attached.is_empty() {
            options.push(
                (prop::sample::select(attached), 1..4usize)
                    .prop_map(|(on, count)| Transition::Send(on, count))
                    .boxed(),
            );
        }
        if state.attached(On::Page) {
            options.push((1..4usize).prop_map(Transition::Request).boxed());
        }
        let frames: Vec<usize> = (0..FRAMES)
            .filter(|frame| state.attached(On::Frame(*frame)))
            .collect();
        if !frames.is_empty() {
            options.push(
                prop::sample::select(frames)
                    .prop_map(Transition::DetachFrame)
                    .boxed(),
            );
        }
        if !state.page_detached {
            options.push(Just(Transition::DetachPage).boxed());
        }
        if !state.closed {
            options.push(Just(Transition::Close).boxed());
        }
        prop::strategy::Union::new(options).boxed()
    }

    fn preconditions(state: &Self, transition: &Transition) -> bool {
        match transition {
            Transition::Subscribe(_) | Transition::Listen | Transition::Wait => true,
            Transition::Leave(_) => !state.parts.is_empty(),
            Transition::Unlisten(_) => !state.listeners.is_empty(),
            Transition::Send(on, _) => state.attached(*on),
            Transition::Request(_) => state.attached(On::Page),
            Transition::DetachFrame(frame) => state.attached(On::Frame(*frame)),
            Transition::DetachPage => !state.page_detached,
            Transition::Close => !state.closed,
        }
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        match transition {
            Transition::Subscribe(on) => {
                // A subscription made once the reader ended ends at once.
                let ended = state.page_detached.then_some(SubscriptionEnd::Detached);
                if let On::Frame(frame) = on
                    && !state.frames[*frame].detached
                {
                    state.frames[*frame].listed = true;
                }
                state.parts.push(Part {
                    on: *on,
                    methods: Vec::new(),
                    ended,
                });
            }
            Transition::Leave(index) => {
                state.parts.remove(index.index(state.parts.len()));
            }
            Transition::Send(on, count) => {
                let listed = match on {
                    On::Page => true,
                    On::Frame(frame) => state.frames[*frame].listed,
                };
                for _ in 0..*count {
                    let method = method(state.sent);
                    state.sent += 1;
                    if listed {
                        state.visits += 1;
                    }
                    state.hand(*on, &method);
                }
            }
            Transition::Request(count) => {
                for _ in 0..*count {
                    let url = url(state.sent);
                    state.sent += 1;
                    state.visits += 1;
                    state.hand(On::Page, REQUESTED);
                    state.requested(&url);
                }
            }
            Transition::Listen => {
                let listened = if state.closed {
                    Listened::Refused
                } else {
                    Listened::Going
                };
                state.listeners.push(Heard {
                    urls: Vec::new(),
                    listened,
                });
            }
            Transition::Unlisten(index) => {
                state.listeners.remove(index.index(state.listeners.len()));
            }
            Transition::Wait => {
                let waited = if state.closed {
                    Waited::Refused
                } else {
                    Waited::Waiting
                };
                state.waits.push(waited);
            }
            Transition::DetachFrame(frame) => {
                state.visits += 1;
                state.frames[*frame].detached = true;
                state.frames[*frame].listed = false;
                state.detach(|on| on == On::Frame(*frame));
                state.hand(On::Page, DETACHED);
            }
            Transition::DetachPage => {
                state.page_detached = true;
                state.detach(|_| true);
            }
            Transition::Close => {
                state.closed = true;
                for heard in &mut state.listeners {
                    if heard.listened == Listened::Going {
                        heard.listened = Listened::Ended;
                    }
                }
                for waited in &mut state.waits {
                    if *waited == Waited::Waiting {
                        *waited = Waited::Ended;
                    }
                }
            }
        }
        state
    }
}

/// The browser's event that a target's session was detached.
const DETACHED: &str = "Target.detachedFromTarget";

/// The browser's event that the page sent a request.
const REQUESTED: &str = "Network.requestWillBeSent";

/// The `n`th event the browser sends.
fn method(n: usize) -> String {
    format!("Test.event{n}")
}

/// The URL of the request the browser tells as its `n`th event.
fn url(n: usize) -> String {
    format!("https://example.test/{n}")
}

/// The browser's event that the page sent its `n`th event's request.
fn requested(n: usize) -> Value {
    json!({
        "method": REQUESTED,
        "sessionId": SESSION,
        "params": {
            "requestId": format!("request-{n}"),
            "loaderId": "loader",
            "documentURL": "https://example.test/",
            "request": {
                "url": url(n),
                "method": "GET",
                "headers": {},
                "initialPriority": "High",
                "referrerPolicy": "strict-origin-when-cross-origin",
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "redirectHasExtraInfo": false,
            "type": "XHR",
            "frameId": replay::FRAME,
        },
    })
}

/// The page the test holds.
fn page_id() -> PageId {
    PageId("page-1".to_owned())
}

/// What one part took so far.
struct Taken {
    events: Subscription,
    methods: Vec<String>,
    ended: Option<SubscriptionEnd>,
}

impl Taken {
    /// Takes every event it holds, and its end once it ended.
    fn take(&mut self) {
        while self.ended.is_none() {
            match self.events.next().now_or_never() {
                Some(Some(event)) => self.methods.push(event.method.clone()),
                Some(None) => {
                    self.ended = Some(self.events.ended().unwrap_or(SubscriptionEnd::Disconnected));
                }
                None => break,
            }
        }
    }
}

/// What one listener heard so far; none once its call raised `closed`.
struct Listener {
    listening: Option<Listening>,
    heard: Heard,
}

impl Listener {
    /// Takes every request it heard, and its end once it ended.
    fn take(&mut self) {
        let Some(listening) = &mut self.listening else {
            return;
        };
        let (taken, ended) = listening.taken();
        self.heard.urls.extend(
            taken
                .iter()
                .map(|request| request["url"].as_str().unwrap().to_owned()),
        );
        if ended {
            self.heard.listened = Listened::Ended;
            self.listening = None;
        }
    }
}

/// A wait's call, until it answered.
enum Wait {
    Waiting(Pin<Box<dyn Future<Output = Result<PageRequest, CallError>>>>),
    Done(Waited),
}

impl Wait {
    /// Takes its answer once it answered.
    fn take(&mut self) {
        let Self::Waiting(waiting) = self else {
            return;
        };
        if let Some(answered) = waiting.as_mut().now_or_never() {
            *self = Self::Done(match answered {
                Ok(request) => Waited::Answered(request.url),
                Err(error) => {
                    assert_eq!(error.code(), CLOSED.name, "{error:?}");
                    Waited::Ended
                }
            });
        }
    }

    fn waited(&self) -> Waited {
        match self {
            Self::Waiting(_) => Waited::Waiting,
            Self::Done(waited) => waited.clone(),
        }
    }
}

struct Subject {
    work: CountedWork,
    connection: Connection,
    played: Played,
    driver: PageDriver,
    owners: PageOwners,
    /// The test's own listener of the page's requests, which syncs a step
    /// with the page's events.
    sync: Listening,
    /// The connection's driver, the reader's task and the engine.
    background: Pin<Box<dyn Future<Output = ()>>>,
    parts: Vec<Taken>,
    listeners: Vec<Listener>,
    waits: Vec<Wait>,
    closed: bool,
    /// How many events the browser sent, which names the next one.
    sent: usize,
    _session: TaskOwner,
    _live: LiveRun,
    _env: TestEnv,
    runtime: Runtime,
}

impl Subject {
    /// Sends `messages` from the browser, then the answer to a command the
    /// client sent before, in one write, and waits until the reader handed
    /// them on.
    fn step(&mut self, messages: Vec<Value>) {
        let Self {
            runtime,
            connection,
            played,
            driver,
            background,
            ..
        } = self;
        runtime.block_on(async {
            let command = driver.page_reader.after_events(connection.send_raw(
                "Browser.getVersion",
                None,
                json!({}),
                Duration::from_secs(30),
            ));
            let browser = async {
                let command = played.command().await;
                let mut messages = messages;
                messages.push(json!({ "id": command["id"], "result": {} }));
                assert!(played.send(&messages).await, "the client left");
            };
            tokio::select! {
                biased;
                (answered, ()) = async { tokio::join!(command, browser) } => {
                    assert!(answered.is_ok(), "{answered:?}");
                }
                () = background.as_mut() => panic!("the connection ended"),
            }
        });
    }

    /// Waits until the page's events took in `count` more requests, which
    /// the test's own listener hears.
    fn heard(&mut self, count: usize) {
        let Self {
            runtime,
            sync,
            background,
            ..
        } = self;
        runtime.block_on(async {
            for _ in 0..count {
                tokio::select! {
                    biased;
                    heard = sync.heard() => assert!(heard.is_some(), "the test's listener ended"),
                    () = background.as_mut() => panic!("the connection ended"),
                }
            }
        });
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(_: &Reference) -> Self {
        let runtime = runtime();
        let env = TestEnv::new();
        let work = CountedWork::default();
        let (live, engine) = runtime.block_on(LiveRun::start(&env));
        let ((connection, cdp), played) = runtime.block_on(async { replay::connected_on(&work) });
        let driver = PageDriver::played(connection.clone());
        let reading = driver.reading(None);
        let session = TaskOwner::root(BlockingPolicy::Inline);
        let owners = PageOwners::default();
        let sync = runtime.block_on(async {
            let events = started(&page_id(), &driver, &live);
            let sync = events.listen(PageEventName::Request);
            let interception = Interception::default();
            let held = SessionId("session-1".to_owned());
            interception.open_session(&held);
            let parts = PageParts {
                events,
                interception: hold_page(&interception, &page_id(), &held),
            };
            let closed = CallError::new(
                &CLOSED,
                "the page is closed",
                &json!({ "page_id": page_id() }),
            );
            let owner = PageOwner::start(
                &session,
                &tracing::Span::none(),
                (&page_id(), parts),
                closed,
                &Metrics::default(),
            );
            owners.kept(page_id(), owner);
            sync
        });
        Self {
            work,
            connection,
            played,
            driver,
            owners,
            sync,
            background: Box::pin(async move {
                let _ = tokio::join!(cdp.run(), reading, engine);
            }),
            parts: Vec::new(),
            listeners: Vec::new(),
            waits: Vec::new(),
            closed: false,
            sent: 0,
            _session: session,
            _live: live,
            _env: env,
            runtime,
        }
    }

    fn apply(mut subject: Self, _: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Subscribe(on) => {
                let events = subject.driver.page_reader.events_on(&on.session());
                subject.parts.push(Taken {
                    events,
                    methods: Vec::new(),
                    ended: None,
                });
            }
            Transition::Leave(index) => {
                subject.parts.remove(index.index(subject.parts.len()));
            }
            Transition::Send(on, count) => {
                let first = subject.sent;
                subject.sent += count;
                let events = (first..first + count)
                    .map(|n| {
                        json!({ "method": method(n), "params": {}, "sessionId": on.session().0 })
                    })
                    .collect();
                subject.step(events);
            }
            Transition::Request(count) => {
                let first = subject.sent;
                subject.sent += count;
                subject.step((first..first + count).map(requested).collect());
                if !subject.closed {
                    subject.heard(count);
                }
            }
            Transition::Listen => {
                let listening = match subject.owners.events(&page_id()) {
                    Ok(events) => Some(events.listen(PageEventName::Request)),
                    Err(error) => {
                        assert_eq!(error.code(), CLOSED.name, "{error:?}");
                        None
                    }
                };
                let listened = if listening.is_some() {
                    Listened::Going
                } else {
                    Listened::Refused
                };
                subject.listeners.push(Listener {
                    listening,
                    heard: Heard {
                        urls: Vec::new(),
                        listened,
                    },
                });
            }
            Transition::Unlisten(index) => {
                subject
                    .listeners
                    .remove(index.index(subject.listeners.len()));
            }
            Transition::Wait => {
                let wait = match subject.owners.events(&page_id()) {
                    Ok(events) => {
                        let deadline = events
                            .wait_deadline(Some(None))
                            .expect("invariant: a wait without a deadline has one");
                        Wait::Waiting(Box::pin(events.wait_for_request(None, deadline)))
                    }
                    Err(error) => {
                        assert_eq!(error.code(), CLOSED.name, "{error:?}");
                        Wait::Done(Waited::Refused)
                    }
                };
                subject.waits.push(wait);
            }
            Transition::DetachFrame(frame) => {
                let detached = json!({
                    "method": DETACHED,
                    "params": { "sessionId": On::Frame(frame).session().0 },
                    "sessionId": SESSION,
                });
                subject.step(vec![detached]);
            }
            Transition::DetachPage => {
                let detached = json!({ "method": DETACHED, "params": { "sessionId": SESSION } });
                subject.step(vec![detached]);
            }
            Transition::Close => {
                subject.owners.close(&[page_id()]);
                subject.closed = true;
            }
        }
        for part in &mut subject.parts {
            part.take();
        }
        for listener in &mut subject.listeners {
            listener.take();
        }
        for wait in &mut subject.waits {
            wait.take();
        }
        subject
    }

    fn check_invariants(subject: &Self, reference: &Reference) {
        let taken: Vec<_> = subject
            .parts
            .iter()
            .map(|part| (part.methods.clone(), part.ended))
            .collect();
        let expected: Vec<_> = reference
            .parts
            .iter()
            .map(|part| (part.methods.clone(), part.ended))
            .collect();
        assert_eq!(taken, expected, "what each part took");
        let heard: Vec<_> = subject
            .listeners
            .iter()
            .map(|listener| listener.heard.clone())
            .collect();
        assert_eq!(heard, reference.listeners, "what each listener heard");
        let waited: Vec<_> = subject.waits.iter().map(Wait::waited).collect();
        assert_eq!(waited, reference.waits, "how each wait stands");
        assert_eq!(
            subject.work.read(Row::CdpEventVisits),
            reference.visits,
            "readers the connection visited"
        );
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    #[test]
    fn a_pages_owner_hands_each_part_listener_and_wait_what_its_page_sent_until_it_closed(
        sequential 1..24 => Subject
    );
}
