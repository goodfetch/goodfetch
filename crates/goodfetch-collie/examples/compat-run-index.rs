//! The Run index's writer for the Compatibility corpus (ADR 0066): runs a
//! fixed scenario against Collie's engine over the in-memory network, on the
//! test `Clock` and a fixed `Entropy` seed, in a home of its own (a Run that
//! finishes with a progress message and one that is aborted, both tagged,
//! on an Attached engine, the first meeting a consent wall, with a "Needs
//! you" item a person dismisses, then a
//! Run a Daemon owns while the Daemon is killed, whose row names the
//! Daemon's endpoint), and writes the Run index it leaves, as SQL text, into
//! the folder given as the only argument, as `run-index.sql`. `xtask compat
//! write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime, writes files and runs on the test Clock (ADRs 0059 and 0066)"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::{DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::needs_you::{
    Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError,
};
use goodfetch_engine::ops::Ops;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Contribution, Dependencies, Engine, Handler, Identity,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::{RUN_INDEX, per_major};
use goodfetch_protocol::needs_you::NeedsYouItem;
use goodfetch_protocol::run::{PointInTime, RunId};
use goodfetch_test_client::Hello;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// What the Client asks, in order; each call waits for its reply.
fn scenario() -> Vec<Value> {
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    vec![
        Hello::needing(Product::Collie)
            .surface(Surface::Rust)
            .message(),
        call(
            1,
            "run.start",
            json!({ "run_id": "finished-run", "tags": ["schedule:nightly", "team:shop"] }),
        ),
        call(
            2,
            "run.set_progress_message",
            json!({ "run_id": "finished-run", "message": "Read 3 of 3 pages" }),
        ),
        call(
            30,
            "compat.consent_wall",
            json!({ "run_id": "finished-run", "domain": "walled.example" }),
        ),
        call(
            31,
            "compat.refetch_stopped",
            json!({ "run_id": "finished-run", "domain": "walled.example", "cause": "challenge" }),
        ),
        call(3, "run.finish", json!({ "run_id": "finished-run" })),
        call(
            4,
            "run.start",
            json!({
                "run_id": "aborted-run",
                "tags": ["manual"],
                "project": {
                    "path": "/corpus/project/goodfetch.toml",
                    "settings": { "wall": { "refetch_domains": ["walled.example"] } },
                },
            }),
        ),
        call(
            32,
            "compat.refetch_stopped",
            json!({ "run_id": "aborted-run", "domain": "walled.example", "cause": "status_403" }),
        ),
        call(5, "run.abort", json!({ "run_id": "aborted-run" })),
        call(6, "needs_you.dismiss", json!({ "item": WAITING })),
    ]
}

/// Records that a Run met a consent wall on a domain, as the page crate
/// does when its Consent prompt runner leaves one.
const CONSENT_WALL: Operation = Operation {
    name: "compat.consent_wall",
    description: "Records that a Run met a consent wall on a domain.",
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
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

async fn consent_wall(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap_or_default())
        .expect("invariant: the scenario names a valid Run ID");
    let run = context
        .runs()
        .live(run_id)
        .await
        .expect("invariant: the scenario's Run is live");
    run.record_consent_wall(params["domain"].as_str().unwrap_or_default().to_owned())
        .await
        .expect("invariant: the corpus's Run index is written");
    Ok(json!({}))
}

/// Records that a Run's refetch without the site's stored data stopped on
/// a domain, as the Wall allowance does when the site refuses it.
const REFETCH_STOPPED: Operation = Operation {
    name: "compat.refetch_stopped",
    description: "Records that a Run's refetch stopped on a domain.",
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
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

async fn refetch_stopped(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap_or_default())
        .expect("invariant: the scenario names a valid Run ID");
    let run = context
        .runs()
        .live(run_id)
        .await
        .expect("invariant: the scenario's Run is live");
    run.record_refetch_stop(
        params["domain"].as_str().unwrap_or_default().to_owned(),
        params["cause"].as_str().unwrap_or_default().to_owned(),
    )
    .await
    .expect("invariant: the corpus's Run index is written");
    Ok(json!({}))
}

