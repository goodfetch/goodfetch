//! The Live view's driver over fake frame sources and outputs, and its
//! channel over an in-memory WebSocket, on the paused `Clock` (ADR 0059).
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests run on the test Clock (ADR 0059), and the fakes' logs are read and written in one step each"
)]

use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use futures_util::{Sink, Stream};
use goodfetch_protocol::audit::RefusalReason;
use goodfetch_protocol::live_view::{
    LiveViewEnd, LiveViewEngineMessage, LiveViewScreen, LiveViewTab, LiveViewTier,
};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{Error, Message};

use super::driver::{self, Frame};
use super::{
    Admission, Channel, FrameSize, FrameSource, LiveView, NotWatched, SourceFrame, Tiers,
    VideoOutput, Viewing, WatchFailure, WatchedSession, WatchedSessions, Watching,
};
use crate::clock::Clock;
use crate::mask::Masks;
use crate::tasks::{BlockingPolicy, TaskOwner};

const TOKEN: &str = "the-daemon-token";

fn session() -> SessionId {
    SessionId("s1".to_owned())
}

fn page(name: &str) -> PageId {
    PageId(name.to_owned())
}

fn screen(width: u32, height: u32, device_pixel_ratio: f64) -> LiveViewScreen {
    LiveViewScreen {
        width,
        height,
        device_pixel_ratio,
    }
}

fn source_frame(data: &str) -> SourceFrame {
    SourceFrame {
        width: 800.0,
        height: 600.0,
        data: data.to_owned(),
    }
}

/// How a fake source answers `start`.
#[derive(Clone)]
enum Start {
    Answers,
    Fails(WatchFailure),
    Never,
}

/// What the fake sources were asked, in order.
#[derive(Default)]
struct Log(Vec<String>);

/// A tier that captures as its test says: each `start` and `stop` logged
/// under its name, its frames those the test sends.
struct Source {
    name: &'static str,
    log: Arc<Mutex<Log>>,
    start: Start,
    frames: mpsc::UnboundedReceiver<Result<SourceFrame, WatchFailure>>,
}

impl FrameSource for Source {
    fn tier(&self) -> LiveViewTier {
        LiveViewTier::Screencast
    }

    fn start<'a>(
        &'a mut self,
        page: &'a PageId,
        size: FrameSize,
    ) -> Watching<'a, Result<(), WatchFailure>> {
        self.log.lock().unwrap().0.push(format!(
            "{} start {} {}x{}",
            self.name, page.0, size.width, size.height
        ));
        let start = self.start.clone();
        Box::pin(async move {
            match start {
                Start::Answers => Ok(()),
                Start::Fails(failure) => Err(failure),
                Start::Never => std::future::pending().await,
            }
        })
    }

    fn next(&mut self) -> Watching<'_, Result<SourceFrame, WatchFailure>> {
        Box::pin(async move {
            self.frames
                .recv()
                .await
                .unwrap_or(Err(WatchFailure::BrowserGone))
        })
    }

    fn stop(&mut self) -> Watching<'_, ()> {
        self.log
            .lock()
            .unwrap()
            .0
            .push(format!("{} stop", self.name));
        Box::pin(async {})
    }
}

/// The open Sessions, each watch of one taking its next entry.
#[derive(Default)]
struct Sessions(Mutex<BTreeMap<SessionId, VecDeque<WatchedSession>>>);

impl WatchedSessions for Sessions {
    fn watch(&self, session: &SessionId) -> Option<WatchedSession> {
        self.0.lock().unwrap().get_mut(session)?.pop_front()
    }
}

/// A Live view of one Session, `s1`, with tabs `p1` then `p2`, captured by
/// one source per entry of `starts`, named `a`, `b`, … in that order.
struct Watched {
    live_view: LiveView,
    owner: TaskOwner,
    log: Arc<Mutex<Log>>,
    tabs: watch::Sender<Vec<PageId>>,
    frames: Vec<mpsc::UnboundedSender<Result<SourceFrame, WatchFailure>>>,
    clock: Clock,
}

