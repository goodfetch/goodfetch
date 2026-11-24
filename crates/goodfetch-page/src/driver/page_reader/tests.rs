//! The model-based test of a Page's reader (spec #2256, seam 3; TESTING.md,
//! Model-based tests): parts of the page subscribe on the page's session or
//! on a frame's, early or late, and leave; the browser sends events on any
//! session still attached, detaches a frame, or detaches the page, in any
//! order, against a browser played in process. After each step, synced by a
//! command whose answer the browser sends after its events
//! ([`PageReader::after_events`]), each part holds, in order, exactly the
//! events of its session the browser sent after it subscribed, a frame's up
//! to its detach and the page's up to the page's, and has ended `Detached`
//! once its session's target, or the page's, detached; and each event the
//! browser sent visited one reader of the connection, the page's, when it
//! is listed under the event's session (`C1.event_visits`).

use std::pin::Pin;
use std::time::Duration;

use futures_util::FutureExt as _;
use goodfetch_cdp::{Connection, SessionId as TargetSession, Subscription, SubscriptionEnd};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_test_support::runtime;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::PageReader;
use crate::driver::replay::{self, Played, SESSION};

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
    /// The browser detaches the frame, which it tells on the page's session.
    DetachFrame(usize),
    /// The browser detaches the page, which it tells on its own session.
    DetachPage,
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

/// What the reference model knows.
#[derive(Clone, Debug, Default)]
struct Reference {
    parts: Vec<Part>,
    frames: [Frame; FRAMES],
    page_detached: bool,
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
        let mut options = vec![on().prop_map(Transition::Subscribe).boxed()];
        if !state.parts.is_empty() {
            options.push(any::<Index>().prop_map(Transition::Leave).boxed());
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
        prop::strategy::Union::new(options).boxed()
    }

    fn preconditions(state: &Self, transition: &Transition) -> bool {
        match transition {
            Transition::Subscribe(_) => true,
            Transition::Leave(_) => !state.parts.is_empty(),
            Transition::Send(on, _) => state.attached(*on),
            Transition::DetachFrame(frame) => state.attached(On::Frame(*frame)),
            Transition::DetachPage => !state.page_detached,
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
        }
        state
    }
}

/// The browser's event that a target's session was detached.
const DETACHED: &str = "Target.detachedFromTarget";

/// The `n`th event the browser sends.
fn method(n: usize) -> String {
    format!("Test.event{n}")
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

struct Subject {
    runtime: Runtime,
    work: CountedWork,
    connection: Connection,
    played: Played,
    reader: PageReader,
    /// The connection's driver and the reader's task.
    background: Pin<Box<dyn Future<Output = ()>>>,
    parts: Vec<Taken>,
    /// How many events the browser sent, which names the next one.
    sent: usize,
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
            reader,
            background,
            ..
        } = self;
        runtime.block_on(async {
            let command = reader.after_events(connection.send_raw(
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
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Reference;

    fn init_test(_: &Reference) -> Self {
        let runtime = runtime();
        let work = CountedWork::default();
        let ((connection, driver), played) =
            runtime.block_on(async { replay::connected_on(&work) });
        let reader = PageReader::new(&connection, &On::Page.session());
        let reading = reader.reading(None);
        Self {
            runtime,
            work,
            connection,
            played,
            reader,
            background: Box::pin(async move {
                let _ = tokio::join!(driver.run(), reading);
            }),
            parts: Vec::new(),
            sent: 0,
        }
    }

    fn apply(mut subject: Self, _: &Reference, transition: Transition) -> Self {
        match transition {
            Transition::Subscribe(on) => {
                let events = subject.reader.events_on(&on.session());
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
        }
        for part in &mut subject.parts {
            part.take();
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
    fn a_page_reader_hands_each_part_the_events_of_its_session_from_when_it_subscribed(
        sequential 1..24 => Subject
    );
}
