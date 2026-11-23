//! A crawl's work-item metrics through the engine's public interface over
//! the in-memory network (spec #213's Seam 3), on the paused clock (ADR
//! 0050): each work item that waits to be leased, in its domain's queue or
//! past it, counts in `goodfetch.work_item.queued`, each attempt in `goodfetch.work_item.leased` while it
//! runs, and each retry in `goodfetch.work_item.retried`, on the meter the
//! engine is given, which the Prometheus exporter the executables use
//! renders.

use std::time::Duration;

use goodfetch_engine::metrics::{METER, Metrics};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_frontier::Url;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_test_support::TestEnv;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    Reply, SITEMAP, connected_on, crawl_on, dependencies, engine, page_of, records, run_file, seeds,
};
use crate::work_items;

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

/// Each work item waiting to be leased counts in
/// `goodfetch.work_item.queued` until it is: three pages of one
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
                    json!({ "run_id": "queued", "seeds": seeds(3), "settings": { "crawl": { "handler": { "concurrency": 1 } } }, "labels": { "page": {} } }),
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

/// `goodfetch.work_item.queued` counts the Frontier's whole depth, not
/// what its domain's queue holds, at most 64 (ADR 0050's note of
/// 2026-11-23): a hundred pages of one domain, admitted one at a time,
/// leave 99 queued as the first is handled and 98 as the second is.
///
/// Covers: story:3-103
#[test]
fn a_crawl_counts_every_work_item_its_frontier_holds_past_its_domain_queue() {
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
                    json!({ "run_id": "deep", "seeds": seeds(100), "settings": { "crawl": { "handler": { "concurrency": 1 } } }, "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                during.push(value_of(&registry, "goodfetch_work_item_queued"));
                Reply::Records(vec![json!({ "handled": true })])
            };
            client
                .call("run.finish", json!({ "run_id": "deep" }), &mut handler)
                .await
        },
    );

    assert_eq!(during.get(..2), Some(&[Some(99.0), Some(98.0)][..]));
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
                    json!({ "run_id": "measured", "seeds": seeds(1), "labels": { "page": {} } }),
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
                    json!({ "run_id": "trips", "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 3 } } }, "labels": { "page": {} } }),
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

/// Crawls five seeds, three pages at a time, at the memory budget
/// `budget`, and returns how many pages the handler saw and how many reads
/// of the run file the leases made, counted as `F3.lease_reads`.
fn pages_and_lease_reads(budget: u64) -> (usize, Option<f64>) {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut pages = 0;

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let settings = json!({
                "crawl": { "handler": { "concurrency": 3 } },
                "run_state": { "memory": { "budget": budget } },
            });
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "leases", "seeds": seeds(5), "settings": settings, "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                pages += 1;
                Reply::Records(Vec::new())
            };
            client
                .call("run.finish", json!({ "run_id": "leases" }), &mut handler)
                .await
        },
    );

    (pages, counted(&registry, "frontier_lease", "reads"))
}

/// Within the memory budget the crawl holds every pending work item whole
/// from its opening on, so its leases read nothing of the run file
/// (#2169).
///
/// Covers: setting:run_state.memory.budget
#[test]
fn within_the_memory_budget_a_crawls_leases_read_nothing_of_the_run_file() {
    let (pages, reads) = pages_and_lease_reads(64 << 20);

    assert_eq!((pages, reads.unwrap_or(0.0)), (5, 0.0));
}

