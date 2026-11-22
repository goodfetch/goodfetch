//! A remote Daemon's dropped link (spec 606, ADR 0079), played by the
//! Scripted engine from the TypeScript Client's scripts: the engine drops
//! the Client with no close, then plays the rest of its script to the next
//! Client that connects. The Client reaches it again by itself, greets it
//! with the same `hello`, resubscribes after the last event it read and
//! resumes a stream of Records after the last one it read, each with the
//! credits it still had, and sends again what it was waiting for only when
//! the catalog classes it a read or an idempotent write; an Effect in
//! flight fails with `connection_lost`. Each case plays over a WebSocket to
//! the Scripted engine as an Exposed Daemon, which exits 0 only once every
//! step played, so its exit says the Client sent exactly what the script
//! holds after the drop; and over the in-memory network, whose player
//! fails on a message the script does not hold.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::cell::RefCell;
use std::future::Future;
use std::io::{self, BufReader};
use std::pin::Pin;
use std::process::Stdio;

use goodfetch_collie_client::transport::{Dial, Endpoint, ExposedDaemon, Link, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode, events};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::HelloParams;
use goodfetch_test_support::{
    Ending, Script, TestEnv, Tool, case_logs, engine_script, play_relinking, reported_address,
    tools,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::scripted_daemon::daemon_hello;

/// The script of a Session's listener whose Subscription resumes.
const RESUBSCRIBES: &str = "connect/remote-drop-resubscribes.json";

/// The script of a read sent again beside an Effect that fails.
const RESENDS_A_READ: &str = "connect/remote-drop-resends-a-read.json";

/// The script of a stream of Records that resumes.
const RESUMES_RECORDS: &str = "connect/remote-drop-resumes-records.json";

/// The credits a stream of a Product's items grants once sent: 16 here,
/// where the TypeScript Client grants 64.
const RECORD_CREDITS: u64 = 16;

/// How a case reaches the Scripted engine.
#[derive(Clone, Copy, Debug)]
enum Reached {
    /// Over a WebSocket, the Scripted engine an Exposed Daemon.
    WebSocket,
    /// Over the in-memory network, the script played in the test.
    InMemory,
}

/// `script` of the corpus as this Client plays it: each `hello` this
/// Client's `hello`, each id the script's messages name one more, as this
/// Client numbers its calls from 1 and the TypeScript Client from 0, and
/// each `$/credit` of a stream of Records less the credits the TypeScript
/// Client grants past this one's.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the corpus's script before any async worker runs"
)]
fn played(script: &str, hello: &HelloParams) -> Value {
    let mut played: Value =
        serde_json::from_slice(&std::fs::read(engine_script(script)).unwrap()).unwrap();
    let records = script == RESUMES_RECORDS;
    for step in played["steps"].as_array_mut().unwrap() {
        if step.pointer("/expect/method") == Some(&json!("hello")) {
            step["expect"]["params"] = serde_json::to_value(hello).unwrap();
        }
        for named in ["/expect/params/id", "/send/notification/params/id"] {
            if let Some(Value::Number(id)) = step.pointer_mut(named) {
                *id = (id.as_u64().unwrap() + 1).into();
            }
        }
        if records && step.pointer("/expect/method") == Some(&json!("$/credit")) {
            let credits = &mut step["expect"]["params"]["credits"];
            *credits = (credits.as_u64().unwrap() - (64 - RECORD_CREDITS)).into();
        }
    }
    played
}

/// Plays `script` with `hello` to `program`, reaching the engine as
/// `reached` says, and checks the engine played every step.
fn playing<T>(
    reached: Reached,
    script: &str,
    hello: HelloParams,
    program: impl AsyncFnOnce(Network, Endpoint, HelloParams) -> T,
) -> T {
    let env = TestEnv::new();
    let played = played(script, &hello);
    let runtime = goodfetch_test_support::real_time_runtime();
    match reached {
        Reached::WebSocket => {
            let (process, endpoint) = remote_daemon(&env, &played);
            let read = runtime.block_on(program(Network::Os, endpoint, hello));
            let status = process.child().wait().unwrap();
            assert!(status.success(), "the Scripted Daemon ended with {status}");
            read
        }
        Reached::InMemory => {
            let script: Script = serde_json::from_value(played).unwrap();
            let (links, dialed) = mpsc::unbounded_channel();
            let handle = runtime.handle().clone();
            #[expect(
                clippy::disallowed_methods,
                reason = "the player blocks on its links, outside the runtime the Client runs on"
            )]
            let engine_thread = std::thread::spawn(move || {
                let engine = EngineEnd::new(handle, dialed);
                play_relinking(
                    &script,
                    BufReader::new(&engine),
                    &engine,
                    io::sink(),
                    || engine.relink(),
                )
            });
            let endpoint = Endpoint::Exposed(ExposedDaemon {
                address: "127.0.0.1:7878".parse().unwrap(),
                authority: "127.0.0.1:7878".to_owned(),
            });
            let read = runtime.block_on(program(
                Network::in_memory(Relinked(links)),
                endpoint,
                hello,
            ));
            assert_eq!(engine_thread.join().unwrap().unwrap(), Ending::Finished);
            read
        }
    }
}

