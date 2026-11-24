use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::oneshot;

use super::{Connection, Driver, Ended, Request, SessionId, SubscriptionEnd};
use crate::Failure;
use crate::framing::{Framer, frame};
use crate::generated::browser::GetVersionParams;
use crate::generated::fetch;

const TIMEOUT: Duration = Duration::from_secs(5);

/// The browser's end of an in-process pipe: what the client wrote, and a
/// way to answer.
struct Browser {
    stream: DuplexStream,
    framer: Framer,
}

impl Browser {
    async fn received(&mut self) -> Value {
        loop {
            if let Some(bytes) = self.framer.next_frame().unwrap() {
                return serde_json::from_slice(&bytes).unwrap();
            }
            let mut buffer = vec![0; 4096];
            let count = self.stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0, "the client closed its end");
            self.framer.push(&buffer[..count]);
        }
    }

    async fn send(&mut self, message: &Value) {
        self.stream.write_all(&frame(message)).await.unwrap();
    }
}

type Pipe = Driver<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

fn connected() -> (Connection, Pipe, Browser) {
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(reader, writer, CountedWork::default(), None);
    (
        connection,
        driver,
        Browser {
            stream: browser,
            framer: Framer::default(),
        },
    )
}

fn version() -> Value {
    json!({
        "protocolVersion": "1.3",
        "product": "Chrome/156.0.8078.4",
        "revision": "@f842f52e",
        "userAgent": "Mozilla/5.0",
        "jsVersion": "15.6.75",
    })
}

/// C1's seam (ADR 0140): each command the driver writes costs one send and
/// the encode of its frame, on the work the connection was given; a typed
/// command's params serialized to a `Value` first are an encode of their own
/// (#1861's Decision 3, recorded under X21).
#[test]
fn each_command_counts_its_encodes_and_one_send_on_the_work_it_was_given() {
    let work = CountedWork::default();
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(reader, writer, work.clone(), None);
    let mut browser = Browser {
        stream: browser,
        framer: Framer::default(),
    };
    runtime().block_on(async {
        let client = async {
            for _ in 0..2 {
                connection
                    .send(None, &GetVersionParams {}, TIMEOUT)
                    .await
                    .unwrap();
            }
            connection
                .send_raw("Browser.getVersion", None, json!({}), TIMEOUT)
                .await
                .unwrap();
            drop(connection);
        };
        let answers = async {
            for _ in 0..3 {
                let command = browser.received().await;
                browser
                    .send(&json!({ "id": command["id"], "result": version() }))
                    .await;
            }
        };
        tokio::join!(driver.run(), client, answers);
    });

    // Two typed commands of two encodes each, and a raw one of one.
    assert_eq!(work.read(Row::CdpEncodes), 5);
    assert_eq!(work.read(Row::CdpSends), 3);
}

/// C5's seam (ADR 0140): each `Fetch` command written and each `Fetch` event
/// read is one message of the interception's, and a command that answers a
/// paused request is one answer; no other domain's message counts.
#[test]
fn each_fetch_message_and_each_answer_to_a_paused_request_counts_on_the_work() {
    let work = CountedWork::default();
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(reader, writer, work.clone(), None);
    let mut browser = Browser {
        stream: browser,
        framer: Framer::default(),
    };
    runtime().block_on(async {
        let client = async {
            connection
                .send(None, &fetch::EnableParams::default(), TIMEOUT)
                .await
                .unwrap();
            connection
                .send(None, &GetVersionParams {}, TIMEOUT)
                .await
                .unwrap();
            connection
                .send(
                    None,
                    &fetch::ContinueRequestParams {
                        request_id: "paused".to_owned(),
                        url: None,
                        method: None,
                        post_data: None,
                        headers: None,
                        intercept_response: None,
                    },
                    TIMEOUT,
                )
                .await
                .unwrap();
            drop(connection);
        };
        let answers = async {
            for answered in 0..3 {
                let command = browser.received().await;
                if answered == 2 {
                    // Read before the last answer, so before the client ends.
                    for method in ["Fetch.requestPaused", "Page.loadEventFired"] {
                        browser
                            .send(&json!({ "method": method, "params": {} }))
                            .await;
                    }
                }
                let result = if answered == 1 { version() } else { json!({}) };
                browser
                    .send(&json!({ "id": command["id"], "result": result }))
                    .await;
            }
        };
        tokio::join!(driver.run(), client, answers);
    });

    assert_eq!(
        (
            work.read(Row::InterceptedFetchMessages),
            work.read(Row::InterceptedAnswers)
        ),
        (3, 1)
    );
}

#[test]
fn a_command_is_answered_with_what_it_returns() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let (ended, (answer, sent)) = tokio::join!(driver.run(), async {
            let answered = tokio::join!(
                connection.send(None, &GetVersionParams {}, TIMEOUT),
                async {
                    let sent = browser.received().await;
                    browser
                        .send(&json!({ "id": sent["id"], "result": version() }))
                        .await;
                    sent
                }
            );
            drop(connection);
            answered
        });
        assert_eq!(
            (
                sent["method"].clone(),
                answer.unwrap().product,
                sent.get("sessionId").cloned(),
                ended
            ),
            (
                json!("Browser.getVersion"),
                "Chrome/156.0.8078.4".to_owned(),
                None,
                Ended::Dropped
            )
        );
    });
}

