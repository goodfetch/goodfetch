//! Run state's writer for the Compatibility corpus (ADR 0066): runs fixed
//! scenarios against Foxhound's engine, whose run files hold every table
//! both Products' compositions add, over the in-memory network, on the test
//! `Clock` and a fixed `Entropy` seed, in a home of its own: a tagged crawl
//! of three seeds, one with a Payload and one the Frontier answers
//! `duplicate`, whose handler pushes Records for the first and fails on the
//! second, whose first byte never came, so its
//! domain still backs off when `run.finish` ends it; and a Run its
//! program drives, under an Operator policy and with a setting that
//! weakens a guarantee, which `run.abort` stops. It writes each Run's file, as
//! SQL text, into the folder given as the only argument: `crawl.sql` and
//! `aborted.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime, writes files and runs on the test Clock (ADRs 0059 and 0066)"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
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
        call(4, "run.abort", json!({ "run_id": "aborted" })),
    ]
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
    let engine = Engine::new(identity, &goodfetch_foxhound::composition())?;
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
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await?.split();
            let mut messages = BufReader::new(reader).lines();
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
                        break;
                    }
                }
            }
            writer.shutdown().await?;
            while messages.next_line().await?.is_some() {}
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (served, talked) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client
        );
        served?;
        talked
    })?;
    for run_id in ["crawl", "aborted"] {
        let file = home
            .path()
            .join("0.0")
            .join("runs")
            .join(run_id)
            .join("run.sqlite");
        let connection = rusqlite::Connection::open(file)?;
        std::fs::write(
            folder.join(format!("{run_id}.sql")),
            goodfetch_test_support::sql_dump(&connection)?,
        )?;
    }
    Ok(())
}