/// At a budget of 0 no pending work item fits in memory: the leases read
/// them from the run file, each read counted as `F3.lease_reads`, at most
/// one read transaction a lease (ADR 0140's F3 contract past the budget).
///
/// Covers: setting:run_state.memory.budget
#[test]
fn at_a_memory_budget_of_0_each_lease_reads_the_run_file_at_most_once() {
    let (pages, reads) = pages_and_lease_reads(0);

    assert_eq!(pages, 5);
    assert!(
        reads.is_some_and(|reads| (1.0..=5.0).contains(&reads)),
        "{reads:?}"
    );
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

/// Every admission keys the work items it adds, in the write that adds
/// them, a sitemap's read included (#2167). One that left a work item with
/// no domain key would leave it out of every domain's queue: the crawl,
/// once idle, keys what it finds so and delivers it, and that write of
/// keys, apart from the write that added the work item, counts as
/// `F3.transactions.sitemap`, the row of the last admission that asked
/// for one.
///
/// Covers: story:3-25
#[test]
fn a_work_item_left_with_no_domain_key_is_keyed_once_its_crawl_is_idle_and_its_keys_write_counts() {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut pages = Vec::new();

    connected_on(
        &engine,
        &json!({ "timeout": { "run": 600_000 } }),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "left",
                        "seeds": seeds(1),
                        "settings": { "crawl": { "keep_alive": true } },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                pages.push(page_of(params));
                Reply::Records(vec![json!({ "handled": true })])
            };
            client
                .call("run.outcomes", json!({ "run_id": "left" }), &mut handler)
                .await;
            // The paused clock moves on once the crawl waits for new work.
            tokio::time::sleep(Duration::from_mins(1)).await;
            {
                let connection = run_file(&env, "left");
                let get = HttpMethod::get();
                work_items::add(
                    &connection,
                    &mut work_items::Positions::of(&connection).unwrap(),
                    &Url::parse("https://shop.test/left").unwrap(),
                    work_items::Adding {
                        method: &get,
                        body: None,
                        label: "page",
                        payload: None,
                        priority: 0,
                        forefront: false,
                        depth: 0,
                    },
                )
                .unwrap();
            }
            client
                .call("run.finish", json!({ "run_id": "left" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        (
            pages,
            counted(&registry, "frontier_sitemap", "transactions")
        ),
        (
            vec![
                "https://shop.test/page/1".to_owned(),
                "https://shop.test/left".to_owned()
            ],
            Some(1.0)
        )
    );
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
                    json!({ "run_id": "unread", "seeds": seeds(3), "labels": { "page": {} } }),
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
                    json!({ "run_id": "subscribed", "seeds": seeds(1), "labels": { "page": {} } }),
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
                    json!({ "run_id": "ended", "seeds": seeds(3), "labels": { "page": {} } }),
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
                    json!({ "run_id": "read", "seeds": seeds(1), "labels": { "page": {} } }),
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

/// Crawls one page whose handler stages three Records as `staging` says,
/// and returns how many Records the crawl committed and how many jobs its
/// Run state's writer ran, counted as `F2.transactions`.
fn records_and_writer_jobs(staging: fn(&Value) -> Reply) -> (usize, Option<f64>) {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "pushed", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| staging(params);
            client
                .call("run.finish", json!({ "run_id": "pushed" }), &mut handler)
                .await;
            assert!(
                client
                    .replies
                    .iter()
                    .all(|reply| reply.get("result").is_some()),
                "{:?}",
                client.replies
            );
        },
    );

    (
        records(&run_file(&env, "pushed")).len(),
        counted(&registry, "run_state", "transactions"),
    )
}

/// The three Records a handler stages.
fn titles() -> [Value; 3] {
    ["Dune", "Emma", "Ulysses"].map(|title| json!({ "title": title }))
}

/// A handler's push asks the Run state's writer for nothing (ADR 0140's
/// F4: a Record takes no transaction of its own): a page whose handler
/// pushes three Records from inside its attempt costs the writer as many
/// jobs as a page whose handler answers the three, and both commit them.
///
/// Covers: story:6-267, operation:crawl_request.push
#[test]
fn a_pushed_record_costs_the_run_states_writer_no_job_of_its_own() {
    let answered = records_and_writer_jobs(|_| Reply::Records(titles().to_vec()));
    let pushed = records_and_writer_jobs(|params| {
        let pushes = titles()
            .into_iter()
            .map(|data| {
                (
                    "crawl_request.push",
                    json!({
                        "run_id": params["run_id"],
                        "crawl_request_id": params["crawl_request"]["id"],
                        "lease_epoch": params["lease_epoch"],
                        "dataset": "pages",
                        "records": [data],
                    }),
                )
            })
            .collect();
        Reply::Calls(pushes, Box::new(Reply::Records(Vec::new())))
    });

    assert_eq!(answered.0, 3);
    assert_eq!(pushed, answered);
}

/// What `goodfetch.run_state.memory` and
/// `goodfetch.run_state.spilled_lookups` read as a crawl's one seed, at the
/// Durability level `level` and the memory budget `budget`, runs its
/// handler, which enqueues a link: the enqueue's duplicate check comes
/// first, so both are read after it.
fn memory_while_a_link_is_enqueued(level: &str, budget: u64) -> (Option<f64>, Option<f64>) {
    let env = TestEnv::new();
    let (engine, registry, _provider) = metered();
    let mut during = None;

    crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "held",
                        "seeds": seeds(1),
                        "settings": {
                            "run_state": { "durability": level, "memory": { "budget": budget } },
                        },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if params["attempt"] == 1 && params["crawl_request"]["url"] != "https://shop.test/b"
                {
                    let enqueue = (
                        "crawl_request.enqueue",
                        json!({
                            "run_id": params["run_id"],
                            "crawl_request_id": params["crawl_request"]["id"],
                            "lease_epoch": params["lease_epoch"],
                            "url": "https://shop.test/b",
                            "label": "page",
                        }),
                    );
                    return Reply::Calls(vec![enqueue], Box::new(Reply::Records(Vec::new())));
                }
                during = Some((
                    value_of(&registry, "goodfetch_run_state_memory_bytes"),
                    value_of(&registry, "goodfetch_run_state_spilled_lookups_total"),
                ));
                Reply::Records(Vec::new())
            };
            client
                .call("run.finish", json!({ "run_id": "held" }), &mut handler)
                .await;
        },
    );

    during.expect("the enqueued link reaches the handler")
}