impl Watched {
    fn new(starts: &[Start]) -> Self {
        let log = Arc::new(Mutex::new(Log::default()));
        let (tabs, tabs_now) = watch::channel(vec![page("p1"), page("p2")]);
        let mut frames = Vec::new();
        let mut sources: Vec<Box<dyn FrameSource>> = Vec::new();
        for (start, name) in starts.iter().zip(["a", "b", "c"]) {
            let (sender, receiver) = mpsc::unbounded_channel();
            frames.push(sender);
            sources.push(Box::new(Source {
                name,
                log: Arc::clone(&log),
                start: start.clone(),
                frames: receiver,
            }));
        }
        let sessions = Sessions::default();
        sessions.0.lock().unwrap().insert(
            session(),
            VecDeque::from([driver::watched(tabs_now, sources)]),
        );
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let sessions: Arc<dyn WatchedSessions> = Arc::new(sessions);
        let live_view = LiveView::new(Arc::from([sessions]), &owner, clock);
        Self {
            live_view,
            owner,
            log,
            tabs,
            frames,
            clock,
        }
    }

    fn viewer(&self, screen: LiveViewScreen) -> Viewing {
        self.live_view
            .watch(&session(), screen, 10, Tiers::Any)
            .unwrap()
    }

    /// What the sources were asked since last read.
    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut self.log.lock().unwrap().0)
    }

    fn capture(&self, source: usize, data: &str) {
        self.frames[source].send(Ok(source_frame(data))).unwrap();
    }

    fn fail(&self, source: usize, failure: WatchFailure) {
        self.frames[source].send(Err(failure)).unwrap();
    }

    /// Lets every task run until all wait.
    async fn settle(&self) {
        self.clock.sleep(Duration::from_millis(1)).await;
    }
}

/// The frames a viewer was sent, by number.
#[derive(Default)]
struct Collected(Vec<u64>);

impl VideoOutput for Collected {
    fn send<'a>(&'a mut self, frame: &'a Frame) -> Watching<'a, Result<(), WatchFailure>> {
        self.0.push(frame.number);
        Box::pin(async { Ok(()) })
    }
}

/// What `viewing` is told, frames to `output`, until nothing changes.
async fn told(viewing: &mut Viewing, output: &mut Collected) -> Vec<LiveViewEngineMessage> {
    let news = viewing.news();
    viewing.send_due(output).await.unwrap();
    news
}

fn tabs_message(shown: &str) -> LiveViewEngineMessage {
    LiveViewEngineMessage::Tabs {
        tabs: vec![
            LiveViewTab { page: page("p1") },
            LiveViewTab { page: page("p2") },
        ],
        shown: Some(page(shown)),
    }
}

fn ended(reason: LiveViewEnd) -> LiveViewEngineMessage {
    LiveViewEngineMessage::Ended { reason }
}

#[test]
fn the_capture_fills_the_largest_screen_watching_in_device_pixels() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let first = watched.viewer(screen(800, 600, 2.0));
        watched.settle().await;
        assert_eq!(watched.asked(), ["a start p2 1600x1200"]);

        let second = watched.viewer(screen(1920, 1080, 1.0));
        watched.settle().await;
        assert_eq!(watched.asked(), ["a stop", "a start p2 1920x1200"]);

        drop(second);
        watched.settle().await;
        assert_eq!(watched.asked(), ["a stop", "a start p2 1600x1200"]);
        drop(first);
    });
}

#[test]
fn a_slow_viewer_gets_only_the_newest_frame_once_it_acknowledged() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        let mut output = Collected::default();
        watched.settle().await;
        assert_eq!(told(&mut viewing, &mut output).await, [tabs_message("p2")]);

        watched.capture(0, "one");
        watched.settle().await;
        told(&mut viewing, &mut output).await;
        for data in ["two", "three", "four"] {
            watched.capture(0, data);
        }
        watched.settle().await;
        told(&mut viewing, &mut output).await;
        assert_eq!(output.0, [1], "nothing more before the acknowledgement");

        viewing.acknowledge(1);
        told(&mut viewing, &mut output).await;
        assert_eq!(output.0, [1, 4], "then the newest, never those between");
        told(&mut viewing, &mut output).await;
        assert_eq!(output.0, [1, 4], "and each frame once");
    });
}

