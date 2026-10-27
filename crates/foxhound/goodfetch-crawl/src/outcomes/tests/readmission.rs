//! `run.failed_requests` and `run.retry_failed`'s re-admission never
//! disagree (ADR 0117), and a re-admitted request's next outcome supersedes
//! its last (ADR 0102), checked on the crawl's tables in real SQLite for any
//! mix of endings. The seam is the work items' and outcomes' module
//! interfaces: `run.retry_failed`, the operation that re-admits through
//! the Runs' engine, is area 6's (#1005), so no engine-level case can
//! re-admit yet.

use goodfetch_dataset::Checked;
use goodfetch_protocol::foxhound::outcomes::{OutcomeKind, SkipReason};
use goodfetch_protocol::run::PointInTime;
use proptest::prelude::*;
use rusqlite::Connection;

use super::super::{Filter, latest_of_readmitted, page};
use crate::work_items::{self, Failure, Unsuccessful};

/// How the test ends one work item's first attempt.
#[derive(Clone, Copy, Debug)]
enum First {
    Succeeds,
    Fails,
    Skipped,
    Blocked,
    Retried,
}

fn first() -> impl Strategy<Value = First> {
    prop_oneof![
        Just(First::Succeeds),
        Just(First::Fails),
        Just(First::Skipped),
        Just(First::Blocked),
        Just(First::Retried),
    ]
}

fn ended_at() -> PointInTime {
    PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned())
}

fn crawl_of(count: usize) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in work_items::TABLES
        .statements
        .iter()
        .chain(crate::labels::TABLES.statements)
        .chain(goodfetch_dataset::TABLES.statements)
        .chain(goodfetch_frontier::VISITED_TABLES.statements)
        .chain(goodfetch_frontier::SCOPE_TABLES.statements)
        .chain(goodfetch_frontier::BOUNDS_TABLES.statements)
        .chain(goodfetch_frontier::RULES_TABLES.statements)
        .chain(goodfetch_frontier::payload::TABLES.statements)
        .chain(goodfetch_frontier::sitemap::TABLES.statements)
        .chain(goodfetch_frontier::trap_links::TABLES.statements)
    {
        connection.execute_batch(statement).unwrap();
    }
    // The engine's own Run row, whose Clean-up rules a commit's Records name
    // in their Provenance: none is held here.
    connection
        .execute_batch("CREATE TABLE run (clean_up_digest TEXT)")
        .unwrap();
    let seeds: Vec<_> = (0..count)
        .map(|index| {
            goodfetch_protocol::foxhound::crawl::Seed::Request(
                goodfetch_protocol::foxhound::crawl::RequestSeed {
                    url: format!("https://shop.test/{index}"),
                    label: "page".to_owned(),
                    payload: None,
                    priority: 0,
                    forefront: false,
                    method: goodfetch_protocol::foxhound::frontier::HttpMethod::get(),
                    body: None,
                    key: None,
                },
            )
        })
        .collect();
    let every_host = goodfetch_engine::egress::Policy::new(&["*"], &[] as &[&str]).unwrap();
    work_items::admit_seeds(
        &connection,
        &mut work_items::Positions::of(&connection).unwrap(),
        &goodfetch_frontier::Scope::default(),
        &every_host,
        &seeds,
    )
    .unwrap();
    connection
}

fn failure() -> Failure {
    Failure {
        code: "response_not_ok".to_owned(),
        message: "404".to_owned(),
        details: serde_json::Map::new(),
    }
}

/// Leases every pending work item and ends each as `end` says.
fn end_each(connection: &Connection, end: impl Fn(usize) -> First) {
    let pending: Vec<String> = connection
        .prepare("SELECT id FROM work_items WHERE status = 'pending' ORDER BY position")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for leased in work_items::lease(connection, &pending).unwrap() {
        let id = &leased.crawl_request.id;
        let index: usize = id.parse::<usize>().unwrap() - 1;
        let epoch = leased.lease_epoch;
        let ended = match end(index) {
            First::Succeeds => {
                let ended_at = ended_at();
                work_items::commit(
                    connection,
                    id,
                    epoch,
                    work_items::Committed {
                        records: &Checked::default(),
                        fetched: &work_items::Fetched::default(),
                        fields: &work_items::FieldsRead::default(),
                        masks: &goodfetch_engine::mask::Masks::new(),
                        ended_at: &ended_at,
                    },
                )
            }
            First::Fails => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Failed(failure()),
                &ended_at(),
            ),
            First::Skipped => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Skipped {
                    reason: SkipReason::Handler,
                    message: Some("no".to_owned()),
                },
                &ended_at(),
            ),
            First::Blocked => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Blocked(goodfetch_protocol::block::BlockCause::RateLimit),
                &ended_at(),
            ),
            First::Retried => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Retried(failure()),
                &ended_at(),
            ),
        };
        assert!(ended.unwrap());
    }
}

fn listed(connection: &Connection, blocked: bool) -> Vec<String> {
    latest_of_readmitted(connection, blocked)
        .unwrap()
        .into_iter()
        .map(|outcome| outcome.request.id)
        .collect()
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-150
    #[test]
    fn the_failed_requests_listed_are_exactly_those_re_admitted(
        endings in proptest::collection::vec(first(), 1..12),
        blocked in any::<bool>(),
    ) {
        let connection = crawl_of(endings.len());
        end_each(&connection, |index| endings[index]);
        let expected: Vec<String> = endings
            .iter()
            .enumerate()
            .filter(|(_, ending)| {
                matches!(ending, First::Fails) || (blocked && matches!(ending, First::Blocked))
            })
            .map(|(index, _)| (index + 1).to_string())
            .collect();

        let before = listed(&connection, blocked);
        let readmitted = work_items::readmit(&connection, blocked).unwrap();

        prop_assert_eq!((&before, &readmitted), (&expected, &expected));
    }

    /// Covers: story:3-149
    #[test]
    fn a_re_admitted_requests_next_outcome_supersedes_its_last(
        endings in proptest::collection::vec(first(), 1..12),
    ) {
        let connection = crawl_of(endings.len());
        end_each(&connection, |index| endings[index]);
        let readmitted = work_items::readmit(&connection, true).unwrap();
        end_each(&connection, |_| First::Succeeds);

        let all = page(&connection, 0, &Filter { kinds: None, labels: None, codes: None }).unwrap();
        let latest_of = |id: &String| {
            all.outcomes
                .iter()
                .filter(|outcome| &outcome.request.id == id)
                .map(|outcome| (outcome.position, outcome.ending.kind()))
                .collect::<Vec<_>>()
        };
        let superseded: Vec<Vec<OutcomeKind>> = readmitted
            .iter()
            .map(|id| latest_of(id).into_iter().map(|(_, kind)| kind).collect())
            .collect();
        let positions: Vec<u64> = all.outcomes.iter().map(|outcome| outcome.position).collect();

        prop_assert_eq!(
            (
                superseded
                    .iter()
                    .all(|kinds| kinds.len() == 2 && kinds[1] == OutcomeKind::Succeeded),
                listed(&connection, true),
                positions,
            ),
            (
                true,
                Vec::<String>::new(),
                (1..=u64::try_from(all.outcomes.len()).unwrap()).collect::<Vec<_>>(),
            )
        );
    }
}
