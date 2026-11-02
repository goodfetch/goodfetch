//! A handler's enqueue measured against the scope the run file holds, in
//! process on a run file in memory (spec #581: pure rules and run state in
//! process).

use goodfetch_engine::egress::Policy;
use goodfetch_frontier::{
    BOUNDS_TABLES, Patterns, SCOPE_TABLES, Scope, ScopeStrategy, VISITED_TABLES,
};
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::{CrawlRequestEnqueueParams, Enqueued, NotAddedReason};
use rusqlite::Connection;
use serde_json::json;

use super::add;
use crate::work_items;

/// The scope a Run recorded as it began decides every enqueue of the Run,
/// since an enqueue reads it from the run file and from no setting, so a
/// resume under other `crawl.scope.*` values never moves it (ADR 0102):
/// a `same_domain` crawl's subdomain is added, a sister domain is not.
///
/// Covers: story:6-224, setting:crawl.scope.strategy
#[test]
fn an_enqueue_is_measured_against_the_scope_the_run_recorded() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in work_items::TABLES
        .statements
        .iter()
        .chain(crate::labels::TABLES.statements)
        .chain(goodfetch_dataset::TABLES.statements)
        .chain(VISITED_TABLES.statements)
        .chain(SCOPE_TABLES.statements)
        .chain(BOUNDS_TABLES.statements)
        .chain(goodfetch_frontier::RULES_TABLES.statements)
        .chain(goodfetch_frontier::payload::TABLES.statements)
        .chain(goodfetch_frontier::sitemap::TABLES.statements)
        .chain(goodfetch_frontier::trap_links::TABLES.statements)
    {
        connection.execute(statement, []).unwrap();
    }
    let scope = Scope::new(
        ScopeStrategy::SameDomain,
        Patterns::default(),
        Patterns::default(),
    );
    scope.record(&connection).unwrap();
    let seed: Seed =
        serde_json::from_value(json!({ "url": "http://shop.test/", "label": "page" })).unwrap();
    let every_host = Policy::new(&["*"], &[] as &[&str]).unwrap();
    work_items::admit_seeds(
        &connection,
        &mut work_items::Positions::of(&connection).unwrap(),
        &scope,
        &every_host,
        &[seed],
    )
    .unwrap();
    let leased = work_items::lease(&connection, &["1".to_owned()]).unwrap();
    let enqueue = |url: &str| {
        let params: CrawlRequestEnqueueParams = serde_json::from_value(json!({
            "run_id": "scoped",
            "crawl_request_id": "1",
            "lease_epoch": leased[0].lease_epoch,
            "url": url,
            "label": "page",
        }))
        .unwrap();
        add(
            &connection,
            &mut work_items::Positions::of(&connection).unwrap(),
            &every_host,
            &params,
            None,
            &goodfetch_counted_work::CountedWork::default(),
        )
        .unwrap()
        .unwrap()
        .map_err(|error| error.code())
        .unwrap()
    };

    let answers = [
        enqueue("http://cdn.shop.test/a"),
        enqueue("http://shop-outlet.test/a"),
    ];

    assert_eq!(
        answers,
        [
            Enqueued::Added {
                id: "2".to_owned(),
                url: "http://cdn.shop.test/a".to_owned()
            },
            Enqueued::NotAdded {
                reason: NotAddedReason::OutOfScope,
                url: "http://shop-outlet.test/a".to_owned()
            },
        ]
    );
}
