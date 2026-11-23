//! The engine through its public interface over the in-memory network
//! (spec #213's Seam 3): fabricated `hello`s, refused before any other
//! message and then closed (ADR 0066), the Product check before the version
//! check (ADR 0095), `$/cancel`, and every failure's shape (ADR 0038).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059); its SQL statements are its own, on no Shared path (ADR 0140)"
)]

mod call_input;
mod changed_certificates;
mod checkpoint;
mod connection_settings;
mod consent_walls;
mod controls;
mod daemon;
mod discard;
mod disk_view;
mod durability;
mod effect;
mod events;
mod following;
mod governor;
mod in_flight_cap;
mod key_value_bytes;
mod key_value_store;
mod layout;
mod lifecycle;
mod list;
mod local_addresses;
mod log;
mod masking;
mod needs_you;
mod opening;
mod ops;
mod panics;
mod pauses;
mod preamble;
mod progress;
mod raw_calls;
mod refetch_stops;
mod refusals;
mod report;
mod run;
mod run_files;
mod run_key;
mod run_limits;
mod settings;
mod settings_edits;
mod starts;
mod storage;
mod takeover;
mod trace;
mod usage;
mod versions;
mod views;
mod websocket;

use std::time::Duration;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

use goodfetch_test_support::TestEnv;

use super::{Dependencies, Engine};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::composition::{CallContext, Composition, CompositionError, Contribution, Handler};
use crate::entropy::Entropy;
use crate::home::{Home, RunStateLocation};
use crate::product::{Channel, Identity};
use crate::run_state::RunStateOptions;
use crate::tasks::BlockingPolicy;
use crate::transport::{self, ConnectionReader, ConnectionWriter, Network};

/// A call that waits until it is cancelled, at its one await point.
const WAIT: Operation = Operation {
    name: "test.wait",
    description: "Waits until it is cancelled.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// A call that sleeps a minute, on the paused clock, then answers.
const SLEEP: Operation = Operation {
    name: "test.sleep",
    description: "Sleeps a minute, then answers.",
    ..WAIT
};

/// A call that answers the surface whose rules its Connection follows, and
/// whether its `hello` came from the gateway.
const SURFACE: Operation = Operation {
    name: "test.surface",
    description: "Answers its Connection's surface and gateway mark.",
    ..WAIT
};

/// A call that refuses a setting only Foxhound carries, as every call that
/// reads a Client's settings refuses it.
const CRAWL_SETTING: Operation = Operation {
    name: "test.crawl_setting",
    description: "Refuses `crawl.max_requests` as Foxhound's.",
    ..WAIT
};

fn test_operations() -> Contribution {
    Contribution::new("test")
        .with_handler(Handler::new(&WAIT, |_: Value, _: CallContext| async {
            std::future::pending::<()>().await;
            Ok::<Value, CallError>(json!({ "waited": true }))
        }))
        .with_handler(Handler::new(&SLEEP, |_: Value, _: CallContext| async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok::<Value, CallError>(json!({ "slept": true }))
        }))
        .with_handler(Handler::new(
            &SURFACE,
            |_: Value, context: CallContext| async move {
                Ok::<Value, CallError>(json!({
                    "surface": context.surface().as_str(),
                    "acts_for_model": context.surface().acts_for_model(),
                    "behind_gateway": context.behind_gateway(),
                }))
            },
        ))
        .with_handler(Handler::new(
            &CRAWL_SETTING,
            |_: Value, context: CallContext| async move {
                Err::<Value, CallError>(crate::run::settings_error(
                    &crate::settings::SettingsError::NotInProduct {
                        key: "crawl.max_requests".to_owned(),
                        needs: Product::Foxhound,
                    },
                    &context,
                ))
            },
        ))
}

fn engine(product: Product) -> Engine {
    engine_with(product, None)
}

/// The Product version [`engine_with`]'s engines run as.
fn version() -> ProductVersion {
    ProductVersion::parse("1.4.0").unwrap()
}

/// `env`'s per-major folder of [`engine_with`]'s engines.
fn per_major(env: &TestEnv) -> std::path::PathBuf {
    goodfetch_protocol::home::per_major(env.home(), &version())
}

/// An engine of `product` with the engine's own operations, the tests' and
/// `more`.
fn engine_with(product: Product, more: Option<Contribution>) -> Engine {
    engine_as(product, version(), more)
}

