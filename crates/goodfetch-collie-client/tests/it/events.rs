//! A page's events read by meaning (ADR 0081), hand-played over the
//! in-memory network on the paused runtime: `page.events` sends its
//! `page.on` call when it is called, grants credits once it hears the call
//! was sent, hands the events in order, grants more as they are read, and its drop cancels
//! the call without a warning. `goodfetch-collie`'s `sdk::events` tests
//! play the `linked` side against a real page.

use goodfetch_collie_client::transport::Endpoint;
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, events};
use goodfetch_test_support::logged;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use crate::connection::{hello, reply, scripted};

/// What the hand-played engine answers `hello`.
fn said() -> Value {
    json!({
        "protocol_version": "0.1",
        "product": "collie",
        "product_version": "0.0.0",
        "engine_instance": "01J0000000000000000000000",
    })
}

/// What the hand-played engine answers each call it does not stream to.
fn answered(method: &str) -> Value {
    match method {
        "hello" => said(),
        "run.start" => {
            json!({"run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc", "lifecycle": "running", "needs": "collie", "started_as": "new"})
        }
        "run.new_page" => json!({"page_id": "page-1", "session_id": "session-1"}),
        "run.abort" => json!({
            "run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc",
            "lifecycle": "stopped",
            "verdict": "verified",
            "completeness": "incomplete",
            "stop_reason": {"kind": "abort", "by": "client"},
            "started_at": "2026-01-01T00:00:00Z",
            "ended_at": "2026-01-01T00:00:00Z",
            "usage": {
                "model": {"calls": 0, "tokens": 0, "input_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0, "output_tokens": 0, "reasoning_tokens": 0, "price": null},
                "browser_time": 0,
                "proxy_bytes": 0,
                "requests": 0,
            },
        }),
        _ => json!({}),
    }
}

/// A `$/events` batch of the call `id`, each of `events` a `(name, data)`.
fn batch(id: &Value, first: u64, events: &[(&str, Value)]) -> Vec<u8> {
    let events: Vec<Value> = (first..)
        .zip(events)
        .map(
            |(position, (event, data))| json!({"position": position, "event": event, "data": data}),
        )
        .collect();
    let mut line = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": "$/events",
        "params": {"id": id, "events": events},
    }))
    .unwrap();
    line.push(b'\n');
    line
}

/// A console message of the page.
fn console(text: &str) -> Value {
    json!({
        "page_id": "page-1",
        "type": "log",
        "text": text,
        "location": {"url": "", "line_number": 0, "column_number": 0},
    })
}

