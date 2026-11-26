//! A live Run for the unit tests of a Page's parts that take one: an
//! in-process engine of the engine's own operations and a test one, on the
//! test `Clock` and `Entropy` with blocking work inline, whose Client calls
//! the test operation once, which starts a Run, hands it out with the
//! call's event stream and Client, and holds it until the test lets it go;
//! the handler requests the engine sends that Client meanwhile reach the
//! test, which replies to them; and a page a played browser drives for it.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_engine::client::Client;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::events::Events;
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
use goodfetch_protocol::run::{RunKind, Tags};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::driver::PageDriver;
use crate::driver::replay::{self, Played};

/// Starts a Run, hands it out and holds it until the test lets it go.
const HOLD_RUN: Operation = Operation {
    name: "test.hold_run",
    description: "Starts a Run and holds it for the test.",
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

/// The engine and its Client, as one future: it ends once the test let
/// its Run go.
pub(crate) type Served = Pin<Box<dyn Future<Output = ()>>>;

/// A Run the test holds, and the event stream and Client of the call that
/// started it; dropping it lets the Run go, after which the engine ends.
pub(crate) struct LiveRun {
    pub(crate) run: Run,
    pub(crate) events: Events,
    /// The Client the engine sends the Run's handler requests.
    pub(crate) client: Client,
    /// Each handler request the Client got, as its message.
    pub(crate) asked: mpsc::UnboundedReceiver<Value>,
    /// The Client's replies to them, each a message it sends.
    pub(crate) replies: mpsc::UnboundedSender<Value>,
    _release: oneshot::Sender<()>,
}

/// What the test operation hands out, and what lets it answer.
type Holding = (
    oneshot::Sender<(Run, Events, Client)>,
    oneshot::Receiver<()>,
);

impl LiveRun {
    /// Starts an engine in process for `env`, and a Run in it: answers the
    /// Run and the engine's future, which the test polls beside its own
    /// work until it dropped the Run.
    pub(crate) async fn start(env: &TestEnv) -> (Self, Served) {
        let (handed, taken) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let holding: Arc<Mutex<Option<Holding>>> = Arc::new(Mutex::new(Some((handed, released))));
        let engine = engine(holding);
        let dependencies = dependencies(env);
        let (asking, asked) = mpsc::unbounded_channel();
        let (replies, mut replying) = mpsc::unbounded_channel::<Value>();
        let mut served: Served = Box::pin(async move {
            let (network, dialer) = transport::in_memory();
            let client = async {
                let (reader, mut writer) = dialer.connect().await.unwrap().split();
                let mut lines = BufReader::new(reader).lines();
                // Calls `method`, and sends the test's replies to the
                // handler requests that come meanwhile, until its answer.
                let mut reply = async |id: i64, method: &str, params: Value| {
                    let call =
                        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
                    writer.write_all(&line_of(&call)).await.unwrap();
                    loop {
                        tokio::select! {
                            line = lines.next_line() => {
                                let line = line.unwrap().unwrap();
                                let message: Value = serde_json::from_str(&line).unwrap();
                                if message.get("method").is_some() {
                                    // A handler request, which the test
                                    // answers; a notification, which it
                                    // does not read.
                                    if message.get("id").is_some() {
                                        let _ = asking.send(message);
                                    }
                                } else if message["id"] == json!(id) {
                                    return message;
                                }
                            }
                            Some(reply) = replying.recv() => {
                                writer.write_all(&line_of(&reply)).await.unwrap();
                            }
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
                let held = reply(1, HOLD_RUN.name, json!({})).await;
                assert!(held.get("result").is_some(), "{held}");
                drop(reply);
                writer.shutdown().await.unwrap();
            };
            let (ran, ()) = tokio::join!(
                engine.run_attached(Network::InMemory(network), dependencies),
                client
            );
            ran.unwrap();
        });
        let (run, events, client) = tokio::select! {
            biased;
            taken = taken => taken.expect("invariant: the test operation hands its Run out"),
            () = &mut served => panic!("the engine ended before its Run started"),
        };
        let held = Self {
            run,
            events,
            client,
            asked,
            replies,
            _release: release,
        };
        (held, served)
    }
}

/// `message` as a line of the Connection.
fn line_of(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// The page a played browser drives, the browser, and the future of the
/// connection's driver and the page's reader, which the test polls beside
/// its own work.
pub(crate) fn played_page() -> (PageDriver, Played, Served) {
    let ((connection, driver), played) = replay::connected();
    let page = PageDriver::played(connection);
    let reading = page.reading(None);
    let served: Served = Box::pin(async move {
        let _ = tokio::join!(driver.run(), reading);
    });
    (page, played, served)
}

/// `test.hold_run`: starts a Run, hands it out with the call's event
/// stream, and answers once the test let it go, the Run stopped.
async fn hold_run(
    holding: &Mutex<Option<Holding>>,
    context: &CallContext,
) -> Result<Value, CallError> {
    let (handed, released) = holding
        .lock()
        .await
        .take()
        .expect("invariant: the test operation is called once");
    let started = context
        .runs()
        .start(Start {
            run_id: None,
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
            detached: false,
        })
        .await
        .map_err(goodfetch_engine::run::start_error)?;
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    let _ = handed.send((
        run.clone(),
        context.events().clone(),
        context.client().clone(),
    ));
    let _ = released.await;
    run.cancellation().cancel();
    Ok(json!({}))
}

/// The engine's own operations and the test's.
fn engine(holding: Arc<Mutex<Option<Holding>>>) -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(Contribution::new("test").with_handler(Handler::new(
            &HOLD_RUN,
            move |_: Value, context: CallContext| {
                let holding = Arc::clone(&holding);
                async move { hold_run(&holding, &context).await }
            },
        )));
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