/// Starts the Scripted engine playing `script` as an Exposed Daemon, which
/// takes only its own `Origin` and answers this Client's subprotocol, and
/// returns it with its endpoint.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
fn remote_daemon(env: &TestEnv, script: &Value) -> (goodfetch_test_support::Process, Endpoint) {
    let path = env.folder().join("script.json");
    std::fs::write(&path, serde_json::to_vec(script).unwrap()).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", "websocket", "--subprotocol"])
        .arg(PROTOCOL_VERSION.websocket_subprotocol())
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let reported = reported_address(&mut process.child()).unwrap();
    let endpoint = Endpoint::Exposed(ExposedDaemon {
        address: reported.address.parse().unwrap(),
        authority: reported.address,
    });
    (process, endpoint)
}

/// An in-memory network whose every dial hands the engine's end of a new
/// link to the player.
struct Relinked(mpsc::UnboundedSender<DuplexStream>);

impl Dial for Relinked {
    fn dial(&self) -> Pin<Box<dyn Future<Output = io::Result<Link>> + Send + '_>> {
        let (client, engine) = tokio::io::duplex(64 * 1024);
        let dialed = self.0.send(engine).map_err(io::Error::other);
        Box::pin(async move {
            dialed?;
            let (reader, writer) = tokio::io::split(client);
            Ok(Link::new(reader, writer))
        })
    }
}

/// The player's end of the link the Client dialed last, read and written
/// from the player's thread through the Client's runtime.
struct EngineEnd {
    runtime: Handle,
    dialed: RefCell<mpsc::UnboundedReceiver<DuplexStream>>,
    current: RefCell<Option<DuplexStream>>,
}

impl EngineEnd {
    /// The end of the first link the Client dials.
    fn new(runtime: Handle, dialed: mpsc::UnboundedReceiver<DuplexStream>) -> Self {
        let engine = Self {
            runtime,
            dialed: RefCell::new(dialed),
            current: RefCell::new(None),
        };
        engine.relink().unwrap();
        engine
    }

    /// Drops the current link, with no close, and waits for the next one
    /// the Client dials.
    fn relink(&self) -> io::Result<()> {
        self.current.borrow_mut().take();
        let next = self
            .dialed
            .borrow_mut()
            .blocking_recv()
            .ok_or_else(|| io::Error::other("the Client dials no more"))?;
        *self.current.borrow_mut() = Some(next);
        Ok(())
    }
}

impl io::Read for &EngineEnd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.current.borrow_mut().as_mut() {
            Some(link) => self.runtime.block_on(link.read(buf)),
            None => Ok(0),
        }
    }
}

impl io::Write for &EngineEnd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.current.borrow_mut().as_mut() {
            Some(link) => self.runtime.block_on(link.write(buf)),
            None => Err(io::ErrorKind::NotConnected.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.current.borrow_mut().as_mut() {
            Some(link) => self.runtime.block_on(link.flush()),
            None => Ok(()),
        }
    }
}

/// Opens a Connection to `endpoint` over `network` with `hello`.
async fn connected(network: Network, endpoint: Endpoint, hello: HelloParams) -> ClientConnection {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(Handle::current()),
        &network,
        &endpoint,
        hello,
    )
    .await
    .unwrap();
    connection
}

/// Starts a Run, listens to a Session's pages, reads two, then stops
/// listening, finishes the Run and closes the Connection: the pages read.
async fn two_pages(network: Network, endpoint: Endpoint, hello: HelloParams) -> [String; 2] {
    let connection = connected(network, endpoint, hello).await;
    let run = connection.start().await.unwrap();
    let session = run.new_session().await.unwrap();
    let mut pages = session.events(events::Page);
    let first = pages.next().await.unwrap().unwrap();
    let second = pages.next().await.unwrap().unwrap();
    drop(pages);
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    [first.id().0.clone(), second.id().0.clone()]
}