/// Within the default memory budget, memory reads the seed's visit as the
/// crawl opens and holds the link's: memory holds bytes, and no lookup
/// reads the run file.
///
/// Covers: setting:run_state.memory.budget
#[test]
fn within_the_memory_budget_memory_holds_the_visits_and_no_lookup_reads_the_run_file() {
    let (bytes, spilled) = memory_while_a_link_is_enqueued("batch", 64 << 20);

    assert!(bytes.is_some_and(|bytes| bytes > 0.0), "{bytes:?}");
    assert_eq!(spilled.unwrap_or(0.0), 0.0);
}

/// ADR 0142's "Nothing removed" row of the file-first Frontier:
/// `run_state.memory.budget: 0` at a level that batches attempts. Memory
/// keeps no visit the file holds, so the link's duplicate check reads the
/// run file and counts as a spilled lookup.
///
/// Covers: setting:run_state.memory.budget, setting:run_state.durability
#[rstest]
#[case::batch("batch")]
#[case::off("off")]
fn at_a_memory_budget_of_0_a_batching_level_reads_the_link_s_visit_from_the_run_file(
    #[case] level: &str,
) {
    let (_, spilled) = memory_while_a_link_is_enqueued(level, 0);

    assert!(spilled.is_some_and(|spilled| spilled >= 1.0), "{spilled:?}");
}

/// ADR 0142's "Nothing removed" row at a level that writes each attempt:
/// memory holds no visit at all and every lookup reads the run file, as
/// before ADR 0142, so neither gauge moves.
///
/// Covers: setting:run_state.memory.budget, setting:run_state.durability
#[rstest]
#[case::full("full")]
#[case::fast("fast")]
#[case::periodic("periodic")]
fn at_a_memory_budget_of_0_a_level_writing_each_attempt_holds_no_visit_in_memory(
    #[case] level: &str,
) {
    let (bytes, spilled) = memory_while_a_link_is_enqueued(level, 0);

    assert_eq!((bytes.unwrap_or(0.0), spilled.unwrap_or(0.0)), (0.0, 0.0));
}
