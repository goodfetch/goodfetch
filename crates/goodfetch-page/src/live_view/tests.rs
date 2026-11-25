//! The Live view's tabs and its `screencast` tier, the browser played in
//! process at the other end of a real `goodfetch_cdp::Connection`, so each
//! command is read as the browser would read it.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::page::StartScreencastParams;
use goodfetch_cdp::{
    Connection, Driver, Failure, Framer, SessionId as TargetSession, Subscription,
};
use goodfetch_engine::live_view::{
    FrameSize, SourceFrame, WatchFailure, WatchedSession, WatchedSessions, Watching,
};
use goodfetch_protocol::live_view::LiveViewTier;
use goodfetch_protocol::page::{PageId, SessionId};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, ReadHalf, WriteHalf};

use super::{ScreencastCommand, ScreencastPage, WatchedTabs, watch_failure};

/// The tab's session on the browser's connection.
const TARGET: &str = "tab";

fn session() -> SessionId {
    SessionId("s1".to_owned())
}

fn page(name: &str) -> PageId {
    PageId(name.to_owned())
}

/// A tab over a connection to a played browser, sending as a page's
/// driver does: on the tab's session, with no timeout of its own.
struct Tab {
    connection: Connection,
    target: TargetSession,
}

impl ScreencastPage for Tab {
    fn events(&self) -> Subscription {
        self.connection.subscribe(Some(&self.target))
    }

    fn send(&self, command: ScreencastCommand) -> Watching<'_, Result<(), Failure>> {
        use goodfetch_cdp::generated::page::{ScreencastFrameAckParams, StopScreencastParams};
        let (target, forever) = (Some(&self.target), Duration::MAX);
        Box::pin(async move {
            match command {
                ScreencastCommand::Start(params) => self
                    .connection
                    .send(target, &params, forever)
                    .await
                    .map(drop),
                ScreencastCommand::Acknowledge(session_id) => self
                    .connection
                    .send(target, &ScreencastFrameAckParams { session_id }, forever)
                    .await
                    .map(drop),
                ScreencastCommand::Stop => self
                    .connection
                    .send(target, &StopScreencastParams {}, forever)
                    .await
                    .map(drop),
            }
        })
    }
}

/// The browser's end of the pipe.
struct Played {
    pipe: DuplexStream,
    framer: Framer,
}

impl Played {
    /// The next command the client wrote.
    async fn command(&mut self) -> Value {
        loop {
            if let Some(bytes) = self
                .framer
                .next_frame()
                .expect("invariant: the client's frames fit")
            {
                return serde_json::from_slice(&bytes).expect("invariant: the client writes JSON");
            }
            let mut buffer = vec![0; 4096];
            let count = self
                .pipe
                .read(&mut buffer)
                .await
                .expect("invariant: the pipe reads");
            assert_ne!(count, 0, "the client closed its end before a command");
            self.framer.push(&buffer[..count]);
        }
    }

    /// Writes `message`, framed as the browser's pipe frames it.
    async fn send(&mut self, message: &Value) {
        let mut bytes = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
        bytes.push(0);
        self.pipe
            .write_all(&bytes)
            .await
            .expect("invariant: the client holds its end");
    }

    /// Answers `command` with `result`.
    async fn answer(&mut self, command: &Value, result: Value) {
        self.send(&json!({ "id": command["id"], "sessionId": TARGET, "result": result }))
            .await;
    }

    /// Refuses `command` with the browser's `message`.
    async fn refuse(&mut self, command: &Value, message: &str) {
        self.send(&json!({
            "id": command["id"],
            "sessionId": TARGET,
            "error": { "code": -32000, "message": message },
        }))
        .await;
    }

    /// The tab's event `method`, with its `params`.
    async fn event(&mut self, method: &str, params: Value) {
        self.send(&json!({ "method": method, "sessionId": TARGET, "params": params }))
            .await;
    }
}

type Piped = Driver<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

/// The Session `s1` with the tab `p1` over a played browser, its tier
/// unstarted, and the connection's driver, which the test runs.
fn played() -> (WatchedTabs, WatchedSession, Piped, Played) {
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(
        reader,
        writer,
        goodfetch_counted_work::CountedWork::default(),
        None,
    );
    let tabs = WatchedTabs::default();
    tabs.session_opened(&session());
    let tab = Tab {
        connection,
        target: TargetSession(TARGET.to_owned()),
    };
    tabs.page_opened(&session(), &page("p1"), Arc::new(tab));
    let watched = tabs.watch(&session()).expect("the Session is open");
    let played = Played {
        pipe: browser,
        framer: Framer::default(),
    };
    (tabs, watched, driver, played)
}

