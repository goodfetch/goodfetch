//! A `page.on` that turns the `Runtime` domain on is armed before the
//! Client's next call on the page acts (ADR 0019), against a scripted browser
//! that holds its answer to `Runtime.enable` back, the one thing a real
//! browser cannot be made to do on demand. The whole engine runs, over the
//! Borrowed browser path, as in the counts' Session.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_page::blocks::NoBlocks;
use goodfetch_page::page_value::ProgramPages;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// How long the browser holds `Runtime.enable`'s answer back: long past what
/// a call that does not wait takes to reach it. The behaviour depends on no
/// figure: a call that waits comes after the answer however long this is.
const HOLD: Duration = Duration::from_millis(500);

/// What the program's next call runs in the page, by which the browser tells
/// its command from those the engine sends on its own.
const SCRIPT: &str = "() => 'the call after the listener'";

/// What the browser heard of the three things that matter.
const ENABLE: &str = "Runtime.enable";
const ANSWERED: &str = "Runtime.enable answered";
const CALL: &str = "the program's call";

/// What the browser answers `method`; `{}` for every command whose answer the
/// engine does not read.
fn answer_of(method: &str) -> Value {
    match method {
        "Browser.getVersion" => json!({
            "protocolVersion": "1.3",
            "product": "Scripted/1.0",
            "revision": "0",
            "userAgent": "Scripted",
            "jsVersion": "0",
        }),
        "Target.getBrowserContexts" => {
            json!({ "browserContextIds": [], "defaultBrowserContextId": "context" })
        }
        "Target.createBrowserContext" => json!({ "browserContextId": "context" }),
        "Target.createTarget" => json!({ "targetId": "target" }),
        "Target.attachToTarget" => json!({ "sessionId": "session" }),
        "Page.getFrameTree" => json!({ "frameTree": { "frame": {
            "id": "target",
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
        "Page.createIsolatedWorld" => json!({ "executionContextId": 1 }),
        "Page.addScriptToEvaluateOnNewDocument" => json!({ "identifier": "script" }),
        "Runtime.evaluate" | "Runtime.callFunctionOn" => {
            json!({ "result": { "type": "undefined" } })
        }
        _ => json!({}),
    }
}

/// The browser: answers each command at once but `Runtime.enable`, whose
/// answer it holds back for [`HOLD`]; answers what it heard, in order, once
/// the engine closed its connection.
async fn browser(listener: TcpListener) -> Vec<&'static str> {
    let (stream, _) = listener
        .accept()
        .await
        .expect("invariant: the engine connects");
    let mut socket = tokio_tungstenite::accept_async(stream)
        .await
        .expect("invariant: the engine speaks WebSocket");
    let mut heard = Vec::new();
    let mut held: Option<Value> = None;
    let hold = tokio::time::sleep(Duration::ZERO);
    let mut hold = std::pin::pin!(hold);
    loop {
        let message = tokio::select! {
            () = &mut hold, if held.is_some() => {
                let answer = held.take().expect("invariant: the guard says one is held");
                heard.push(ANSWERED);
                if socket.send(Message::text(answer.to_string())).await.is_err() {
                    return heard;
                }
                continue;
            }
            message = socket.next() => message,
        };
        let Some(Ok(message)) = message else {
            return heard;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let command: Value =
            serde_json::from_str(text.as_str()).expect("invariant: a command is JSON");
        let method = command["method"].as_str().unwrap_or_default();
        let mut answer = json!({ "id": command["id"], "result": answer_of(method) });
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        if method == ENABLE {
            heard.push(ENABLE);
            held = Some(answer);
            hold.as_mut().reset(tokio::time::Instant::now() + HOLD);
            continue;
        }
        if text.as_str().contains("the call after the listener") {
            heard.push(CALL);
        }
        if socket
            .send(Message::text(answer.to_string()))
            .await
            .is_err()
        {
            return heard;
        }
    }
}

/// The program's end of its Connection.
struct Client {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl Client {
    /// Writes each of `messages` in one write, so the engine reads them
    /// together, in order.
    async fn write(&mut self, messages: &[Value]) {
        let mut bytes = Vec::new();
        for message in messages {
            bytes.extend(message.to_string().into_bytes());
            bytes.push(b'\n');
        }
        self.writer
            .write_all(&bytes)
            .await
            .expect("invariant: the engine reads its Connection");
    }

    /// The answer to the call `id`.
    async fn answer(&mut self, id: u64) -> Value {
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
                return message;
            }
        }
    }

    /// Calls `method` as the call `id`, answering its `result`.
    async fn call(&mut self, id: u64, method: &str, params: &Value) -> Value {
        self.write(&[request(id, method, params)]).await;
        let answer = self.answer(id).await;
        answer
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("`{method}` failed: {answer}"))
    }
}

/// The call `id` of `method`.
fn request(id: u64, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// What the Client sends behind `page.on`, in one write with it.
#[derive(Clone, Copy)]
enum Behind {
    /// Only its next call on the page.
    TheCall,
    /// The cancel of that `page.on`, then its next call on the page.
    ACancelThenTheCall,
}

/// A program that opens a page, then sends `page.on` for `console` with
/// `behind` in one write, and ends once the call answered.
async fn program(client: &mut Client, cdp_url: &str, behind: Behind) {
    client
        .call(
            1,
            "hello",
            &json!({ "protocol_version": "0.1", "surface": "rust", "needs": "collie", "product_version": "0.0.0" }),
        )
        .await;
    client
        .call(2, "run.start", &json!({ "run_id": "arming" }))
        .await;
    let session = client
        .call(
            3,
            "run.new_session",
            &json!({ "run_id": "arming", "source": { "kind": "cdp_url", "url": cdp_url } }),
        )
        .await;
    let opened = client
        .call(
            4,
            "session.new_page",
            &json!({ "session_id": session["session_id"] }),
        )
        .await;
    let page = &opened["page_id"];
    let listens = request(
        5,
        "page.on",
        &json!({ "page_id": page, "event": "console" }),
    );
    let evaluates = request(
        6,
        "page.evaluate",
        &json!({ "page_id": page, "function": SCRIPT }),
    );
    match behind {
        Behind::TheCall => client.write(&[listens, evaluates]).await,
        Behind::ACancelThenTheCall => {
            let cancels = json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 5 } });
            client.write(&[listens, cancels, evaluates]).await;
        }
    }
    // Whatever it answers: only when its command came is under test.
    client.answer(6).await;
    client
        .writer
        .shutdown()
        .await
        .expect("invariant: in memory");
    while let Ok(Some(_)) = client.lines.next_line().await {}
}