#[test]
fn a_viewer_slow_or_gone_holds_up_no_other() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let mut slow = watched.viewer(screen(800, 600, 1.0));
        let mut quick = watched.viewer(screen(800, 600, 1.0));
        let (mut slow_frames, mut quick_frames) = (Collected::default(), Collected::default());
        for data in ["one", "two"] {
            watched.capture(0, data);
            watched.settle().await;
            told(&mut slow, &mut slow_frames).await;
            told(&mut quick, &mut quick_frames).await;
            quick.acknowledge(quick_frames.0.last().copied().unwrap());
        }
        assert_eq!((slow_frames.0, quick_frames.0), (vec![1], vec![1, 2]));
    });
}

#[test]
fn a_viewer_past_the_limit_is_refused_until_one_leaves() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let watch = || {
            watched
                .live_view
                .watch(&session(), screen(800, 600, 1.0), 2, Tiers::Any)
        };
        let first = watch().unwrap();
        let _second = watch().unwrap();
        assert_eq!(watch().err(), Some(NotWatched::ViewerLimit));

        drop(first);
        assert!(watch().is_ok());
    });
}

#[test]
fn a_session_no_driver_offers_or_no_tier_takes_is_not_watched() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let other = SessionId("s2".to_owned());
        let screen = screen(800, 600, 1.0);
        assert_eq!(
            watched
                .live_view
                .watch(&other, screen, 10, Tiers::Any)
                .err(),
            Some(NotWatched::UnknownSession)
        );

        let none = Watched::new(&[]);
        assert_eq!(
            none.live_view
                .watch(&session(), screen, 10, Tiers::Any)
                .err(),
            Some(NotWatched::NoTier)
        );
    });
}

#[test]
fn the_capture_stops_once_the_last_viewer_left() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let first = watched.viewer(screen(800, 600, 1.0));
        let second = watched.viewer(screen(800, 600, 1.0));
        watched.settle().await;
        watched.asked();

        drop(first);
        watched.settle().await;
        assert!(watched.asked().is_empty(), "a viewer still watches");
        drop(second);
        watched.settle().await;
        assert_eq!(watched.asked(), ["a stop"]);
        assert_eq!(
            watched
                .live_view
                .watch(&session(), screen(800, 600, 1.0), 10, Tiers::Any)
                .err(),
            Some(NotWatched::UnknownSession),
            "a viewer after the end asks the driver of Sessions again"
        );
        assert_eq!(watched.owner.running(), 0);
    });
}

/// What each failure of a capture comes to: the failure table every
/// translation point maps into.
#[rstest]
#[case::page_gone(WatchFailure::PageGone, &["a stop", "a start p1 800x600"], None)]
#[case::capture_refused(WatchFailure::CaptureRefused("no".to_owned()), &["a stop", "b start p2 800x600"], None)]
#[case::capture_timed_out(WatchFailure::CaptureTimedOut, &["a stop", "b start p2 800x600"], None)]
#[case::unrecognized(WatchFailure::Unrecognized("odd".to_owned()), &["a stop", "b start p2 800x600"], None)]
#[case::browser_gone(WatchFailure::BrowserGone, &["a stop"], Some(LiveViewEnd::CaptureLost))]
#[case::viewer_gone(WatchFailure::ViewerGone, &["a stop"], Some(LiveViewEnd::CaptureLost))]
fn a_failed_capture_shows_another_tab_takes_another_tier_or_ends(
    #[case] failure: WatchFailure,
    #[case] asked: &[&str],
    #[case] end: Option<LiveViewEnd>,
) {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers, Start::Answers]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        let mut output = Collected::default();
        watched.settle().await;
        told(&mut viewing, &mut output).await;
        watched.asked();

        watched.fail(0, failure);
        watched.settle().await;
        assert_eq!(watched.asked(), asked);
        let news = told(&mut viewing, &mut output).await;
        let told_end = news
            .into_iter()
            .find(|message| matches!(message, LiveViewEngineMessage::Ended { .. }));
        assert_eq!(told_end, end.map(ended));
    });
}