/// A frame of 800 by 600 CSS pixels whose data is `data`, numbered `ack`.
fn frame(data: &str, ack: i64) -> Value {
    json!({
        "data": data,
        "metadata": {
            "offsetTop": 0, "pageScaleFactor": 1, "deviceWidth": 800, "deviceHeight": 600,
            "scrollOffsetX": 0, "scrollOffsetY": 0,
        },
        "sessionId": ack,
    })
}

const SIZE: FrameSize = FrameSize {
    width: 1600,
    height: 1200,
};

/// The tier starts the tab's screencast at the size its viewers' screens
/// fill, in JPEG, takes each frame as it came with no other event, and
/// acknowledges it before it gives it; its stop stops the screencast.
#[test]
fn a_screencast_starts_at_the_viewers_size_and_acknowledges_each_frame_it_gives() {
    goodfetch_test_support::runtime().block_on(async {
        let (tabs, mut watched, driver, mut played) = played();
        let mut source = watched.sources.remove(0);
        assert_eq!(source.tier(), LiveViewTier::Screencast);
        let browser = async {
            let start = played.command().await;
            played.answer(&start, json!({})).await;
            played
                .event("Page.loadEventFired", json!({ "timestamp": 1.0 }))
                .await;
            played.event("Page.screencastFrame", frame("AAAA", 7)).await;
            let ack = played.command().await;
            played.answer(&ack, json!({})).await;
            let stop = played.command().await;
            played.answer(&stop, json!({})).await;
            [start, ack, stop].map(|command| json!([command["method"], command["params"]]))
        };
        let tier = async {
            source.start(&page("p1"), SIZE).await.expect("it starts");
            let frame = source.next().await.expect("a frame comes");
            source.stop().await;
            drop((source, tabs));
            frame
        };
        let (_, commands, frame) = tokio::join!(driver.run(), browser, async {
            let frame = tier.await;
            drop(watched);
            frame
        });
        assert_eq!(
            commands,
            [
                json!(["Page.startScreencast", {
                    "format": "jpeg", "quality": 80, "maxWidth": 1600, "maxHeight": 1200,
                    "everyNthFrame": 1,
                }]),
                json!(["Page.screencastFrameAck", { "sessionId": 7 }]),
                json!(["Page.stopScreencast", {}]),
            ]
        );
        assert_eq!(
            frame,
            SourceFrame {
                width: 800.0,
                height: 600.0,
                data: "AAAA".to_owned(),
            }
        );
    });
}

/// A `next` dropped while it acknowledges loses no frame: the next one
/// acknowledges the same frame again and gives it.
#[test]
fn a_next_dropped_while_it_acknowledges_gives_the_frame_to_the_next() {
    goodfetch_test_support::runtime().block_on(async {
        let (tabs, mut watched, driver, mut played) = played();
        let mut source = watched.sources.remove(0);
        let run = async {
            let p1 = page("p1");
            let (started, ()) = tokio::join!(source.start(&p1, SIZE), async {
                let start = played.command().await;
                played.answer(&start, json!({})).await;
            });
            started.expect("it starts");
            played.event("Page.screencastFrame", frame("BBBB", 3)).await;
            let first_ack = tokio::select! {
                _ = source.next() => panic!("the frame was given before its acknowledgement"),
                ack = played.command() => ack,
            };
            let (given, second_ack) = tokio::join!(source.next(), async {
                let ack = played.command().await;
                played.answer(&ack, json!({})).await;
                ack
            });
            drop((source, tabs, watched, played));
            (first_ack, second_ack, given)
        };
        let (_, (first, second, given)) = tokio::join!(driver.run(), run);
        assert_eq!(first["params"], json!({ "sessionId": 3 }));
        assert_eq!(second["params"], json!({ "sessionId": 3 }));
        assert_eq!(given.expect("the frame comes").data, "BBBB");
    });
}

/// Each answer the browser refuses a screencast with is the
/// `WatchFailure` the Live view acts on.
#[rstest]
#[case::the_tab_is_gone("Target closed", WatchFailure::PageGone)]
#[case::its_session_is_gone("Session with given id not found", WatchFailure::PageGone)]
#[case::the_browser_does_not_capture(
    "Screencast is not supported",
    WatchFailure::CaptureRefused(
        "the browser refused `Page.startScreencast` (-32000): <27 bytes, redacted>".to_owned()
    )
)]
fn a_refused_start_is_the_failure_it_reads_as(
    #[case] message: &str,
    #[case] expected: WatchFailure,
) {
    goodfetch_test_support::runtime().block_on(async {
        let (tabs, mut watched, driver, mut played) = played();
        let mut source = watched.sources.remove(0);
        let run = async {
            let p1 = page("p1");
            let (started, ()) = tokio::join!(source.start(&p1, SIZE), async {
                let start = played.command().await;
                played.refuse(&start, message).await;
            });
            drop((source, tabs, watched, played));
            started
        };
        let (_, started) = tokio::join!(driver.run(), run);
        assert_eq!(started, Err(expected));
    });
}

