//! The model-based test of a Page's owner (spec #2256, seam 3; TESTING.md,
//! Model-based tests), at the parts it holds so far: its one reader, its
//! events, its dialogs, its init scripts and its Gesture order. Parts of
//! the page subscribe on the page's session or on a frame's, early or late,
//! and leave; `page.on("request")` listeners start and end, and waits for a
//! request arm; dialog handlers arm, each answering the dialogs it is asked
//! about in its own way, and leave, and a call takes the failure one left;
//! init scripts are added and dropped, and the browser takes each addition
//! and removal in any order; Gestures ask for the turn, come to between two
//! of their checks, and end or are cut, while what clicks on the page
//! beside them asks for the pointer and gives it back; the browser sends
//! events or requests on any session still attached, opens a dialog,
//! detaches a frame, or detaches the page; the page closes; and its
//! Session's owner ends, in any order, against a browser played in process
//! and a live Run, whose Client replies to the dialog handlers' requests.
//! After each step, synced by a command whose answer the browser sends
//! after its events ([`super::PageReader::after_events`]), by a listener of
//! the test's own that heard the step's requests, and by the browser's
//! commands each dialog and init script step awaits:
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
//! - the Gestures got the turn in the order they asked for it, one at a
//!   time, whatever else the page did; the pointer is at one holder at a
//!   time, lent while no Gesture has the turn or as the one that has it
//!   comes to between two checks; and once the Session's owner ended, each
//!   ask still waiting, and each later one, raises `closed`;
//! - each dialog the page opened while it was open was asked of its armed
//!   handlers, the latest first, until one answered or failed, and the
//!   browser got that answer, or else the dialog's default; a handler that
//!   failed, before its answer or after, left, its subscription ending with
//!   `page_handler_failed`, and left that failure for the page's next call
//!   to take, once; each other handler's subscription ended once the page
//!   closed, and one armed after raised `closed`;
//! - each init script is registered once the browser took its addition, and
//!   the page's navigations wait, while the page is open, exactly while an
//!   addition or a removal is on its way; a script dropped is removed, at
//!   once once registered, or else once its addition was taken;
//! - each Gesture asked the owner once (`C3.owner_round_trips`);
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
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::dialog::{DialogId, DialogsEnded};
use goodfetch_protocol::page::events::{PageEventName, PageRequest};
use goodfetch_protocol::page::route::PAGE_HANDLER_FAILED;
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_test_support::{TestEnv, runtime};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::dialogs::{Answer, Dialogs};
use crate::driver::PageDriver;
use crate::driver::replay::{self, Played, SESSION};
use crate::gesture_order::{GestureOrder, GestureTurn, LentPointer, PointerAsks};
use crate::init_scripts::Registered;
use crate::interception::Interception;
use crate::interception::tests::hold_page;
use crate::live_run::LiveRun;
use crate::page_events::Listening;
use crate::page_events::tests::started;
use crate::page_owner::tests::parts;
use crate::page_owner::{PageOwner, PageOwners};

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
    /// A call arms a dialog handler, which replies so to each dialog.
    Arm(Reply),
    /// A dialog handler's call ends, its subscription dropped.
    Disarm(Index),
    /// The page opens a dialog, a `beforeunload` one or else a `confirm`.
    Dialog(bool),
    /// A call takes the failure a dialog handler left.
    TakeFailure,
    /// A call adds an init script.
    AddScript,
    /// The browser takes an addition or a removal of an init script.
    Script(Index),
    /// An init script's call is dropped.
    DropScript(Index),
    /// The browser detaches the frame, which it tells on the page's session.
    DetachFrame(usize),
    /// The browser detaches the page, which it tells on its own session.
    DetachPage,
    /// The page closes, as its Session takes it out.
    Close,
    /// A step of the page's Gesture order.
    Order(OrderStep),
}

#[derive(Clone, Debug)]
enum OrderStep {
    /// A Gesture asks for the turn.
    Gesture,
    /// The Gesture whose turn it is comes to between two of its checks.
    BetweenChecks,
    /// A Gesture ends, or is cut while it waits.
    EndGesture(Index),
    /// What clicks on the page beside the Gestures asks for its pointer.
    AskPointer,
    /// What asked for the pointer gives it back, or gives up its wait.
    LeavePointer(Index),
    /// The Session's owner, and the Page's under it, ends.
    EndOwner,
}

/// How an ask for the turn or the pointer stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asked {
    Waiting,
    Held,
    /// The page's owner, or the page, went first: it raised `closed`.
    Refused,
}