/// `page.events(events::Console)` sends `page.on` for `console` before the
/// stream is first read, grants 16 credits at its first read, hands the
/// page's console messages in order, and dropping it sends `$/cancel` for
/// the call and logs no warning.
/// Covers: story:11-61, story:11-62, story:11-63
#[test]
fn page_events_subscribes_on_call_hands_events_in_order_and_its_drop_cancels_without_a_warning() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (armed, armed_in) = tokio::sync::oneshot::channel();
    let ((texts, heard), log) = logged(|| {
        runtime.block_on(async {
            let peer = async {
                let mut armed = Some(armed);
                let mut heard = Vec::new();
                while let Some(line) = lines.next_line().await.unwrap() {
                    let message: Value = serde_json::from_str(&line).unwrap();
                    let method = message["method"].as_str().unwrap().to_owned();
                    match method.as_str() {
                        "page.on" => {
                            if let Some(armed) = armed.take() {
                                armed.send(()).unwrap();
                            }
                        }
                        "$/credit" => {
                            let id = &message["params"]["id"];
                            writer
                                .write_all(&batch(
                                    id,
                                    1,
                                    &[
                                        ("page.console", console("first")),
                                        ("page.console", console("second")),
                                    ],
                                ))
                                .await
                                .unwrap();
                        }
                        "$/cancel" => {
                            let id = message["params"]["id"].clone();
                            let ended = json!({"jsonrpc": "2.0", "id": id, "result": {}});
                            let mut line = serde_json::to_vec(&ended).unwrap();
                            line.push(b'\n');
                            writer.write_all(&line).await.unwrap();
                        }
                        _ => {}
                    }
                    if message.get("id").is_some() && method != "page.on" {
                        let _unread = writer.write_all(&reply(&line, &answered(&method))).await;
                    }
                    heard.push((method, message["params"].clone(), message["id"].clone()));
                }
                writer.shutdown().await.unwrap();
                heard
            };
            let client = async {
                let (connection, _) = ClientConnection::open(
                    ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                    &network,
                    &Endpoint::InMemory,
                    hello(),
                )
                .await
                .unwrap();
                let run = connection.start().await.unwrap();
                let page = run.new_page().await.unwrap();
                let mut stream = page.events(events::Console);
                // Sent before the stream is read.
                armed_in.await.unwrap();
                let mut texts = Vec::new();
                for _ in 0..2 {
                    texts.push(stream.next().await.unwrap().unwrap().text);
                }
                drop(stream);
                page.close().await.unwrap();
                run.close().await.unwrap();
                connection.close().await.unwrap();
                texts
            };
            tokio::join!(client, peer)
        })
    });

    assert_eq!(texts, ["first", "second"]);
    let on = heard
        .iter()
        .find(|(method, _, _)| method == "page.on")
        .unwrap();
    assert_eq!(on.1, json!({"page_id": "page-1", "event": "console"}));
    let id = on.2.clone();
    let streamed: Vec<(&str, &Value)> = heard
        .iter()
        .filter(|(method, _, _)| method.starts_with("$/"))
        .map(|(method, params, _)| (method.as_str(), params))
        .collect();
    assert_eq!(
        streamed,
        [
            ("$/credit", &json!({"id": id, "credits": 16})),
            ("$/cancel", &json!({"id": id})),
        ]
    );
    // The Experimental entries' own warnings aside (ADR 0030), nothing
    // warns.
    let log = String::from_utf8(log).unwrap();
    let warnings: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("WARN") && !line.contains("goodfetch_collie_client::markers"))
        .collect();
    assert!(warnings.is_empty(), "{log}");
}

