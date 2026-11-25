//! B3's message scenario (ADR 0140's E1), shared by its counts cell
//! (`examples/counts.rs`), its Callgrind and DHAT targets and its test: a
//! Client that says `hello`, subscribes to events and then sends a fixed
//! traffic over the in-memory network, on the tests' paused current-thread
//! runtime, to an engine whose stand-in handlers do only what the protocol
//! asks of them. Each outcome the path has is in it: small params
//! (`engine.info`), large params (a `route.fulfill` body), and a Plugin's
//! messages and events: a call whose handler sends the program a handler
//! request (`plugin_record.handle`), reads a large Record from its reply,
//! publishes an event and logs a line. The Plugin's entries are stand-ins
//! of this file, shaped as a crawl's are: the engine names none of
//! Foxhound's (ADR 0099). A body stored in a Run and read back by
//! `page.body` under the Client's credits sends its `$/chunk`s too, and
//! `run.finish` ends its Run while the Client still reads its events. The
//! scenario runs on either carriage (ADR 0049): an Attached engine's
//! stdio, or a WebSocket to an Exposed Daemon, through the same `rpc`
//! dispatch. Its Valgrind targets' scenarios dispatch [`MESSAGES`]
//! messages each beyond [`NO_MESSAGES`], which `xtask counts` divides by.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::{DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::endpoint::Authorities;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::log::LogEvents;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::ops::Ops;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::{EndedBy, Start, Started};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{
    self, ConnectionReader, ConnectionWriter, InMemoryDialer, InMemoryNetwork, Network,
};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Guard,
    Handler, Identity, Params,
};
use goodfetch_protocol::catalog::{
    HandlerRequest, Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach,
    Stability, schema_of,
};
use goodfetch_protocol::checkpoint::RUN_CHECKPOINTING;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::route::{ROUTE_FULFILL, RouteAnswered, RouteFulfillParams};
use goodfetch_protocol::page::{BodyHandle, CHUNK_BYTES};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Join, Lines};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tracing_subscriber::layer::SubscriberExt as _;

/// The bytes of a large params' body and of a large Record's text.
const LARGE_BYTES: usize = 64 * 1024;

/// The bytes of a body a Run stores, which `page.body` sends in
/// [`BODY_CHUNKS`] `$/chunk`s, the last of one byte.
const BODY_BYTES: usize = 2 * CHUNK_BYTES + 1;

/// The `$/chunk`s that carry one stored body.
pub(crate) const BODY_CHUNKS: usize = BODY_BYTES.div_ceil(CHUNK_BYTES);

/// The address an Exposed Daemon's listener names, on the in-memory
/// network.
const ADDRESS: &str = "127.0.0.1:7900";

/// The Daemon's per-launch token, which a `hello` over WebSocket carries.
const TOKEN: &str = "b3-launch-token";

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
    bodies: 0,
};

/// [`MESSAGES`] messages of small params.
pub(crate) const SMALL_PARAMS: Traffic = Traffic {
    small: MESSAGES,
    large: 0,
    plugin: 0,
    bodies: 0,
};

/// [`MESSAGES`] messages of large params: half of them calls with a large
/// body, half a Plugin's calls and replies with a large Record.
pub(crate) const LARGE_PARAMS: Traffic = Traffic {
    small: 0,
    large: MESSAGES / 2,
    plugin: MESSAGES / 4,
    bodies: 0,
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
    /// Bodies, each stored in a Run of its own by a call, then read by
    /// `page.body` under one `$/credit` for its [`BODY_CHUNKS`] chunks, and
    /// its Run ended by `run.finish`: four messages in each.
    pub(crate) bodies: usize,
}

impl Traffic {
    /// What opens a run: `hello`, `events.subscribe` and `$/credit`.
    const OPENING: usize = 3;

    /// What ends it: the `$/cancel` of the subscription.
    const CLOSING: usize = 1;

    /// The messages the Client sends, what opens and ends the run included.
    pub(crate) const fn incoming(self) -> usize {
        Self::OPENING + self.small + self.large + 2 * self.plugin + 4 * self.bodies + Self::CLOSING
    }

    /// The requests among them: `hello`, `events.subscribe` and each call,
    /// every one with params; a `$/credit` and a reply to a handler request
    /// are none.
    pub(crate) const fn requests(self) -> usize {
        2 + self.small + self.large + self.plugin + 3 * self.bodies
    }

