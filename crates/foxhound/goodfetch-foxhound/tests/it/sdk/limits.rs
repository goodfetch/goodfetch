//! Foxhound's Limits in code through its Rust SDK (ADRs 0080 and 0087):
//! [`PAID_FETCHES`], the Usage kind only Foxhound counts, caps the
//! connection at connect and a Run at its start, a price in millionths.

use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_foxhound::{Limits, Money, PAID_FETCHES};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::usage::{USAGE_LIST, UsageListParams, UsageListResult};
use goodfetch_test_support::{TestEnv, real_time_runtime};

use super::options;

/// Covers: story:11-51
#[test]
fn paid_fetches_cap_a_foxhound_connection_and_its_run() {
    let env = TestEnv::new();
    let engine = Composed {
        product: Product::Foxhound,
        composition: goodfetch_foxhound::composition,
    };
    let connect = Limits::new().count(PAID_FETCHES, 3);
    let run = Limits::new().price(PAID_FETCHES, Money::new("0.25", "USD").unwrap());
    real_time_runtime().block_on(async {
        let linked = ProductConnection::connect(engine, options(&env).limits(connect))
            .await
            .unwrap();

        let started = linked.connection().start().limits(run).await.unwrap();
        let listed: UsageListResult = linked
            .connection()
            .call(USAGE_LIST.name, &UsageListParams {})
            .await
            .unwrap();
        started.finish().await.unwrap();
        linked.close().await.unwrap();

        let limits: Vec<_> = listed
            .limits
            .into_iter()
            .map(|usage| {
                (
                    usage.limit.scope.kind,
                    usage.limit.usage_kind,
                    usage.limit.currency,
                    usage.limit.amount,
                )
            })
            .collect();
        assert_eq!(
            limits,
            [
                ("connection".to_owned(), "paid_fetch".to_owned(), None, 3),
                (
                    "run".to_owned(),
                    "paid_fetch".to_owned(),
                    Some("USD".to_owned()),
                    250_000
                ),
            ]
        );
    });
}
