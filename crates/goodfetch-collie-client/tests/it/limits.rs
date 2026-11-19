//! Reaching a Limit through the client crate (ADRs 0041 and 0080), with
//! the Scripted engine as the Daemon playing the corpus's script of the
//! `limit-reached-stops-the-run` client conformance case, which lists the
//! Rust SDK once it has the Daemon transport: a Run started with a price
//! Limit sends it in millionths, the call that finds it used up fails with
//! `limit_reached` naming its Usage kind, scope and currency and where to
//! raise it, and the Run ends `stopped` and `incomplete`, its report
//! holding what it used.
//! Covers: story:11-51, story:11-52

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{Connection, ConnectionRuntime, Error, Limits, Money, UsageKind};
use goodfetch_protocol::run::{Completeness, Lifecycle, RunId, StopReason};
use goodfetch_test_support::TestEnv;
use serde_json::json;

use crate::scripted_daemon::{daemon_hello, scripted_daemon};

/// The corpus's script of the case, which the TypeScript SDK plays too.
const SCRIPT: &str = "run/limit-reached-stops-the-run.json";

#[test]
fn a_call_that_finds_the_runs_limit_used_up_fails_and_the_run_ends_stopped_and_incomplete() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, SCRIPT);
    let capped = RunId::parse("capped").unwrap();
    let limits = Limits::new().price(UsageKind::PROXY_BYTES, Money::new("1", "USD").unwrap());
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (connection, _said) = Connection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            daemon_hello(),
        )
        .await
        .unwrap();

        let run = connection
            .start()
            .run_id(&capped)
            .limits(limits)
            .await
            .unwrap();
        let page = run.new_page().await.unwrap();
        let refused = page.goto("https://shop.test/").await;
        let report = run.finish().await.unwrap();
        connection.close().await.unwrap();

        let Err(Error::Failed(failure)) = refused else {
            panic!("{refused:?}");
        };
        let details = &failure.data.details;
        assert_eq!(
            (
                failure.data.code.as_str(),
                &details["scope"],
                &details["usage_kind"],
                &details["currency"],
                details["fix"].is_string(),
            ),
            (
                "limit_reached",
                &json!({ "kind": "run", "id": "capped" }),
                &json!("proxy_bytes"),
                &json!("USD"),
                true,
            )
        );
        let Some(StopReason::Limit(reached)) = &report.stop_reason else {
            panic!("{:?}", report.stop_reason);
        };
        let used = serde_json::to_value(&report.usage).unwrap();
        assert_eq!(
            (
                report.lifecycle,
                report.completeness,
                reached.usage_kind.as_str(),
                reached.currency.as_deref(),
                (&used["requests"], &used["proxy_bytes"]),
            ),
            (
                Lifecycle::Stopped,
                Completeness::Incomplete,
                "proxy_bytes",
                Some("USD"),
                (&json!(1), &json!(4096)),
            )
        );
    });
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
