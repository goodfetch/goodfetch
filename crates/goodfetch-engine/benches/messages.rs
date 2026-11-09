//! B3's message scenario (ADR 0140's E1), shared by its counts cell
//! (`examples/counts.rs`), its Callgrind and DHAT targets and its test: a
//! Client that says `hello`, subscribes to events and then sends a fixed
//! traffic over the in-memory network, on the tests' paused current-thread
//! runtime, to an engine whose stand-in handlers do only what the protocol
//! asks of them. Each outcome the path has is in it: small params
//! (`engine.info`), large params (a `route.fulfill` body), and a Plugin's
//! messages and events: a call whose handler sends the program a handler
//! request (`crawl_request.handle`), reads a large Record from its reply,
//! publishes an event and logs a line. Its Valgrind targets' scenarios
//! dispatch [`MESSAGES`] messages each beyond [`NO_MESSAGES`], which `xtask
//! counts` divides by.

use std::path::Path;
use std::sync::Arc;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::log::LogEvents;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, InMemoryDialer, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Guard,
    Handler, Identity,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::RUN_CHECKPOINTING;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::foxhound::crawl::{CRAWL_REQUEST_HANDLE, CRAWL_START, CrawlRequestHandled};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::route::{ROUTE_FULFILL, RouteAnswered, RouteFulfillParams};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;
use tracing_subscriber::layer::SubscriberExt as _;

/// The bytes of a large params' body and of a large Record's text.
const LARGE_BYTES: usize = 64 * 1024;

/// The bytes a Valgrind target's plant encodes beside the call's params.
const PLANT_BYTES: usize = 8 * 1024;

/// The messages each of the Valgrind targets' scenarios sends beyond what
/// opens it.
pub(crate) const MESSAGES: usize = 100;

/// The variable the Valgrind targets read a plant from: any value plants.
pub(crate) const PLANT_VARIABLE: &str = "GOODFETCH_COUNTS_PLANT";

/// Only what opens a run.
pub(crate) const NO_MESSAGES: Traffic = Traffic {
    small: 0,
    large: 0,
    plugin: 0,
};

/// [`MESSAGES`] messages of small params.
pub(crate) const SMALL_PARAMS: Traffic = Traffic {
    small: MESSAGES,
    large: 0,
    plugin: 0,
};

/// [`MESSAGES`] messages of large params: half of them calls with a large
/// body, half a Plugin's calls and replies with a large Record.
pub(crate) const LARGE_PARAMS: Traffic = Traffic {
    small: 0,
    large: MESSAGES / 2,
    plugin: MESSAGES / 4,
};

/// The messages a Client sends behind what opens every run of the scenario.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Traffic {
    /// `engine.info` calls.
    pub(crate) small: usize,
    /// `route.fulfill` calls with a large body.
    pub(crate) large: usize,
    /// Calls that each send a handler request, read its reply with a large
    /// Record, publish an event and log a line: two messages in each.
    pub(crate) plugin: usize,
}

impl Traffic {
    /// What opens a run: `hello`, `events.subscribe` and `$/credit`.
    const OPENING: usize = 3;

    /// The messages the Client sends, what opens the run included.
    pub(crate) const fn incoming(self) -> usize {
        Self::OPENING + self.small + self.large + 2 * self.plugin
    }
}

/// Whether the Connection asks for log events, the feature on this path that
/// can be switched off: `hello`'s `log_level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Logs {
    On,
    Off,
}

/// One extra unit of work a call does, given its params.
pub(crate) type Plant = Arc<dyn Fn(&Value) + Send + Sync>;

/// What a Valgrind target's run of the scenario reads, made outside what it
/// measures.
pub(crate) struct Input {
    traffic: Traffic,
    logs: Logs,
    plant: Option<Plant>,
    home: TempDir,
}