/// The engine drops the link after the Session's first page: the Client
/// greets it again and resubscribes after that page, with the credits it
/// still had, and sends `session.on` again by its id, so the listener
/// loses no page.
///
/// Covers: story:11-6
#[rstest::rstest]
fn a_dropped_link_resubscribes_after_the_last_event_read(
    #[values(Reached::WebSocket, Reached::InMemory)] reached: Reached,
) {
    let pages = playing(reached, RESUBSCRIBES, daemon_hello(), two_pages);

    assert_eq!(pages, ["page-1", "page-2"].map(str::to_owned));
}

/// What [`resent_and_lost`] reads: the Effect's failure and the read's
/// answer.
type ResentAndLost = (Error, Value);

/// Starts a Run, then opens a page, an Effect, and lists the Runs with
/// the raw call, a read, both in flight as the link drops; then finishes
/// the Run and closes the Connection.
async fn resent_and_lost(
    network: Network,
    endpoint: Endpoint,
    hello: HelloParams,
) -> ResentAndLost {
    let connection = connected(network, endpoint, hello).await;
    let run = connection.start().await.unwrap();
    let (page, listed) = tokio::join!(run.new_page(), connection.raw_call("run.list"));
    let lost = page.err().unwrap();
    let listed = listed.unwrap();
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    (lost, listed)
}

/// What the read and the Effect in flight as the link dropped come to:
/// the read is sent again and answered; the Effect fails with
/// `connection_lost`, never sent again.
fn assert_resent_and_lost((lost, listed): &ResentAndLost) {
    assert_eq!(
        (lost.code(), lost.operation(), listed),
        (
            ErrorCode::CONNECTION_LOST,
            Some("run.new_page"),
            &json!({"runs": []})
        ),
        "{lost:?}"
    );
}

/// The `dropped-link-resends-reads` client conformance case, played by
/// the Scripted engine as an Exposed Daemon.
///
/// Covers: story:11-6, story:11-32, error:connection_lost
#[test]
fn a_dropped_link_resends_a_read_and_loses_an_effect() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "dropped-link-resends-reads",
        RustMode::Daemon,
    )
    .unwrap();

    let read = playing(
        Reached::WebSocket,
        RESENDS_A_READ,
        daemon_hello(),
        resent_and_lost,
    );

    assert_resent_and_lost(&read);
}

/// The same over the in-memory network.
///
/// Covers: story:11-6, story:11-32, error:connection_lost
#[test]
fn a_dropped_in_memory_link_resends_a_read_and_loses_an_effect() {
    let read = playing(
        Reached::InMemory,
        RESENDS_A_READ,
        daemon_hello(),
        resent_and_lost,
    );

    assert_resent_and_lost(&read);
}

/// Starts the crawl, reads its Records to the stream's end, then finishes
/// the Run and closes the Connection: each Record's number.
async fn records(network: Network, endpoint: Endpoint, hello: HelloParams) -> Vec<u64> {
    let connection = connected(network, endpoint, hello).await;
    let crawl = json!({
        "labels": {"listing": {}, "product": {}},
        "datasets": [{"name": "products", "key": ["id"]}],
        "seeds": [{"url": "https://shop.test/a", "label": "listing"}],
        "run_id": "nightly",
    });
    connection
        .call::<Value>("crawl.start", &crawl)
        .await
        .unwrap();
    let mut read = connection.stream::<Value>(
        "run.records",
        &json!({"run_id": "nightly", "dataset": "products"}),
        "$/record",
    );
    let mut numbers = Vec::new();
    while let Some(record) = read.next().await {
        numbers.push(record.unwrap()["record"]["number"].as_u64().unwrap());
    }
    connection
        .call::<Value>("run.finish", &json!({"run_id": "nightly"}))
        .await
        .unwrap();
    connection.close().await.unwrap();
    numbers
}

/// The engine drops the link after the first Record: the Client greets it
/// again and sends `run.records` again by its id, after that Record, with
/// the credits it still had, so the stream loses no Record.
///
/// Covers: story:11-6
#[rstest::rstest]
fn a_dropped_link_resumes_a_stream_of_records_after_the_last_read(
    #[values(Reached::WebSocket, Reached::InMemory)] reached: Reached,
) {
    let hello = HelloParams {
        needs: Some(Product::Foxhound),
        ..daemon_hello()
    };

    let numbers = playing(reached, RESUMES_RECORDS, hello, records);

    assert_eq!(numbers, [1, 2]);
}
