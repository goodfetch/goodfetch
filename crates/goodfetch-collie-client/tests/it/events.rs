//! A page's events read by meaning (ADR 0081), hand-played over the
//! in-memory network on the paused runtime: `page.events` sends its
//! `page.on` call when it is called, grants credits once it is sent, hands
//! the events in order, grants more as they are read, and its drop cancels
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
/// stream is first read, grants 16 credits once it is sent, hands the
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

/// A stream skips the events of its call it was not asked for, grants 8
/// credits again once 8 events were read, ends with `None` once its call
/// answers, and hands the call's error once, then `None`.
#[test]
fn a_stream_regrants_credits_as_it_reads_and_ends_with_its_calls_reply() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    let (texts, ended, failed, after, credits) = runtime.block_on(async {
        let peer = async {
            let mut credits = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                let message: Value = serde_json::from_str(&line).unwrap();
                let method = message["method"].as_str().unwrap().to_owned();
                if method == "$/credit" {
                    let id = message["params"]["id"].clone();
                    credits.push(message["params"]["credits"].as_u64().unwrap());
                    if credits.len() == 1 {
                        // Ten events, one of them not the stream's.
                        let mut events = vec![("page.page_error", json!({}))];
                        events.extend((0..9).map(|n| ("page.console", console(&n.to_string()))));
                        writer.write_all(&batch(&id, 1, &events)).await.unwrap();
                        let ended = json!({"jsonrpc": "2.0", "id": id, "result": {}});
                        let mut line = serde_json::to_vec(&ended).unwrap();
                        line.push(b'\n');
                        writer.write_all(&line).await.unwrap();
                    } else if credits.len() == 3 {
                        let failed = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "events lost", "data": {"code": "page_events_lost", "group": "unavailable", "retryable": false, "fix": "Read the events faster."}}});
                        let mut line = serde_json::to_vec(&failed).unwrap();
                        line.push(b'\n');
                        writer.write_all(&line).await.unwrap();
                    }
                }
                if message.get("id").is_some() && method != "page.on" {
                    let _unread = writer.write_all(&reply(&line, &answered(&method))).await;
                }
            }
            writer.shutdown().await.unwrap();
            credits
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
            let mut texts = Vec::new();
            while let Some(item) = stream.next().await {
                texts.push(item.unwrap().text);
            }
            let ended = stream.next().await.is_none();
            let mut failing = page.events(events::Console);
            let failed = failing.next().await;
            let after = failing.next().await.is_none();
            drop(failing);
            run.close().await.unwrap();
            connection.close().await.unwrap();
            (texts, ended, failed, after)
        };
        let ((texts, ended, failed, after), credits) = tokio::join!(client, peer);
        (texts, ended, failed, after, credits)
    });

    let expected: Vec<String> = (0..9).map(|n| n.to_string()).collect();
    assert_eq!(texts, expected);
    assert!(ended);
    assert!(matches!(failed, Some(Err(Error::Failed(_)))), "{failed:?}");
    assert!(after);
    assert_eq!(credits, [16, 8, 16]);
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
