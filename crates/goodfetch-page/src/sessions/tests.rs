//! The Sessions table against a Run that stops while a Session opens
//! (#1763), and a page that ends past its memory cap: an in-process engine
//! of the engine's operations, the page crate's and the test's own, on the
//! test `Clock` and `Entropy` with blocking work inline, whose test
//! operations drive the table themselves, step by step. One opens a Run's
//! default Session and stops the Run while the open waits for its record in
//! run state: a Session left in the table past its Run's clean-up would
//! hold the Run, its browsers and its egress relay forever. The other ends
//! a page past its memory cap as its watch does: a page found in neither
//! the open pages nor the ended ones would answer `closed`.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::task::Poll;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::{EndedBy, Run, Start, Started};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Handler,
    Identity,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::governor::BROWSER_PAGE_MEMORY_CAP;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::watch;

use super::life::{Ending, PageLife};
use super::{Opener, Sessions};
use crate::dialogs::Dialogs;
use crate::interception::Interception;
use crate::page_events;
use crate::page_value::ProgramPages;

/// Opens a Run's default Session and stops the Run while the open waits
/// for its record.
const STOPPED_OPEN: Operation = Operation {
    name: "test.stopped_open",
    description: "Stops a Run while its default Session opens.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Ends a page past its memory cap and confirms its close, as the page's
/// watch does when its target's end arrives first.
const ENDED_PAST_CAP: Operation = Operation {
    name: "test.ended_past_cap",
    description: "Ends a page past its memory cap and reads it after.",
    ..STOPPED_OPEN
};

/// The Run `run_id`, started for the test's own operation.
async fn started(run_id: &RunId, context: &CallContext) -> Result<Run, CallError> {
    let started = context
        .runs()
        .start(Start {
            run_id: Some(run_id.clone()),
            tags: Tags::default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Driver,
            initialize: None,
            policy_version: None,
            weakened: Vec::new(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: None,
            timeout: None,
            settings: None,
        })
        .await
        .map_err(goodfetch_engine::run::start_error)?;
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    Ok(run)
}

/// A Sessions table of its own for the test's operation.
fn table() -> Sessions {
    Sessions::new(
        Vec::new(),
        Interception::default(),
        page_events::Table::default(),
        Dialogs::default(),
        Arc::new(ProgramPages),
    )
}

/// `test.ended_past_cap`: starts the Run `run_id` and reaches it in a
/// Sessions table of its own, then ends one of its pages past its memory
/// cap, confirms the close and acts on its end, as the page's watch does
/// when its target's end comes before the closing call's answer. Answers
/// the code a call on the page raises after.
async fn ended_past_cap(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = started(&run_id, &context).await?;
    let sessions = table();
    sessions.reach(&run, &context).await?;
    let page_id = PageId("page-1".to_owned());
    let life = PageLife::new(page_id.clone(), None, watch::channel(0).1);
    let limit = SettingLimit {
        key: BROWSER_PAGE_MEMORY_CAP.name.to_owned(),
        value: json!(32 << 20),
        layer: SettingsLayer::Run,
    };
    life.end(&Ending::MemoryExceeded {
        memory: 64 << 20,
        limit,
    })?;
    let confirmed = life.end(&Ending::Destroyed)?;

    sessions.page_ended(&page_id, &run, &life, confirmed).await;

    let raised = sessions
        .page(&page_id)
        .await
        .err()
        .map(|error| error.code());
    run.cancellation().cancel();
    Ok(json!({ "raised": raised }))
}

/// `test.stopped_open`: starts the Run `run_id`, reaches it in a Sessions
/// table of its own and opens its default Session, polled by hand until it
/// took its position and waits; then stops the Run and lets the table's
/// clean-up take the Run out before the open goes on. Answers whether the
/// open succeeded, and how many Sessions of the Run the table holds after.
async fn stopped_open(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = started(&run_id, &context).await?;
    let sessions = table();
    sessions.reach(&run, &context).await?;
    let positioned = async || {
        sessions
            .state
            .lock()
            .await
            .runs
            .get(run.id())
            .is_some_and(|runs| runs.last > 0)
    };
    let opened = {
        let mut open = std::pin::pin!(sessions.open_held(&run, None, Opener::Default, &context));
        loop {
            let polled = std::future::poll_fn(|waker| Poll::Ready(open.as_mut().poll(waker))).await;
            assert!(polled.is_pending(), "the open ended before the Run stopped");
            if positioned().await {
                break;
            }
            tokio::task::yield_now().await;
        }
        run.cancellation().cancel();
        while sessions.state.lock().await.runs.contains_key(run.id()) {
            tokio::task::yield_now().await;
        }
        open.await
    };
    let left = sessions
        .state
        .lock()
        .await
        .sessions
        .values()
        .filter(|session| session.run.id() == run.id())
        .count();
    Ok(json!({ "opened": opened.is_ok(), "left": left }))
}

/// The engine's operations, the page crate's and the test's.
fn engine() -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(crate::contribution(Vec::new()))
        .with(
            Contribution::new("test")
                .with_handler(Handler::new(&STOPPED_OPEN, stopped_open))
                .with_handler(Handler::new(&ENDED_PAST_CAP, ended_past_cap)),
        );
    Engine::new(identity, &composition).unwrap()
}

fn dependencies(env: &TestEnv) -> Dependencies {
    Dependencies {
        environment: BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// Runs the engine while a Client says `hello`, then calls `method` with
/// `params`, and answers the call's reply.
fn call(env: &TestEnv, method: &str, params: Value) -> Value {
    let engine = engine();
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            let mut reply = async |id: i64, method: &str, params: Value| {
                let mut line = serde_json::to_vec(
                    &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
                )
                .unwrap();
                line.push(b'\n');
                writer.write_all(&line).await.unwrap();
                loop {
                    let line = lines.next_line().await.unwrap().unwrap();
                    let message: Value = serde_json::from_str(&line).unwrap();
                    if message["id"] == json!(id) {
                        return message;
                    }
                }
            };
            let hello = reply(
                0,
                "hello",
                json!({
                    "protocol_version": "0.1",
                    "surface": "rust",
                    "needs": "collie",
                    "product_version": "1.4.0",
                }),
            )
            .await;
            assert_eq!(hello["result"]["product"], "collie");
            let answer = reply(1, method, params).await;
            drop(reply);
            writer.shutdown().await.unwrap();
            answer
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(env)),
            client
        );
        served.unwrap();
        answer
    })
}

/// A Run stopped while its default Session waits for its record in run
/// state leaves no Session of it in the table: the open, which took its
/// position before the Run stopped, finds the Run gone under the table's
/// lock and raises `run_not_found` instead of putting the Session in.
/// Covers: story:5-60
#[test]
fn a_default_session_whose_run_stops_while_it_opens_is_never_left_in_the_table() {
    let env = TestEnv::new();

    let reply = call(
        &env,
        "test.stopped_open",
        json!({ "run_id": "stopped-open" }),
    );

    assert_eq!(reply["result"], json!({ "opened": false, "left": 0 }));
}

/// A page the Resource governor ended past its memory cap, whose close is
/// confirmed, leaves the open pages straight for the ended ones its later
/// calls read, under one lock: a call on it raises `page_memory_exceeded`
/// again, never `closed`.
/// Covers: story:3-59
#[test]
fn a_page_ended_past_its_memory_cap_answers_page_memory_exceeded_once_its_close_is_confirmed() {
    let env = TestEnv::new();

    let reply = call(
        &env,
        "test.ended_past_cap",
        json!({ "run_id": "ended-past-cap" }),
    );

    assert_eq!(reply["result"], json!({ "raised": "page_memory_exceeded" }));
}