/// An engine of `product` that runs as Product version `version`, with
/// the engine's own operations and the tests'.
fn engine_at(product: Product, version: &str) -> Engine {
    engine_as(product, ProductVersion::parse(version).unwrap(), None)
}

fn engine_as(
    product: Product,
    product_version: ProductVersion,
    more: Option<Contribution>,
) -> Engine {
    let identity = Identity {
        product,
        product_version,
        channel: Channel::CratesIo,
    };
    let mut composition = Composition::default()
        .with(crate::info::contribution())
        .with(test_operations());
    if let Some(more) = more {
        composition = composition.with(more);
    }
    Engine::new(identity, &composition).unwrap()
}

#[test]
fn a_second_handler_for_one_operation_is_refused_naming_both_contributions() {
    let again = Contribution::new("again")
        .with_handler(Handler::new(&WAIT, |_: Value, _: CallContext| async {
            Ok::<Value, CallError>(json!({}))
        }));
    let composition = Composition::default().with(test_operations()).with(again);
    let identity = Identity {
        product: Product::Collie,
        product_version: version(),
        channel: Channel::CratesIo,
    };

    let refused = Engine::new(identity, &composition).err();

    assert_eq!(
        refused,
        Some(CompositionError::DuplicateHandler {
            operation: WAIT.name,
            first: "test",
            second: "again",
        })
    );
}

fn hello(product: &str, protocol_version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": protocol_version,
            "surface": "sdk_typescript",
            "needs": product,
            "product_version": "1.4.0",
        },
    })
}

fn request(id: i64, method: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
}

fn cancel(id: i64) -> Value {
    json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": id } })
}

/// How many calls a Connection keeps in flight, as `too_many_calls`'s
/// `max` and `hello`'s `max_calls_in_flight` name it.
const IN_FLIGHT_CAP: i64 = 256;

/// How long a test waits, on the paused clock, for a message the engine
/// owes: one it never sends fails the test instead of hanging it.
const OWED: Duration = Duration::from_secs(5);

/// The next message the engine owes, or `None` once it closed the
/// Connection.
async fn owed(client: &mut Client) -> Option<Value> {
    tokio::time::timeout(OWED, client.next())
        .await
        .expect("the engine sends what it owes")
}

/// A Client past its `hello`.
async fn opened(client: &mut Client) {
    client.send(&[hello("collie", "0.1")]).await;
    assert!(client.next().await.unwrap().get("result").is_some());
}

/// The Client's end of a Connection.
struct Client {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl Client {
    async fn send(&mut self, messages: &[Value]) {
        self.try_send(messages).await.unwrap();
    }

    /// Sends `messages`, answering whether the Connection took them.
    async fn try_send(&mut self, messages: &[Value]) -> std::io::Result<()> {
        let mut bytes = Vec::new();
        for message in messages {
            bytes.extend(serde_json::to_vec(message).unwrap());
            bytes.push(b'\n');
        }
        self.writer.write_all(&bytes).await
    }

    async fn send_bytes(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).await.unwrap();
    }

    /// The next message, or `None` once the engine closed the Connection.
    async fn next(&mut self) -> Option<Value> {
        let line = self.lines.next_line().await.unwrap()?;
        Some(serde_json::from_str(&line).unwrap())
    }

    async fn close(&mut self) {
        self.writer.shutdown().await.unwrap();
    }
}

/// What an in-process test's engine runs on: the test's home, the test
/// `Clock` and `Entropy`, and blocking work inline. Made on the runtime the
/// engine runs on, as the paused `Clock` must be.
fn dependencies(env: &TestEnv) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: crate::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: crate::politeness::PolitenessOptions::default(),
    }
}

/// Runs `engine` as an Attached engine on the in-memory network while
/// `client` talks to it, on the tests' paused runtime.
fn attached<F, Fut, T>(engine: &Engine, client: F) -> T
where
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    attached_on(engine, &TestEnv::new(), client)
}

/// [`attached`], in `env`.
fn attached_on<F, Fut, T>(engine: &Engine, env: &TestEnv, client: F) -> T
where
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    attached_with(engine, || dependencies(env), client)
}

/// [`attached`], on the `Dependencies` that `dependencies` makes on the
/// runtime. The test `tracing` subscriber, timed by their `Clock`, writes
/// the dispatch records into the test's exercise record (ADR 0072), unless
/// the test installed its own, as [`dispatch_records`] does.
fn attached_with<D, F, Fut, T>(engine: &Engine, dependencies: D, client: F) -> T
where
    D: FnOnce() -> Dependencies,
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    attached_on_runtime(
        &goodfetch_test_support::runtime(),
        engine,
        dependencies,
        client,
    )
}