#[test]
fn a_page_gone_shows_the_next_tab_and_no_frame_of_it() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        let mut output = Collected::default();
        watched.capture(0, "of p2");
        watched.settle().await;
        told(&mut viewing, &mut output).await;
        viewing.acknowledge(1);

        watched.fail(0, WatchFailure::PageGone);
        watched.settle().await;
        assert_eq!(told(&mut viewing, &mut output).await, [tabs_message("p1")]);
        assert_eq!(output.0, [1]);
    });
}

#[test]
fn the_last_tier_failing_loses_the_capture() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[
            Start::Answers,
            Start::Fails(WatchFailure::CaptureRefused("no".to_owned())),
        ]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        let mut output = Collected::default();
        watched.settle().await;
        told(&mut viewing, &mut output).await;

        watched.fail(0, WatchFailure::CaptureTimedOut);
        watched.settle().await;
        assert_eq!(
            watched.asked(),
            [
                "a start p2 800x600",
                "a stop",
                "b start p2 800x600",
                "b stop"
            ]
        );
        assert_eq!(
            told(&mut viewing, &mut output).await,
            [ended(LiveViewEnd::CaptureLost)]
        );
    });
}

#[test]
fn a_tier_that_never_starts_times_out_into_the_next() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Never, Start::Answers]);
        let _viewing = watched.viewer(screen(800, 600, 1.0));
        watched.clock.sleep(Duration::from_secs(11)).await;
        assert_eq!(
            watched.asked(),
            ["a start p2 800x600", "a stop", "b start p2 800x600"]
        );
    });
}

#[test]
fn a_tab_opened_is_shown_and_a_closed_session_ends_the_watch() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        let mut output = Collected::default();
        watched.settle().await;
        told(&mut viewing, &mut output).await;
        watched.asked();

        watched.tabs.send_modify(|tabs| tabs.push(page("p3")));
        watched.settle().await;
        assert_eq!(watched.asked(), ["a stop", "a start p3 800x600"]);
        let Some(LiveViewEngineMessage::Tabs { shown, .. }) =
            told(&mut viewing, &mut output).await.pop()
        else {
            panic!("the tabs are told");
        };
        assert_eq!(shown, Some(page("p3")));

        let Watched { tabs, .. } = &watched;
        drop(tabs.clone());
        watched.tabs.send_replace(Vec::new());
        watched.settle().await;
        assert!(watched.asked().contains(&"a stop".to_owned()));
    });
}

#[test]
fn the_daemon_stopping_ends_every_watch() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        let mut output = Collected::default();
        watched.settle().await;
        told(&mut viewing, &mut output).await;

        watched.live_view.stop().await;
        assert_eq!(watched.asked(), ["a start p2 800x600", "a stop"]);
        assert_eq!(
            told(&mut viewing, &mut output).await,
            [ended(LiveViewEnd::DaemonStopping)]
        );
    });
}

#[test]
fn a_session_closing_ends_the_watch() {
    goodfetch_test_support::runtime().block_on(async {
        let Watched {
            live_view,
            log,
            tabs,
            clock,
            owner: _owner,
            frames: _frames,
        } = Watched::new(&[Start::Answers]);
        let mut viewing = live_view
            .watch(&session(), screen(800, 600, 1.0), 10, Tiers::Any)
            .unwrap();
        let mut output = Collected::default();
        clock.sleep(Duration::from_millis(1)).await;
        told(&mut viewing, &mut output).await;

        drop(tabs);
        clock.sleep(Duration::from_millis(1)).await;
        assert_eq!(
            log.lock().unwrap().0.last().map(String::as_str),
            Some("a stop")
        );
        assert_eq!(
            told(&mut viewing, &mut output).await,
            [ended(LiveViewEnd::SessionClosed)]
        );
    });
}