    /// The answers the Client waits for before it ends the subscription:
    /// `hello`'s and each call's; `events.subscribe` answers only once its
    /// subscription ends.
    const fn calls_answered(self) -> usize {
        self.requests() - 1
    }

    /// The ID of the call that stores the first body: the calls before it
    /// count up from `hello`'s 0. Each body's `page.body` call takes the ID
    /// [`Self::bodies`] past its store's.
    const fn first_body(self) -> usize {
        2 + self.small + self.large + self.plugin
    }
}

/// The carriage a Connection's messages travel on (ADR 0049).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Carriage {
    /// An Attached engine's stdio: one JSON-RPC message a line.
    Stdio,
    /// A WebSocket to an Exposed Daemon: one JSON-RPC message a WebSocket
    /// message, its `hello` carrying the Daemon's token.
    WebSocket,
}

/// Whether the Connection asks for log events, the feature on this path that
/// can be switched off: `hello`'s `log_level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Logs {
    On,
    Off,
}

/// One extra unit of work a call does, given its params.
pub(crate) type Plant = Arc<dyn Fn(Params<'_>) + Send + Sync>;

/// What a Valgrind target's run of the scenario reads, made outside what it
/// measures.
pub(crate) struct Input {
    traffic: Traffic,
    logs: Logs,
    carriage: Carriage,
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
pub(crate) fn input(traffic: Traffic, logs: Logs, carriage: Carriage) -> Input {
    let plant = std::env::var_os(PLANT_VARIABLE).map(|_| {
        let document = json!({ "text": "x".repeat(PLANT_BYTES) });
        Arc::new(move |params: Params<'_>| {
            let copied: Value =
                serde_json::from_str(params.json()).expect("invariant: a call's params are JSON");
            drop(serde_json::to_vec(&copied));
            drop(serde_json::to_vec(&document));
        }) as Plant
    });
    let warming = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    run(
        NO_MESSAGES,
        logs,
        carriage,
        Metrics::none(),
        plant.clone(),
        warming.path(),
    );
    drop(warming);
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    Input {
        traffic,
        logs,
        carriage,
        plant,
        home,
    }
}

/// The Valgrind targets' run: answers how many messages left the engine.
pub(crate) fn dispatched(input: &Input) -> usize {
    run(
        input.traffic,
        input.logs,
        input.carriage,
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

impl Sent {
    /// The answers and events among them, each its own encode: every
    /// message but the `$/events` batches, which copy the events they
    /// carry, encoded as they were published (ADR 0140's E1).
    pub(crate) fn answers_and_events(&self) -> usize {
        self.outgoing - self.event_batches + self.events
    }
}

/// How many events the `$/events` batch `line` carries.
fn carried(line: &str) -> usize {
    let batch: Value = serde_json::from_str(line).expect("invariant: the engine sends JSON lines");
    batch["params"]["events"]
        .as_array()
        .expect("invariant: a `$/events` batch carries its events")
        .len()
}

/// What a run of the scenario saw leave the engine.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sent {
    /// Every message: answers, handler requests, event batches, log events.
    pub(crate) outgoing: usize,
    /// The `$/events` batches among them.
    pub(crate) event_batches: usize,
    /// The events those batches carry.
    pub(crate) events: usize,
    /// The `$/log` lines among them.
    pub(crate) log_lines: usize,
    /// The handler requests among them.
    pub(crate) handler_requests: usize,
    /// The `$/chunk`s among them.
    pub(crate) chunks: usize,
}

/// The Plugin's call, which starts its work as a crawl's start does.
const PLUGIN_CALL: Operation = Operation {
    name: "plugin.call",
    description: "Asks the stand-in Plugin to handle one attempt.",
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
    arrived_in: PROTOCOL_VERSION,
};

/// The handler request the Plugin's call sends the program.
const PLUGIN_HANDLE: HandlerRequest = HandlerRequest {
    name: "plugin_record.handle",
    description: "Asks the program's handler to process one attempt.",
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// What the program's handler answers: the Records it pushed, or its skip.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Handled {
    #[serde(default)]
    records: Vec<Pushed>,
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "a handler's answer has the field, which no scenario sets"
    )]
    skipped: Option<Value>,
}

/// One Record a handler pushed into a Dataset.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Pushed {
    #[expect(
        dead_code,
        reason = "read off the wire, as a pushed Record's Dataset is"
    )]
    dataset: String,
    #[expect(dead_code, reason = "read off the wire, as a pushed Record's data is")]
    data: Value,
}