/// An answer past the framing's limit fails its own command alone, and the
/// connection carries the next (finding 563-1: a body read past 256 MiB
/// ended the browser's whole connection).
#[test]
fn an_answer_past_the_framing_limit_fails_its_command_and_the_connection_goes_on() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let driver = driver.with_frame_limit(1024);
        let (ended, (large, next)) = tokio::join!(driver.run(), async {
            let answered = tokio::join!(
                connection.send(None, &GetVersionParams {}, TIMEOUT),
                async {
                    let sent = browser.received().await;
                    let mut answer = version();
                    answer["product"] = json!("a".repeat(64 * 1024));
                    browser
                        .send(&json!({ "id": sent["id"], "result": answer }))
                        .await;
                }
            );
            let next = tokio::join!(
                connection.send(None, &GetVersionParams {}, TIMEOUT),
                async {
                    let sent = browser.received().await;
                    browser
                        .send(&json!({ "id": sent["id"], "result": version() }))
                        .await;
                }
            );
            drop(connection);
            (answered.0, next.0)
        });
        assert_eq!(
            (large.err(), next.map(|version| version.product), ended),
            (
                Some(Failure::TooLarge {
                    method: "Browser.getVersion"
                }),
                Ok("Chrome/156.0.8078.4".to_owned()),
                Ended::Dropped
            )
        );
    });
}

#[test]
fn a_refusal_and_a_closed_pipe_fail_the_commands_in_flight() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let session = SessionId("session-1".to_owned());
        let (ended, (refused, gone, ())) = tokio::join!(driver.run(), async {
            tokio::join!(
                connection.send(Some(&session), &GetVersionParams {}, TIMEOUT),
                connection.send(None, &GetVersionParams {}, TIMEOUT),
                async {
                    let received = [browser.received().await, browser.received().await];
                    let of_the_session = received
                        .iter()
                        .find(|command| command["sessionId"] == "session-1")
                        .expect("the session's command reaches the browser");
                    browser
                        .send(&json!({
                            "id": of_the_session["id"],
                            "error": { "code": -32000, "message": "Target closed" },
                        }))
                        .await;
                    drop(browser);
                }
            )
        });
        assert!(
            matches!(refused, Err(Failure::Refused { .. })),
            "{refused:?}"
        );
        assert_eq!(
            (gone.err(), ended),
            (Some(Failure::Disconnected), Ended::Closed)
        );
    });
}

/// A command whose sender took its place in the queue before the browser
/// closed its end, and queued it only once the driver had stopped taking
/// commands, still fails. On a runtime of several threads the two steps of
/// a send are apart, and a driver that ends between them left that command,
/// and every one queued behind it, waiting for good (#2525).
#[test]
fn a_command_queued_as_the_connection_ends_still_fails() {
    runtime().block_on(async {
        let (connection, driver, browser) = connected();
        let (answer, answered) = oneshot::channel();
        let (ended, answered) = tokio::join!(driver.run(), async {
            let place = connection
                .requests
                .reserve()
                .await
                .expect("the driver takes commands");
            drop(browser);
            while !connection.requests.is_closed() {
                tokio::task::yield_now().await;
            }
            place.send(Request {
                method: "Browser.getVersion",
                session: None,
                params: json!({}),
                answer,
            });
            tokio::time::timeout(HUNG, answered).await
        });
        assert_eq!(
            (ended, answered),
            (Ended::Closed, Ok(Ok(Err(Failure::Disconnected))))
        );
    });
}

#[test]
fn a_command_the_browser_never_answers_times_out() {
    runtime().block_on(async {
        let (connection, driver, _browser) = connected();
        let (_, answer) = tokio::join!(driver.run(), async {
            let answer = connection
                .send(None, &GetVersionParams {}, Duration::from_secs(30))
                .await;
            drop(connection);
            answer
        });
        assert_eq!(
            answer.err(),
            Some(Failure::TimedOut {
                method: "Browser.getVersion"
            })
        );
    });
}

/// How long a test lets a call hang before it calls it hung: far past
/// every timeout the test gives.
const HUNG: Duration = Duration::from_secs(600);

/// A connection whose browser end reads only `room` bytes and then nothing
/// more, as a wedged browser stops reading its pipe, and that end.
fn wedged(room: usize) -> (Connection, Pipe, DuplexStream) {
    let (client, browser) = tokio::io::duplex(room);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(reader, writer, CountedWork::default(), None);
    (connection, driver, browser)
}

