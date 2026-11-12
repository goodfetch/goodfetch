//! What a Session's proxy charges its Run ahead (ADR 0021; the deep
//! review's 599-7 and 599-12), against an in-process engine on the test
//! `Clock` and `Entropy` with blocking work inline, whose test operation
//! drives one direction's `Ahead` by hand, step by step: an allowance its
//! reserve's task granted that the direction never took, as when the Run's
//! end drops a relayed connection between the grant and the direction's
//! next poll, is given back, so `proxy_bytes` counts only what passed
//! (#1787).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::pin::pin;
use std::sync::Arc;
#[expect(
    clippy::disallowed_types,
    reason = "the test's waker records its wake in one flag (`Woken`)"
)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::{Context, Wake, Waker};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::{EndedBy, Start, Started};
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
use goodfetch_protocol::page::proxy::ProxyKind;
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::super::{FIRST_ALLOWANCE, ProxyUsage};

/// Starts a direction's reserve and drops the direction once the reserve's
/// task granted the allowance, before the direction takes it.
const UNTAKEN_ALLOWANCE: Operation = Operation {
    name: "test.untaken_allowance",
    description: "Drops a direction between its allowance's grant and its take.",
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

/// A waker that records being woken.
#[expect(
    clippy::disallowed_types,
    reason = "one flag set by a wake and read by the test's own task, never with a lock or across an `.await`, so there is no interleaving to test under loom (ADR 0060)"
)]
#[derive(Default)]
struct Woken(AtomicBool);

impl Wake for Woken {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The most yields the reserve's task takes to grant its allowance.
const YIELDS: usize = 1_000;

/// `test.untaken_allowance`: starts the Run `run_id`, a provider Session's
/// usage of it, and one direction of a connection, whose room for a chunk
/// is polled once by hand, so its reserve's task starts; then, at once or,
/// when `granted`, once that task granted the allowance, which wakes the
/// direction, the direction is dropped without being polled again, as the
/// Run's end drops a relayed connection.
async fn untaken_allowance(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let started = context
        .runs()
        .start(Start {
            run_id: Some(run_id),
            tags: Tags::default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Finish,
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
    let usage = Arc::new(ProxyUsage::new(run, ProxyKind::Isp, None));
    let mut ahead = usage.ahead();
    {
        let woken = Arc::new(Woken::default());
        let waker = Waker::from(Arc::clone(&woken));
        let mut room = pin!(ahead.room(FIRST_ALLOWANCE));
        assert!(
            room.as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending(),
            "the room was made before the reserve's task ran"
        );
        let mut yields = 0;
        while params["granted"] == json!(true) && !woken.0.load(Ordering::SeqCst) {
            assert!(
                yields < YIELDS,
                "the reserve's task never granted its allowance"
            );
            yields += 1;
            tokio::task::yield_now().await;
        }
    }
    drop(ahead);
    drop(usage);
    Ok(json!({}))
}

/// The engine's operations and the test's.
fn engine() -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(
            Contribution::new("test")
                .with_handler(Handler::new(&UNTAKEN_ALLOWANCE, untaken_allowance)),
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

/// Runs the engine while a Client says `hello`, then makes each of
/// `calls` in turn, and answers their replies.
fn calls(env: &TestEnv, calls: &[(&str, Value)]) -> Vec<Value> {
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
            let mut answers = Vec::new();
            for (id, (method, params)) in (1..).zip(calls) {
                answers.push(reply(id, method, params.clone()).await);
            }
            drop(reply);
            writer.shutdown().await.unwrap();
            answers
        };
        let (served, answers) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(env)),
            client
        );
        served.unwrap();
        answers
    })
}

/// An allowance a direction's reserve asked for, which the direction never
/// took because the Run's end dropped it first, before the reserve's task
/// granted it or after, carried no byte: the Run's `proxy_bytes` counts
/// none of it.
/// Covers: story:8-92
#[rstest]
#[case::before_its_grant(false)]
#[case::after_its_grant(true)]
fn an_allowance_a_direction_dropped_before_it_took_it_counts_no_byte(#[case] granted: bool) {
    let env = TestEnv::new();

    let answers = calls(
        &env,
        &[
            (
                "test.untaken_allowance",
                json!({ "run_id": "untaken", "granted": granted }),
            ),
            ("run.finish", json!({ "run_id": "untaken" })),
            ("run.usage", json!({ "run_id": "untaken" })),
        ],
    );

    assert_eq!(
        (
            answers[0].get("result").is_some(),
            answers[1].get("result").is_some(),
            &answers[2]["result"]["proxy_bytes"],
        ),
        (true, true, &json!(0)),
        "{answers:?}"
    );
}
