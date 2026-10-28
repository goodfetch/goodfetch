//! A crawl's work-item metrics through the engine's public interface over
//! the in-memory network (spec #213's Seam 3), on the paused clock (ADR
//! 0050): each work item its domain queues hold counts in
//! `goodfetch.work_item.queued`, each attempt in `goodfetch.work_item.leased` while it
//! runs, and each retry in `goodfetch.work_item.retried`, on the meter the
//! engine is given, which the Prometheus exporter the executables use
//! renders.

use goodfetch_engine::metrics::{METER, Metrics};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use serde_json::{Value, json};

use super::{Reply, crawl_on, dependencies, engine, seeds};

/// The value of the series `name` of `registry`, if it has one.
fn value_of(registry: &prometheus::Registry, name: &str) -> Option<f64> {
    registry
        .gather()
        .iter()
        .find(|family| family.name() == name)
        .and_then(|family| family.get_metric().first())
        .map(|metric| {
            if name.ends_with("_total") {
                metric.get_counter().get_value()
            } else {
                metric.get_gauge().get_value()
            }
        })
}

/// Each work item waiting in its domain's queue counts in
/// `goodfetch.work_item.queued` until it is leased: three pages of one
/// domain, admitted one at a time, leave two, then one, then none queued,
/// and the crawl that ended counts none.
///
/// Covers: story:3-103
#[test]
fn a_crawl_counts_the_work_items_its_domain_queues_hold() {
    let env = TestEnv::new();
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let engine = engine().with_metrics(Metrics::new(&provider.meter(METER)));
    let mut during = Vec::new();

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "queued", "seeds": seeds(3) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                during.push(value_of(&registry, "goodfetch_work_item_queued"));
                Reply::Records(vec![json!({ "handled": true })])
            };
            client
                .call("run.finish", json!({ "run_id": "queued" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        (during, value_of(&registry, "goodfetch_work_item_queued")),
        (vec![Some(2.0), Some(1.0), Some(0.0)], Some(0.0))
    );
}

/// Covers: story:3-103
#[test]
fn a_crawl_counts_each_attempt_while_it_runs_and_each_retry() {
    let env = TestEnv::new();
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let engine = engine().with_metrics(Metrics::new(&provider.meter(METER)));
    let mut during = Vec::new();

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "measured", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                during.push((
                    value_of(&registry, "goodfetch_work_item_leased"),
                    value_of(&registry, "goodfetch_work_item_retried_total"),
                ));
                match params["attempt"].as_u64() {
                    Some(1) => Reply::Fails("retry_requested", true),
                    _ => Reply::Records(vec![json!({ "attempt": "second" })]),
                }
            };
            client
                .call("run.finish", json!({ "run_id": "measured" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        (
            during,
            value_of(&registry, "goodfetch_work_item_leased"),
            value_of(&registry, "goodfetch_work_item_retried_total"),
        ),
        (
            vec![(Some(1.0), None), (Some(1.0), Some(1.0))],
            Some(0.0),
            Some(1.0),
        )
    );
}