/// Runs [`program`] on an engine in the home `home`, against [`browser`]:
/// what the browser heard.
async fn heard_behind_a_listener(home: &Path, behind: Behind) -> Vec<&'static str> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("invariant: loopback binds");
    let address = listener.local_addr().expect("invariant: it is bound");
    let cdp_url = format!("ws://{address}/devtools/browser/scripted");
    let (network, dialer) = transport::in_memory();
    let client = async {
        let (reader, writer) = dialer
            .connect()
            .await
            .expect("invariant: the engine listens")
            .split();
        let mut client = Client {
            lines: BufReader::new(reader).lines(),
            writer,
        };
        program(&mut client, &cdp_url, behind).await;
    };
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            Vec::new(),
            ProgramPages,
            Arc::new(NoBlocks),
        ));
    let engine = Engine::new(identity, &composition)
        .expect("invariant: one handler each")
        .with_metrics(Metrics::none());
    let dependencies = Dependencies {
        environment: BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()),
        clock: Clock::system(),
        entropy: Entropy::seeded(2224),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    };
    let (ended, (), heard) = tokio::join!(
        engine.run_attached(Network::InMemory(network), dependencies),
        client,
        browser(listener),
    );
    ended.expect("invariant: the engine got its Connection");
    heard
}

/// The Client's next call on a page waits for the browser's answer to the
/// `Runtime.enable` of a `page.on` sent before it, so what that call makes the
/// page log is read live, never from the 1,000 messages V8 keeps while
/// `Runtime` is off; and it does though that `page.on` was cancelled
/// meanwhile, whose `Runtime.enable` still goes on, once.
/// Covers: operation:page.on, event:page.console, operation:page.evaluate
#[rstest]
#[case::while_it_listens(Behind::TheCall)]
#[case::once_cancelled(Behind::ACancelThenTheCall)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test builds its own runtime with real time, since its browser answers over a real socket (ADR 0060)"
)]
fn a_call_after_a_listener_acts_once_the_browser_answered_runtime_enable(#[case] behind: Behind) {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the runtime starts");

    let heard = runtime.block_on(heard_behind_a_listener(home.path(), behind));

    assert_eq!(heard, [ENABLE, ANSWERED, CALL]);
}