/// What the model expects of the Gesture order: each ask for the turn and
/// for the pointer, the oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Order {
    turns: Vec<Asked>,
    pointers: Vec<Asked>,
    /// Whether the Gesture whose turn it is waits for the pointer it lent.
    lending: bool,
    /// Whether the page's owner ended.
    ended: bool,
    /// How many Gestures asked the page's owner for the turn.
    asks: u64,
}

impl Order {
    /// The steps to choose from, each with its weight.
    fn steps(&self) -> Vec<(u32, BoxedStrategy<OrderStep>)> {
        let index = any::<Index>;
        let mut steps = vec![
            (12, Just(OrderStep::Gesture).boxed()),
            (6, Just(OrderStep::AskPointer).boxed()),
        ];
        if !self.turns.is_empty() {
            steps.push((6, index().prop_map(OrderStep::EndGesture).boxed()));
        }
        if !self.pointers.is_empty() {
            steps.push((6, index().prop_map(OrderStep::LeavePointer).boxed()));
        }
        for (weight, step) in [(6, OrderStep::BetweenChecks), (1, OrderStep::EndOwner)] {
            if self.allows(&step) {
                steps.push((weight, Just(step).boxed()));
            }
        }
        steps
    }

    fn allows(&self, step: &OrderStep) -> bool {
        match step {
            OrderStep::Gesture | OrderStep::AskPointer => true,
            OrderStep::BetweenChecks => self.running() && !self.lending,
            OrderStep::EndGesture(_) => !self.turns.is_empty(),
            OrderStep::LeavePointer(_) => !self.pointers.is_empty(),
            OrderStep::EndOwner => !self.ended,
        }
    }

    /// Takes in `step`, the page open or, when `closed`, not.
    fn take(&mut self, step: &OrderStep, closed: bool) {
        match step {
            OrderStep::Gesture if closed => self.turns.push(Asked::Refused),
            OrderStep::Gesture => {
                self.asks += 1;
                self.turns.push(Asked::Waiting);
            }
            OrderStep::BetweenChecks => {
                self.lending = Self::grant(&mut self.pointers);
                return;
            }
            OrderStep::EndGesture(index) => {
                self.turns.remove(index.index(self.turns.len()));
            }
            OrderStep::AskPointer => self.pointers.push(Asked::Waiting),
            OrderStep::LeavePointer(index) => {
                self.pointers.remove(index.index(self.pointers.len()));
            }
            OrderStep::EndOwner => self.ended = true,
        }
        self.settle();
    }

    /// Grants the oldest ask of `asks` still waiting, and answers whether
    /// one was.
    fn grant(asks: &mut [Asked]) -> bool {
        let oldest = asks.iter_mut().find(|asked| **asked == Asked::Waiting);
        oldest.map(|asked| *asked = Asked::Held).is_some()
    }

    fn refuse(asks: &mut [Asked]) {
        for asked in asks {
            if *asked == Asked::Waiting {
                *asked = Asked::Refused;
            }
        }
    }

    fn running(&self) -> bool {
        self.turns.contains(&Asked::Held)
    }

    /// Grants what can be, once something asked, ended or was given back.
    fn settle(&mut self) {
        if self.ended {
            Self::refuse(&mut self.turns);
            // A Gesture whose turn it was keeps the pointer, and lends it.
            if !self.running() {
                Self::refuse(&mut self.pointers);
            }
        }
        if self.pointers.contains(&Asked::Held) {
            // One that ended with its pointer lent lends it no more.
            self.lending &= self.running();
            return;
        }
        self.lending = false;
        if !self.running() && !Self::grant(&mut self.turns) {
            Self::grant(&mut self.pointers);
        }
    }
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

/// How a dialog handler replies to each dialog it is asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Accept,
    Dismiss,
    /// It ends without an answer: the dialog falls through.
    Through,
    /// It fails before it answers.
    Fail,
    /// It accepts, then fails.
    AcceptThenFail,
}

/// How a dialog handler's subscription stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handled {
    Armed,
    /// The page closed: its subscription ended.
    Ended,
    /// It failed: its subscription ended with `page_handler_failed`.
    Failed,
    /// The page had closed when it was armed: its call raised `closed`.
    Refused,
}

/// What a call that takes a dialog handler's failure got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Took {
    Nothing,
    /// The failure a handler left, `page_handler_failed`.
    Failure,
    /// The page had closed: its call raised `closed`.
    Refused,
}

/// A command of an init script's the browser has yet to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scripted {
    /// The `n`th script's addition.
    Add(usize),
    /// The `n`th script's removal.
    Remove(usize),
}