/// [`attached_with`], on `runtime`, such as one whose blocking threads a
/// test bounds.
fn attached_on_runtime<D, F, Fut, T>(
    runtime: &tokio::runtime::Runtime,
    engine: &Engine,
    dependencies: D,
    client: F,
) -> T
where
    D: FnOnce() -> Dependencies,
    F: FnOnce(Client) -> Fut,
    Fut: Future<Output = T>,
{
    runtime.block_on(async {
        let dependencies = dependencies();
        let clock = dependencies.clock;
        let unset = tracing::dispatcher::get_default(
            tracing::Dispatch::is::<tracing::subscriber::NoSubscriber>,
        );
        let _subscriber = unset.then(|| {
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }))
        });
        let (network, dialer) = transport::in_memory();
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            client(Client {
                lines: BufReader::new(reader).lines(),
                writer,
            })
            .await
        };
        let (served, observed) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            talk
        );
        served.unwrap();
        observed
    })
}

/// Sends `messages`, ends the input, then reads every message until the
/// engine closes.
fn exchange(engine: &Engine, messages: &[Value]) -> Vec<Value> {
    attached(engine, |mut client| async move {
        client.send(messages).await;
        client.close().await;
        let mut received = Vec::new();
        while let Some(message) = client.next().await {
            received.push(message);
        }
        received
    })
}