/// The failures no browser's message words read by their kind.
#[rstest]
#[case::no_answer_in_time(
    Failure::TimedOut { method: "Page.screencastFrameAck" },
    WatchFailure::CaptureTimedOut
)]
#[case::the_browser_is_gone(Failure::Disconnected, WatchFailure::BrowserGone)]
#[case::the_lease_ended(
    Failure::Withheld { method: "Page.startScreencast" },
    WatchFailure::CaptureRefused(
        "`Page.startScreencast` was withheld: the lease of the page's work item ended".to_owned()
    )
)]
#[case::an_answer_too_long(
    Failure::TooLarge { method: "Page.screencastFrameAck" },
    WatchFailure::Unrecognized(
        "the browser's answer to `Page.screencastFrameAck` is longer than one message the connection reads"
            .to_owned()
    )
)]
fn a_failure_with_no_message_reads_by_its_kind(
    #[case] failure: Failure,
    #[case] expected: WatchFailure,
) {
    assert_eq!(watch_failure(&failure), expected);
}

/// A browser gone while a frame is awaited ends the tier with
/// `BrowserGone`.
#[test]
fn a_browser_gone_while_a_frame_is_awaited_ends_the_tier() {
    goodfetch_test_support::runtime().block_on(async {
        let (tabs, mut watched, driver, mut played) = played();
        let mut source = watched.sources.remove(0);
        let run = async {
            let p1 = page("p1");
            let (started, ()) = tokio::join!(source.start(&p1, SIZE), async {
                let start = played.command().await;
                played.answer(&start, json!({})).await;
            });
            started.expect("it starts");
            drop(played);
            let next = source.next().await;
            drop((source, tabs, watched));
            next
        };
        let (_, next) = tokio::join!(driver.run(), run);
        assert_eq!(next, Err(WatchFailure::BrowserGone));
    });
}

/// A tab the Session no longer holds is never started: its start fails
/// with `PageGone` and sends nothing.
#[test]
fn a_tab_no_longer_held_is_never_started() {
    goodfetch_test_support::runtime().block_on(async {
        let (tabs, mut watched, driver, played) = played();
        let mut source = watched.sources.remove(0);
        tabs.page_closed(&session(), &page("p1"));
        let started = source.start(&page("p1"), SIZE).await;
        assert_eq!(started, Err(WatchFailure::PageGone));
        drop((source, tabs, watched));
        let mut pipe = played.pipe;
        let (_, read) = tokio::join!(driver.run(), async {
            let mut sent = Vec::new();
            pipe.read_to_end(&mut sent)
                .await
                .expect("invariant: the pipe reads");
            sent
        });
        assert!(read.is_empty(), "nothing was sent: {read:?}");
    });
}

/// A Session's tabs are its open pages, oldest first, as they open and
/// close; its watchers see it gone once it closed, and a Session never
/// opened is no Session to watch.
#[test]
fn a_sessions_tabs_follow_its_pages_until_it_closes() {
    goodfetch_test_support::runtime().block_on(async {
        let (tabs, watched, _driver, _played) = played();
        let mut seen = watched.tabs;
        assert_eq!(*seen.borrow_and_update(), [page("p1")]);
        let (other, _) = Connection::new(
            tokio::io::empty(),
            tokio::io::sink(),
            goodfetch_counted_work::CountedWork::default(),
            None,
        );
        let second = Arc::new(Tab {
            connection: other,
            target: TargetSession("other".to_owned()),
        });
        tabs.page_opened(&session(), &page("p2"), Arc::<Tab>::clone(&second));
        assert_eq!(*seen.borrow_and_update(), [page("p1"), page("p2")]);
        tabs.page_closed(&session(), &page("p1"));
        assert_eq!(*seen.borrow_and_update(), [page("p2")]);
        tabs.page_opened(&SessionId("s9".to_owned()), &page("p3"), second);
        assert!(!seen.has_changed().expect("the Session is open"));
        assert!(tabs.watch(&SessionId("s9".to_owned())).is_none());
        tabs.session_closed(&session());
        assert!(seen.changed().await.is_err(), "the Session closed");
        assert!(tabs.watch(&session()).is_none());
    });
}

/// `StartScreencastParams` keeps every option the tier leaves unset out
/// of the command.
#[test]
fn the_start_command_sends_only_what_the_tier_sets() {
    let unset = serde_json::to_value(StartScreencastParams::default()).expect("it serializes");
    assert_eq!(unset, json!({}));
}