/// The Valgrind targets' setup: their home, and their plant when
/// [`PLANT_VARIABLE`] asks for it: the call's params copied and encoded and a
/// few KiB encoded, enough to pass the 0.5 % a figure may vary by.
///
/// It first runs what opens every run, on a home of its own, so the work
/// done once per process stays out of what each scenario measures, alike in
/// every scenario, and cancels in the difference `xtask counts` takes:
/// - the sandbox probe's fork (`sandbox_probe`'s `this_host`): the child's
///   profile carries every count the parent made before the fork, which
///   happens later in a run with more messages, so each scenario counted
///   its own share of the run twice (about 11,550 instructions and 18
///   allocations per small message);
/// - where the one-time zeroed buffers land (the Connection's read buffer,
///   the Client's): on pages fresh from the OS the allocator skips their
///   zeroing, and one run of a pair skipping it while the other paid it
///   moved the figure per message by about 330 instructions.
pub(crate) fn input(traffic: Traffic, logs: Logs) -> Input {
    let plant = std::env::var_os(PLANT_VARIABLE).map(|_| {
        let document = json!({ "text": "x".repeat(PLANT_BYTES) });
        Arc::new(move |params: &Value| {
            let copied = params.clone();
            drop(serde_json::to_vec(&copied));
            drop(serde_json::to_vec(&document));
        }) as Plant
    });
    let warming = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    run(
        NO_MESSAGES,
        logs,
        Metrics::none(),
        plant.clone(),
        warming.path(),
    );
    drop(warming);
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    Input {
        traffic,
        logs,
        plant,
        home,
    }
}

/// The Valgrind targets' run: answers how many messages left the engine.
pub(crate) fn dispatched(input: &Input) -> usize {
    run(
        input.traffic,
        input.logs,
        Metrics::none(),
        input.plant.clone(),
        input.home.path(),
    )
    .outgoing
}

/// The Valgrind targets' teardown: removes the home.
pub(crate) fn done((outgoing, input): (usize, Input)) -> usize {
    drop(input);
    outgoing
}

/// What a run of the scenario saw leave the engine.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sent {
    /// Every message: answers, handler requests, event batches, log events.
    pub(crate) outgoing: usize,
    /// The `$/events` batches among them.
    pub(crate) event_batches: usize,
    /// The `$/log` lines among them.
    pub(crate) log_lines: usize,
    /// The handler requests among them.
    pub(crate) handler_requests: usize,
}

/// A call of a Plugin: its handler request, the Record its reply carries,
/// its event and its log line.
async fn plugin_call(params: Value, context: CallContext) -> Result<Value, CallError> {
    let failed = |error: goodfetch_engine::client::ClientRequestError| {
        CallError::protocol(ProtocolFailure::InternalError, error.to_string())
    };
    let handled: CrawlRequestHandled = context
        .client()
        .send(&CRAWL_REQUEST_HANDLE, &params)
        .await
        .map_err(failed)?
        .answer()
        .await
        .map_err(failed)?;
    let records = handled.records.len();
    context
        .events()
        .publish(&RUN_CHECKPOINTING, None, &json!({ "records": records }))
        .await;
    tracing::info!(records, "a Plugin's call was handled");
    Ok(json!({ "records": records }))
}

/// `route.fulfill`, its params read and nothing reaching a browser.
async fn fulfill(
    _params: RouteFulfillParams,
    _context: CallContext,
) -> Result<RouteAnswered, CallError> {
    Ok(RouteAnswered {})
}

/// The engine's own operations and the stand-ins, each call doing `plant`
/// first.
fn composition(plant: Option<Plant>) -> Composition {
    let stand_ins = Contribution::new("b3-stand-ins")
        .with_handler(Handler::new(&ROUTE_FULFILL, fulfill))
        .with_handler(Handler::new(&CRAWL_START, plugin_call));
    let engine = goodfetch_engine::contribution();
    match plant {
        None => Composition::default().with(engine).with(stand_ins),
        Some(plant) => {
            let guard: Guard = Arc::new(move |_operation, params, _context| {
                plant(params);
                None
            });
            Composition::default()
                .with(engine.guarded(&guard))
                .with(stand_ins.guarded(&guard))
        }
    }
}

/// The lines a run opens with and those its traffic sends unprompted.
fn request_lines(traffic: Traffic, logs: Logs) -> Vec<u8> {
    let log_level = match logs {
        Logs::On => "info",
        Logs::Off => "off",
    };
    let mut lines = format!(
        "{}\n{}\n{}\n",
        json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": {
            "protocol_version": "0.1", "surface": "rust", "needs": "collie",
            "product_version": "0.0.0", "log_level": log_level,
        } }),
        r#"{"jsonrpc":"2.0","id":1,"method":"events.subscribe","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"$/credit","params":{"id":1,"credits":100000}}"#,
    )
    .into_bytes();
    let mut id = 1;
    let mut next = || {
        id += 1;
        id
    };
    for _ in 0..traffic.small {
        lines.extend_from_slice(
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"engine.info\",\"params\":{{}}}}\n",
                next()
            )
            .as_bytes(),
        );
    }
    let body = "QUJD".repeat(LARGE_BYTES / 4);
    for _ in 0..traffic.large {
        lines.extend_from_slice(
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"route.fulfill\",\"params\":{{\"route_id\":\"route-1\",\"status\":200,\"body\":\"{body}\"}}}}\n",
                next()
            )
            .as_bytes(),
        );
    }
    for _ in 0..traffic.plugin {
        lines.extend_from_slice(
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"crawl.start\",\"params\":{{\"attempt\":1}}}}\n",
                next()
            )
            .as_bytes(),
        );
    }
    lines
}

