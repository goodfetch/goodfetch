//! B2 (ADR 0140): a Collie Session the whole engine drives, through the
//! protocol, against a scripted browser that answers each command at once. The
//! engine's Borrowed browser path reaches it, so every line of a Session's
//! code runs, its CDP connection and its command log included, with no
//! test-only way in: the connection runs over the in-memory pipe that
//! `check_and_arm` uses, which the engine bridges to the browser's WebSocket
//! on loopback.
//!
//! Shared by the counts cell (`examples/counts.rs`) and its test
//! (`tests/it/session_counts.rs`). The browser counts each command by its ID,
//! from outside the connection, so a command's unit does not depend on what
//! the connection counts: the same command written twice is one unit and two
//! sends. Every count but one repeats exactly. The exception is `C1.file_opens`:
//! the command log's writer opens per batch, and how many commands a batch holds
//! follows the socket's timing, so its figure moves below the 2 per command
//! that one batch per command would cost, which is its limit.
//!
//! Each new outcome of a family's rows (a read, an action, a request) is a
//! step of [`program`]'s and, where it needs one, an answer of [`answered`].

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

/// One extra unit of a row's work, done once per command the browser reads.
pub(crate) type Plant = Arc<dyn Fn() + Send + Sync>;

/// A plant of one extra unit of `row` on `work`: the work of a row with no
/// counted function a benchmark could call, so counted at the seam that counts.
pub(crate) fn counting(work: &CountedWork, row: Row) -> Plant {
    let work = work.clone();
    Arc::new(move || work.count(row))
}

/// The Run the scenario starts.
const RUN: &str = "session-counts";

/// The browser's one page's session, target and frame.
const SESSION: &str = "page-session";
const TARGET: &str = "page-target";
const FRAME: &str = TARGET;

/// How long the counts must hold still for the Session's work to have ended:
/// what the engine sends on its own after a call answered.
const QUIET: Duration = Duration::from_millis(100);

/// The pages the scenario navigates to, a navigation in the middle of a
/// Session's life. Literal addresses with no network: the browser answers.
const PAGES: [&str; 2] = ["http://192.0.2.1/first", "http://192.0.2.1/second"];

/// What one run of the scenario counted.
pub(crate) struct Counted {
    /// The commands the browser read, each once.
    pub(crate) commands: u64,
    /// Its Counted work.
    pub(crate) work: CountedWork,
}

/// The `result` of `method` as the browser answers it, and the events it
/// sends right after.
fn answered(method: &str, params: &Value) -> (Value, Vec<Value>) {
    match method {
        "Browser.getVersion" => (
            json!({
                "protocolVersion": "1.3",
                "product": "Scripted/1.0",
                "revision": "0",
                "userAgent": "Scripted",
                "jsVersion": "0",
            }),
            Vec::new(),
        ),
        "Target.getBrowserContexts" => (
            json!({ "browserContextIds": [], "defaultBrowserContextId": "context" }),
            Vec::new(),
        ),
        "Target.createBrowserContext" => (json!({ "browserContextId": "context" }), Vec::new()),
        "Target.createTarget" => (json!({ "targetId": TARGET }), Vec::new()),
        "Target.attachToTarget" => (json!({ "sessionId": SESSION }), Vec::new()),
        "Page.getFrameTree" => (
            json!({ "frameTree": { "frame": {
                "id": FRAME,
                "loaderId": "loader",
                "url": "about:blank",
                "domainAndRegistry": "",
                "securityOrigin": "null",
                "mimeType": "text/html",
                "adFrameStatus": { "adFrameType": "none" },
                "secureContextType": "InsecureScheme",
                "crossOriginIsolatedContextType": "NotIsolated",
                "gatedAPIFeatures": [],
            } } }),
            Vec::new(),
        ),
        "Page.createIsolatedWorld" => (json!({ "executionContextId": 7 }), Vec::new()),
        "Page.addScriptToEvaluateOnNewDocument" => (json!({ "identifier": "script" }), Vec::new()),
        "Runtime.evaluate" | "Runtime.callFunctionOn" => {
            (json!({ "result": { "type": "undefined" } }), Vec::new())
        }
        "Page.navigate" => {
            let url = params["url"].as_str().unwrap_or_default();
            (json!({ "frameId": FRAME, "loaderId": url }), navigated(url))
        }
        _ => (json!({}), Vec::new()),
    }
}

/// The events of a navigation to `url` that committed and loaded.
fn navigated(url: &str) -> Vec<Value> {
    let frame = json!({
        "id": FRAME,
        "loaderId": url,
        "url": url,
        "domainAndRegistry": "",
        "securityOrigin": url,
        "mimeType": "text/html",
        "adFrameStatus": { "adFrameType": "none" },
        "secureContextType": "InsecureScheme",
        "crossOriginIsolatedContextType": "NotIsolated",
        "gatedAPIFeatures": [],
    });
    let lifecycle = |name: &str| {
        json!({
            "method": "Page.lifecycleEvent",
            "sessionId": SESSION,
            "params": { "frameId": FRAME, "loaderId": url, "name": name, "timestamp": 0.0 },
        })
    };
    vec![
        lifecycle("init"),
        json!({
            "method": "Page.frameNavigated",
            "sessionId": SESSION,
            "params": { "frame": frame, "type": "Navigation" },
        }),
        lifecycle("commit"),
        lifecycle("DOMContentLoaded"),
        lifecycle("load"),
        json!({
            "method": "Page.loadEventFired",
            "sessionId": SESSION,
            "params": { "timestamp": 0.0 },
        }),
    ]
}

