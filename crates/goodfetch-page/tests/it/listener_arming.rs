//! A `page.on` that turns the `Runtime` domain on is armed before the
//! Client's next call on the page acts, and that wait counts inside the call's
//! own limit (ADR 0019), against a real browser behind a relay that holds its
//! answer to `Runtime.enable` back. The whole engine runs, over the Borrowed
//! browser path.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the relay and the silent site listen on their own sockets, and the relay dials the test's browser (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test builds its own runtime with real time, since the browser answers over a real socket, starts that browser and reads its port, and the relay's hold runs on that time (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_page::Launcher;
use goodfetch_page::page_value::ProgramPages;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HELLO, ProductVersion};
use goodfetch_test_client::Hello;
use goodfetch_test_support::{
    DebuggedBrowser, TestEnv, browser_cache, chrome_for_testing_debugged,
};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

use crate::handler_page::NoBlocks;

/// How long the relay holds `Runtime.enable`'s answer back behind a call that
/// must come after it: long past what a call that does not wait takes to reach
/// it. The behaviour depends on no figure: a call that waits comes after the
/// answer however long this is.
const HOLD: Duration = Duration::from_millis(500);

/// How long it holds the answer back behind a call whose limit passes first:
/// far past that limit, so a call that waits the answer out is seen to.
const LONG_HOLD: Duration = Duration::from_secs(5);

/// That call's limit.
const SHORT_LIMIT: u64 = 300;

/// A limit the hold of [`SPENT_HOLD`] takes half of.
const LIMIT: u64 = 4_000;

/// How long the relay holds the answer back behind a call of [`LIMIT`].
const SPENT_HOLD: Duration = Duration::from_secs(2);

/// How long the browser has to exit before it's killed.
const BROWSER_STOP: Duration = Duration::from_secs(10);

/// What the program's next call runs in the page, by which the relay tells
/// its command from those the engine sends on its own.
const SCRIPT: &str = "() => 'the call after the listener'";

/// What the relay saw of the three things that matter.
const ENABLE: &str = "Runtime.enable";
const ANSWERED: &str = "Runtime.enable answered";
const CALL: &str = "the program's call";

/// What the relay saw.
struct Relayed {
    /// The three things that matter, in order.
    heard: Vec<&'static str>,
    /// When it let the answer go.
    released: Option<Instant>,
}

