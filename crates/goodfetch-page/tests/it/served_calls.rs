//! A call on a page the Product serves itself, as a fetch tier serves a
//! page it opened from a document (ADR 0093): the page crate's guard finds
//! the page the call names and hands the call to the Product, in process,
//! with no browser. The call decodes its params once, the guard's search
//! for its page included (ADR 0140's E1, issue 2617).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::future::ready;

use goodfetch_counted_work::Row;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    Answered, CallContext, Channel, Composition, Dependencies, Engine, GuardAnswer, Identity,
    Params,
};
use goodfetch_page::Launcher;
use goodfetch_page::blocks::NoBlocks;
use goodfetch_page::page_value::{HandlerPages, Observer, PageValue};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The page the Product serves.
const SERVED: &str = "served-page";

/// A Product that answers every call on [`SERVED`] with the call's name.
struct Served;

impl HandlerPages for Served {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::empty()
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        None
    }

    fn call(
        &self,
        page_id: &PageId,
        operation: &'static str,
        _params: Params<'_>,
        context: &CallContext,
    ) -> Option<GuardAnswer> {
        (page_id.0 == SERVED)
            .then(|| Box::pin(ready(Answered::encode(context, &operation))) as GuardAnswer)
    }
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

/// Says `hello` to an engine of the engine's operations and the page
/// crate's, its pages [`Served`]'s, then calls `method` with `params`: the
/// call's reply and the decodes of params it counted.
fn called(method: &str, params: &Value) -> (Value, u64) {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            (Vec::new(), Launcher::Executable),
            Served,
            std::sync::Arc::new(NoBlocks),
        ));
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").expect("invariant: a version"),
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &composition)
        .expect("invariant: the composition composes")
        .with_metrics(metrics);
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            let mut reply = async |id: i64, method: &str, params: &Value| {
                let mut line = serde_json::to_vec(
                    &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
                )
                .expect("invariant: JSON");
                line.push(b'\n');
                writer.write_all(&line).await.expect("invariant: in memory");
                loop {
                    let line = lines
                        .next_line()
                        .await
                        .expect("invariant: in memory")
                        .expect("invariant: the engine answers");
                    let message: Value =
                        serde_json::from_str(&line).expect("invariant: the engine writes JSON");
                    if message["id"] == json!(id) {
                        return message;
                    }
                }
            };
            let hello = reply(
                0,
                "hello",
                &json!({
                    "protocol_version": "0.1",
                    "surface": "rust",
                    "needs": "collie",
                    "product_version": "1.4.0",
                }),
            )
            .await;
            assert_eq!(hello["result"]["product"], "collie");
            let before = work.read(Row::MessageDecodes);
            let answer = reply(1, method, params).await;
            let decodes = work.read(Row::MessageDecodes) - before;
            drop(reply);
            writer.shutdown().await.expect("invariant: in memory");
            (answer, decodes)
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(&env)),
            client
        );
        served.expect("invariant: the engine got its Connection");
        answer
    })
}

/// A call on a page the Product serves reaches the Product, whether the
/// call names its page itself or through its locator, and decodes its
/// params once: the guard finds the page in the pass that reads them into
/// the handler's type.
/// Covers: operation:page.goto, operation:locator.click
#[rstest]
#[case::by_its_page_id(
    "page.goto",
    json!({ "page_id": SERVED, "url": "https://a.test/" }),
)]
#[case::by_its_locator(
    "locator.click",
    json!({ "locator": { "selector": "#go", "page_id": SERVED } }),
)]
fn a_call_on_a_served_page_decodes_its_params_once(#[case] method: &str, #[case] params: Value) {
    let (reply, decodes) = called(method, &params);

    assert_eq!((&reply["result"], decodes), (&json!(method), 1), "{reply}");
}
