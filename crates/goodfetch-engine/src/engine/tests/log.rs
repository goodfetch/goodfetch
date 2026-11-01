//! The log events each Client of one Daemon receives as `$/log`, over the
//! in-memory network (spec #213's Seam 3): at the level its `hello` names,
//! which `connection.set_log_level` changes without a new `hello` (ADR
//! 0078). Each test installs its own subscriber, which keeps the dispatch
//! records out of its exercise record, so none declares
//! `connection.set_log_level`: Collie's executable's test of `$/log`
//! does (ADR 0072).
//!
//! Covers: story:3-105

use std::collections::BTreeMap;
use std::fmt;
use std::sync::mpsc;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::daemon::{TOKEN, connect, daemon, hello_with, the_pick};
use super::run::call;
use super::{Client, attached_on, engine, hello};
use crate::log::LogEvents;
use crate::rpc::DISPATCH_TARGET;
use crate::transport::InMemoryDialer;

/// A Client and every message it read that answered none of its requests.
struct Reading {
    client: Client,
    read: Vec<Value>,
}

impl Reading {
    /// Sends the request `method` with `params` and reads until its reply,
    /// which it answers (`Null` when the Connection closed first).
    async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.client
            .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
            .await;
        while let Some(message) = self.client.next().await {
            if message["id"] == json!(id) {
                return message;
            }
            self.read.push(message);
        }
        Value::Null
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

/// Ends the Client's input and answers every `$/log` it received until the
/// Daemon closed its Connection.
async fn logged(reading: Reading) -> Vec<Value> {
    let Reading {
        mut client,
        mut read,
    } = reading;
    client.close().await;
    while let Some(message) = client.next().await {
        read.push(message);
    }
    read.retain(|message| message["method"] == json!("$/log"));
    read
}

/// Ends the Client's input and answers the method of every dispatch record
/// it received as `$/log` until the Daemon closed its Connection, sorted.
async fn dispatched(reading: Reading) -> Vec<String> {
    let mut methods: Vec<String> = logged(reading)
        .await
        .into_iter()
        .filter(|message| message["params"]["target"] == json!(DISPATCH_TARGET))
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
fn a_client_of_an_engine_given_no_log_events_sets_its_level_and_receives_no_log_event() {
    let engine = engine(Product::Collie);
    let subscriber = tracing_subscriber::registry();

    let (_, (set, received)) = tracing::subscriber::with_default(subscriber, || {
        daemon(&engine, &TestEnv::new(), the_pick(), |dialer| async move {
            let mut client = opened(&dialer, "trace").await;
            client.call(1, "engine.info", json!({})).await;
            let set = client
                .call(
                    2,
                    "connection.set_log_level",
                    json!({ "log_level": "debug" }),
                )
                .await;
            (set["result"].clone(), logged(client).await)
        })
    });

    assert_eq!((set, received), (json!({}), vec![]));
}

/// A span a test's subscriber saw open: its name, whether it is a root,
/// and its fields.
type Span = (String, bool, BTreeMap<String, String>);

/// Sends each span the subscriber sees open.
struct Opened(mpsc::Sender<Span>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Opened {
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        _: &tracing::span::Id,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        #[derive(Default)]
        struct Fields(BTreeMap<String, String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
                self.0.insert(field.name().to_owned(), format!("{value:?}"));
            }
        }
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        let _ = self.0.send((
            attributes.metadata().name().to_owned(),
            attributes.is_root(),
            fields.0,
        ));
    }
}

/// Covers: story:3-105
#[test]
fn a_run_is_a_root_span_of_its_own_whose_field_is_its_run_id() {
    let (opened, spans) = mpsc::channel();
    let subscriber = tracing_subscriber::registry().with(Opened(opened));
    let env = TestEnv::new();

    tracing::subscriber::with_default(subscriber, || {
        attached_on(&engine(Product::Collie), &env, |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "one" })).await;
            call(&mut client, 2, "run.finish", json!({ "run_id": "one" })).await;
        });
    });

    let runs: Vec<Span> = spans
        .try_iter()
        .filter(|(name, _, _)| name == "run")
        .collect();
    assert_eq!(
        runs,
        [(
            "run".to_owned(),
            true,
            BTreeMap::from([("run_id".to_owned(), "one".to_owned())])
        )]
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