/// Takes the engine instance out of an accepted `hello`'s `reply`,
/// answering whether it names one: 16 lowercase hex digits, new in each
/// engine process.
fn takes_engine_instance(reply: &mut Value) -> bool {
    reply["result"]
        .as_object_mut()
        .and_then(|result| result.remove("engine_instance"))
        .and_then(|instance| instance.as_str().map(str::to_owned))
        .is_some_and(|instance| {
            instance.len() == 16
                && instance
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap()
}

/// Covers: story:3-3
#[test]
fn an_accepted_hello_answers_the_engines_product_and_versions_and_instance() {
    let mut received = exchange(&engine(Product::Collie), &[hello("collie", "0.1")]);
    let named = takes_engine_instance(&mut received[0]);

    assert_eq!(
        (received, named),
        (
            vec![json!({
                "jsonrpc": "2.0",
                "id": 0,
                "result": {
                    "protocol_version": "0.1", "product": "collie", "product_version": "1.4.0",
                    "max_calls_in_flight": IN_FLIGHT_CAP,
                },
            })],
            true
        )
    );
}

/// Covers: story:3-132, error:protocol_version_unsupported, format:protocol
#[rstest]
#[case::newer_preview("0.2")]
#[case::newer_major("1.0")]
fn a_hello_of_another_version_is_refused_before_any_other_message_and_closed(
    #[case] protocol_version: &str,
) {
    let received = exchange(
        &engine(Product::Collie),
        &[hello("collie", protocol_version), request(1, "engine.info")],
    );

    assert_eq!(received.len(), 1, "{received:#?}");
    insta::assert_snapshot!(
        format!("protocol_{}_refused", protocol_version.replace('.', "_")),
        pretty(&received[0])
    );
}

/// Covers: story:3-135, error:not_in_product
#[rstest]
#[case::same_version("0.1")]
#[case::before_the_version_check("0.2")]
fn a_collie_engine_refuses_a_client_that_needs_foxhound_and_closes(#[case] protocol_version: &str) {
    let received = exchange(
        &engine(Product::Collie),
        &[
            hello("foxhound", protocol_version),
            request(1, "engine.info"),
        ],
    );

    assert_eq!(received.len(), 1, "{received:#?}");
    insta::assert_snapshot!(
        format!(
            "foxhound_client_at_protocol_{}_refused",
            protocol_version.replace('.', "_")
        ),
        pretty(&received[0])
    );
}

/// Covers: story:3-135
#[rstest]
#[case::collie_client("collie")]
#[case::foxhound_client("foxhound")]
fn a_foxhound_engine_accepts_both_products_clients(#[case] needs: &str) {
    let received = exchange(&engine(Product::Foxhound), &[hello(needs, "0.1")]);

    assert_eq!(received[0]["result"]["product"], json!("foxhound"));
}

#[test]
fn a_browser_hosts_hello_needs_no_product() {
    let mut browser_host = hello("collie", "0.1");
    browser_host["params"]["surface"] = json!("browser_host");
    browser_host["params"]
        .as_object_mut()
        .unwrap()
        .remove("needs");

    let received = exchange(&engine(Product::Collie), &[browser_host]);

    assert_eq!(received[0]["result"]["product"], json!("collie"));
}

#[test]
fn a_clients_hello_without_the_product_it_needs_is_invalid_and_closed() {
    let mut client = hello("collie", "0.1");
    client["params"].as_object_mut().unwrap().remove("needs");

    let received = exchange(
        &engine(Product::Collie),
        &[client, request(1, "engine.info")],
    );

    assert_eq!(
        (received.len(), &received[0]["error"]["data"]["code"]),
        (1, &json!("invalid_params"))
    );
}

/// Covers: error:not_in_product
#[test]
fn behind_the_gateway_not_in_product_gives_the_gateways_fix() {
    let mut gateway = hello("collie", "0.1");
    gateway["params"]["surface"] = json!("cloud");

    let received = exchange(
        &engine(Product::Collie),
        &[gateway, request(1, "sample.crawl")],
    );

    insta::assert_snapshot!(pretty(&received[1]));
}

/// A gateway's `hello` names the surface of the Client it relays, whose
/// rules the Connection then follows, a model's included (ADR 0092),
/// while its mark still words `not_in_product` (ADR 0095).
#[test]
fn a_gateways_hello_holds_the_connection_to_the_surface_of_the_client_it_relays() {
    let mut gateway = hello("collie", "0.1");
    gateway["params"]["surface"] = json!("cloud");
    gateway["params"]["client_surface"] = json!("mcp");

    let received = exchange(
        &engine(Product::Collie),
        &[gateway, request(1, "test.surface")],
    );

    assert_eq!(
        received[1]["result"],
        json!({ "surface": "mcp", "acts_for_model": true, "behind_gateway": true })
    );
}

#[rstest]
#[case::from_no_gateway("rust", "mcp")]
#[case::a_gateway_relaying_a_gateway("cloud", "cloud")]
#[case::a_gateway_relaying_a_browser_host("cloud", "browser_host")]
fn a_client_surface_only_a_gateway_names_for_a_client_is_invalid_and_closed(
    #[case] surface: &str,
    #[case] client_surface: &str,
) {
    let mut client = hello("collie", "0.1");
    client["params"]["surface"] = json!(surface);
    client["params"]["client_surface"] = json!(client_surface);

    let received = exchange(
        &engine(Product::Collie),
        &[client, request(1, "engine.info")],
    );

    assert_eq!(
        (received.len(), &received[0]["error"]["data"]["code"]),
        (1, &json!("invalid_params"))
    );
}

/// Every call that reads a Client's settings words a setting only Foxhound
/// carries for the gateway behind it, naming no install (ADR 0096), and
/// names the install command, as data, everywhere else.
///
/// Covers: error:not_in_product
#[rstest]
#[case::behind_the_gateway("cloud", None)]
#[case::a_local_client(
    "sdk_typescript",
    Some("cargo install goodfetch-foxhound --locked --features executable")
)]
fn a_setting_only_foxhound_carries_is_worded_for_the_gateway_on_every_call_behind_it(
    #[case] surface: &str,
    #[case] install: Option<&str>,
) {
    let mut client = hello("collie", "0.1");
    client["params"]["surface"] = json!(surface);

    let received = exchange(
        &engine(Product::Collie),
        &[client, request(1, "test.crawl_setting")],
    );

    let data = &received[1]["error"]["data"];
    assert_eq!(
        (&data["code"], &data["product"], data.get("install")),
        (
            &json!("not_in_product"),
            &json!("collie"),
            install.map(|install| json!(install)).as_ref()
        )
    );
}

/// Covers: story:3-136, error:not_in_product
#[test]
fn a_call_only_foxhound_carries_is_refused_on_a_collie_engine_naming_foxhounds_install() {
    let received = exchange(
        &engine(Product::Collie),
        &[hello("collie", "0.1"), request(1, "sample.crawl")],
    );

    insta::assert_snapshot!(pretty(&received[1]));
}