/// What the reference model knows.
#[derive(Clone, Debug, Default)]
struct Reference {
    parts: Vec<Part>,
    listeners: Vec<Heard>,
    waits: Vec<Waited>,
    /// The dialog handlers, in the order they were armed.
    handlers: Vec<(Reply, Handled)>,
    /// Whether the browser accepted each dialog, in order.
    answers: Vec<bool>,
    /// Whether a dialog handler's failure waits for the page's next call.
    failure: bool,
    took: Vec<Took>,
    /// Each init script's call, by its number, and whether it is
    /// registered.
    scripts: Vec<(usize, bool)>,
    /// The init scripts' commands the browser has yet to take, in the
    /// order sent.
    commands: Vec<Scripted>,
    /// How many init scripts are on their way in or out.
    pending: usize,
    /// How many init scripts were added, which numbers the next one.
    added: usize,
    frames: [Frame; FRAMES],
    order: Order,
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

impl Reference {
    /// The page opened a dialog: its armed handlers are asked, the latest
    /// first, until one answers or fails; the browser gets that answer, or
    /// else the dialog's default, which accepts a `beforeunload` dialog
    /// alone.
    fn opened(&mut self, before_unload: bool) {
        let mut answer = None;
        for (reply, handled) in self.handlers.iter_mut().rev() {
            if *handled != Handled::Armed {
                continue;
            }
            match reply {
                Reply::Through => continue,
                Reply::Accept => answer = Some(true),
                Reply::Dismiss => answer = Some(false),
                Reply::Fail => *handled = Handled::Failed,
                Reply::AcceptThenFail => {
                    answer = Some(true);
                    *handled = Handled::Failed;
                }
            }
            if *handled == Handled::Failed {
                self.failure = true;
            }
            break;
        }
        self.answers.push(answer.unwrap_or(before_unload));
    }

    /// A handler answering `reply` is armed, unless the page closed.
    fn arm(&mut self, reply: Reply) {
        let handled = if self.closed {
            Handled::Refused
        } else {
            Handled::Armed
        };
        self.handlers.push((reply, handled));
    }

    /// The failure of a handler is taken, unless the page closed.
    fn take_failure(&mut self) {
        let took = if self.closed {
            Took::Refused
        } else if std::mem::take(&mut self.failure) {
            Took::Failure
        } else {
            Took::Nothing
        };
        self.took.push(took);
    }

    /// An init script's call is made: its command is on its way.
    fn add_script(&mut self) {
        self.scripts.push((self.added, false));
        self.commands.push(Scripted::Add(self.added));
        self.added += 1;
        self.pending += 1;
    }

    /// The browser answers the init-script command `index` picks: an added
    /// script is registered, or removed once its call was dropped.
    fn script(&mut self, index: Index) {
        match self.commands.remove(index.index(self.commands.len())) {
            Scripted::Add(n) => match self.scripts.iter_mut().find(|(script, _)| *script == n) {
                Some((_, registered)) => {
                    *registered = true;
                    self.pending -= 1;
                }
                // Its call was dropped: it is removed, still on its way.
                None => self.commands.push(Scripted::Remove(n)),
            },
            Scripted::Remove(_) => self.pending -= 1,
        }
    }

    /// The init script's call `index` picks is dropped: a registered script
    /// is removed.
    fn drop_script(&mut self, index: Index) {
        let (n, registered) = self.scripts.remove(index.index(self.scripts.len()));
        if registered {
            self.commands.push(Scripted::Remove(n));
            self.pending += 1;
        }
    }