/// The ID of the corpus's "Needs you" item, which a person dismisses.
const WAITING: &str = "corpus_waiting:1";

/// A source of one "Needs you" item a person may dismiss, which never
/// changes: the Run index keeps its dismissal.
struct Waiting;

impl Primitives for Waiting {
    fn dismissable(&self) -> bool {
        true
    }

    fn items<'a>(
        &'a self,
        _: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        Box::pin(async {
            Ok(vec![NeedsYouItem {
                id: WAITING.to_owned(),
                kind: "corpus_waiting".to_owned(),
                since: PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned()),
                run: None,
                schedule: None,
                summary: "The corpus waits for a person.".to_owned(),
                actions: Vec::new(),
                details: Map::new(),
            }])
        })
    }

    fn changes(&self, _: &SourceContext) -> Changes {
        Changes::none()
    }
}

/// The Daemon's token, which its Client's `hello` carries.
const TOKEN: &str = "corpus";

/// What the Daemon's Client asks before the Daemon is killed.
fn daemon_scenario() -> Vec<Value> {
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    vec![
        Hello::needing(Product::Collie)
            .surface(Surface::Cli)
            .token(TOKEN)
            .message(),
        call(
            1,
            "run.start",
            json!({ "run_id": "live-run", "tags": ["daemon"] }),
        ),
    ]
}

/// A Daemon that is its major's pick and never hands over.
struct ThePick;

impl Succession for ThePick {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

/// Sends each call of `scenario` on `connection` and waits for its reply.
async fn play(
    connection: transport::Connection,
    scenario: Vec<Value>,
) -> Result<transport::ConnectionWriter, Box<dyn std::error::Error>> {
    let (reader, mut writer) = connection.split();
    let mut replies = BufReader::new(reader).lines();
    for message in scenario {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        writer.write_all(&line).await?;
        let reply: Value = serde_json::from_str(
            &replies
                .next_line()
                .await?
                .ok_or("the engine closed early")?,
        )?;
        if reply.get("error").is_some() {
            return Err(format!("the scenario failed: {reply}").into());
        }
    }
    Ok(writer)
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
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: compat-run-index <folder>")?,
    );
    let version = ProductVersion::parse("0.0.0")?;
    let identity = Identity {
        product: Product::Collie,
        product_version: version.clone(),
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(
        identity,
        &goodfetch_collie::composition().with(
            Contribution::new("compat-run-index")
                .with_handler(Handler::new(&CONSENT_WALL, consent_wall))
                .with_handler(Handler::new(&REFETCH_STOPPED, refetch_stopped))
                .with_needs_you_source(NeedsYouSource::new(Waiting)),
        ),
    )?;
    let home = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let mut writer = play(dialer.connect().await?, scenario()).await?;
            writer.shutdown().await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (served, talked) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client
        );
        served?;
        talked
    })?;
    // The Daemon is killed with its Run live: its runtime is dropped.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let (listener, endpoint) = Network::InMemory(network)
            .listen(Path::new("daemon.sock"))?;
        let options = DaemonOptions {
            token: TOKEN.to_owned(),
            endpoint,
            pid: 4242,
            idle_timeout: Duration::from_hours(1),
            keep_alive: true,
            holds: Holds::new(),
            succession: Arc::new(ThePick),
            drain: Drain {
                asked: None,
                grace: Duration::from_secs(110),
            },
            install: None,
            ops: Ops::default(),
            websocket: None,
        };
        let serving = engine.run_daemon(listener, dependencies(home.path()), options);
        tokio::select! {
            _ = serving => Err("the Daemon ended".into()),
            played = async { play(dialer.connect().await?, daemon_scenario()).await } => played.map(drop),
        }
    })?;
    drop(runtime);
    // The folder of the engine's own pinned version, not of the workspace's.
    let index = rusqlite::Connection::open(per_major(home.path(), &version).join(RUN_INDEX))?;
    std::fs::write(
        folder.join("run-index.sql"),
        goodfetch_test_support::sql_dump(&index)?,
    )?;
    Ok(())
}