/// The relay on `listener`: passes every message between the engine and the
/// browser at `browser`, as they come, but the browser's answer to
/// `Runtime.enable`, which it holds back for `hold`; answers what it saw once
/// the engine closed its connection.
async fn relay(listener: TcpListener, browser: String, hold: Duration) -> Relayed {
    let (stream, _) = listener
        .accept()
        .await
        .expect("invariant: the engine connects");
    let mut engine = tokio_tungstenite::accept_async(stream)
        .await
        .expect("invariant: the engine speaks WebSocket");
    let address = browser
        .strip_prefix("ws://")
        .and_then(|rest| rest.split('/').next())
        .expect("invariant: the browser's address is a WebSocket URL");
    let dialed = TcpStream::connect(address)
        .await
        .expect("invariant: the browser listens");
    let (mut upstream, _) = tokio_tungstenite::client_async(browser.as_str(), dialed)
        .await
        .expect("invariant: the browser speaks WebSocket");
    let mut relayed = Relayed {
        heard: Vec::new(),
        released: None,
    };
    let mut enable = None;
    let mut unanswered: Option<Message> = None;
    let release = tokio::time::sleep(Duration::ZERO);
    let mut release = std::pin::pin!(release);
    loop {
        tokio::select! {
            () = &mut release, if unanswered.is_some() => {
                let answer = unanswered.take().expect("invariant: the guard says one is held");
                relayed.heard.push(ANSWERED);
                relayed.released = Some(Instant::now());
                if engine.send(answer).await.is_err() {
                    return relayed;
                }
            }
            message = engine.next() => {
                let Some(Ok(message)) = message else {
                    return relayed;
                };
                if let Message::Text(text) = &message {
                    let command: Value =
                        serde_json::from_str(text.as_str()).expect("invariant: a command is JSON");
                    if command["method"] == ENABLE {
                        relayed.heard.push(ENABLE);
                        enable = Some(command["id"].clone());
                    }
                    if text.as_str().contains("the call after the listener") {
                        relayed.heard.push(CALL);
                    }
                }
                if upstream.send(message).await.is_err() {
                    return relayed;
                }
            }
            message = upstream.next() => {
                let Some(Ok(message)) = message else {
                    return relayed;
                };
                let answers_enable = enable.is_some() && matches!(&message, Message::Text(text)
                    if serde_json::from_str::<Value>(text.as_str())
                        .is_ok_and(|answer| Some(&answer["id"]) == enable.as_ref()));
                if answers_enable {
                    enable = None;
                    unanswered = Some(message);
                    release.as_mut().reset(Instant::now() + hold);
                } else if engine.send(message).await.is_err() {
                    return relayed;
                }
            }
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

    /// Ends the Connection, once the engine read to its end.
    async fn leave(&mut self) {
        self.writer.shutdown().await.expect("invariant: in memory");
        while let Ok(Some(_)) = self.lines.next_line().await {}
    }

    /// Opens a page in a new Run of `settings` on the browser at `cdp_url`:
    /// its ID.
    async fn page(&mut self, cdp_url: &str, settings: &Value) -> Value {
        self.call(1, HELLO, &Hello::needing(Product::Collie).params())
            .await;
        self.call(
            2,
            "run.start",
            &json!({ "run_id": "arming", "settings": settings }),
        )
        .await;
        let session = self
            .call(
                3,
                "run.new_session",
                &json!({ "run_id": "arming", "source": { "kind": "cdp_url", "url": cdp_url } }),
            )
            .await;
        let opened = self
            .call(
                4,
                "session.new_page",
                &json!({ "session_id": session["session_id"] }),
            )
            .await;
        opened["page_id"].clone()
    }
}

/// The call `id` of `method`.
fn request(id: u64, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// `params` of a call on `page`, which a locator's call names in its locator.
fn on_page(params: &Value, page: &Value) -> Value {
    let mut params = params.clone();
    let names_it = if params.get("locator").is_some() {
        &mut params["locator"]
    } else {
        &mut params
    };
    names_it["page_id"] = page.clone();
    params
}

/// The `page.on` that turns `Runtime` on, on the page `page`, as the call 5.
fn listens(page: &Value) -> Value {
    request(
        5,
        "page.on",
        &json!({ "page_id": page, "event": "console" }),
    )
}

/// Runs `program` on an engine in `env`'s Goodfetch home, against a real browser
/// behind a relay that holds `Runtime.enable`'s answer for `hold`: what the
/// relay saw, and what the program answered. The program takes the relay's
/// address, and the address of a site that never answers.
async fn behind_a_relay<T>(
    env: &TestEnv,
    hold: Duration,
    program: impl AsyncFnOnce(&mut Client, String, u16) -> T,
) -> (Relayed, T) {
    // The dispatch records go into the test's exercise record (ADR 0072).
    let clock = Clock::system();
    let _subscriber =
        tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || clock.wall()));
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    let DebuggedBrowser {
        process,
        url: browser,
        ..
    } = chrome_for_testing_debugged(
        env,
        &cache,
        &env.folder().join("relayed-browser-profile"),
        chromium_background_traffic_off()
            .into_iter()
            .chain(["--host-resolver-rules=MAP * 127.0.0.1".to_owned()]),
    )
    .expect("the browser starts");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("invariant: loopback binds");
    let address = listener.local_addr().expect("invariant: it is bound");
    let cdp_url = format!("ws://{address}/devtools/browser/relayed");
    // It takes connections and never reads them, so a document it is asked
    // for never comes.
    let silent = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("invariant: loopback binds");
    let silent_port = silent.local_addr().expect("invariant: it is bound").port();
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
        program(&mut client, cdp_url, silent_port).await
    };
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            (Vec::new(), Launcher::Executable),
            ProgramPages,
            Arc::new(NoBlocks),
        ));
    let engine = Engine::new(identity, &composition)
        .expect("invariant: one handler each")
        .with_metrics(Metrics::none());
    let dependencies = Dependencies {
        environment: BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::system(),
        entropy: Entropy::seeded(2224),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    };
    let (ended, answered, relayed) = tokio::join!(
        engine.run_attached(Network::InMemory(network), dependencies),
        client,
        relay(listener, browser, hold),
    );
    ended.expect("invariant: the engine got its Connection");
    let _ = process.interrupt_and_wait(BROWSER_STOP);
    (relayed, answered)
}

/// A runtime for one test, on real time, as the browser answers.
fn runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .expect("invariant: the runtime starts")
}

/// What the Client sends behind `page.on`, in one write with it.
#[derive(Clone, Copy)]
enum Behind {
    /// Only its next call on the page.
    TheCall,
    /// The cancel of that `page.on`, then its next call on the page.
    ACancelThenTheCall,
}

/// The Client's next call on a page waits for the browser's answer to the
/// `Runtime.enable` of a `page.on` sent before it, so what that call makes the
/// page log is read live, never from the 1,000 messages V8 keeps while
/// `Runtime` is off; and it does though that `page.on` was cancelled
/// meanwhile, whose `Runtime.enable` still goes on, once.
/// Covers: operation:page.on, operation:page.evaluate
#[rstest]
#[case::while_it_listens(Behind::TheCall)]
#[case::once_cancelled(Behind::ACancelThenTheCall)]
fn a_call_after_a_listener_acts_once_the_browser_answered_runtime_enable(#[case] behind: Behind) {
    let env = TestEnv::new();

    let (relayed, ()) =
        runtime().block_on(behind_a_relay(&env, HOLD, async |client, cdp_url, _| {
            let page = client.page(&cdp_url, &json!({})).await;
            let evaluates = request(
                6,
                "page.evaluate",
                &json!({ "page_id": page, "function": SCRIPT }),
            );
            match behind {
                Behind::TheCall => client.write(&[listens(&page), evaluates]).await,
                Behind::ACancelThenTheCall => {
                    let cancels =
                        json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 5 } });
                    client.write(&[listens(&page), cancels, evaluates]).await;
                }
            }
            // Whatever it answers: only when its command came is under test.
            client.answer(6).await;
            client.leave().await;
        }));

    assert_eq!(relayed.heard, [ENABLE, ANSWERED, CALL]);
}