/// While the browser reads nothing, so the driver waits to write, each
/// command still ends at its timeout, however many wait behind it.
#[test]
fn a_command_ends_at_its_timeout_while_the_browser_reads_nothing() {
    runtime().block_on(async {
        let (connection, driver, browser) = wedged(64);
        let (_, answers) = tokio::join!(driver.run(), async {
            let mut answers = Vec::new();
            // More than the commands that may wait for the driver.
            for _ in 0..80 {
                answers.push(
                    tokio::time::timeout(
                        HUNG,
                        connection.send_raw(
                            "Browser.getVersion",
                            None,
                            json!({}),
                            Duration::from_secs(1),
                        ),
                    )
                    .await,
                );
            }
            drop((connection, browser));
            answers
        });
        let timed_out = answers
            .iter()
            .filter(|answer| {
                matches!(
                    answer,
                    Ok(Err(Failure::TimedOut {
                        method: "Browser.getVersion"
                    }))
                )
            })
            .count();
        assert_eq!(timed_out, 80);
    });
}

/// While the browser reads nothing, a subscription still starts, and it
/// ends with the connection.
#[test]
fn a_subscription_starts_while_the_browser_reads_nothing() {
    runtime().block_on(async {
        let (connection, driver, browser) = wedged(64);
        let (_, ended) = tokio::join!(driver.run(), async {
            for _ in 0..2 {
                let _ = connection
                    .send_raw("Browser.getVersion", None, json!({}), TIMEOUT)
                    .await;
            }
            let mut subscription = connection.subscribe(None);
            drop((connection, browser));
            tokio::time::timeout(HUNG, subscription.next()).await
        });
        assert_eq!(ended, Ok(None));
    });
}

/// A subscription its holder never reads holds up no other answer of the
/// connection: past what it may hold, it ends, and the driver goes on.
#[test]
fn a_subscription_never_read_holds_up_no_answer() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let page = SessionId("page".to_owned());
        let (_, answer) = tokio::join!(driver.run(), async {
            let unread = connection.subscribe(Some(&page));
            let (answer, _) = tokio::join!(
                tokio::time::timeout(
                    HUNG,
                    connection.send(None, &GetVersionParams {}, TIMEOUT)
                ),
                tokio::time::timeout(HUNG, async {
                    let sent = browser.received().await;
                    for _ in 0..5_000 {
                        browser
                            .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "page" }))
                            .await;
                    }
                    browser
                        .send(&json!({ "id": sent["id"], "result": version() }))
                        .await;
                })
            );
            drop((connection, unread, browser));
            answer
        });
        assert_eq!(
            answer.ok().and_then(Result::ok).map(|version| version.product),
            Some("Chrome/156.0.8078.4".to_owned())
        );
    });
}

/// A command in flight on a target's session whose target the browser
/// detached, such as a page of a browser context disposed under it, fails
/// at once as the target gone, rather than at its timeout: the browser
/// never answers it.
#[test]
fn a_command_on_a_session_the_browser_detached_fails_at_once() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let page = SessionId("page".to_owned());
        let (_, failed) = tokio::join!(driver.run(), async {
            let (failed, ()) = tokio::join!(
                tokio::time::timeout(
                    TIMEOUT / 2,
                    connection.send(Some(&page), &GetVersionParams {}, TIMEOUT)
                ),
                async {
                    let _ = browser.received().await;
                    browser
                        .send(&json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": "page", "targetId": "target-1" } }))
                        .await;
                }
            );
            drop((connection, browser));
            failed
        });
        assert!(
            matches!(
                failed,
                Ok(Err(Failure::Refused {
                    method: "Browser.getVersion",
                    refusal: crate::Refusal::TargetGone,
                    ..
                }))
            ),
            "{failed:?}"
        );
    });
}

/// A target's session the browser detached, as when its page closed or its
/// browser context was disposed, ends that session's subscriptions at
/// once, while the connection and other sessions' subscriptions go on: its
/// target sends nothing more.
#[test]
fn a_session_the_browser_detached_ends_its_subscriptions() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let (page, other) = (
            SessionId("page".to_owned()),
            SessionId("other".to_owned()),
        );
        let (_, (ended, why, still)) = tokio::join!(driver.run(), async {
            let mut on_page = connection.subscribe(Some(&page));
            let mut on_other = connection.subscribe(Some(&other));
            browser
                .send(&json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": "page", "targetId": "target-1" } }))
                .await;
            browser
                .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "other" }))
                .await;
            let ended = tokio::time::timeout(TIMEOUT, on_page.next()).await;
            let still = on_other.next().await.map(|event| event.method.clone());
            let why = on_page.ended();
            drop((connection, browser));
            (ended, why, still)
        });
        assert_eq!(
            (ended, why, still),
            (
                Ok(None),
                Some(SubscriptionEnd::Detached),
                Some("Page.loadEventFired".to_owned())
            )
        );
    });
}

