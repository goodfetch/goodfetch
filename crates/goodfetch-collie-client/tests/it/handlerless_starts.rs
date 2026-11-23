//! The handler-less starts (ADRs 0036 and 0102) through the client crate,
//! with the Scripted engine as the Daemon playing the script the Python SDK
//! plays too, the `handlerless-starts` client conformance case: a Recipe's
//! run and a code-free catalog run of each target kind, started with every
//! start option, four awaited and two through `blocking`. A Run ID resumes
//! an interrupted Run, logged at info, or returns a finished one ended,
//! logged at warning. The Daemon checks every message the crate sends and
//! fails on a step left unused, so its clean exit says each start carried
//! exactly what the script holds. Each Run is detached, so closing its
//! value and the Connection sends no `run.abort`: the Daemon keeps the Run
//! once its program is gone. A view `open_run` opens sends nothing either,
//! opening or closing. The Scripted engine leaves no exercise record of
//! what it plays, so the CLI's handler-less start test declares the two
//! starts.
//! Covers: story:11-131

use std::num::NonZeroUsize;
use std::time::Duration;

use goodfetch_collie_client::settings::{Settings, timeout};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{
    ClientConnection, ConnectionRuntime, ErrorCode, Limits, Run, UsageKind, blocking,
};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::{CatalogTarget, Recording, RecordingMode, RunId, StartedAs, Tags};
use goodfetch_test_support::{TestEnv, case_logs, logged, real_time_runtime};
use serde_json::json;

use crate::scripted_daemon::{assert_start_lines, scripted_daemon_hearing, traced_daemon_hello};

/// The six starts, each detached, then the Client's close with no
/// `run.abort`.
const SCRIPT: &str = "sdk-python/run/detached-starts-from-a-daemon.json";

/// The Run ID `text` names.
fn id(text: &str) -> RunId {
    RunId::parse(text).unwrap()
}

/// What the test played: each start's Run, then the Daemon's exit and a
/// view opened once the Connection closed.
struct Played {
    started: Vec<(String, StartedAs)>,
    success: bool,
    after: Result<(), ErrorCode>,
}

/// Each start sends every option given, a resume and an ended Run are
/// logged, closing stops no detached Run, and a view opens by the Run's ID
/// with no round trip until the Connection closes.
#[test]
fn a_handlerless_start_takes_every_option_and_leaves_its_detached_run() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "handlerless-starts",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon_hearing(&env, SCRIPT, &traced_daemon_hello());
    let (played, log) = logged(|| {
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
                traced_daemon_hello(),
            ))
            .unwrap();

        let [recipe, extract, resumed, ended] = runtime.block_on(awaited_starts(&opened));
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
        let after = connection
            .open_run(&id("checkout-replay"))
            .map(|_| ())
            .map_err(|error| error.code());
        let awaited = [recipe, extract, resumed, ended]
            .map(|run| (run.id().as_str().to_owned(), run.started_as()));
        let blocked = [agent, replay].map(|run| (run.id().as_str().to_owned(), run.started_as()));
        Played {
            started: awaited.into_iter().chain(blocked).collect(),
            success: status.success(),
            after,
        }
    });
    let log = String::from_utf8(log).unwrap();

    case.check(
        "each start answers its Run as the engine started it",
        || {
            let started: Vec<(&str, StartedAs)> = played
                .started
                .iter()
                .map(|(id, started_as)| (id.as_str(), *started_as))
                .collect();
            assert_eq!(
                started,
                [
                    ("nightly-prices", StartedAs::New),
                    ("extract-products", StartedAs::New),
                    ("weekly-prices", StartedAs::Resumed),
                    ("checkout-done", StartedAs::Ended),
                    ("cheapest-kettle", StartedAs::New),
                    ("checkout-replay", StartedAs::New),
                ]
            );
        },
    );
    case.check(
        "the resume is logged at info, the ended Run at warning, on the start's target",
        || assert_start_lines(&log, "weekly-prices", "checkout-done"),
    );
    case.check(
        "the Daemon heard every option and no run.abort, so it played its script whole",
        || assert!(played.success, "the Daemon heard no `run.abort`"),
    );
    case.check("a closed Connection opens no view", || {
        assert_eq!(played.after, Err(ErrorCode::CLOSED));
    });
}

/// The script's first four starts, awaited: a Recipe's run with every
/// option, closed at once, an Extraction, a Recipe's run its Run ID
/// resumes and a Plan replay its Run ID finds ended.
async fn awaited_starts(opened: &ClientConnection) -> [Run; 4] {
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
    let resumed = opened
        .start_recipe("shop-prices")
        .run_id(&id("weekly-prices"))
        .detached()
        .await
        .unwrap();
    let ended = opened
        .start_catalog_run(CatalogTarget::PlanReplay {
            plan: "plans/checkout.json".to_owned(),
        })
        .run_id(&id("checkout-done"))
        .detached()
        .await
        .unwrap();
    recipe.close().await.unwrap();
    [recipe, extract, resumed, ended]
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