/// A call whose own limit passes while its page's `Runtime` is still turning
/// on raises `deadline_exceeded` naming that limit, what it waited for being
/// the browser's answer, and never reaches the page: the limit a program set
/// is the one that holds.
/// Covers: operation:page.on, error:deadline_exceeded
#[rstest]
#[case::evaluate(
    "page.evaluate",
    json!({ "function": SCRIPT }),
    json!({ "timeout": { "action": SHORT_LIMIT } }),
    "timeout.action"
)]
#[case::locator_action(
    "locator.click",
    json!({ "locator": { "selector": "#missing" } }),
    json!({ "timeout": { "action": SHORT_LIMIT } }),
    "timeout.action"
)]
#[case::wait_for_function(
    "page.wait_for_function",
    json!({ "function": SCRIPT, "timeout": SHORT_LIMIT }),
    json!({}),
    "timeout.wait"
)]
#[case::wait_on_a_locator(
    "locator.wait_for",
    json!({ "locator": { "selector": "#missing" }, "timeout": SHORT_LIMIT }),
    json!({}),
    "timeout.wait"
)]
#[case::navigation(
    "page.goto",
    json!({ "url": "about:blank", "timeout": SHORT_LIMIT }),
    json!({}),
    "timeout.navigation"
)]
fn a_calls_limit_holds_while_its_page_turns_runtime_on(
    #[case] method: &str,
    #[case] params: Value,
    #[case] settings: Value,
    #[case] limit: &str,
) {
    let env = TestEnv::new();

    let (relayed, answer) = runtime().block_on(behind_a_relay(
        &env,
        LONG_HOLD,
        async |client, cdp_url, _| {
            let page = client.page(&cdp_url, &settings).await;
            let call = request(6, method, &on_page(&params, &page));
            client.write(&[listens(&page), call]).await;
            let answer = client.answer(6).await;
            client.leave().await;
            answer
        },
    ));

    let data = &answer["error"]["data"];
    assert_eq!(
        (
            &data["code"],
            &data["limit"]["key"],
            &data["waited_for"],
            relayed.heard,
        ),
        (
            &json!("deadline_exceeded"),
            &json!(limit),
            &json!("browser_answer"),
            vec![ENABLE],
        ),
        "{answer}"
    );
}

/// What a call spends waiting for its page's `Runtime` comes off what its work
/// may take: its deadline passes `LIMIT` after it arrived, not after the
/// browser answered, and the time it reports counts the wait.
/// Covers: operation:page.on, error:deadline_exceeded
#[rstest]
#[case::locator_action(
    "locator.click",
    json!({ "locator": { "selector": "#missing" } }),
    json!({ "timeout": { "action": LIMIT } })
)]
#[case::wait_for_function(
    "page.wait_for_function",
    json!({ "function": "() => false", "timeout": LIMIT }),
    json!({})
)]
#[case::wait_on_a_locator(
    "locator.wait_for",
    json!({ "locator": { "selector": "#missing" }, "timeout": LIMIT }),
    json!({})
)]
#[case::navigation(
    "page.goto",
    json!({ "timeout": LIMIT }),
    json!({ "network": { "resolve": { "site.test": ["93.184.215.14"] } } })
)]
fn a_calls_limit_counts_its_wait_for_runtime(
    #[case] method: &str,
    #[case] params: Value,
    #[case] settings: Value,
) {
    let env = TestEnv::new();

    let (relayed, (answer, answered)) = runtime().block_on(behind_a_relay(
        &env,
        SPENT_HOLD,
        async |client, cdp_url, silent_port| {
            let page = client.page(&cdp_url, &settings).await;
            let mut params = on_page(&params, &page);
            if method == "page.goto" {
                params["url"] = json!(format!("http://site.test:{silent_port}/"));
            }
            client
                .write(&[listens(&page), request(6, method, &params)])
                .await;
            let answer = client.answer(6).await;
            let answered = Instant::now();
            client.leave().await;
            (answer, answered)
        },
    ));

    let released = relayed.released.expect("the relay let the answer go");
    let data = &answer["error"]["data"];
    let elapsed = data["elapsed"]
        .as_u64()
        .unwrap_or_else(|| panic!("{answer}"));
    assert_eq!(
        (
            &data["code"],
            elapsed >= LIMIT,
            answered.duration_since(released) < Duration::from_millis(LIMIT),
        ),
        (&json!("deadline_exceeded"), true, true),
        "{answer}"
    );
}
