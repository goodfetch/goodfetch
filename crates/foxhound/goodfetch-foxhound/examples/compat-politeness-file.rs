//! The Politeness file's writer for the Compatibility corpus (ADR 0066):
//! runs fixed scenarios against Foxhound's engine, whose composition adds
//! the per-domain limits to the file, over the in-memory network, on the
//! test `Clock` and a fixed `Entropy` seed, in a home of its own. A crawl of
//! `shop.test` under a one-second delay meets a 429 on its first request,
//! which halves the domain's concurrency and backs it off, and goes on once
//! the backoff ended, its second page meeting a 503 that backs it off again
//! (the backoff's last wait kept); then a crawl of `hold.test` with one request in flight
//! at a time leaves its first request unanswered, so its second waits for a
//! wake. A copy of the file taken then, its Run folder and wake endpoint
//! written relative to the home, goes as SQL text into the folder given as
//! the only argument: `politeness-file.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime, writes files and runs on the test Clock (ADRs 0059 and 0066)"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader, Lines};

type Messages = Lines<BufReader<goodfetch_engine::transport::ConnectionReader>>;

fn call(id: i64, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The handler's answer to one handler request: a 429 for `shop.test`'s
/// first page, a 503 for its second, Records for every other, and none for
/// `hold.test`'s first, which stays in flight.
fn handle(request: &Value) -> Option<Value> {
    let id = &request["id"];
    match request["params"]["crawl_request"]["url"].as_str() {
        Some("https://hold.test/1") => None,
        Some("https://shop.test/1") => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32003,
                "message": "too many requests",
                "data": { "code": "blocked", "retryable": false, "cause": "rate_limit" },
            },
        })),
        Some("https://shop.test/2") => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32003,
                "message": "service unavailable",
                "data": { "code": "response_unavailable", "retryable": false, "status": 503 },
            },
        })),
        _ => Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "records": [] } })),
    }
}

fn dependencies(home: &Path) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()),
        // 2026-01-05T09:00:00Z, test support's `START_TIME`.
        clock: Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001)),
        entropy: Entropy::seeded(0),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: PolitenessOptions::default(),
    }
}

fn line(message: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    Ok(line)
}

/// Sends `message` and answers each handler request until its reply.
async fn exchange(
    writer: &mut (impl AsyncWrite + Unpin),
    messages: &mut Messages,
    message: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    writer.write_all(&line(message)?).await?;
    loop {
        let received: Value = serde_json::from_str(
            &messages
                .next_line()
                .await?
                .ok_or("the engine closed early")?,
        )?;
        if received["method"] == "crawl_request.handle" {
            if let Some(answer) = handle(&received) {
                writer.write_all(&line(&answer)?).await?;
            }
        } else if received["id"] == message["id"] {
            if received.get("error").is_some() {
                return Err(format!("the scenario failed: {received}").into());
            }
            return Ok(());
        }
    }
}

/// Copies the Politeness file of `home` into `copy`, its Run folders and
/// wake endpoints written relative to the home, so the corpus never names
/// a temporary folder.
fn copy_of(home: &Path, copy: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let file = rusqlite::Connection::open(home.join("politeness.sqlite"))?;
    file.execute("VACUUM INTO ?1", [copy.to_str().ok_or("not Unicode")?])?;
    let copy = rusqlite::Connection::open(copy)?;
    let prefix = format!("{}/", home.display());
    copy.execute(
        "UPDATE permits SET run_folder = replace(run_folder, ?1, '')",
        [&prefix],
    )?;
    copy.execute("UPDATE waiters SET endpoint = 'wake.sock'", [])?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: compat-politeness-file <folder>")?,
    );
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("0.0.0")?,
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &goodfetch_foxhound::composition())?;
    let home = tempfile::tempdir()?;
    let copy = home.path().join("copy.sqlite");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await?.split();
            let mut messages = BufReader::new(reader).lines();
            exchange(
                &mut writer,
                &mut messages,
                &call(
                    0,
                    "hello",
                    &json!({ "protocol_version": "0.1", "surface": "rust", "needs": "foxhound", "product_version": "0.0.0" }),
                ),
            )
            .await?;
            exchange(
                &mut writer,
                &mut messages,
                &call(
                    1,
                    "crawl.start",
                    &json!({
                        "run_id": "throttled",
                        "seeds": [
                            { "url": "https://shop.test/1", "label": "page" },
                            { "url": "https://shop.test/2", "label": "page" },
                        ],
                        "settings": { "politeness": { "domain": { "concurrency": 2, "delay": 1_000 } } },
                    }),
                ),
            )
            .await?;
            exchange(
                &mut writer,
                &mut messages,
                &call(2, "run.finish", &json!({ "run_id": "throttled" })),
            )
            .await?;
            exchange(
                &mut writer,
                &mut messages,
                &call(
                    3,
                    "crawl.start",
                    &json!({
                        "run_id": "holding",
                        "seeds": [
                            { "url": "https://hold.test/1", "label": "page" },
                            { "url": "https://hold.test/2", "label": "page" },
                        ],
                        "settings": { "politeness": { "domain": { "concurrency": 1 } } },
                    }),
                ),
            )
            .await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            copy_of(home.path(), &copy)?;
            exchange(
                &mut writer,
                &mut messages,
                &call(4, "run.abort", &json!({ "run_id": "holding" })),
            )
            .await?;
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
    let connection = rusqlite::Connection::open(&copy)?;
    std::fs::write(
        folder.join("politeness-file.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    Ok(())
}
