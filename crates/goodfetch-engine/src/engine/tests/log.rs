//! The log events each Client of one Daemon receives as `$/log`, over the
//! in-memory network (spec #213's Seam 3): at the level its `hello` names,
//! which `connection.set_log_level` changes without a new `hello` (ADR
//! 0078).
//!
//! Covers: story:3-105, operation:connection.set_log_level

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::Client;
use super::daemon::{TOKEN, connect, daemon, hello_with, the_pick};
use super::engine;
use crate::log::LogEvents;
use crate::rpc::DISPATCH_TARGET;
use crate::transport::InMemoryDialer;

/// A Client and every message it read that answered none of its requests.
struct Reading {
    client: Client,
    read: Vec<Value>,
}

impl Reading {
    /// Sends the request `method` with `params` and reads until its reply.
    async fn call(&mut self, id: i64, method: &str, params: Value) {
        self.client
            .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
            .await;
        while let Some(message) = self.client.next().await {
            if message["id"] == json!(id) {
                return;
            }
            self.read.push(message);
        }
    }
}

/// A Client of the Daemon that said `hello` with the token at `log_level`.
async fn opened(dialer: &InMemoryDialer, log_level: &str) -> Reading {
    let mut hello = hello_with("collie", "0.1", "sdk_typescript", Some(TOKEN));
    hello["params"]["log_level"] = json!(log_level);
    let mut reading = Reading {
        client: connect(dialer).await,
        read: Vec::new(),
    };
    reading.call(0, "hello", hello["params"].clone()).await;
    reading
}

/// Ends the Client's input and answers the method of every dispatch record
/// it received as `$/log` until the Daemon closed its Connection, sorted.
async fn dispatched(reading: Reading) -> Vec<String> {
    let Reading {
        mut client,
        mut read,
    } = reading;
    client.close().await;
    while let Some(message) = client.next().await {
        read.push(message);
    }
    let mut methods: Vec<String> = read
        .into_iter()
        .filter(|message| {
            message["method"] == json!("$/log")
                && message["params"]["target"] == json!(DISPATCH_TARGET)
        })
        .map(|message| {
            message["params"]["fields"]["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    methods.sort();
    methods
}

#[test]
fn two_clients_of_one_daemon_each_receive_the_log_events_at_their_own_level() {
    let log_events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(log_events.layer());
    let engine = engine(Product::Collie).with_log_events(log_events);

    let (_, (tracing, informed)) = tracing::subscriber::with_default(subscriber, || {
        daemon(&engine, &TestEnv::new(), the_pick(), |dialer| async move {
            let mut tracing = opened(&dialer, "trace").await;
            let mut informed = opened(&dialer, "info").await;
            tracing.call(1, "engine.info", json!({})).await;
            informed.call(1, "engine.info", json!({})).await;
            (dispatched(tracing).await, dispatched(informed).await)
        })
    });

    assert_eq!(
        (tracing, informed),
        (
            vec![
                "engine.info".to_owned(),
                "engine.info".to_owned(),
                "hello".to_owned()
            ],
            vec![]
        )
    );
}

#[test]
fn a_level_a_client_sets_takes_effect_without_a_new_hello() {
    let log_events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(log_events.layer());
    let engine = engine(Product::Collie).with_log_events(log_events);

    let (_, received) = tracing::subscriber::with_default(subscriber, || {
        daemon(&engine, &TestEnv::new(), the_pick(), |dialer| async move {
            let mut client = opened(&dialer, "off").await;
            client.call(1, "engine.info", json!({})).await;
            client
                .call(
                    2,
                    "connection.set_log_level",
                    json!({ "log_level": "trace" }),
                )
                .await;
            client.call(3, "engine.info", json!({})).await;
            dispatched(client).await
        })
    });

    assert_eq!(received, ["connection.set_log_level", "engine.info"]);
}
