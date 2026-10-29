use std::time::Duration;

use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, ReadHalf, WriteHalf};

use super::{Connection, Driver, Ended, SessionId, SubscriptionEnd};
use crate::Failure;
use crate::framing::{Framer, frame};
use crate::generated::browser::GetVersionParams;

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
    let (connection, driver) = Connection::new(reader, writer);
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
                async {
                    tokio::task::yield_now().await;
                    connection.send(None, &GetVersionParams {}, TIMEOUT).await
                },
                async {
                    let first = browser.received().await;
                    let _second = browser.received().await;
                    browser
                        .send(&json!({
                            "id": first["id"],
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
    let (connection, driver) = Connection::new(reader, writer);
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
    let event = super::Event {
        method: "Page.loadEventFired".to_owned(),
        params: json!({ "timestamp": "not a time" }),
    };

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