    /// The page closes: what listened, waited or was armed on it ends.
    fn close(&mut self) {
        self.closed = true;
        for heard in &mut self.listeners {
            if heard.listened == Listened::Going {
                heard.listened = Listened::Ended;
            }
        }
        for waited in &mut self.waits {
            if *waited == Waited::Waiting {
                *waited = Waited::Ended;
            }
        }
        for (_, handled) in &mut self.handlers {
            if *handled == Handled::Armed {
                *handled = Handled::Ended;
            }
        }
    }
}

fn reply() -> impl Strategy<Value = Reply> {
    prop_oneof![
        Just(Reply::Accept),
        Just(Reply::Dismiss),
        Just(Reply::Through),
        Just(Reply::Fail),
        Just(Reply::AcceptThenFail),
    ]
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
        // The page's session detaches once no init script's command waits
        // for it, whose failure the model leaves out.
        if !state.page_detached && state.commands.is_empty() {
            options.push(Just(Transition::DetachPage).boxed());
        }
        let mut owned = vec![
            reply().prop_map(Transition::Arm).boxed(),
            Just(Transition::TakeFailure).boxed(),
        ];
        if !state.handlers.is_empty() {
            owned.push(any::<Index>().prop_map(Transition::Disarm).boxed());
        }
        if !state.commands.is_empty() {
            owned.push(any::<Index>().prop_map(Transition::Script).boxed());
        }
        if state.attached(On::Page) && !state.closed {
            owned.push(any::<bool>().prop_map(Transition::Dialog).boxed());
            owned.push(Just(Transition::AddScript).boxed());
        }
        if state.attached(On::Page) && !state.scripts.is_empty() {
            owned.push(any::<Index>().prop_map(Transition::DropScript).boxed());
        }
        // Half the steps act on the Gesture order, and few end the page or
        // its owner, so the few cases of the Quick suite hold several
        // Gestures waiting at once.
        let order = prop::strategy::Union::new_weighted(state.order.steps());
        let mut groups = vec![
            (16, prop::strategy::Union::new(options).boxed()),
            (12, prop::strategy::Union::new(owned).boxed()),
            (30, order.prop_map(Transition::Order).boxed()),
        ];
        if !state.closed {
            groups.push((1, Just(Transition::Close).boxed()));
        }
        prop::strategy::Union::new_weighted(groups).boxed()
    }

    fn preconditions(state: &Self, transition: &Transition) -> bool {
        match transition {
            Transition::Subscribe(_)
            | Transition::Listen
            | Transition::Wait
            | Transition::Arm(_)
            | Transition::TakeFailure => true,
            Transition::Order(step) => state.order.allows(step),
            Transition::Leave(_) => !state.parts.is_empty(),
            Transition::Unlisten(_) => !state.listeners.is_empty(),
            Transition::Disarm(_) => !state.handlers.is_empty(),
            Transition::Script(_) => !state.commands.is_empty(),
            Transition::Dialog(_) | Transition::AddScript => {
                state.attached(On::Page) && !state.closed
            }
            Transition::DropScript(_) => state.attached(On::Page) && !state.scripts.is_empty(),
            Transition::Send(on, _) => state.attached(*on),
            Transition::Request(_) => state.attached(On::Page),
            Transition::DetachFrame(frame) => state.attached(On::Frame(*frame)),
            Transition::DetachPage => !state.page_detached && state.commands.is_empty(),
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
            Transition::Arm(reply) => state.arm(*reply),
            Transition::Disarm(index) => {
                state.handlers.remove(index.index(state.handlers.len()));
            }
            Transition::Dialog(before_unload) => {
                state.visits += 1;
                state.hand(On::Page, OPENING);
                state.opened(*before_unload);
            }
            Transition::TakeFailure => state.take_failure(),
            Transition::AddScript => state.add_script(),
            Transition::Script(index) => state.script(*index),
            Transition::DropScript(index) => state.drop_script(*index),
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
            Transition::Close => state.close(),
            Transition::Order(step) => state.order.take(step, state.closed),
        }
        state
    }
}

/// The browser's event that a target's session was detached.
const DETACHED: &str = "Target.detachedFromTarget";

/// The browser's event that the page sent a request.
const REQUESTED: &str = "Network.requestWillBeSent";

/// The browser's event that the page opened a dialog.
const OPENING: &str = "Page.javascriptDialogOpening";

/// The URL of the document that opens each dialog.
const DIALOG_URL: &str = "https://example.test/";

/// The source of the `n`th init script.
fn source(n: usize) -> String {
    format!("window.script{n} = true;")
}

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

/// How many times a step polls what asked and lets the page's owner run:
/// more than the steps one change of the order takes to reach everyone.
const SETTLED: usize = 8;

/// A Gesture's call, or what asked for the pointer, as far as it came.
enum Asking<T> {
    /// It waits for its answer, or for the pointer it lent.
    Waiting {
        future: Pin<Box<dyn Future<Output = Option<T>>>>,
        lending: bool,
    },
    Held(T),
    Refused,
}

impl<T: 'static> Asking<T> {
    fn of(asked: impl Future<Output = Option<T>> + 'static) -> Self {
        Self::Waiting {
            future: Box::pin(asked),
            lending: false,
        }
    }

    /// Takes its answer once it has one.
    fn take(&mut self) {
        let Self::Waiting { future, .. } = self else {
            return;
        };
        if let Some(answered) = future.as_mut().now_or_never() {
            *self = answered.map_or(Self::Refused, Self::Held);
        }
    }

    fn asked(&self) -> Asked {
        match self {
            Self::Waiting { lending: false, .. } => Asked::Waiting,
            // A Gesture lends the pointer within its turn.
            Self::Waiting { lending: true, .. } | Self::Held(_) => Asked::Held,
            Self::Refused => Asked::Refused,
        }
    }
}