/// When a detach ends a session's subscription, the commands in flight on
/// that session already hold their failure: a holder that reads the end
/// as its target gone, not the connection's, waits on them for what ended
/// its call, such as a Limit that stopped the Run as it closed the
/// browser.
#[test]
fn a_detached_sessions_commands_fail_before_its_subscriptions_end() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let page = SessionId("page".to_owned());
        let (_, (ended, why, answered)) = tokio::join!(driver.run(), async {
            let mut on_page = connection.subscribe(Some(&page));
            let (ended, why, answered) = {
                let mut command = std::pin::pin!(connection.send(
                    Some(&page),
                    &GetVersionParams {},
                    TIMEOUT
                ));
                tokio::select! {
                    biased;
                    answered = &mut command => panic!("answered before it was sent: {answered:?}"),
                    _ = browser.received() => {}
                }
                browser
                    .send(&json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": "page", "targetId": "target-1" } }))
                    .await;
                let ended = tokio::time::timeout(TIMEOUT, on_page.next()).await;
                let why = on_page.ended();
                // Polled once, without the driver running in between.
                let answered = tokio::time::timeout(Duration::ZERO, &mut command).await;
                (ended, why, answered)
            };
            drop((connection, browser));
            (ended.map(|event| event.is_none()), why, answered)
        });
        assert!(
            matches!(
                (&ended, &why, &answered),
                (
                    Ok(true),
                    Some(SubscriptionEnd::Detached),
                    Ok(Err(Failure::Refused {
                        refusal: crate::Refusal::TargetGone,
                        ..
                    }))
                )
            ),
            "{ended:?} {why:?} {answered:?}"
        );
    });
}

/// A subscription whose holder fell `EVENTS` behind ends saying so, so its
/// holder knows the events it missed are lost, unlike one that ends with
/// the connection.
#[test]
fn a_subscription_that_fell_behind_says_its_events_were_lost() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let page = SessionId("page".to_owned());
        let (_, (behind, disconnected)) = tokio::join!(driver.run(), async {
            let mut behind = connection.subscribe(Some(&page));
            let mut on_browser = connection.subscribe(None);
            for _ in 0..5_000 {
                browser
                    .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "page" }))
                    .await;
            }
            browser
                .send(&json!({ "method": "Target.targetCreated", "params": {} }))
                .await;
            let _ = on_browser.next().await;
            while behind.next().await.is_some() {}
            drop((connection, browser));
            while on_browser.next().await.is_some() {}
            (behind.ended(), on_browser.ended())
        });
        assert_eq!(
            (behind, disconnected),
            (
                Some(SubscriptionEnd::FellBehind),
                Some(SubscriptionEnd::Disconnected)
            )
        );
    });
}

#[test]
fn events_reach_the_subscribers_of_their_session_only() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let page = SessionId("page".to_owned());
        let (_, methods) = tokio::join!(driver.run(), async {
            let mut on_page = connection.subscribe(Some(&page));
            let mut on_browser = connection.subscribe(None);
            browser
                .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "page" }))
                .await;
            browser
                .send(&json!({ "method": "Target.targetCreated", "params": {} }))
                .await;
            let methods = (
                on_page.next().await.unwrap().method.clone(),
                on_browser.next().await.unwrap().method.clone(),
            );
            drop(connection);
            methods
        });
        assert_eq!(
            methods,
            (
                "Page.loadEventFired".to_owned(),
                "Target.targetCreated".to_owned()
            )
        );
    });
}

/// Sends the name and fields of each span its subscriber sees open.
struct Opened(std::sync::mpsc::Sender<(String, String)>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Opened {
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        _: &tracing::span::Id,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        struct Fields(Vec<String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push(format!("{}={value:?}", field.name()));
            }
        }
        let mut fields = Fields(Vec::new());
        attributes.record(&mut fields);
        let _ = self
            .0
            .send((attributes.metadata().name().to_owned(), fields.0.join(" ")));
    }
}

/// Each command is a span of its own, its method its one field, and never
/// its params (ADR 0132).
///
/// Covers: story:3-105
#[test]
fn each_command_is_a_span_that_records_its_method_alone() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let (opened, spans) = std::sync::mpsc::channel();
    let subscriber = tracing_subscriber::registry().with(Opened(opened));

    tracing::subscriber::with_default(subscriber, || {
        runtime().block_on(async {
            let (connection, driver, mut browser) = connected();
            tokio::join!(driver.run(), async {
                let _ = tokio::join!(
                    connection.send_raw(
                        "Input.insertText",
                        None,
                        json!({ "text": "hunter2" }),
                        TIMEOUT
                    ),
                    async {
                        let sent = browser.received().await;
                        browser
                            .send(&json!({ "id": sent["id"], "result": {} }))
                            .await;
                    }
                );
                drop(connection);
            });
        });
    });

    assert_eq!(
        spans.try_iter().collect::<Vec<_>>(),
        [(
            "cdp_command".to_owned(),
            "cdp.method=\"Input.insertText\"".to_owned()
        )]
    );
}

/// Sends the fields of each `warn` its subscriber sees.
struct Warned(std::sync::mpsc::Sender<String>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warned {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Fields(Vec<String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push(format!("{}={value:?}", field.name()));
            }
        }
        if *event.metadata().level() == tracing::Level::WARN {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            let _ = self.0.send(fields.0.join(" "));
        }
    }
}