/// A line the hand-played engine writes: `message` and its newline.
fn line(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// Plays an engine over `lines` and `writer` that answers each call but
/// `page.on` with [`answered`], answers `$/cancel` by ending its call, and
/// first writes what `streamed` returns for each message; returns every
/// message it heard.
async fn engine(
    mut lines: tokio::io::Lines<impl tokio::io::AsyncBufRead + Unpin>,
    mut writer: impl tokio::io::AsyncWrite + Unpin,
    mut streamed: impl FnMut(&Value) -> Vec<Vec<u8>>,
) -> Vec<Value> {
    let mut heard = Vec::new();
    while let Some(text) = lines.next_line().await.unwrap() {
        let message: Value = serde_json::from_str(&text).unwrap();
        let method = message["method"].as_str().unwrap().to_owned();
        for written in streamed(&message) {
            writer.write_all(&written).await.unwrap();
        }
        if method == "$/cancel" {
            let ended = json!({"jsonrpc": "2.0", "id": message["params"]["id"], "result": {}});
            writer.write_all(&line(&ended)).await.unwrap();
        }
        if message.get("id").is_some() && method != "page.on" {
            let _unread = writer.write_all(&reply(&text, &answered(&method))).await;
        }
        heard.push(message);
    }
    writer.shutdown().await.unwrap();
    heard
}

/// The `$/` notifications among `heard`, as `(method, params)`.
fn streamed(heard: &[Value]) -> Vec<(String, Value)> {
    heard
        .iter()
        .filter_map(|message| {
            let method = message["method"].as_str()?;
            method
                .starts_with("$/")
                .then(|| (method.to_owned(), message["params"].clone()))
        })
        .collect()
}

/// The ID of the `page.on` call among `heard`.
fn page_on(heard: &[Value]) -> Value {
    heard
        .iter()
        .find(|message| message["method"] == "page.on")
        .unwrap()["id"]
        .clone()
}

/// Opens a Connection over `network`, starts a Run and opens a page, runs
/// `read` on it, then closes them.
async fn on_a_page<T, F: Future<Output = T>>(
    network: &goodfetch_collie_client::transport::Network,
    read: impl FnOnce(goodfetch_collie_client::Page) -> F,
) -> T {
    let (connection, _) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        network,
        &Endpoint::InMemory,
        hello(),
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    let read = read(page).await;
    run.close().await.unwrap();
    connection.close().await.unwrap();
    read
}

/// A stream read to its end hands only its own event, grants 8 credits
/// again once 8 events were read, and ends with `None` once its call
/// answers.
#[test]
fn a_stream_hands_its_own_event_regrants_credits_as_it_reads_and_ends_with_its_calls_reply() {
    let (network, lines, writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (texts, heard) = runtime.block_on(async {
        let engine = engine(lines, writer, |message| {
            if message["method"] != "$/credit" || message["params"]["credits"] != 16 {
                return Vec::new();
            }
            let id = &message["params"]["id"];
            // Ten events, one of them not the stream's.
            let mut events = vec![("page.page_error", json!({}))];
            events.extend((0..9).map(|n| ("page.console", console(&n.to_string()))));
            vec![
                batch(id, 1, &events),
                line(&json!({"jsonrpc": "2.0", "id": id, "result": {}})),
            ]
        });
        let client = on_a_page(&network, |page| async move {
            let mut stream = page.events(events::Console);
            let mut texts = Vec::new();
            while let Some(item) = stream.next().await {
                texts.push(item.unwrap().text);
            }
            texts
        });
        tokio::join!(client, engine)
    });

    assert_eq!(texts, ["0", "1", "2", "3", "4", "5", "6", "7", "8"]);
    let id = page_on(&heard);
    assert_eq!(
        streamed(&heard),
        [
            ("$/credit".to_owned(), json!({"id": id, "credits": 16})),
            ("$/credit".to_owned(), json!({"id": id, "credits": 8})),
        ]
    );
}

/// A stream whose call fails hands the call's error once, then `None`.
#[test]
fn a_failed_calls_stream_hands_its_error_once_then_ends() {
    let (network, lines, writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (items, _heard) = runtime.block_on(async {
        let engine = engine(lines, writer, |message| {
            if message["method"] != "$/credit" {
                return Vec::new();
            }
            vec![line(&json!({
                "jsonrpc": "2.0",
                "id": message["params"]["id"],
                "error": {"code": -32000, "message": "events lost", "data": {"code": "page_events_lost", "group": "unavailable", "retryable": false, "fix": "Read the events faster."}},
            }))]
        });
        let client = on_a_page(&network, |page| async move {
            let mut stream = page.events(events::Console);
            (stream.next().await, stream.next().await)
        });
        tokio::join!(client, engine)
    });

    let (Some(Err(Error::Failed(failure))), None) = items else {
        panic!("{items:?}");
    };
    assert_eq!(failure.data.code, "page_events_lost");
}

/// A stream dropped before the Connection sent its call cancels the call
/// once it is sent, and grants it nothing.
#[test]
fn a_stream_dropped_before_its_call_is_sent_cancels_it() {
    let (network, lines, writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let ((), heard) = runtime.block_on(async {
        let engine = engine(lines, writer, |_| Vec::new());
        // No await between the call and the drop: the Connection's task
        // has not sent the call yet.
        let client = on_a_page(&network, |page| async move {
            drop(page.events(events::Console));
        });
        tokio::join!(client, engine)
    });

    let id = page_on(&heard);
    assert_eq!(
        streamed(&heard),
        [("$/cancel".to_owned(), json!({"id": id}))]
    );
}

/// A closed page's `events` sends nothing: its stream hands
/// [`Error::Closed`] once, then `None`.
#[test]
fn a_closed_pages_events_hand_closed_once_and_send_nothing() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (items, methods) = runtime.block_on(async {
        let peer = async {
            let mut methods = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                let message: Value = serde_json::from_str(&line).unwrap();
                let method = message["method"].as_str().unwrap().to_owned();
                let _unread = writer.write_all(&reply(&line, &answered(&method))).await;
                methods.push(method);
            }
            writer.shutdown().await.unwrap();
            methods
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let run = connection.start().await.unwrap();
            let page = run.new_page().await.unwrap();
            page.close().await.unwrap();
            let mut stream = page.events(events::WebError);
            let items = (stream.next().await, stream.next().await);
            drop(stream);
            run.close().await.unwrap();
            connection.close().await.unwrap();
            items
        };
        tokio::join!(client, peer)
    });

    assert!(
        matches!(items, (Some(Err(Error::Closed)), None)),
        "{items:?}"
    );
    assert_eq!(
        methods,
        [
            "hello",
            "run.start",
            "run.new_page",
            "page.close",
            "run.abort"
        ]
    );
}