/// The browser: reads each command of its one connection, does `plant`,
/// answers at once, until the engine closes it.
async fn browser(stream: TcpStream, commands: Arc<AtomicU64>, plant: Option<Plant>) {
    let mut socket = tokio_tungstenite::accept_async(stream)
        .await
        .expect("invariant: the engine speaks WebSocket");
    let mut seen = BTreeSet::new();
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let command: Value =
            serde_json::from_str(text.as_str()).expect("invariant: a command is JSON");
        let id = command["id"]
            .as_u64()
            .expect("invariant: a command has an ID");
        if seen.insert(id) {
            commands.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(plant) = &plant {
            plant();
        }
        let method = command["method"].as_str().unwrap_or_default();
        let (result, events) = answered(method, &command["params"]);
        let mut answer = json!({ "id": id, "result": result });
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        let mut messages = vec![answer];
        messages.extend(events);
        for message in messages {
            if socket
                .send(Message::text(message.to_string()))
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

/// The engine's answer to one request of the program.
struct Client {
    lines: tokio::io::Lines<BufReader<transport::ConnectionReader>>,
    writer: transport::ConnectionWriter,
    next_id: u64,
}

impl Client {
    /// Calls `method`, failing the scenario on an error: a benchmark whose
    /// work did not happen proves nothing.
    async fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
            .to_string()
            .into_bytes();
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .await
            .expect("invariant: the engine reads its Connection");
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .expect("invariant: the engine's lines read")
                .expect("invariant: the engine answers before it closes");
            let message: Value =
                serde_json::from_str(&line).expect("invariant: the engine writes JSON");
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| panic!("`{method}` failed: {message}"));
            }
        }
    }
}

/// What a program does on a Session: opens a page and navigates it, twice, so
/// a navigation sits in the middle. Answers once the browser heard all of it.
async fn program(client: &mut Client, cdp_url: &str, settings: Map<String, Value>) {
    client
        .call(
            "hello",
            &json!({ "protocol_version": "0.1", "surface": "rust", "needs": "collie", "product_version": "0.0.0" }),
        )
        .await;
    client
        .call("run.start", &json!({ "run_id": RUN, "settings": settings }))
        .await;
    let session = client
        .call(
            "run.new_session",
            &json!({ "run_id": RUN, "source": { "kind": "cdp_url", "url": cdp_url } }),
        )
        .await;
    let opened = client
        .call(
            "session.new_page",
            &json!({ "session_id": session["session_id"] }),
        )
        .await;
    for url in PAGES {
        client
            .call(
                "page.goto",
                &json!({ "page_id": opened["page_id"], "url": url }),
            )
            .await;
    }
}

/// Waits until nothing is counted or sent any more, as the engine still sends
/// what no call waits for.
async fn quiet(commands: &AtomicU64, work: &CountedWork) {
    let reading = || {
        let counts: Vec<u64> = Row::ALL.iter().map(|row| work.read(*row)).collect();
        (commands.load(Ordering::Relaxed), counts)
    };
    let mut last = reading();
    loop {
        tokio::time::sleep(QUIET).await;
        let now = reading();
        if now == last {
            return;
        }
        last = now;
    }
}

/// Runs the scenario in the home `home`, with `settings` on its Run, doing
/// `plant` once per command the browser reads, and answers what it counted.
///
/// `metrics` is the engine's, whose [`CountedWork`] counts it all.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark builds its own runtime, with real time as the browser's socket needs (ADR 0060)"
)]
pub(crate) fn counted(
    home: &Path,
    metrics: Metrics,
    settings: Map<String, Value>,
    plant: Option<Plant>,
) -> Counted {
    let work = metrics.counted_work().clone();
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution(Vec::new()));
    let engine = Engine::new(identity, &composition)
        .expect("invariant: one handler each")
        .with_metrics(metrics);
    let commands = Arc::new(AtomicU64::new(0));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the runtime starts");
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("invariant: loopback binds");
        let address = listener.local_addr().expect("invariant: it is bound");
        let cdp_url = format!("ws://{address}/devtools/browser/scripted");
        let (network, dialer) = transport::in_memory();
        let served = browser_end(listener, Arc::clone(&commands), plant);
        let client = async {
            let (reader, writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
                next_id: 0,
            };
            program(&mut client, &cdp_url, settings).await;
            quiet(&commands, &work).await;
            client
                .writer
                .shutdown()
                .await
                .expect("invariant: in memory");
            while let Ok(Some(_)) = client.lines.next_line().await {}
        };
        let dependencies = Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: goodfetch_engine::secrets::Keychain::absent(),
            home: Home::new(home.to_owned()),
            clock: Clock::system(),
            entropy: Entropy::seeded(1861),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
            politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
        };
        let (engine_ended, (), ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client,
            served,
        );
        engine_ended.expect("invariant: the engine got its Connection");
    });
    Counted {
        commands: commands.load(Ordering::Relaxed),
        work,
    }
}

/// Accepts the engine's one connection and plays the browser on it.
async fn browser_end(listener: TcpListener, commands: Arc<AtomicU64>, plant: Option<Plant>) {
    let (stream, _) = listener
        .accept()
        .await
        .expect("invariant: the engine connects");
    browser(stream, commands, plant).await;
}