/// An event whose params do not read as its type's is logged at `warn`
/// with its method, so a wait that then times out has its cause logged.
#[test]
fn an_event_whose_params_do_not_read_is_logged_at_warn_with_its_method() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let (warned, warnings) = std::sync::mpsc::channel();
    let subscriber = tracing_subscriber::registry().with(Warned(warned));
    let event = super::Event::new("Page.loadEventFired", &json!({ "timestamp": "not a time" }));

    let read = tracing::subscriber::with_default(subscriber, || {
        event.read::<crate::generated::page::LoadEventFiredEvent>()
    });

    let warnings: Vec<String> = warnings.try_iter().collect();
    assert_eq!(
        (
            read.is_none(),
            warnings
                .iter()
                .filter(|warning| warning.contains("cdp.method=\"Page.loadEventFired\""))
                .count()
        ),
        (true, 1),
        "{warnings:?}"
    );
}

/// A frame's target detached on its page's session ends the frame's
/// subscriptions only: the page's own goes on, the detach included, so a
/// page whose out-of-process frame went is not read as gone.
#[test]
fn a_frame_detached_on_its_page_session_ends_only_the_frame_subscriptions() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let (page, frame) = (
            SessionId("page".to_owned()),
            SessionId("frame".to_owned()),
        );
        let (_, (frame_ended, why, on_page)) = tokio::join!(driver.run(), async {
            let mut on_page = connection.subscribe(Some(&page));
            let mut on_frame = connection.subscribe(Some(&frame));
            browser
                .send(&json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": "frame", "targetId": "frame-1" }, "sessionId": "page" }))
                .await;
            browser
                .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "page" }))
                .await;
            let frame_ended = tokio::time::timeout(TIMEOUT, on_frame.next()).await;
            let why = on_frame.ended();
            let mut seen = Vec::new();
            for _ in 0..2 {
                seen.push(on_page.next().await.map(|event| event.method.clone()));
            }
            let still_open = on_page.ended();
            drop((connection, browser));
            (frame_ended, why, (seen, still_open))
        });
        assert_eq!(
            (frame_ended, why, on_page),
            (
                Ok(None),
                Some(SubscriptionEnd::Detached),
                (
                    vec![
                        Some("Target.detachedFromTarget".to_owned()),
                        Some("Page.loadEventFired".to_owned())
                    ],
                    None
                )
            )
        );
    });
}

/// A connection, its driver and the browser's end, counting on `work`.
fn connected_on(work: &CountedWork) -> (Connection, Pipe, Browser) {
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    let (connection, driver) = Connection::new(reader, writer, work.clone(), None);
    (
        connection,
        driver,
        Browser {
            stream: browser,
            framer: Framer::default(),
        },
    )
}

/// C1's seam (ADR 0140): the driver hands an event to the readers of its
/// session alone, visiting no other session's.
#[test]
fn an_event_visits_the_readers_of_its_session_alone() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    let sessions = ["page", "frame", "worker"].map(|on| SessionId(on.to_owned()));
    runtime().block_on(async {
        let (_, method) = tokio::join!(driver.run(), async {
            let mut readers = sessions
                .each_ref()
                .map(|session| connection.subscribe(Some(session)));
            browser
                .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "frame" }))
                .await;
            let method = readers[1].next().await.map(|event| event.method.clone());
            drop((connection, browser));
            method
        });
        assert_eq!(method.as_deref(), Some("Page.loadEventFired"));
    });

    assert_eq!(work.read(Row::CdpEventVisits), 1);
}

/// C1's seam (ADR 0140): a command whose caller's timeout ended its wait
/// leaves the pending map, though the browser never answers it.
#[test]
fn a_command_whose_caller_timed_out_leaves_the_pending_map() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    runtime().block_on(async {
        let (_, answer) = tokio::join!(driver.run(), async {
            let (answer, _) = tokio::join!(
                connection.send_raw(
                    "Browser.getVersion",
                    None,
                    json!({}),
                    Duration::from_secs(1)
                ),
                browser.received(),
            );
            drop((connection, browser));
            answer
        });
        assert_eq!(
            answer.err(),
            Some(Failure::TimedOut {
                method: "Browser.getVersion"
            })
        );
    });

    assert_eq!(work.read(Row::CdpPendingOrphans), 0);
}

/// C1's seam (ADR 0140): a command whose caller dropped its wait leaves the
/// pending map, though the browser never answers it.
#[test]
fn a_command_whose_caller_was_dropped_leaves_the_pending_map() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    runtime().block_on(async {
        let (_, received) = tokio::join!(driver.run(), async {
            let received = tokio::select! {
                answer = connection.send_raw("Browser.getVersion", None, json!({}), HUNG) => {
                    panic!("the browser answered nothing, yet the caller got {answer:?}")
                }
                command = browser.received() => command,
            };
            drop((connection, browser));
            received
        });
        assert_eq!(received["method"], "Browser.getVersion");
    });

    assert_eq!(work.read(Row::CdpPendingOrphans), 0);
}