/// A call of a Plugin: its handler request, the Record its reply carries,
/// its event and its log line.
async fn plugin_call(params: Value, context: CallContext) -> Result<Value, CallError> {
    let failed = |error: goodfetch_engine::client::ClientRequestError| {
        CallError::protocol(ProtocolFailure::InternalError, error.to_string())
    };
    let handled: Handled = context
        .client()
        .send(&PLUGIN_HANDLE, &params)
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

/// The Plugin's call that keeps a page body, as a crawl's fetch does: it
/// starts a Run and stores the body in it.
const PLUGIN_STORE_BODY: Operation = Operation {
    name: "plugin.store_body",
    description: "Asks the stand-in Plugin to keep one page body in a Run of its own.",
    ..PLUGIN_CALL
};

/// What the call that stores a body takes: the Run it starts.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreBody {
    run_id: RunId,
}

/// Starts the Run `run_id` and stores [`BODY_BYTES`] in it, answering the
/// body's handle, which `page.body` reads.
async fn store_body(params: StoreBody, context: CallContext) -> Result<BodyHandle, CallError> {
    let failed = |error: String| CallError::protocol(ProtocolFailure::InternalError, error);
    let started = context
        .runs()
        .start(Start {
            run_id: Some(params.run_id),
            tags: Tags::default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: std::collections::BTreeMap::new(),
            ended_by: EndedBy::Finish,
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
        .map_err(|error| failed(error.to_string()))?;
    let Started::New(run) = started else {
        return Err(failed("the body's Run began before".to_owned()));
    };
    run.store_body(vec![b'x'; BODY_BYTES].into())
        .await
        .map_err(|error| failed(error.to_string()))
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
        .with_handler(Handler::new(&PLUGIN_CALL, plugin_call))
        .with_handler(Handler::new(&PLUGIN_STORE_BODY, store_body));
    let engine = goodfetch_engine::contribution();
    match plant {
        None => Composition::default().with(engine).with(stand_ins),
        Some(plant) => {
            let guard: Guard = Arc::new(move |_operation, params, _read, _context| {
                plant(params);
                None
            });
            Composition::default()
                .with(engine.guarded(&guard))
                .with(stand_ins.guarded(&guard))
        }
    }
}

/// The lines a run opens with and those its traffic sends unprompted, the
/// `hello` over WebSocket with the Daemon's token.
fn request_lines(traffic: Traffic, logs: Logs, carriage: Carriage) -> Vec<u8> {
    let log_level = match logs {
        Logs::On => "info",
        Logs::Off => "off",
    };
    let mut hello = json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": {
        "protocol_version": "0.1", "surface": "rust", "needs": "collie",
        "product_version": "0.0.0", "log_level": log_level,
    } });
    if carriage == Carriage::WebSocket {
        hello["params"]["token"] = json!(TOKEN);
    }
    let mut lines = format!(
        "{}\n{}\n{}\n",
        hello,
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
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"plugin.call\",\"params\":{{\"attempt\":1}}}}\n",
                next()
            )
            .as_bytes(),
        );
    }
    for body in 0..traffic.bodies {
        let id = next();
        assert_eq!(
            id,
            traffic.first_body() + body,
            "invariant: the bodies' calls come last, where `body_lines` looks for their answers"
        );
        lines.extend_from_slice(
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"plugin.store_body\",\"params\":{{\"run_id\":\"body-{body}\"}}}}\n"
            )
            .as_bytes(),
        );
    }
    lines
}

/// What ends a run: the `$/cancel` of the subscription, which answers then.
const CANCEL_SUBSCRIPTION: &str =
    "{\"jsonrpc\":\"2.0\",\"method\":\"$/cancel\",\"params\":{\"id\":1}}\n";

/// What the Client sends once the call `id` answered `result`: when it
/// stored a body, the `page.body` call that reads it and the `$/credit`
/// for its chunks; when it read one, the `run.finish` of the body's Run.
fn body_lines(traffic: Traffic, id: usize, result: &Value) -> Option<Vec<u8>> {
    let first = traffic.first_body();
    let next = id + traffic.bodies;
    if (first..first + traffic.bodies).contains(&id) {
        return Some(
            format!(
                "{}\n{}\n",
                json!({ "jsonrpc": "2.0", "id": next, "method": "page.body", "params": {
                    "run_id": result["run_id"], "body": result["body"],
                } }),
                json!({ "jsonrpc": "2.0", "method": "$/credit", "params": {
                    "id": next, "credits": BODY_CHUNKS,
                } }),
            )
            .into_bytes(),
        );
    }
    let read = first + traffic.bodies;
    (read..read + traffic.bodies).contains(&id).then(|| {
        format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "id": next, "method": "run.finish", "params": {
                "run_id": format!("body-{}", id - read),
            } }),
        )
        .into_bytes()
    })
}