/// What a dialog handler's call holds: its subscription's end, until it
/// ended.
type HandlerEnd = Pin<Box<dyn Future<Output = Result<DialogsEnded, CallError>>>>;

/// A dialog handler's call, as far as it came.
struct HandlerCall {
    /// The id of its call, which the engine's requests to it name.
    call: i64,
    reply: Reply,
    ended: Option<HandlerEnd>,
    handled: Handled,
}

impl HandlerCall {
    /// Takes its subscription's end once it ended.
    fn take(&mut self) {
        let Some(ended) = &mut self.ended else {
            return;
        };
        if let Some(ended) = ended.as_mut().now_or_never() {
            self.ended = None;
            self.handled = Self::handled(ended);
        }
    }

    /// How a handler stands whose subscription ended so.
    fn handled(ended: Result<DialogsEnded, CallError>) -> Handled {
        match ended {
            Ok(_) => Handled::Ended,
            Err(error) => {
                assert_eq!(error.code(), PAGE_HANDLER_FAILED.name, "{error:?}");
                Handled::Failed
            }
        }
    }
}

/// An init script's registration on its way.
type Adding = Pin<Box<dyn Future<Output = Result<Registered, CallError>>>>;

/// An init script's call: its registration on its way, then the script it
/// keeps.
struct ScriptCall {
    /// The script's number.
    n: usize,
    adding: Option<Adding>,
    registered: Option<Registered>,
}

impl ScriptCall {
    /// Takes its registration once it answered.
    fn take(&mut self) {
        let Some(adding) = &mut self.adding else {
            return;
        };
        if let Some(added) = adding.as_mut().now_or_never() {
            self.adding = None;
            self.registered = Some(added.expect("the page takes its init script"));
        }
    }
}

/// The browser's command that adds an init script.
const ADD_SCRIPT: &str = "Page.addScriptToEvaluateOnNewDocument";

/// The browser's command that removes one.
const REMOVE_SCRIPT: &str = "Page.removeScriptToEvaluateOnNewDocument";

/// The browser's ID of the `n`th init script.
fn identifier(n: usize) -> String {
    format!("script-{n}")
}

/// The Client's reply to the handler request `asked`: none, or a failure.
fn replied(asked: &Value, failed: bool) -> Value {
    if failed {
        json!({
            "jsonrpc": "2.0",
            "id": asked["id"],
            "error": {
                "code": -32000,
                "message": "the dialog handler threw",
                "data": { "code": "handler_failed", "retryable": false },
            },
        })
    } else {
        json!({ "jsonrpc": "2.0", "id": asked["id"], "result": {} })
    }
}

struct Subject {
    /// The page's owner, as a call that reached it before the page closed
    /// holds it.
    owner: PageOwner,
    /// Where the Consent prompt runner asks for the page's pointer.
    pointer: PointerAsks,
    gestures: Vec<Asking<GestureTurn>>,
    pointers: Vec<Asking<LentPointer>>,
    /// Where the page's owner counts its asks.
    owner_work: CountedWork,
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
    /// The engine's open dialogs, which handlers answer.
    dialogs: Dialogs,
    handlers: Vec<HandlerCall>,
    /// How many handlers were armed, which numbers the next one's call.
    armed: i64,
    /// Whether the browser accepted each dialog, in order.
    answers: Vec<bool>,
    took: Vec<Took>,
    scripts: Vec<ScriptCall>,
    /// The init scripts' commands the browser has yet to take.
    commands: Vec<(Scripted, Value)>,
    /// How many init scripts were added, which numbers the next one.
    added: usize,
    closed: bool,
    /// How many events the browser sent, which names the next one.
    sent: usize,
    session: TaskOwner,
    live: LiveRun,
    _env: TestEnv,
    runtime: Runtime,
}

impl Drop for Subject {
    fn drop(&mut self) {
        // A registered init script removes itself on a task of its own.
        let _entered = self.runtime.enter();
        self.scripts.clear();
    }
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