/// The browser's answer to a command whose caller had gone is dropped: no
/// other command takes it, no reader sees it, and the connection goes on.
#[test]
fn an_answer_to_a_command_whose_caller_had_gone_is_dropped() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let (_, (late, next, unread)) = tokio::join!(driver.run(), async {
            let mut on_browser = connection.subscribe(None);
            let (late, command) = tokio::join!(
                connection.send_raw(
                    "Browser.getVersion",
                    None,
                    json!({}),
                    Duration::from_secs(1)
                ),
                browser.received(),
            );
            browser
                .send(&json!({ "id": command["id"], "result": { "late": true } }))
                .await;
            let (next, ()) = tokio::join!(
                connection.send(None, &GetVersionParams {}, TIMEOUT),
                async {
                    let command = browser.received().await;
                    browser
                        .send(&json!({ "id": command["id"], "result": version() }))
                        .await;
                },
            );
            let unread = tokio::time::timeout(TIMEOUT, on_browser.next()).await;
            drop((connection, browser));
            (late, next, unread.is_err())
        });
        assert_eq!(
            (late.err(), next.map(|version| version.product), unread),
            (
                Some(Failure::TimedOut {
                    method: "Browser.getVersion"
                }),
                Ok("Chrome/156.0.8078.4".to_owned()),
                true
            )
        );
    });
}

/// An event of the page's session, and its length as the browser's message.
fn page_event() -> (Value, usize) {
    let event = json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "page" });
    let length = serde_json::to_vec(&event).unwrap().len();
    (event, length)
}

/// Sends `events` on the page's session, then one on the browser's, and
/// waits for the driver to have handed that one on, so it has handed on
/// every event before.
async fn handed_on(browser: &mut Browser, connection: &Connection, events: &[Value]) {
    let mut on_browser = connection.subscribe(None);
    for event in events {
        browser.send(event).await;
    }
    browser
        .send(&json!({ "method": "Target.targetCreated", "params": {} }))
        .await;
    on_browser.next().await.unwrap();
}

/// The events a subscription holds, taken until it ends, and why it ended.
async fn held(subscription: &mut super::Subscription) -> (usize, Option<SubscriptionEnd>) {
    let mut count = 0;
    while tokio::time::timeout(Duration::from_millis(100), subscription.next())
        .await
        .ok()
        .flatten()
        .is_some()
    {
        count += 1;
    }
    (count, subscription.ended())
}

/// A reader that leaves events untaken holds no more bytes than its quota:
/// the event that would take it past its quota ends it, as one that fell
/// too many events behind, and the events it holds stay its to take.
#[test]
fn a_reader_that_leaves_events_untaken_holds_no_more_bytes_than_its_quota() {
    let (event, length) = page_event();
    let quota = 10 * length + length / 2;
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let driver = driver.with_event_bytes(quota);
        let page = SessionId("page".to_owned());
        let (_, (count, ended)) = tokio::join!(driver.run(), async {
            let mut untaken = connection.subscribe(Some(&page));
            handed_on(&mut browser, &connection, &vec![event; 100]).await;
            let held = held(&mut untaken).await;
            drop((connection, browser));
            held
        });
        assert!(count * length <= quota, "{count} events of {length} bytes");
        assert_eq!((count, ended), (10, Some(SubscriptionEnd::FellBehind)));
    });
}

/// An event larger than the quota alone reaches a reader that holds none,
/// so a reader that keeps up loses no event; a taken event no longer counts
/// against it, and one over the quota while it holds another ends it.
#[test]
fn an_event_over_the_quota_reaches_a_reader_that_holds_none() {
    let (event, length) = page_event();
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let driver = driver.with_event_bytes(length / 2);
        let page = SessionId("page".to_owned());
        let (_, (taken, held)) = tokio::join!(driver.run(), async {
            let mut reader = connection.subscribe(Some(&page));
            let mut taken = 0;
            for _ in 0..2 {
                browser.send(&event).await;
                taken += usize::from(reader.next().await.is_some());
            }
            handed_on(&mut browser, &connection, &[event.clone(), event.clone()]).await;
            let held = held(&mut reader).await;
            drop((connection, browser));
            (taken, held)
        });
        assert_eq!((taken, held), (2, (1, Some(SubscriptionEnd::FellBehind))));
    });
}

/// C1's seam (ADR 0140): the driver reads only what routes an event, so one
/// no reader of its session wants is not decoded past its envelope, and its
/// params are decoded once, from the browser's message, for all its readers.
#[test]
fn an_event_is_decoded_once_for_all_its_readers() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    let page = SessionId("page".to_owned());
    runtime().block_on(async {
        let (_, read) = tokio::join!(driver.run(), async {
            let mut readers = [
                connection.subscribe(Some(&page)),
                connection.subscribe(Some(&page)),
            ];
            browser
                .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "frame" }))
                .await;
            browser.send(&page_event().0).await;
            let mut read = Vec::new();
            for reader in &mut readers {
                let event = reader.next().await.unwrap();
                read.push(
                    event
                        .read::<crate::generated::page::LoadEventFiredEvent>()
                        .map(|read| read.timestamp),
                );
            }
            drop((connection, browser));
            read
        });
        assert!(read.iter().all(Option::is_some), "{read:?}");
    });

    assert_eq!(work.read(Row::CdpEventDecodes), 1);
}

