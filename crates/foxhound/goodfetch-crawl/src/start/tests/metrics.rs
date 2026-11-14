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

use super::{Reply, SITEMAP, crawl_on, dependencies, engine, seeds};

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

/// An engine whose meter the registry renders, that registry, and the
/// meter's provider, which renders while it lives.
fn metered() -> (
    goodfetch_engine::Engine,
    prometheus::Registry,
    SdkMeterProvider,
) {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    (
        engine().with_metrics(Metrics::new(&provider.meter(METER))),
        registry,
        provider,
    )
}

/// Each work item waiting in its domain's queue counts in
/// `goodfetch.work_item.queued` until it is leased: three pages of one
/// domain, admitted one at a time to a handler of concurrency 1, leave two,
/// then one, then none queued, and the crawl that ended counts none.
///
/// Covers: story:3-103
#[test]
fn a_crawl_counts_the_work_items_its_domain_queues_hold() {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut during = Vec::new();

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "queued", "seeds": seeds(3), "settings": { "crawl": { "handler": { "concurrency": 1 } } } }),
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
    let (engine, registry, _provider) = metered();
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

/// The count of `work` on the Shared path `shared_path` in `registry`.
fn counted(registry: &prometheus::Registry, shared_path: &str, work: &str) -> Option<f64> {
    registry
        .gather()
        .iter()
        .find(|family| family.name() == "goodfetch_counted_work_total")?
        .get_metric()
        .iter()
        .find(|metric| {
            let labelled = |name: &str, value: &str| {
                metric
                    .get_label()
                    .iter()
                    .any(|label| label.name() == name && label.value() == value)
            };
            labelled("shared_path", shared_path) && labelled("work", work)
        })
        .map(|metric| metric.get_counter().get_value())
}

/// Crawls from `seeds`, handling three pages at a time, and returns how many
/// pages the handler saw and how many round trips to the actor that owns the
/// Run state the crawl counted as `F3.actor_round_trips`.
fn pages_and_round_trips(seeds: &Value) -> (usize, Option<f64>) {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut pages = 0;

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "trips", "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 3 } } } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                pages += 1;
                Reply::Records(vec![json!({ "handled": true })])
            };
            client
                .call("run.finish", json!({ "run_id": "trips" }), &mut handler)
                .await
        },
    );

    (
        pages,
        counted(&registry, "frontier_admission", "actor_round_trips"),
    )
}

/// A page's task tells the actor that owns its crawl's Run state of its
/// attempt's end in one message, whose answer carries its next attempt
/// (ADR 0142): five pages, three at a time, cost five round trips, counted
/// as `F3.actor_round_trips`.
///
/// Covers: story:3-25
#[test]
fn each_attempt_costs_the_run_states_actor_one_round_trip() {
    assert_eq!(pages_and_round_trips(&seeds(5)), (5, Some(5.0)));
}

/// A sitemap's attempt ends in one message to the actor as a page's does:
/// a crawl from one sitemap seed whose sitemap lists three pages handles
/// the three and costs four round trips.
///
/// Covers: story:3-25
#[test]
fn a_sitemap_attempt_costs_the_run_states_actor_one_round_trip() {
    let seeds = json!([{
        "sitemap": format!("https://shop.test{SITEMAP}"),
        "rules": [{ "pattern": "**", "label": "page" }],
    }]);

    assert_eq!(pages_and_round_trips(&seeds), (3, Some(4.0)));
}

/// The serializations of the requests in flight `registry` counted, as
/// `F3.in_flight_serializations`.
fn in_flight_serializations(registry: &prometheus::Registry) -> Option<f64> {
    counted(registry, "frontier_admission", "in_flight_serializations")
}

/// Attempts that begin, go on to handling and end while no reader takes a
/// value of their crawl's progress serialize no request in flight: three
/// pages, handled to the crawl's end, count none.
///
/// Covers: story:3-162
#[test]
fn attempts_whose_crawls_progress_no_reader_takes_serialize_no_request_in_flight() {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut pages = 0;

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "unread", "seeds": seeds(3) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                pages += 1;
                Reply::Records(vec![json!({ "handled": true })])
            };
            client
                .call("run.finish", json!({ "run_id": "unread" }), &mut handler)
                .await
        },
    );

    assert_eq!((pages, in_flight_serializations(&registry)), (3, Some(0.0)));
}

/// A subscription to the engine's events is a reader of a crawl's progress
/// while it is open and no longer once its Client cancelled it: a crawl
/// under the subscription has its requests in flight serialized for the
/// `run.progress` events it publishes, and the three attempts of a crawl
/// started after the cancellation serialize none.
///
/// Covers: story:3-162, event:run.progress, operation:events.subscribe
#[test]
fn attempts_after_a_subscription_to_the_engines_events_ended_serialize_no_request_in_flight() {
    const SUBSCRIPTION: i64 = 1000;
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut pages = 0;

    let ((subscribed, ended), _) = crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut handler = |_: &Value| {
                pages += 1;
                Reply::Records(vec![json!({ "handled": true })])
            };
            client
                .send(&json!({
                    "jsonrpc": "2.0", "id": SUBSCRIPTION, "method": "events.subscribe", "params": {},
                }))
                .await;
            client
                .send(&json!({
                    "jsonrpc": "2.0", "method": "$/credit",
                    "params": { "id": SUBSCRIPTION, "credits": 1000 },
                }))
                .await;
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "subscribed", "seeds": seeds(1) }),
                    &mut handler,
                )
                .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "subscribed" }),
                    &mut handler,
                )
                .await;
            let subscribed = in_flight_serializations(&registry);

            client
                .send(&json!({
                    "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": SUBSCRIPTION },
                }))
                .await;
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "ended", "seeds": seeds(3) }),
                    &mut handler,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "ended" }), &mut handler)
                .await;
            (subscribed, in_flight_serializations(&registry))
        },
    );

    assert_eq!(
        (
            pages,
            subscribed.is_some_and(|subscribed| subscribed > 0.0),
            ended
                .zip(subscribed)
                .map(|(ended, subscribed)| ended - subscribed),
        ),
        (4, true, Some(0.0))
    );
}

/// A reader that takes one value of a crawl's progress has its requests in
/// flight serialized once: with a request in its handler, one more
/// `run.progress` read counts one more serialization, and its value lists
/// that request.
///
/// Covers: story:3-162, operation:run.progress
#[test]
fn a_reader_that_takes_one_value_of_a_crawls_progress_serializes_its_requests_in_flight_once() {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut handler = |_: &Value| Reply::Silent;

    let ((before, after, read), _) = crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "read", "seeds": seeds(1) }),
                    &mut handler,
                )
                .await;
            let mut params = json!({ "run_id": "read" });
            loop {
                let progress = client
                    .call("run.progress", params.clone(), &mut handler)
                    .await;
                if progress["result"]["requests"]["handling"] == 1 {
                    break;
                }
                params["after"] = progress["result"]["position"].clone();
            }
            let before = in_flight_serializations(&registry);
            let read = client
                .call("run.progress", json!({ "run_id": "read" }), &mut handler)
                .await;
            (before, in_flight_serializations(&registry), read)
        },
    );

    assert_eq!(
        (
            after.zip(before).map(|(after, before)| after - before),
            read["result"]["in_flight"][0]["url"].as_str(),
        ),
        (Some(1.0), Some("https://shop.test/page/1"))
    );
}
