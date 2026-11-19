//! Limits in code through the Rust SDK (ADRs 0041, 0080 and 0102), the
//! `limits` client conformance cases in `linked` mode: the connection's
//! Limits travel as one `usage.set` each on its own scope before connect
//! returns, a Run's travel in its `run.start`, each price in millionths of
//! its currency; a Limit the engine refuses fails the connect; and a start
//! that names a Run again lowers its Limits and never raises them.

use goodfetch_collie::linked::{Composed, LinkedConnection};
use goodfetch_collie::{Collie, Error, Limits, Money, RunId, StartedAs, UsageKind, blocking};
use goodfetch_collie_client::ClientConnection;
use goodfetch_protocol::usage::{
    LimitRaiseRefusedDetails, USAGE_LIST, UsageListParams, UsageListResult,
};
use goodfetch_test_support::{TestEnv, real_time_runtime};
use serde_json::json;

use super::options;

/// Each Limit of the ledger on a scope of `kind`, as its Usage kind,
/// currency and amount, in the order `usage.list` gives them.
async fn listed(connection: &ClientConnection, kind: &str) -> Vec<(String, Option<String>, u64)> {
    let listed: UsageListResult = connection
        .call(USAGE_LIST.name, &UsageListParams {})
        .await
        .unwrap();
    listed
        .limits
        .into_iter()
        .filter(|usage| usage.limit.scope.kind == kind)
        .map(|usage| {
            (
                usage.limit.usage_kind,
                usage.limit.currency,
                usage.limit.amount,
            )
        })
        .collect()
}

/// The `limits-sent-in-millionths` case: a count goes as given, a price in
/// millionths of its currency, at connect and at a Run's start.
/// Covers: story:11-51
#[test]
fn limits_reach_the_engine_at_connect_and_at_a_runs_start_with_each_price_in_millionths() {
    let env = TestEnv::new();
    let connect = Limits::new()
        .count(UsageKind::REQUESTS, 500)
        .price(UsageKind::PAID_SOLVES, Money::new("0.50", "USD").unwrap());
    let run = Limits::new().price(
        UsageKind::PROXY_BYTES,
        Money::new("1.000001", "EUR").unwrap(),
    );
    real_time_runtime().block_on(async {
        let linked = LinkedConnection::connect(Composed::COLLIE, options(&env).limits(connect))
            .await
            .unwrap();

        let started = linked.connection().start().limits(run).await.unwrap();
        let on_connection = listed(linked.connection(), "connection").await;
        let on_run = listed(linked.connection(), "run").await;
        started.finish().await.unwrap();
        linked.close().await.unwrap();

        assert_eq!(
            (on_connection, on_run),
            (
                vec![
                    ("requests".to_owned(), None, 500),
                    ("paid_solves".to_owned(), Some("USD".to_owned()), 500_000),
                ],
                vec![("proxy_bytes".to_owned(), Some("EUR".to_owned()), 1_000_001)],
            )
        );
    });
}

/// A connection's Limit holds its Runs: with nothing left, a start is
/// refused with `limit_reached` on the connection's scope, through
/// `blocking` too.
/// Covers: story:11-52, error:limit_reached
#[test]
fn a_connection_limit_with_nothing_left_refuses_a_runs_start() {
    let env = TestEnv::new();
    let options = options(&env).limits(Limits::new().count(UsageKind::REQUESTS, 0));
    let collie = blocking::Collie::connect(options).unwrap();

    let refused = collie.start().send();
    collie.close().unwrap();

    let Err(Error::Failed(failure)) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(
        (
            failure.data.code.as_str(),
            &failure.data.details["scope"]["kind"],
            &failure.data.details["usage_kind"],
        ),
        ("limit_reached", &json!("connection"), &json!("requests"))
    );
}

/// The `connect-limit-refused-closes` case: a Usage kind only Foxhound
/// counts fails Collie's connect with `not_in_product`, and the engine it
/// linked has stopped, so the next connect to the same home opens.
/// Covers: story:11-51, error:not_in_product
#[test]
fn a_connection_limit_the_engine_refuses_fails_the_connect() {
    let env = TestEnv::new();
    let paid_fetches = Limits::new().count(UsageKind::named("paid_fetch"), 3);
    real_time_runtime().block_on(async {
        let refused = Collie::connect(options(&env).limits(paid_fetches)).await;

        let Err(Error::Failed(failure)) = refused else {
            panic!("{refused:?}");
        };
        assert_eq!(failure.data.code, "not_in_product");
        let next = Collie::connect(options(&env)).await.unwrap();
        next.close().await.unwrap();
    });
}

/// The `limits-lowered-never-raised` case: the Run `nightly`, started with
/// 5 requests and closed unfinished, resumes with 5, which changes nothing,
/// and with 3, which lowers it; 8 is refused with `limit_raise_refused`.
/// Covers: story:11-51, error:limit_raise_refused
#[test]
fn a_start_naming_a_run_again_lowers_its_limits_and_never_raises_them() {
    let env = TestEnv::new();
    let nightly = RunId::parse("nightly").unwrap();
    let requests = |amount| Limits::new().count(UsageKind::REQUESTS, amount);
    real_time_runtime().block_on(async {
        let linked = LinkedConnection::connect(Composed::COLLIE, options(&env))
            .await
            .unwrap();
        let start = |amount| {
            linked
                .connection()
                .start()
                .run_id(&nightly)
                .limits(requests(amount))
        };

        let mut started_as = Vec::new();
        for amount in [5, 5, 3] {
            let run = start(amount).await.unwrap();
            started_as.push(run.started_as());
            run.close().await.unwrap();
        }
        let raised = start(8).await;
        let held = listed(linked.connection(), "run").await;
        linked.close().await.unwrap();

        assert_eq!(
            (started_as, held),
            (
                vec![StartedAs::New, StartedAs::Resumed, StartedAs::Resumed],
                vec![
                    ("requests".to_owned(), None, 5),
                    ("requests".to_owned(), None, 3),
                ],
            )
        );
        let Err(Error::Failed(failure)) = raised else {
            panic!("{raised:?}");
        };
        let details: LimitRaiseRefusedDetails =
            serde_json::from_value(failure.data.details.clone().into()).unwrap();
        assert_eq!(
            (failure.data.code.as_str(), details),
            (
                "limit_raise_refused",
                LimitRaiseRefusedDetails {
                    usage_kind: "requests".to_owned(),
                    currency: None,
                    given: 8,
                    standing: 3,
                },
            )
        );
    });
}
