//! Results a newer engine answers (ADR 0082): the typed view of a result
//! ignores a field this SDK does not know, and the result keeps it, read
//! through `raw()`. The scripted case plays the corpus's script of the
//! `result-keeps-unknown-fields` client conformance case, which the
//! TypeScript SDK plays too, with the Scripted engine as the Daemon: it
//! fails on a message it does not expect and on a step left unused.

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime};
use goodfetch_protocol::run::{Lifecycle, RunId};
use goodfetch_test_support::TestEnv;
use serde_json::json;

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// The `result-keeps-unknown-fields` client conformance case, played by
/// the Scripted engine's Daemon: the engine answers `run.start` and
/// `run.finish` each with a field the SDK does not know, and the Run report
/// keeps its own in `raw()` while its typed view reads the rest.
/// Covers: story:11-68
#[test]
fn a_result_keeps_the_fields_the_sdk_does_not_know() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/result-keeps-unknown-fields.json");
    let newer = RunId::parse("newer").unwrap();
    let report = goodfetch_test_support::real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        )
        .await
        .unwrap();
        let run = connection.start().run_id(&newer).await.unwrap();
        let report = run.finish().await.unwrap();
        connection.close().await.unwrap();
        report
    });

    assert_eq!(
        (&report.lifecycle, &report.raw()["carbon"]),
        (&Lifecycle::Finished, &json!({"grams": 1.5}))
    );
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