/// One end of an in-memory WebSocket.
struct Socket {
    incoming: mpsc::UnboundedReceiver<Message>,
    outgoing: mpsc::UnboundedSender<Message>,
}

impl Stream for Socket {
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.incoming
            .poll_recv(context)
            .map(|message| message.map(Ok))
    }
}

impl Sink<Message> for Socket {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, message: Message) -> Result<(), Error> {
        self.outgoing
            .send(message)
            .map_err(|_| Error::ConnectionClosed)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }
}

/// The viewer's end.
struct Viewer {
    sends: mpsc::UnboundedSender<Message>,
    receives: mpsc::UnboundedReceiver<Message>,
}

impl Viewer {
    fn say(&self, message: &Value) {
        self.sends.send(Message::text(message.to_string())).unwrap();
    }

    /// Every message it was sent so far, as JSON.
    fn heard(&mut self) -> Vec<Value> {
        let mut heard = Vec::new();
        while let Ok(message) = self.receives.try_recv() {
            let Message::Text(text) = message else {
                continue;
            };
            heard.push(serde_json::from_str(text.as_str()).unwrap());
        }
        heard
    }
}

fn socket() -> (Socket, Viewer) {
    let (sends, incoming) = mpsc::unbounded_channel();
    let (outgoing, receives) = mpsc::unbounded_channel();
    (Socket { incoming, outgoing }, Viewer { sends, receives })
}

fn channel(watched: &Watched, admission: Admission) -> Channel {
    Channel {
        live_view: watched.live_view.clone(),
        token: Arc::from(TOKEN),
        masks: Masks::new(),
        clock: watched.clock,
        admission,
        deadline: Some(watched.clock.now() + Duration::from_secs(5)),
        max_open_size: 4096,
    }
}

fn open(token: &str) -> Value {
    json!({
        "kind": "open",
        "token": token,
        "session": "s1",
        "screen": { "width": 800, "height": 600, "device_pixel_ratio": 1.0 },
    })
}

/// What a channel whose viewer says `first` comes to, and what it heard.
fn opened_with(first: Option<Value>) -> (Result<(), RefusalReason>, Vec<Value>) {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let (socket, mut viewer) = socket();
        if let Some(first) = first {
            viewer.say(&first);
        }
        let served = super::serve(
            channel(&watched, Admission::of(None)),
            socket,
            std::future::pending(),
        )
        .await;
        (served, viewer.heard())
    })
}

#[rstest]
#[case::not_open(Some(json!({"kind": "screen", "screen": {"width": 1, "height": 1, "device_pixel_ratio": 1.0}})), RefusalReason::HelloNotFirst)]
#[case::no_json(Some(json!("open")), RefusalReason::HelloNotFirst)]
#[case::token_missing(Some(json!({"kind": "open", "session": "s1"})), RefusalReason::TokenMissing)]
#[case::token_wrong(Some(open("another-token")), RefusalReason::TokenWrong)]
#[case::too_large(Some(json!({"kind": "open", "token": TOKEN, "pad": "x".repeat(5000)})), RefusalReason::HelloTooLarge)]
#[case::too_late(None, RefusalReason::HelloTimedOut)]
fn a_channel_that_does_not_open_with_the_token_is_closed_unanswered(
    #[case] first: Option<Value>,
    #[case] refused: RefusalReason,
) {
    assert_eq!(opened_with(first), (Err(refused), Vec::new()));
}

#[test]
fn an_open_with_the_token_but_no_screen_is_answered_invalid_params() {
    let mut first = open(TOKEN);
    first.as_object_mut().unwrap().remove("screen");
    let (served, heard) = opened_with(Some(first));
    assert_eq!(served, Ok(()));
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0]["kind"], "error");
    assert_eq!(heard[0]["error"]["code"], -32602);
}