    /// Waits for `future`, beside the connection, the reader and the engine.
    fn until<T>(
        runtime: &Runtime,
        background: &mut Pin<Box<dyn Future<Output = ()>>>,
        future: impl Future<Output = T>,
    ) -> T {
        runtime.block_on(async {
            tokio::select! {
                biased;
                done = future => done,
                () = background.as_mut() => panic!("the connection ended"),
            }
        })
    }

    /// The next command the page sends the browser.
    fn command(&mut self) -> Value {
        let Self {
            runtime,
            played,
            background,
            ..
        } = self;
        Self::until(runtime, background, played.command())
    }

    /// The browser tells the page opened a dialog, a `beforeunload` one or
    /// else a `confirm`: replies to each request of a handler about it as
    /// that handler does, until the browser gets its answer.
    fn dialog(&mut self, before_unload: bool) {
        let opening = replay::event(
            OPENING,
            json!({
                "url": DIALOG_URL,
                "frameId": replay::FRAME,
                "message": "Leave the page?",
                "type": if before_unload { "beforeunload" } else { "confirm" },
                "hasBrowserHandler": false,
            }),
        );
        let Self {
            runtime,
            played,
            background,
            live,
            handlers,
            dialogs,
            answers,
            ..
        } = self;
        let asked_of = |asked: &Value| {
            let call = asked["params"]["id"].as_i64();
            handlers
                .iter()
                .position(|handler| Some(handler.call) == call)
                .expect("the engine asks an armed handler")
        };
        let (accepted, answering) = runtime.block_on(async {
            assert!(played.send(&[opening]).await, "the client left");
            let mut answering = None;
            loop {
                tokio::select! {
                    biased;
                    () = background.as_mut() => panic!("the connection ended"),
                    asked = live.asked.recv() => {
                        let asked = asked.expect("invariant: the Client hears the engine until the test ends");
                        assert_eq!(asked["params"]["dialog"]["message"], "Leave the page?");
                        let handler = asked_of(&asked);
                        match handlers[handler].reply {
                            reply @ (Reply::Through | Reply::Fail) => {
                                live.replies.send(replied(&asked, reply == Reply::Fail)).unwrap();
                            }
                            reply => {
                                let dialog_id = DialogId(asked["params"]["dialog_id"].as_str().unwrap().to_owned());
                                let answer = if reply == Reply::Dismiss {
                                    Answer::Dismiss
                                } else {
                                    Answer::Accept(None)
                                };
                                let answered: Pin<Box<dyn Future<Output = _>>> =
                                    Box::pin(dialogs.answer(&dialog_id, answer).expect("the dialog is open"));
                                answering = Some((asked, reply, answered));
                            }
                        }
                    }
                    command = played.command() => {
                        assert_eq!(command["method"], "Page.handleJavaScriptDialog", "{command}");
                        assert!(played.send(&[replay::answer(&command, json!({}))]).await, "the client left");
                        break (command["params"]["accept"].as_bool().unwrap(), answering);
                    }
                }
            }
        });
        answers.push(accepted);
        let Some((asked, reply, answered)) = answering else {
            return;
        };
        let answered = Self::until(runtime, background, answered);
        assert!(answered.is_ok(), "{answered:?}");
        let fails = reply == Reply::AcceptThenFail;
        live.replies.send(replied(&asked, fails)).unwrap();
        if fails {
            let index = asked_of(&asked);
            let handler = &mut handlers[index];
            let ended = handler
                .ended
                .take()
                .expect("invariant: an armed handler's subscription goes on");
            handler.handled = HandlerCall::handled(Self::until(runtime, background, ended));
        }
    }

    /// The browser takes the init scripts' command `index`: a registration
    /// whose call waits answers it, and one whose call was dropped is
    /// removed.
    fn script(&mut self, index: Index) {
        let (scripted, command) = self.commands.remove(index.index(self.commands.len()));
        let Scripted::Add(n) = scripted else {
            self.step(vec![replay::answer(&command, json!({}))]);
            return;
        };
        self.step(vec![replay::answer(
            &command,
            json!({ "identifier": identifier(n) }),
        )]);
        let Self {
            runtime,
            background,
            scripts,
            ..
        } = self;
        match scripts.iter_mut().find(|script| script.n == n) {
            Some(script) => {
                let adding = script
                    .adding
                    .take()
                    .expect("invariant: a script waits for its addition");
                let added = Self::until(runtime, background, adding);
                script.registered = Some(added.expect("the page takes its init script"));
            }
            None => self.removed(n),
        }
    }

