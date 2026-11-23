//! The handler-less starts (ADRs 0036 and 0102) through the client crate,
//! with the Scripted engine as the Daemon playing the script the Python SDK
//! plays too: a Recipe's run and a code-free catalog run of each target
//! kind, started with every start option, two awaited and two through
//! `blocking`. The Daemon checks every message the crate sends and fails on
//! a step left unused, so its clean exit says each start carried exactly
//! what the script holds. Each Run is detached, so closing its value and
//! the Connection sends no `run.abort`: the Daemon keeps the Run once its
//! program is gone. A view `open_run` opens sends nothing either, opening
//! or closing.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::num::NonZeroUsize;
use std::time::Duration;

use goodfetch_collie_client::settings::{Settings, timeout};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{
    ClientConnection, ConnectionRuntime, ErrorCode, Limits, Run, UsageKind, blocking,
};
use goodfetch_protocol::run::{CatalogTarget, Recording, RecordingMode, RunId, StartedAs, Tags};
use goodfetch_test_support::{TestEnv, real_time_runtime};
use serde_json::json;

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// The four starts, each detached, then the Client's close with no
/// `run.abort`.
const SCRIPT: &str = "sdk-python/run/detached-starts-from-a-daemon.json";

/// The Run ID `text` names.
fn id(text: &str) -> RunId {
    RunId::parse(text).unwrap()
}

/// Each start sends every option given, closing stops no detached Run, and
/// a view opens by the Run's ID with no round trip until the Connection
/// closes.
/// Covers: operation:recipe.start, operation:catalog_run.start
#[test]
fn a_handlerless_start_takes_every_option_and_leaves_its_detached_run() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, SCRIPT);
    let one = NonZeroUsize::MIN;
    let owned = goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();
    let runtime = real_time_runtime();
    let (opened, _said) = runtime
        .block_on(ClientConnection::open(
            ConnectionRuntime::owned(owned),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        ))
        .unwrap();

    let (recipe, extract) = runtime.block_on(awaited_starts(&opened));
    let view = opened.open_run(&id("nightly-prices")).unwrap();
    assert_eq!(view.id().as_str(), "nightly-prices");
    view.close();
    let connection = blocking::ClientConnection::from(opened);
    let (agent, replay) = blocking_starts(&connection);
    let view = connection.open_run(&id("checkout-replay")).unwrap();
    assert_eq!(view.id().as_str(), "checkout-replay");
    view.close();
    connection.close().unwrap();
    let status = process.child().wait().unwrap();
    let after = connection.open_run(&id("checkout-replay"));

    let started = [
        (recipe.id().as_str(), recipe.started_as()),
        (extract.id().as_str(), extract.started_as()),
        (agent.id().as_str(), agent.started_as()),
        (replay.id().as_str(), replay.started_as()),
    ];
    assert_eq!(
        started,
        [
            ("nightly-prices", StartedAs::New),
            ("extract-products", StartedAs::New),
            ("cheapest-kettle", StartedAs::New),
            ("checkout-replay", StartedAs::New),
        ]
    );
    assert!(status.success(), "the Daemon heard no `run.abort`");
    assert!(
        matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
        "a closed Connection opens no view: {after:?}"
    );
}

/// The script's first two starts, awaited: a Recipe's run with every
/// option, closed at once, and an Extraction.
async fn awaited_starts(opened: &ClientConnection) -> (Run, Run) {
    let recipe = opened
        .start_recipe("shop-prices")
        .run_id(&id("nightly-prices"))
        .tags(Tags::parse(["nightly", "prices"]).unwrap())
        .limits(Limits::new().count(UsageKind::REQUESTS, 500))
        .settings(Settings::new().set(timeout::WAIT, Duration::from_secs(5)))
        .recording(Recording {
            mode: RecordingMode::Replay,
            path: "recordings/shop.warc".to_owned(),
        })
        .timeout(Duration::from_secs(3600))
        .detached()
        .await
        .unwrap();
    let extract = opened
        .start_catalog_run(CatalogTarget::Extract {
            url: "https://shop.example/products".to_owned(),
            schema: json!({
                "type": "object",
                "properties": { "productName": { "type": "string" } }
            }),
            instruction: Some("Read each product's name.".to_owned()),
        })
        .run_id(&id("extract-products"))
        .detached()
        .await
        .unwrap();
    recipe.close().await.unwrap();
    (recipe, extract)
}

/// The script's last two starts, through `blocking`: an Agent task with
/// no timeout and a Plan replay that records.
fn blocking_starts(connection: &blocking::ClientConnection) -> (blocking::Run, blocking::Run) {
    let agent = connection
        .start_catalog_run(CatalogTarget::Agent {
            url: "https://shop.example/".to_owned(),
            task: "Find the cheapest kettle.".to_owned(),
            expect: Some(json!({ "found": true })),
            schema: Some(json!({
                "type": "object",
                "properties": { "unitPrice": { "type": "number" } }
            })),
            max_steps: Some(20),
        })
        .run_id(&id("cheapest-kettle"))
        .no_timeout()
        .detached()
        .send()
        .unwrap();
    let replay = connection
        .start_catalog_run(CatalogTarget::PlanReplay {
            plan: "plans/checkout.json".to_owned(),
        })
        .run_id(&id("checkout-replay"))
        .recording(Recording {
            mode: RecordingMode::Record,
            path: "recordings/checkout.warc".to_owned(),
        })
        .detached()
        .send()
        .unwrap();
    (agent, replay)
}