#[test]
fn a_channel_opens_then_tells_the_tabs_and_paces_frames_by_acknowledgement() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let (socket, mut viewer) = socket();
        let (stop, stopping) = tokio::sync::oneshot::channel::<()>();
        let serving = super::serve(channel(&watched, Admission::of(None)), socket, async {
            let _ = stopping.await;
        });
        let test = async {
            viewer.say(&open(TOKEN));
            watched.settle().await;
            let kinds: Vec<Value> = viewer
                .heard()
                .iter()
                .map(|heard| heard["kind"].clone())
                .collect();
            assert_eq!(kinds, ["opened", "tabs"]);

            watched.capture(0, "one");
            watched.capture(0, "two");
            watched.settle().await;
            watched.capture(0, "three");
            watched.settle().await;
            let heard = viewer.heard();
            assert_eq!(heard.len(), 1);
            assert_eq!(heard[0]["kind"], "frame");
            let first = heard[0]["frame"].as_u64().unwrap();

            viewer.say(&json!({"kind": "tab", "page": "p1"}));
            watched.settle().await;
            assert_eq!(
                viewer.heard()[0]["error"]["code"],
                -32601,
                "known, not carried yet"
            );

            viewer.say(&json!({"kind": "acknowledge", "frame": first}));
            watched.settle().await;
            let heard = viewer.heard();
            assert_eq!(heard.len(), 1);
            assert_eq!(heard[0]["data"], "three");

            let _ = stop.send(());
            watched.settle().await;
            assert_eq!(
                viewer.heard(),
                [json!({"kind": "ended", "reason": "daemon_stopping"})]
            );
        };
        let (served, ()) = tokio::join!(serving, test);
        assert_eq!(served, Ok(()));
    });
}

#[test]
fn a_viewer_past_max_viewers_is_answered_live_view_viewer_limit() {
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let _watching = watched.viewer(screen(800, 600, 1.0));
        let limit = SettingLimit {
            key: "live_view.max_viewers".to_owned(),
            value: json!(1),
            layer: SettingsLayer::UserConfiguration,
            origin: None,
        };
        let admission = Admission {
            max_viewers: limit,
            ..Admission::of(None)
        };
        let (socket, mut viewer) = socket();
        viewer.say(&open(TOKEN));
        let served =
            super::serve(channel(&watched, admission), socket, std::future::pending()).await;
        assert_eq!(served, Ok(()));
        let heard = viewer.heard();
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0]["error"]["data"]["code"], "live_view_viewer_limit");
        assert_eq!(
            heard[0]["error"]["data"]["limit"],
            json!({"key": "live_view.max_viewers", "value": 1, "layer": "user_configuration"})
        );
    });
}

#[test]
fn the_settings_default_to_on_ten_viewers_and_any_tier() {
    let admission = Admission::of(None);
    assert!(admission.enabled);
    assert_eq!(admission.max_viewers.value, json!(10));
    assert_eq!(admission.tiers, Tiers::Any);
}

#[test]
fn a_frame_is_encoded_once_for_every_viewer() {
    struct Texts(Vec<tokio_tungstenite::tungstenite::Utf8Bytes>);
    impl VideoOutput for Texts {
        fn send<'a>(&'a mut self, frame: &'a Frame) -> Watching<'a, Result<(), WatchFailure>> {
            self.0.push(frame.text());
            self.0.push(frame.text());
            Box::pin(async { Ok(()) })
        }
    }
    goodfetch_test_support::runtime().block_on(async {
        let watched = Watched::new(&[Start::Answers]);
        let mut viewing = watched.viewer(screen(800, 600, 1.0));
        watched.capture(0, "one");
        watched.settle().await;
        let mut texts = Texts(Vec::new());
        viewing.send_due(&mut texts).await.unwrap();
        assert_eq!(texts.0[0].as_str().as_ptr(), texts.0[1].as_str().as_ptr());
        let frame: Value = serde_json::from_str(texts.0[0].as_str()).unwrap();
        assert_eq!(
            frame,
            json!({"kind": "frame", "frame": 1, "page": "p2", "width": 800.0, "height": 600.0, "data": "one"})
        );
    });
}

mod live_view_viewer_message {
    goodfetch_test_support::fuzz_replay!(
        super::super::fuzz_viewer_message,
        "fuzz/live_view_viewer_message"
    );
}
