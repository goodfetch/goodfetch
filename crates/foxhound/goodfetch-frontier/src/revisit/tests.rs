//! Table-driven and property tests for the pure revisit schedule function and
//! the `revisits` table's covering-index queries (ADR 0139).

use goodfetch_protocol::foxhound::continuous::RevisitPolicy;
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;

use super::{
    DueEntry, PreviousState, RevisitRow, SiteHints, TABLES, VisitResult, due_window, earliest_due,
    get, next_visit, read_all, upsert,
};

/// Table-driven tests of `next_visit` for `never` and `every` policies with
/// and without site floors.
#[rstest]
#[case::never_returns_none(RevisitPolicy::Never, 1_000_000, None, None)]
#[case::never_stays_never_with_floor(RevisitPolicy::Never, 1_000_000, Some(2_000_000), None)]
#[case::every_adds_fixed_interval(
    RevisitPolicy::Every { interval: 60_000 },
    1_000_000,
    None,
    Some((60_000, 1_060_000)),
)]
#[case::every_respects_higher_site_floor(
    RevisitPolicy::Every { interval: 60_000 },
    1_000_000,
    Some(1_500_000),
    Some((60_000, 1_500_000)),
)]
#[case::every_keeps_policy_when_floor_is_earlier(
    RevisitPolicy::Every { interval: 60_000 },
    1_000_000,
    Some(1_010_000),
    Some((60_000, 1_060_000)),
)]
fn next_visit_matches_table(
    #[case] policy: RevisitPolicy,
    #[case] now_ms: u64,
    #[case] floor_at_ms: Option<u64>,
    #[case] expected: Option<(u64, u64)>,
) {
    let scheduled = next_visit(
        &policy,
        PreviousState::default(),
        VisitResult::Changed,
        SiteHints { floor_at_ms },
        now_ms,
    );
    assert_eq!(
        scheduled.map(|s| (s.interval_ms, s.next_visit_at)),
        expected
    );
}

proptest! {
    #[test]
    fn never_policy_always_returns_none(
        now_ms in 0_u64..10_000_000_000_000,
        floor in proptest::option::of(0_u64..10_000_000_000_000),
        visits in 0_u64..1000,
        changes in 0_u64..1000,
    ) {
        let scheduled = next_visit(
            &RevisitPolicy::Never,
            PreviousState { interval_ms: Some(60_000), visits, changes },
            VisitResult::Changed,
            SiteHints { floor_at_ms: floor },
            now_ms,
        );
        prop_assert_eq!(scheduled, None);
    }

    #[test]
    fn every_policy_never_schedules_before_interval_or_floor(
        interval in 1_u64..2_592_000_000,
        now_ms in 0_u64..10_000_000_000_000,
        floor in proptest::option::of(0_u64..10_000_000_000_000),
    ) {
        let scheduled = next_visit(
            &RevisitPolicy::Every { interval },
            PreviousState::default(),
            VisitResult::Changed,
            SiteHints { floor_at_ms: floor },
            now_ms,
        ).expect("every policy always schedules");

        prop_assert_eq!(scheduled.interval_ms, interval);
        prop_assert!(scheduled.next_visit_at >= now_ms.saturating_add(interval));
        if let Some(floor_ms) = floor {
            prop_assert!(scheduled.next_visit_at >= floor_ms);
        }
    }
}

#[test]
fn revisits_table_due_window_uses_covering_index_and_bounds_results() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection
            .prepare_cached(statement)
            .unwrap()
            .execute([])
            .unwrap();
    }

    // Verify EXPLAIN QUERY PLAN uses the covering index `revisits_due`.
    let plan: String = connection
        .prepare_cached(
            "EXPLAIN QUERY PLAN
             SELECT work_item, domain_key, next_visit_at
             FROM revisits
             WHERE domain_key = 'news.test' AND next_visit_at IS NOT NULL AND next_visit_at <= 5000
             ORDER BY next_visit_at ASC, work_item ASC
             LIMIT 64",
        )
        .unwrap()
        .query_row([], |row| row.get(3))
        .unwrap();
    assert!(
        plan.contains("USING COVERING INDEX revisits_due"),
        "expected covering index scan, got: {plan}"
    );

    for (id, domain, due) in [
        ("1", "news.test", Some(2000)),
        ("2", "news.test", Some(1000)),
        ("3", "news.test", Some(3000)),
        ("4", "news.test", None),
        ("5", "other.test", Some(500)),
    ] {
        upsert(
            &connection,
            &RevisitRow {
                work_item: id.to_owned(),
                identity: format!("https://{domain}/{id}"),
                domain_key: domain.to_owned(),
                next_visit_at: due,
                interval_ms: due.map(|_| 1000),
                last_fetched_at: Some(0),
                last_changed_at: Some(0),
                visits: 1,
                changes: 1,
                ..RevisitRow::default()
            },
        )
        .unwrap();
    }

    // Bounded window of 2 for `news.test` due at 5000 returns IDs 2 and 1 in due order.
    let window = due_window(&connection, Some("news.test"), 5000, 2).unwrap();
    assert_eq!(
        window,
        vec![
            DueEntry {
                work_item: "2".to_owned(),
                domain_key: "news.test".to_owned(),
                next_visit_at: 1000,
            },
            DueEntry {
                work_item: "1".to_owned(),
                domain_key: "news.test".to_owned(),
                next_visit_at: 2000,
            },
        ]
    );
    assert_eq!(earliest_due(&connection).unwrap(), Some(500));
    assert_eq!(read_all(&connection).unwrap().len(), 5);
    assert_eq!(get(&connection, "2").unwrap().unwrap().visits, 1);
}