    /// The `n`th init script is removed: its removal is on its way.
    fn removed(&mut self, n: usize) {
        let removal = self.command();
        assert_eq!(removal["method"], REMOVE_SCRIPT, "{removal}");
        assert_eq!(removal["params"]["identifier"], identifier(n));
        self.commands.push((Scripted::Remove(n), removal));
    }

    /// Listens to the page's requests, unless the page closed.
    fn listen(&mut self) {
        let listening = match self.owners.events(&page_id()) {
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
        self.listeners.push(Listener {
            listening,
            heard: Heard {
                urls: Vec::new(),
                listened,
            },
        });
    }

    /// Waits for the page's next request, unless the page closed.
    fn wait(&mut self) {
        let wait = match self.owners.events(&page_id()) {
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
        self.waits.push(wait);
    }

    /// Arms a handler that answers `reply`, unless the page closed.
    fn arm(&mut self, reply: Reply) {
        let call = self.armed;
        self.armed += 1;
        let armed = self
            .owners
            .dialogs(&page_id())
            .and_then(|dialogs| dialogs.arm(&self.live.client, &RequestId::Number(call)));
        let (ended, handled) = match armed {
            Ok(armed) => (Some(Box::pin(armed.ended()) as HandlerEnd), Handled::Armed),
            Err(error) => {
                assert_eq!(error.code(), CLOSED.name, "{error:?}");
                (None, Handled::Refused)
            }
        };
        self.handlers.push(HandlerCall {
            call,
            reply,
            ended,
            handled,
        });
    }

    /// Takes a handler's failure, unless the page closed.
    fn take_failure(&mut self) {
        let took = match self.owners.dialogs(&page_id()) {
            Ok(dialogs) => match dialogs.take_failure() {
                Some(error) => {
                    assert_eq!(error.code(), PAGE_HANDLER_FAILED.name, "{error:?}");
                    Took::Failure
                }
                None => Took::Nothing,
            },
            Err(error) => {
                assert_eq!(error.code(), CLOSED.name, "{error:?}");
                Took::Refused
            }
        };
        self.took.push(took);
    }

    /// Calls `page.add_init_script` for the next script: its command goes
    /// to the browser.
    fn add_script(&mut self) {
        let n = self.added;
        self.added += 1;
        let scripts = self.owner.init_scripts().clone();
        let adding = {
            let _entered = self.runtime.enter();
            scripts.register(source(n), scripts.begin())
        };
        self.scripts.push(ScriptCall {
            n,
            adding: Some(Box::pin(adding)),
            registered: None,
        });
        let command = self.command();
        assert_eq!(command["method"], ADD_SCRIPT, "{command}");
        assert_eq!(command["params"]["source"], source(n));
        self.commands.push((Scripted::Add(n), command));
    }

    /// Drops the init script's call `index` picks: a registered script is
    /// removed.
    fn drop_script(&mut self, index: Index) {
        let script = self.scripts.remove(index.index(self.scripts.len()));
        let (n, registered) = (script.n, script.registered.is_some());
        {
            let _entered = self.runtime.enter();
            drop(script);
        }
        if registered {
            self.removed(n);
        }
    }

    /// Takes `step` of the page's Gesture order.
    fn order_step(&mut self, step: &OrderStep) {
        match step {
            OrderStep::Gesture => {
                let owner = (!self.closed).then(|| self.owner.clone());
                self.gestures.push(Asking::of(async move {
                    match owner?.turn().await {
                        Ok(turn) => Some(turn),
                        Err(error) => {
                            assert_eq!(error.code(), CLOSED.name, "{error:?}");
                            None
                        }
                    }
                }));
            }
            OrderStep::BetweenChecks => {
                let running = self
                    .gestures
                    .iter_mut()
                    .find(|gesture| matches!(gesture, Asking::Held(_)))
                    .expect("invariant: the model's running Gesture has the turn");
                if let Asking::Held(mut turn) = std::mem::replace(running, Asking::Refused) {
                    *running = Asking::Waiting {
                        future: Box::pin(async move {
                            turn.lend_pointer().await;
                            Some(turn)
                        }),
                        lending: true,
                    };
                }
            }
            OrderStep::EndGesture(index) => {
                self.gestures.remove(index.index(self.gestures.len()));
            }
            OrderStep::AskPointer => {
                let pointer = self.pointer.clone();
                self.pointers
                    .push(Asking::of(async move { pointer.lent().await }));
            }
            OrderStep::LeavePointer(index) => {
                self.pointers.remove(index.index(self.pointers.len()));
            }
            OrderStep::EndOwner => self.session.cancel(),
        }
    }

    /// Polls each Gesture and each ask for the pointer, the oldest first,
    /// letting the page's owner run between, until the order's last change
    /// reached them.
    fn settle(&mut self) {
        let Self {
            runtime,
            gestures,
            pointers,
            ..
        } = self;
        runtime.block_on(async {
            for _ in 0..SETTLED {
                gestures.iter_mut().for_each(Asking::take);
                pointers.iter_mut().for_each(Asking::take);
                tokio::task::yield_now().await;
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
        let metrics = Metrics::default();
        let owner_work = metrics.counted_work().clone();
        let (order, pointer) = GestureOrder::new();
        let dialogs = Dialogs::default();
        let (sync, owner) = runtime.block_on(async {
            let events = started(&page_id(), &driver, &live);
            let sync = events.listen(PageEventName::Request);
            let interception = Interception::default();
            let held = SessionId("session-1".to_owned());
            interception.open_session(&held);
            let parts = parts(
                (&page_id(), &driver, &live),
                &dialogs,
                (events, hold_page(&interception, &page_id(), &held)),
            );
            let closed = CallError::new(
                &CLOSED,
                "the page is closed",
                &json!({ "page_id": page_id() }),
            );
            let owner = PageOwner::start(
                &session,
                &tracing::Span::none(),
                (&page_id(), parts, order),
                closed,
                &metrics,
            );
            owners.kept(page_id(), owner.clone());
            (sync, owner)
        });
        Self {
            owner,
            pointer,
            gestures: Vec::new(),
            pointers: Vec::new(),
            owner_work,
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
            dialogs,
            handlers: Vec::new(),
            armed: 0,
            answers: Vec::new(),
            took: Vec::new(),
            scripts: Vec::new(),
            commands: Vec::new(),
            added: 0,
            closed: false,
            sent: 0,
            session,
            live,
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
            Transition::Listen => subject.listen(),
            Transition::Unlisten(index) => {
                subject
                    .listeners
                    .remove(index.index(subject.listeners.len()));
            }
            Transition::Wait => subject.wait(),
            Transition::Arm(reply) => subject.arm(reply),
            Transition::Disarm(index) => {
                subject.handlers.remove(index.index(subject.handlers.len()));
            }
            Transition::Dialog(before_unload) => subject.dialog(before_unload),
            Transition::TakeFailure => subject.take_failure(),
            Transition::AddScript => subject.add_script(),
            Transition::Script(index) => subject.script(index),
            Transition::DropScript(index) => subject.drop_script(index),
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
            Transition::Order(step) => subject.order_step(&step),
        }
        subject.settle();
        for part in &mut subject.parts {
            part.take();
        }
        for listener in &mut subject.listeners {
            listener.take();
        }
        for wait in &mut subject.waits {
            wait.take();
        }
        for handler in &mut subject.handlers {
            handler.take();
        }
        for script in &mut subject.scripts {
            script.take();
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
        let handled: Vec<_> = subject
            .handlers
            .iter()
            .map(|handler| (handler.reply, handler.handled))
            .collect();
        assert_eq!(
            handled, reference.handlers,
            "how each dialog handler stands"
        );
        assert_eq!(
            subject.answers, reference.answers,
            "the browser's answers to the dialogs"
        );
        assert_eq!(subject.took, reference.took, "the failures calls took");
        let scripts: Vec<_> = subject
            .scripts
            .iter()
            .map(|script| (script.n, script.registered.is_some()))
            .collect();
        assert_eq!(scripts, reference.scripts, "the init scripts registered");
        let commands: Vec<_> = subject
            .commands
            .iter()
            .map(|(scripted, _)| *scripted)
            .collect();
        assert_eq!(
            commands, reference.commands,
            "the init scripts' commands on their way"
        );
        assert_eq!(
            subject.owners.settling(&page_id()).is_some(),
            reference.pending > 0 && !reference.closed,
            "whether the page's navigations wait for its init scripts"
        );
        let order = Order {
            turns: subject.gestures.iter().map(Asking::asked).collect(),
            pointers: subject.pointers.iter().map(Asking::asked).collect(),
            lending: subject
                .gestures
                .iter()
                .any(|gesture| matches!(gesture, Asking::Waiting { lending: true, .. })),
            ended: reference.order.ended,
            asks: subject.owner_work.read(Row::ActionOwnerRoundTrips),
        };
        assert_eq!(order, reference.order, "the Gesture order");
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
    fn a_pages_owner_hands_on_what_its_page_sent_until_it_closed_and_grants_gestures_their_turn_in_order(
        sequential 8..32 => Subject
    );
}