/// The browser's next message, which must come within a second.
async fn written(browser: &mut Browser) -> Value {
    tokio::time::timeout(Duration::from_secs(1), browser.received())
        .await
        .expect("the command is written without its answer being awaited")
}

/// A started command is written without its answer being awaited, in the
/// order started, and its answer comes back later, whatever the order the
/// browser answers in; each counts C1's encodes and send as a sent one does.
#[test]
fn a_started_command_is_written_before_its_answer_is_awaited() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    runtime().block_on(async {
        let (_, products) = tokio::join!(driver.run(), async {
            let first = connection.start(None, &GetVersionParams {}, TIMEOUT);
            let second = connection.start(None, &fetch::EnableParams::default(), TIMEOUT);
            let commands = [written(&mut browser).await, written(&mut browser).await];
            let methods = commands.each_ref().map(|command| command["method"].clone());
            assert_eq!(
                methods,
                ["Browser.getVersion", "Fetch.enable"].map(Value::from)
            );
            for command in commands.iter().rev() {
                let result = if command["method"] == "Fetch.enable" {
                    json!({})
                } else {
                    version()
                };
                browser
                    .send(&json!({ "id": command["id"], "result": result }))
                    .await;
            }
            let products = (first.await.unwrap().product, second.await);
            drop((connection, browser));
            products
        });
        assert_eq!(products.0, "Chrome/156.0.8078.4");
        products.1.unwrap();
    });

    assert_eq!(work.read(Row::CdpEncodes), 4);
    assert_eq!(work.read(Row::CdpSends), 2);
}

/// A started command whose answer was dropped is still written, as nothing
/// takes it back, and leaves the commands in flight: its answer is dropped.
#[test]
fn a_started_command_is_written_though_its_answer_was_dropped() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    runtime().block_on(async {
        let (_, method) = tokio::join!(driver.run(), async {
            drop(connection.start(None, &GetVersionParams {}, TIMEOUT));
            let command = written(&mut browser).await;
            browser
                .send(&json!({ "id": command["id"], "result": version() }))
                .await;
            // The dropped one's answer goes to no later command.
            let (answer, ()) = tokio::join!(
                connection.send(None, &GetVersionParams {}, TIMEOUT),
                async {
                    let command = written(&mut browser).await;
                    browser
                        .send(&json!({ "id": command["id"], "result": version() }))
                        .await;
                }
            );
            answer.unwrap();
            drop((connection, browser));
            command["method"].clone()
        });
        assert_eq!(method, "Browser.getVersion");
    });

    assert_eq!(work.read(Row::CdpPendingOrphans), 0);
}

/// A command started while the connection's queue is full waits for its
/// place as its answer is awaited, and is then written and answered.
#[test]
fn a_command_started_on_a_full_queue_is_written_as_its_answer_is_awaited() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    runtime().block_on(async {
        let started: Vec<_> = (0..=super::REQUESTS)
            .map(|_| connection.start(None, &GetVersionParams {}, TIMEOUT))
            .collect();
        let (_, answered) = tokio::join!(driver.run(), async {
            let answers = async {
                for _ in 0..=super::REQUESTS {
                    let command = written(&mut browser).await;
                    browser
                        .send(&json!({ "id": command["id"], "result": version() }))
                        .await;
                }
                browser
            };
            let (browser, answered) = tokio::join!(answers, async {
                let mut answered = 0;
                for answer in started {
                    answer.await.unwrap();
                    answered += 1;
                }
                answered
            });
            drop((connection, browser));
            answered
        });
        assert_eq!(answered, super::REQUESTS + 1);
    });

    assert_eq!(work.read(Row::CdpSends), 65);
}

/// An event on `session`, sent by the browser.
fn on(session: &str, method: &str) -> Value {
    json!({ "method": method, "params": {}, "sessionId": session })
}

/// The detach of `gone`'s target, sent on `session`.
fn detach(gone: &str, session: &str) -> Value {
    json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": gone, "targetId": gone }, "sessionId": session })
}

/// The methods of the events a subscription takes until none comes for a
/// while, and why it ended, if it did.
async fn taken(subscription: &mut super::Subscription) -> (Vec<String>, Option<SubscriptionEnd>) {
    let mut methods = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), subscription.next()).await
    {
        methods.push(event.method.clone());
    }
    (methods, subscription.ended())
}

/// A subscription listed under more sessions takes their events in the one
/// order the browser sent them, and no other session's; the detach of a
/// session it was listed under after takes that listing out alone.
#[test]
fn a_subscription_listed_under_more_sessions_takes_their_events_in_order() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    runtime().block_on(async {
        let (_, taken) = tokio::join!(driver.run(), async {
            let mut reader = connection.subscribe(Some(&SessionId("page".to_owned())));
            reader
                .more_sessions()
                .add(Some(&SessionId("frame".to_owned())));
            for event in [
                on("frame", "Frame.first"),
                on("page", "Page.second"),
                on("worker", "Worker.unlisted"),
                detach("frame", "page"),
                on("frame", "Frame.afterItsDetach"),
                on("page", "Page.last"),
            ] {
                browser.send(&event).await;
            }
            let taken = taken(&mut reader).await;
            drop((connection, browser));
            taken
        });
        assert_eq!(
            taken,
            (
                [
                    "Frame.first",
                    "Page.second",
                    "Target.detachedFromTarget",
                    "Page.last"
                ]
                .map(str::to_owned)
                .to_vec(),
                None
            )
        );
    });

    assert_eq!(work.read(Row::CdpEventVisits), 4);
}

