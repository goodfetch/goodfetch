use std::time::Duration;

use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, ReadHalf, WriteHalf};

use super::{Connection, Driver, Ended, SessionId};
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

#[test]
fn events_reach_the_subscribers_of_their_session_only() {
    runtime().block_on(async {
        let (connection, driver, mut browser) = connected();
        let page = SessionId("page".to_owned());
        let (_, methods) = tokio::join!(driver.run(), async {
            let mut on_page = connection.subscribe(Some(&page)).await;
            let mut on_browser = connection.subscribe(None).await;
            browser
                .send(&json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }, "sessionId": "page" }))
                .await;
            browser
                .send(&json!({ "method": "Target.targetCreated", "params": {} }))
                .await;
            let methods = (
                on_page.next().await.unwrap().method,
                on_browser.next().await.unwrap().method,
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
