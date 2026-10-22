//! Run state's writer for the Compatibility corpus (ADR 0066): runs fixed
//! scenarios against Foxhound's engine, whose run files hold every table
//! both Products' compositions add, over the in-memory network, on the test
//! `Clock` and a fixed `Entropy` seed, in a home of its own: under an
//! install Limit, whose Slices its run state journals, a tagged crawl of
//! three seeds, one with a Payload and one the Frontier answers
//! `duplicate`, whose handler pushes Records for the first and fails on the
//! second, whose first byte never came, so its
//! domain still backs off when `run.finish` ends it; and a Run its
//! program drives, under an Operator policy and with a setting that
//! weakens a guarantee, whose Key-value store keeps a cursor, which
//! `run.abort` stops; and a Run bound to a
//! Browser host and holding to a Clean-up rules digest, started through a
//! contribution of the writer's own since no Client sets those yet, which
//! `run.checkpoint` checkpoints and `run.pause` pauses with a Checkpoint.
//! It writes each Run's file, as SQL text, into the folder given as the
//! only argument: `crawl.sql`, `aborted.sql` and `paused.sql`; and the
//! pause's real Checkpoint file, `checkpoint.sqlite`, with the Checkpoint
//! the pause reported, `checkpoint.json`, its location the file's name
//! (ADR 0066). `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime, writes files and runs on the test Clock (ADRs 0059 and 0066)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::runs::{EndedBy, Start};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Contribution, Dependencies, Engine, Handler, Identity,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::checkpoint::RunBinding;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// What the Client asks, in order; each call waits for its reply.
fn scenario() -> Vec<Value> {
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    vec![
        call(
            0,
            "hello",
            json!({ "protocol_version": "0.1", "surface": "rust", "needs": "foxhound", "product_version": "0.0.0" }),
        ),
        call(
            10,
            "usage.set",
            json!({ "scope": "install", "usage_kind": "requests", "amount": 1_000 }),
        ),
        call(
            1,
            "crawl.start",
            json!({
                "run_id": "crawl",
                "tags": ["schedule:nightly"],
                "seeds": [
                    { "url": "https://shop.test/", "label": "home", "payload": { "depth": 0 } },
                    { "url": "https://shop.test/broken", "label": "product" },
                    { "url": "https://SHOP.test:443/", "label": "home" },
                ],
            }),
        ),
        call(2, "run.finish", json!({ "run_id": "crawl" })),
        call(
            3,
            "run.start",
            json!({
                "run_id": "aborted",
                "tags": ["manual"],
                "settings": { "tls": { "accept_invalid_certs": { "intranet.test": null } } },
            }),
        ),
        call(
            11,
            "key_value_store.set",
            json!({ "run_id": "aborted", "key": "cursor", "value": { "page": 2, "seen": [1, null] } }),
        ),
        call(4, "run.abort", json!({ "run_id": "aborted" })),
        call(5, "compat.bound", json!({})),
        call(6, "run.checkpoint", json!({ "run_id": "paused" })),
        call(7, "run.pause", json!({ "run_id": "paused" })),
    ]
}

/// The digest of the Clean-up rules the bound Run records.
const CLEAN_UP_DIGEST: &str = "6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b";

/// Starts the Run `paused`, bound to a Browser host and holding to
/// [`CLEAN_UP_DIGEST`], which no Client's call sets yet.
const BOUND: Operation = Operation {
    name: "compat.bound",
    description: "Starts a Run bound to a Browser host with a Clean-up rules digest.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

fn bound() -> Contribution {
    Contribution::new("compat-run-state").with_handler(Handler::new(
        &BOUND,
        |_: Value, context: CallContext| async move {
            context
                .runs()
                .start(Start {
                    run_id: Some(RunId::parse("paused").expect("a Run ID")),
                    tags: Tags::default(),
                    kind: RunKind::Collie,
                    needs: Product::Collie,
                    inputs: BTreeMap::new(),
                    ended_by: EndedBy::Finish,
                    initialize: None,
                    policy_version: None,
                    weakened_by: Vec::new(),
                    binding: Some(RunBinding::BrowserHost {
                        browser_host: "laptop".to_owned(),
                    }),
                    clean_up_digest: Some(CLEAN_UP_DIGEST.to_owned()),
                    checkpoint_interval: None,
                    timeout: None,
                    settings: None,
                })
                .await
                .map_err(goodfetch_engine::run::start_error)?;
            Ok::<Value, CallError>(json!({ "run_id": "paused" }))
        },
    ))
}

/// The handler's answer to one handler request.
fn handle(request: &Value) -> Value {
    let id = &request["id"];
    if request["params"]["crawl_request"]["label"] == "home" {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "records": [
                { "dataset": "pages", "data": { "url": "https://shop.test/", "title": "Shop" } },
                { "dataset": "links", "data": { "href": "https://shop.test/broken" } },
            ] },
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32005,
                "message": "no first byte",
                "data": {
                    "code": "deadline_exceeded",
                    "retryable": false,
                    "limit": { "key": "timeout.first_byte", "value": 30_000, "layer": "default" },
                    "elapsed": 30_000,
                    "waited_for": "first_byte",
                },
            },
        })
    }
}

fn dependencies(home: &Path) -> Dependencies {
    Dependencies {
        home: Home::new(home.to_owned()),
        // 2026-01-05T09:00:00Z, test support's `START_TIME`.
        clock: Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001)),
        entropy: Entropy::seeded(0),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

fn line(message: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    Ok(line)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: compat-run-state <folder>")?,
    );
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("0.0.0")?,
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &goodfetch_foxhound::composition().with(bound()))?;
    let home = tempfile::tempdir()?;
    std::fs::create_dir_all(home.path().join("0.0"))?;
    std::fs::write(
        home.path().join("0.0").join("operator-policy.toml"),
        "version = \"0.1\"\n",
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    let paused = runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await?.split();
            let mut messages = BufReader::new(reader).lines();
            let mut last = Value::Null;
            for message in scenario() {
                writer.write_all(&line(&message)?).await?;
                loop {
                    let received: Value = serde_json::from_str(
                        &messages
                            .next_line()
                            .await?
                            .ok_or("the engine closed early")?,
                    )?;
                    if received["method"] == "crawl_request.handle" {
                        writer.write_all(&line(&handle(&received))?).await?;
                    } else if received["id"] == message["id"] {
                        if received.get("error").is_some() {
                            return Err(format!("the scenario failed: {received}").into());
                        }
                        last = received;
                        break;
                    }
                }
            }
            writer.shutdown().await?;
            while messages.next_line().await?.is_some() {}
            Ok::<_, Box<dyn std::error::Error>>(last)
        };
        let (served, talked) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client
        );
        served?;
        talked
    })?;
    let runs = home.path().join("0.0").join("runs");
    for run_id in ["crawl", "aborted", "paused"] {
        let connection = rusqlite::Connection::open(runs.join(run_id).join("run.sqlite"))?;
        std::fs::write(
            folder.join(format!("{run_id}.sql")),
            goodfetch_test_support::sql_dump(&connection)?,
        )?;
    }
    std::fs::copy(
        runs.join("paused").join("checkpoint.sqlite"),
        folder.join("checkpoint.sqlite"),
    )?;
    let mut checkpoint = paused["result"]["checkpoint"].clone();
    checkpoint["location"] = json!("checkpoint.sqlite");
    let mut text = serde_json::to_string_pretty(&checkpoint)?;
    text.push('\n');
    std::fs::write(folder.join("checkpoint.json"), text)?;
    Ok(())
}