/// The detach of a subscription's first session ends it with every listing
/// under more sessions, and so does falling behind on any of them: no
/// listing keeps it open.
#[test]
fn a_subscription_ends_with_all_its_listings() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let (_, ends) = tokio::join!(driver.run(), async {
            let [mut detached, mut behind] = ["page", "other"].map(|first| {
                let subscription = connection.subscribe(Some(&SessionId(first.to_owned())));
                subscription
                    .more_sessions()
                    .add(Some(&SessionId(format!("{first}-frame"))));
                subscription
            });
            browser.send(&detach("page", "page")).await;
            browser.send(&on("page-frame", "Frame.afterItsPage")).await;
            for _ in 0..5_000 {
                browser.send(&on("other-frame", "Frame.untaken")).await;
            }
            let ends = (taken(&mut detached).await, held(&mut behind).await.1);
            drop((connection, browser));
            ends
        });
        assert_eq!(
            ends,
            (
                (Vec::new(), Some(SubscriptionEnd::Detached)),
                Some(SubscriptionEnd::FellBehind)
            )
        );
    });
}

/// A reader's handout hands each event on to the subscriptions of its
/// session that started before the connection took it in, though the
/// reader hands it on after; its visits are not the connection's.
#[test]
fn a_handout_hands_each_event_to_the_subscriptions_that_started_before_it() {
    let work = CountedWork::default();
    let (connection, driver, mut browser) = connected_on(&work);
    let page = SessionId("page".to_owned());
    runtime().block_on(async {
        let (_, (early, late, unlisted)) = tokio::join!(driver.run(), async {
            let (handout, mut handing) = connection.handout();
            let mut reader = connection.subscribe(Some(&page));
            let mut early = handout.subscribe(Some(&page));
            let mut unlisted = handout.subscribe(Some(&SessionId("frame".to_owned())));
            browser.send(&on("page", "Page.first")).await;
            let first = reader.next().await.unwrap();
            let mut late = handout.subscribe(Some(&page));
            browser.send(&on("page", "Page.second")).await;
            let second = reader.next().await.unwrap();
            for event in [first, second] {
                handing.hand(&event);
            }
            let taken = (
                taken(&mut early).await.0,
                taken(&mut late).await.0,
                taken(&mut unlisted).await.0,
            );
            drop((connection, browser));
            taken
        });
        assert_eq!(
            (early, late, unlisted),
            (
                vec!["Page.first".to_owned(), "Page.second".to_owned()],
                vec!["Page.second".to_owned()],
                Vec::<String>::new()
            )
        );
    });

    assert_eq!(work.read(Row::CdpEventVisits), 2);
}

/// A handing ends the subscriptions of a session its reader read the
/// detach of, saying so, and every other with its own end, as does one
/// that subscribes after.
#[test]
fn a_handing_ends_its_subscriptions_on_a_detach_and_with_its_end() {
    runtime().block_on(async {
        let (connection, _driver, _browser) = connected();
        let (handout, mut handing) = connection.handout();
        let [mut frame, mut page] =
            ["frame", "page"].map(|on| handout.subscribe(Some(&SessionId(on.to_owned()))));
        handing.detached(&SessionId("frame".to_owned()));
        let frame_end = taken(&mut frame).await;
        handing.end(SubscriptionEnd::Disconnected);
        let mut after = handout.subscribe(None);
        assert_eq!(
            (frame_end, taken(&mut page).await, taken(&mut after).await),
            (
                (Vec::new(), Some(SubscriptionEnd::Detached)),
                (Vec::new(), Some(SubscriptionEnd::Disconnected)),
                (Vec::new(), Some(SubscriptionEnd::Disconnected))
            )
        );
    });
}

/// A connection is closed once its driver ended, or was dropped without
/// running, and not before.
#[test]
fn a_connection_is_closed_once_its_driver_ended() {
    runtime().block_on(async {
        let (connection, driver, browser) = connected();
        let (_, (before, after)) = tokio::join!(driver.run(), async {
            let before = tokio::time::timeout(Duration::from_millis(50), connection.closed()).await;
            drop(browser);
            let after = tokio::time::timeout(TIMEOUT, connection.closed()).await;
            (before.is_ok(), (after.is_ok(), connection.is_closed()))
        });
        assert_eq!((before, after), (false, (true, true)));

        let (connection, driver, _browser) = connected();
        drop(driver);
        assert!(
            tokio::time::timeout(TIMEOUT, connection.closed())
                .await
                .is_ok()
        );
    });
}

mod cdp_pipe {
    goodfetch_test_support::fuzz_replay!(crate::connection::fuzz_pipe, "fuzz/cdp_pipe");
}