/// Covers: story:3-7
#[test]
fn a_call_of_an_operation_no_product_carries_is_method_not_found() {
    let received = exchange(
        &engine(Product::Foxhound),
        &[hello("collie", "0.1"), request(1, "page.levitate")],
    );

    assert_eq!(
        received[1]["error"],
        json!({
            "code": -32601,
            "message": "no Product carries the operation `page.levitate`",
            "data": { "code": "method_not_found", "retryable": false },
        })
    );
}

#[test]
fn a_connection_that_does_not_open_with_hello_is_refused_and_closed() {
    let received = exchange(
        &engine(Product::Collie),
        &[request(1, "engine.info"), hello("collie", "0.1")],
    );

    assert_eq!(
        received,
        [json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32600,
                "message": "a Connection opens with `hello`",
                "data": { "code": "invalid_request", "retryable": false },
            },
        })]
    );
}

/// Covers: operation:engine.info
#[test]
fn engine_info_answers_the_product_its_version_and_major_and_the_protocol_version() {
    let received = exchange(
        &engine(Product::Collie),
        &[hello("collie", "0.1"), request(1, "engine.info")],
    );

    assert_eq!(
        received[1]["result"],
        json!({
            "product": "collie",
            "product_version": "1.4.0",
            "protocol_version": "0.1",
            "product_major": 1,
        })
    );
}

/// Covers: story:3-6, error:call_cancelled
#[test]
fn cancel_answers_the_call_in_flight_with_call_cancelled_and_leaves_the_others() {
    let replies = attached(&engine(Product::Collie), |mut client| async move {
        client
            .send(&[
                hello("collie", "0.1"),
                request(1, "test.wait"),
                request(2, "test.sleep"),
                cancel(1),
            ])
            .await;
        let mut replies = Vec::new();
        while replies.len() < 3 {
            replies.push(client.next().await.unwrap());
        }
        client.close().await;
        replies.sort_by_key(|reply| reply["id"].as_i64());
        replies
    });

    assert_eq!(
        (&replies[1]["error"]["data"], &replies[2]["result"]),
        (
            &json!({ "code": "call_cancelled", "retryable": false, "by": "client" }),
            &json!({ "slept": true })
        )
    );
}

/// Covers: story:3-6
#[test]
fn cancel_of_a_finished_call_changes_nothing() {
    let replies = attached(&engine(Product::Collie), |mut client| async move {
        client
            .send(&[hello("collie", "0.1"), request(1, "engine.info")])
            .await;
        let (_, finished) = (client.next().await, client.next().await);
        client.send(&[cancel(1), request(2, "engine.info")]).await;
        client.close().await;
        let mut rest = Vec::new();
        while let Some(message) = client.next().await {
            rest.push(message["id"].clone());
        }
        (finished.unwrap()["result"]["product"].clone(), rest)
    });

    assert_eq!(replies, (json!("collie"), vec![json!(2)]));
}

#[test]
fn the_end_of_input_cancels_the_calls_in_flight_which_answer_before_the_connection_closes() {
    let received = exchange(
        &engine(Product::Collie),
        &[hello("collie", "0.1"), request(1, "test.wait")],
    );

    assert_eq!(
        received[1]["error"]["data"]["code"],
        json!("call_cancelled")
    );
}

#[test]
fn an_oversized_message_ends_in_its_limit_error_and_the_next_one_is_answered() {
    let oversized = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"engine.info\",\"params\":{{\"pad\":\"{}\"}}}}\n",
        "x".repeat(crate::rpc::framing::MAX_MESSAGE_BYTES)
    );

    let received = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        let _ = client.next().await;
        let Client {
            mut lines,
            mut writer,
        } = client;
        let (written, oversized_reply) =
            tokio::join!(writer.write_all(oversized.as_bytes()), lines.next_line());
        written.unwrap();
        let mut client = Client { lines, writer };
        client.send(&[request(2, "engine.info")]).await;
        client.close().await;
        let next = client.next().await.unwrap();
        (oversized_reply, next)
    });

    let oversized_reply: Value = serde_json::from_str(&received.0.unwrap().unwrap()).unwrap();
    assert_eq!(
        (
            &oversized_reply["id"],
            &oversized_reply["error"]["data"]["code"],
            &received.1["id"]
        ),
        (&Value::Null, &json!("message_too_large"), &json!(2))
    );
}