/// The reply to the handler request `id`: a large Record.
fn handled_line(id: usize, text: &str) -> Vec<u8> {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"records\":[{{\"dataset\":\"default\",\"data\":{{\"url\":\"https://example.test/\",\"text\":\"{text}\"}}}}]}}}}\n"
    )
    .into_bytes()
}

/// The Client: sends `lines`, answers each handler request, closes once each
/// of `answers` calls was answered and reads to the end.
async fn client(dialer: InMemoryDialer, lines: &[u8], answers: usize) -> Sent {
    let (reader, mut writer) = dialer
        .connect()
        .await
        .expect("invariant: the engine listens")
        .split();
    let (replies, mut to_send) = mpsc::unbounded_channel::<Vec<u8>>();
    let writing = async {
        writer.write_all(lines).await.expect("invariant: in memory");
        while let Some(line) = to_send.recv().await {
            writer.write_all(&line).await.expect("invariant: in memory");
        }
        writer.shutdown().await.expect("invariant: in memory");
    };
    let receive = async {
        let text = "x".repeat(LARGE_BYTES);
        let mut replies = Some(replies);
        let mut answered = 0;
        let mut sent = Sent {
            outgoing: 0,
            event_batches: 0,
            log_lines: 0,
            handler_requests: 0,
        };
        let mut read = BufReader::new(reader).lines();
        while let Ok(Some(line)) = read.next_line().await {
            sent.outgoing += 1;
            let start = line.get(..line.len().min(64)).unwrap_or(&line);
            if start.contains(r#""method":"$/events""#) {
                sent.event_batches += 1;
            } else if start.contains(r#""method":"$/log""#) {
                sent.log_lines += 1;
            } else if start.contains(r#""method":"crawl_request.handle""#) {
                let reply = handled_line(sent.handler_requests, &text);
                sent.handler_requests += 1;
                replies
                    .as_ref()
                    .expect("invariant: a call in flight keeps the Client open")
                    .send(reply)
                    .expect("invariant: the sending side reads until this one ends");
            } else {
                answered += 1;
                if answered == answers {
                    replies = None;
                }
            }
        }
        drop(replies);
        sent
    };
    tokio::join!(writing, receive).1
}

/// Runs `traffic` on an engine counting on `metrics` and answers what left it.
pub(crate) fn run(
    traffic: Traffic,
    logs: Logs,
    metrics: Metrics,
    plant: Option<Plant>,
    home: &Path,
) -> Sent {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let log_events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(log_events.layer());
    let engine = Engine::new(identity, &composition(plant))
        .expect("invariant: one handler each")
        .with_metrics(metrics)
        .with_log_events(log_events);
    let lines = request_lines(traffic, logs);
    // `hello` and `events.subscribe` are answered like any other call.
    let answers = 1 + traffic.small + traffic.large + traffic.plugin;
    tracing::subscriber::with_default(subscriber, || {
        goodfetch_test_support::runtime().block_on(async {
            let (network, dialer) = transport::in_memory();
            let dependencies = Dependencies {
                environment: std::collections::BTreeMap::new(),
                keychain: goodfetch_engine::secrets::Keychain::absent(),
                home: Home::new(home.to_owned()),
                clock: Clock::paused(std::time::UNIX_EPOCH),
                entropy: Entropy::seeded(0),
                blocking: BlockingPolicy::Inline,
                run_state_location: RunStateLocation::PerMajor,
                run_state: RunStateOptions::default(),
                politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
            };
            let (served, what_left) = tokio::join!(
                engine.run_attached(Network::InMemory(network), dependencies),
                client(dialer, &lines, answers)
            );
            served.expect("invariant: the engine got its Connection");
            what_left
        })
    })
}
