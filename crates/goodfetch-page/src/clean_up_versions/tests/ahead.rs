//! A program's Run's Filter lists load as the Run starts, off its first
//! page's path, when its own settings turn a list on (#2688), and a
//! crawl's are left to its crawl, which knows whether its pages read them
//! (#2573), against an in-process engine on the test `Clock` and `Entropy`
//! with blocking work inline, whose test operation starts a Run and
//! reports what the versions loaded before any page asked.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_clean_up_rules::rules::EMBEDDED_DIGEST;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::{EndedBy, Start, Started};
use goodfetch_engine::settings::Opening;
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
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::super::{Versions, lock};
use crate::interception::Interception;

/// The test's own operation.
const STARTED_LISTS: Operation = Operation {
    name: "test.started_lists",
    description: "Starts a Run and answers whether its Filter lists loaded before any page asked.",
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

/// How many times the operation lets the Run's tasks run before it looks.
const YIELDS: usize = 100;

/// Whether `versions` holds the embedded lists loaded.
fn embedded_loaded(versions: &Versions) -> bool {
    lock(&versions.loaded)
        .get(EMBEDDED_DIGEST)
        .and_then(|cell| cell.get().cloned())
        .is_some_and(|loaded| loaded.is_ok_and(|lists| lists.strong_count() > 0))
}

/// `test.started_lists`: starts a Run of the kind `kind` under the run
/// layer `settings`, lets its tasks run, and answers whether `versions`
/// loaded its lists.
async fn started_lists(
    versions: Arc<Versions>,
    params: Value,
    context: CallContext,
) -> Result<Value, CallError> {
    let settings: Map<String, Value> = serde_json::from_value(params["settings"].clone()).unwrap();
    let kind: RunKind = serde_json::from_value(params["kind"].clone()).unwrap();
    let (settings, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Collie),
            settings,
            ..Opening::default()
        })
        .await
        .unwrap();
    let started = context
        .runs()
        .start(Start {
            run_id: Some(RunId::parse("ahead").unwrap()),
            tags: Tags::default(),
            kind,
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
            settings: Some(settings),
        })
        .await
        .map_err(goodfetch_engine::run::start_error)?;
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    for _ in 0..YIELDS {
        tokio::task::yield_now().await;
    }
    drop(run);
    Ok(json!({ "loaded": embedded_loaded(&versions) }))
}

/// The engine's operations, `versions` as its Clean-up rules, and the
/// test's.
fn engine(versions: &Arc<Versions>) -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let operated = Arc::clone(versions);
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(
            Contribution::new("test")
                .with_clean_up_versions(Arc::clone(versions) as _)
                .with_handler(Handler::new(&STARTED_LISTS, move |params, context| {
                    started_lists(Arc::clone(&operated), params, context)
                })),
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

/// Runs the engine while a Client says `hello`, calls `test.started_lists`
/// with `params`, and answers its reply.
fn call(env: &TestEnv, versions: &Arc<Versions>, params: Value) -> Value {
    let engine = engine(versions);
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
            let answer = reply(1, STARTED_LISTS.name, params).await;
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

/// A Run whose own settings turn a Filter list on has its lists loaded as
/// it starts, before any of its pages asks for them; one whose settings
/// turn every list off loads none, and a Session that turns one on loads
/// them on its first page, as before.
#[rstest]
#[case::at_the_defaults(json!({}), true)]
#[case::with_one_list_on(json!({ "filter_lists": { "ads": false, "trackers": false } }), true)]
#[case::with_every_list_off(
    json!({ "filter_lists": { "ads": false, "trackers": false, "annoyances": false } }),
    false
)]
fn a_runs_start_loads_its_filter_lists_when_its_settings_turn_one_on(
    #[case] settings: Value,
    #[case] loaded: bool,
) {
    let env = TestEnv::new();
    let versions = Interception::default().versions();

    let answer = call(
        &env,
        &versions,
        json!({ "settings": settings, "kind": RunKind::Collie }),
    );

    assert_eq!(answer["result"], json!({ "loaded": loaded }), "{answer:?}");
}

/// A crawl's Run loads no Filter list as it starts, even when its settings
/// turn them all on: its crawl asks for them once it knows its pages read
/// a format, and a crawl whose pages read none never holds them (#2573).
#[test]
fn a_crawls_start_leaves_its_filter_lists_to_its_crawl() {
    let env = TestEnv::new();
    let versions = Interception::default().versions();

    let answer = call(
        &env,
        &versions,
        json!({ "settings": {}, "kind": RunKind::Crawl }),
    );

    assert_eq!(answer["result"], json!({ "loaded": false }), "{answer:?}");
}