#[test]
fn deep_nesting_ends_in_its_limit_error() {
    let deep = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"engine.info\",\"params\":{}{}}}\n",
        "[".repeat(100),
        "]".repeat(100)
    );

    let received = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.send_bytes(deep.as_bytes()).await;
        client.close().await;
        let mut received = Vec::new();
        while let Some(message) = client.next().await {
            received.push(message);
        }
        received
    });

    assert_eq!(
        received[1]["error"]["data"]["code"],
        json!("message_too_deep")
    );
}

#[test]
fn every_failure_takes_a_predefined_code_or_one_of_the_servers_range() {
    let received = exchange(
        &engine(Product::Collie),
        &[
            hello("collie", "0.1"),
            request(1, "sample.crawl"),
            request(2, "page.levitate"),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "engine.info", "params": { "unknown": 1 } }),
            request(4, "test.wait"),
            cancel(4),
        ],
    );

    let codes: Vec<(i64, bool)> = received[1..]
        .iter()
        .map(|reply| {
            let code = reply["error"]["code"].as_i64().unwrap();
            let predefined = [-32700, -32600, -32601, -32602, -32603].contains(&code);
            (code, predefined || (-32099..=-32000).contains(&code))
        })
        .collect();
    assert!(codes.iter().all(|(_, valid)| *valid), "{codes:?}");
    assert!(
        received[1..]
            .iter()
            .all(|reply| reply["error"]["data"]["code"].is_string()
                && reply["error"]["data"]["retryable"].is_boolean()),
        "{received:#?}"
    );
}

/// A `tracing` writer that sends each write to the test.
#[derive(Clone)]
struct LogWriter(std::sync::mpsc::Sender<Vec<u8>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.send(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The records dispatch emits while `run` runs, sorted, each from its
/// target on.
fn dispatch_records<T>(run: impl FnOnce() -> T) -> Vec<String> {
    records_of(crate::rpc::DISPATCH_TARGET, run)
}

/// The records `run` logs under `target`, sorted, each without its level
/// and target. Its subscriber stands in for the test subscriber, so it also
/// writes the test's exercise record (ADR 0072).
fn records_of<T>(target: &str, run: impl FnOnce() -> T) -> Vec<String> {
    use tracing_subscriber::layer::SubscriberExt as _;

    let (sender, log) = std::sync::mpsc::channel();
    let writer = LogWriter(sender);
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .without_time()
                .with_writer(move || writer.clone()),
        )
        .with(goodfetch_test_support::exercise_layer(|| {
            std::time::SystemTime::UNIX_EPOCH
        }));
    // With one dispatcher alive, tracing-core judges a callsite by the
    // registering thread's own: another test's thread would cache that no
    // one listens to a record this test reads. A second one, alive for the
    // whole test, makes it judge every callsite by each thread's dispatcher.
    let _second = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());

    tracing::subscriber::with_default(subscriber, run);

    let mut records: Vec<String> = log
        .try_iter()
        .map(|bytes| String::from_utf8(bytes).unwrap())
        .filter_map(|line| {
            line.split_once(target)
                .map(|(_, record)| record.trim_end().to_owned())
        })
        .collect();
    records.sort();
    records
}

#[test]
fn dispatch_records_each_requests_method_and_its_error_code() {
    let records = dispatch_records(|| {
        exchange(
            &engine(Product::Collie),
            &[
                hello("collie", "0.1"),
                request(1, "engine.info"),
                request(2, "sample.crawl"),
            ],
        )
    });

    assert_eq!(
        records,
        [
            ": request answered method=\"engine.info\"",
            ": request answered method=\"hello\"",
            ": request failed method=\"sample.crawl\" error_code=\"not_in_product\"",
        ]
    );
}

/// A method no catalog holds is a value its Client wrote, which may hold a
/// Secret: its record names none (ADR 0132's bounded values).
///
/// Covers: story:3-105
#[test]
fn dispatch_records_a_method_no_catalog_holds_without_its_name() {
    let records = dispatch_records(|| {
        let engine = engine(Product::Collie);
        exchange(&engine, &[request(1, "token=GFCANARY-hunter2")]);
        exchange(
            &engine,
            &[hello("collie", "0.1"), request(2, "token=GFCANARY-hunter2")],
        )
    });

    assert_eq!(
        records,
        [
            ": request answered method=\"hello\"",
            ": request failed method=\"(unknown)\" error_code=\"invalid_request\"",
            ": request failed method=\"(unknown)\" error_code=\"method_not_found\"",
        ]
    );
}