/// The reply to the handler request `id`: a large Record.
fn handled_line(id: usize, text: &str) -> Vec<u8> {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"records\":[{{\"dataset\":\"default\",\"data\":{{\"url\":\"https://example.test/\",\"text\":\"{text}\"}}}}]}}}}\n"
    )
    .into_bytes()
}

/// A WebSocket's Client end over the in-memory network.
type ClientSocket = WebSocketStream<Join<ConnectionReader, ConnectionWriter>>;

/// The Client's sending side on its carriage.
enum Sending {
    Lines(ConnectionWriter),
    Messages(SplitSink<ClientSocket, Message>),
}

impl Sending {
    /// Sends `lines`, each ended by a newline: as they are, or each as one
    /// WebSocket message, flushed once all are in.
    async fn send(&mut self, lines: &[u8]) {
        match self {
            Self::Lines(writer) => writer.write_all(lines).await.expect("invariant: in memory"),
            Self::Messages(sink) => {
                for line in lines.split(|byte| *byte == b'\n') {
                    if line.is_empty() {
                        continue;
                    }
                    let text = std::str::from_utf8(line)
                        .expect("invariant: the Client's lines are JSON text");
                    sink.feed(Message::text(text.to_owned()))
                        .await
                        .expect("invariant: in memory");
                }
                sink.flush().await.expect("invariant: in memory");
            }
        }
    }

    /// Shuts the sending side down, or closes the WebSocket.
    async fn close(self) {
        match self {
            Self::Lines(mut writer) => writer.shutdown().await.expect("invariant: in memory"),
            Self::Messages(mut sink) => sink.close().await.expect("invariant: in memory"),
        }
    }
}

/// The Client's receiving side on its carriage.
enum Receiving {
    Lines(Lines<BufReader<ConnectionReader>>),
    Messages(SplitStream<ClientSocket>),
}

/// One message the engine sent, as it arrived.
enum Received {
    Line(String),
    Message(Utf8Bytes),
}

impl Received {
    fn text(&self) -> &str {
        match self {
            Self::Line(line) => line,
            Self::Message(text) => text.as_str(),
        }
    }
}

impl Receiving {
    /// The next message, or none once the engine closed its side.
    async fn next(&mut self) -> Option<Received> {
        match self {
            Self::Lines(lines) => lines.next_line().await.ok().flatten().map(Received::Line),
            Self::Messages(stream) => loop {
                match stream.next().await? {
                    Ok(Message::Text(text)) => return Some(Received::Message(text)),
                    Ok(Message::Close(_)) | Err(_) => return None,
                    Ok(_) => {}
                }
            },
        }
    }
}

/// Opens the Client's Connection on `carriage`: over WebSocket, by a
/// handshake from the Daemon's own origin.
async fn connect(dialer: &InMemoryDialer, carriage: Carriage) -> (Sending, Receiving) {
    let (reader, writer) = dialer
        .connect()
        .await
        .expect("invariant: the engine listens")
        .split();
    match carriage {
        Carriage::Stdio => (
            Sending::Lines(writer),
            Receiving::Lines(BufReader::new(reader).lines()),
        ),
        Carriage::WebSocket => {
            let request = Request::builder()
                .uri(format!("ws://{ADDRESS}/"))
                .header("Host", ADDRESS)
                .header("Origin", format!("http://{ADDRESS}"))
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", generate_key())
                .body(())
                .expect("invariant: a literal handshake");
            let (socket, _) =
                tokio_tungstenite::client_async(request, tokio::io::join(reader, writer))
                    .await
                    .expect("invariant: the Daemon accepts its own origin");
            let (sink, stream) = socket.split();
            (Sending::Messages(sink), Receiving::Messages(stream))
        }
    }
}

