//! A handler's enqueue measured against the scope the run file holds, in
//! process on a run file in memory (spec #581: pure rules and run state in
//! process).

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::Policy;
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::{
    BOUNDS_TABLES, Patterns, SCOPE_TABLES, Scope, ScopeStrategy, VISITED_TABLES,
};
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::{CrawlRequestEnqueueParams, Enqueued, NotAddedReason};
use rusqlite::Connection;
use serde_json::json;

use crate::invariants::{Invariants, fixture};
use crate::labels::Fragments;
use crate::run_state::admission::enqueue as add;
use crate::staging::Staging;
use crate::work_items;

/// A run file in memory under a `same_domain` scope, holding the seed
/// `http://shop.test/` leased to its first attempt, with the lease epoch
/// of that attempt.
fn leased() -> (Connection, u64) {
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
        .chain(crate::invariants::fixture::PAGE_DECLARED.statements)
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
    let invariants = Invariants::read(&connection, Fragments::read(&connection).unwrap()).unwrap();
    work_items::admit_seeds(
        &connection,
        &mut work_items::Positions::of(&connection).unwrap(),
        &invariants,
        &every_host(),
        &fixture::checked(&invariants, [seed]),
    )
    .unwrap();
    let leased = work_items::lease(&connection, &["1".to_owned()]).unwrap();
    (connection, leased[0].lease_epoch)
}

fn every_host() -> Policy {
    Policy::new(&["*"], &[] as &[&str]).unwrap()
}

/// The scope a Run recorded as it began decides every enqueue of the Run,
/// since the invariants an enqueue is measured against read it from the run
/// file as the Run resumes, and from no setting, so a resume under other
/// `crawl.scope.*` values never moves it (ADR 0102):
/// a `same_domain` crawl's subdomain is added, a sister domain is not.
///
/// Covers: story:6-224, setting:crawl.scope.strategy
#[test]
fn an_enqueue_is_measured_against_the_scope_the_run_recorded() {
    let (connection, lease_epoch) = leased();
    let invariants = Invariants::read(&connection, Fragments::read(&connection).unwrap()).unwrap();
    let enqueue = |url: &str| {
        let params: CrawlRequestEnqueueParams = serde_json::from_value(json!({
            "run_id": "scoped",
            "crawl_request_id": "1",
            "lease_epoch": lease_epoch,
            "url": url,
            "label": "page",
        }))
        .unwrap();
        add(
            &connection,
            &mut work_items::Positions::of(&connection).unwrap(),
            (&every_host(), &invariants),
            (&params, None),
            None,
            &CountedWork::default(),
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

/// A staged enqueue's Payload is checked against its Label's schema and
/// serialized once, as its call stages it: its attempt's commit adds the
/// request, with that Payload, at the ID the call answered, and validates,
/// serializes and compiles nothing, reading no invariant (#2069).
///
/// Covers: story:6-236
#[test]
fn a_staged_enqueue_is_checked_once_and_its_commit_adds_it_with_its_payload() {
    const ROWS: [Row; 5] = [
        Row::PayloadValidations,
        Row::PayloadSerializations,
        Row::PayloadCompilations,
        Row::BatchInvariantReads,
        Row::BatchCompilations,
    ];
    let (connection, lease_epoch) = leased();
    connection
        .execute(
            "UPDATE payload_labels SET schema = ?1, hash = 'h' WHERE label = 'page'",
            [json!({ "type": "object", "required": ["sku"] }).to_string()],
        )
        .unwrap();
    let invariants = Invariants::read(&connection, Fragments::read(&connection).unwrap()).unwrap();
    let work = CountedWork::default();
    let params: CrawlRequestEnqueueParams = serde_json::from_value(json!({
        "run_id": "scoped",
        "crawl_request_id": "1",
        "lease_epoch": lease_epoch,
        "url": "http://shop.test/a",
        "label": "page",
    }))
    .unwrap();
    let payload = Arc::new(Payload::of(json!({ "sku": 7 }), &work));
    let staging = Staging::default();
    staging.open("1", lease_epoch);

    let answered = staging
        .admit(&connection, "1", lease_epoch, |connection, positions| {
            add(
                connection,
                positions,
                (&every_host(), &invariants),
                (&params, Some(&payload)),
                None,
                &work,
            )
        })
        .unwrap()
        .unwrap()
        .map_err(|error| error.code())
        .unwrap();
    let staged = ROWS.map(|row| work.read(row));
    let held_before: Option<String> = work_items::url_of(&connection, "2").unwrap();
    let not_added = staging
        .take("1", lease_epoch)
        .admit(&connection, None)
        .unwrap()
        .not_added;

    assert_eq!(
        (
            answered,
            held_before,
            not_added,
            work_items::url_of(&connection, "2").unwrap(),
            goodfetch_frontier::payload::of_request(&connection, "2").unwrap(),
            staged,
            ROWS.map(|row| work.read(row)),
        ),
        (
            Enqueued::Added {
                id: "2".to_owned(),
                url: "http://shop.test/a".to_owned()
            },
            None,
            Vec::new(),
            Some("http://shop.test/a".to_owned()),
            Some(json!({ "sku": 7 })),
            [1, 1, 0, 0, 0],
            [1, 1, 0, 0, 0],
        )
    );
}