/// The Client: sends `lines` on `carriage`, answers each handler request,
/// reads each stored body and finishes its Run, ends the subscription once
/// each of `traffic`'s calls was answered, closes once that was answered too
/// and reads to the end.
async fn client(
    dialer: InMemoryDialer,
    carriage: Carriage,
    traffic: Traffic,
    lines: &[u8],
) -> Sent {
    let (mut sending, mut receiving) = connect(&dialer, carriage).await;
    let (replies, mut to_send) = mpsc::unbounded_channel::<Vec<u8>>();
    let writing = async {
        sending.send(lines).await;
        while let Some(line) = to_send.recv().await {
            sending.send(&line).await;
        }
        sending.close().await;
    };
    let receive = async {
        let text = "x".repeat(LARGE_BYTES);
        let mut replies = Some(replies);
        let reply = |replies: &Option<mpsc::UnboundedSender<Vec<u8>>>, line: Vec<u8>| {
            replies
                .as_ref()
                .expect("invariant: a call in flight keeps the Client open")
                .send(line)
                .expect("invariant: the sending side reads until this one ends");
        };
        let (mut answered, mut followed) = (0, 0);
        let mut sent = Sent {
            outgoing: 0,
            event_batches: 0,
            events: 0,
            log_lines: 0,
            handler_requests: 0,
            chunks: 0,
        };
        while let Some(received) = receiving.next().await {
            let line = received.text();
            sent.outgoing += 1;
            let start = line.get(..line.len().min(64)).unwrap_or(line);
            if start.contains(r#""method":"$/events""#) {
                sent.event_batches += 1;
                sent.events += carried(line);
            } else if start.contains(r#""method":"$/log""#) {
                sent.log_lines += 1;
            } else if start.contains(r#""method":"plugin_record.handle""#) {
                reply(&replies, handled_line(sent.handler_requests, &text));
                sent.handler_requests += 1;
            // Only a run that stores bodies looks for their chunks, so the
            // Valgrind targets' Client does what it did before them.
            } else if traffic.bodies > 0 && start.contains(r#""method":"$/chunk""#) {
                sent.chunks += 1;
            } else {
                // Only a run that stores bodies reads its answers, until
                // each body was stored and read.
                if followed < 2 * traffic.bodies {
                    let answer: Value =
                        serde_json::from_str(line).expect("invariant: the engine sends JSON");
                    let id = answer["id"]
                        .as_u64()
                        .and_then(|id| usize::try_from(id).ok());
                    if let Some(next) = id.and_then(|id| body_lines(traffic, id, &answer["result"]))
                    {
                        followed += 1;
                        reply(&replies, next);
                    }
                }
                answered += 1;
                // Each call answered, the Client ends the subscription and
                // closes once it answered: closed first, a WebSocket would
                // drop that answer, encoded all the same.
                if answered == traffic.calls_answered() {
                    reply(&replies, CANCEL_SUBSCRIPTION.as_bytes().to_vec());
                } else if answered > traffic.calls_answered() {
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
    carriage: Carriage,
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
    let lines = request_lines(traffic, logs, carriage);
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
            tokio::join!(
                serve(&engine, carriage, network, dependencies),
                client(dialer, carriage, traffic, &lines)
            )
            .1
        })
    })
}

/// The Daemon's Succession: it is its major's pick, so it never hands over.
struct Pick;

impl Succession for Pick {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

/// Serves the Client's one Connection on `carriage`: as an Attached engine
/// over stdio, or as an Exposed Daemon at [`ADDRESS`], which stops idle a
/// second after the Client left.
async fn serve(
    engine: &Engine,
    carriage: Carriage,
    network: InMemoryNetwork,
    dependencies: Dependencies,
) {
    let network = Network::InMemory(network);
    match carriage {
        Carriage::Stdio => engine
            .run_attached(network, dependencies)
            .await
            .expect("invariant: the engine got its Connection"),
        Carriage::WebSocket => {
            let address: SocketAddr = ADDRESS.parse().expect("invariant: a literal address");
            let (listener, _) = network
                .listen(Path::new("b3.sock"))
                .expect("invariant: an in-memory listener");
            let options = DaemonOptions {
                token: TOKEN.to_owned(),
                endpoint: DaemonEndpoint::Tcp(address),
                pid: 1,
                idle_timeout: Duration::from_secs(1),
                keep_alive: false,
                holds: Holds::new(),
                succession: Arc::new(Pick),
                drain: Drain {
                    asked: None,
                    grace: Duration::from_secs(1),
                },
                install: None,
                ops: Ops::default(),
                websocket: Some(Authorities::loopback(address)),
                live_view: None,
            };
            let end = engine.run_daemon(listener, dependencies, options).await;
            assert!(
                matches!(end, DaemonEnd::Idle),
                "the Daemon stopped idle, not {end:?}"
            );
        }
    }
}
