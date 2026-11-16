//! `run.failed_requests` and `run.retry_failed`'s re-admission never
//! disagree (ADR 0117), and a re-admitted request's next outcome supersedes
//! its last (ADR 0102), checked on the crawl's tables in real SQLite for any
//! mix of endings. The seam is the work items' and outcomes' module
//! interfaces: `run.retry_failed`, the operation that re-admits through
//! the Runs' engine, is area 6's (#1005), so no engine-level case can
//! re-admit yet.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_dataset::Checked;
use goodfetch_protocol::foxhound::outcomes::{
    CrawlRequestOutcome, Ending, OutcomeKind, SkipReason,
};
use goodfetch_protocol::foxhound::report::{CountOf, RequestCounts};
use goodfetch_protocol::run::PointInTime;
use proptest::prelude::*;
use rusqlite::Connection;

use super::super::{Filter, latest_of_readmitted, page};
use crate::invariants::fixture;
use crate::work_items::{self, Changed, Failure, Unsuccessful};

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
    let invariants = fixture::within(&connection, goodfetch_frontier::Scope::default()).unwrap();
    work_items::admit_seeds(
        &connection,
        &mut work_items::Positions::of(&connection).unwrap(),
        &invariants,
        &every_host,
        &fixture::checked(&invariants, seeds),
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
                        not_added: &[],
                        work: &goodfetch_counted_work::CountedWork::default(),
                        keeps_records: true,
                        clean_up_rules: &work_items::CleanUpRules::default(),
                    },
                    &mut Changed::default(),
                )
            }
            First::Fails => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Failed(failure()),
                &ended_at(),
                &mut Changed::default(),
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
                &mut Changed::default(),
            ),
            First::Blocked => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Blocked(goodfetch_protocol::block::BlockCause::RateLimit),
                &ended_at(),
                &mut Changed::default(),
            ),
            First::Retried => work_items::end(
                connection,
                id,
                epoch,
                Unsuccessful::Retried(failure()),
                &ended_at(),
                &mut Changed::default(),
            ),
        };
        assert!(ended.unwrap());
    }
}

/// The failure of an attempt whose engine stopped while it held its lease.
fn lost() -> Failure {
    Failure::new("handler_lost", "the handler was lost")
}

/// The IDs of the work items pending, in order.
fn pending(connection: &Connection) -> Vec<String> {
    connection
        .prepare("SELECT id FROM work_items WHERE status = 'pending' ORDER BY position")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A lease its engine lost, which a resume finds past its last allowed
/// attempt, ends `failed` with `handler_lost` as a live failure ends:
/// its attempt's error and its outcome written, so `run.failed_requests`,
/// the re-admission and the report's counts name the same requests.
///
/// Covers: story:6-306
#[test]
fn a_lease_a_resume_fails_ends_with_its_handler_lost_outcome() {
    let connection = crawl_of(2);
    work_items::lease(&connection, &pending(&connection)).unwrap();

    work_items::recover_leases(
        &connection,
        &lost(),
        0,
        &ended_at(),
        &mut Changed::default(),
    )
    .unwrap();

    let (counts, _) = crate::report::ended(&connection).unwrap();
    let errors: Vec<Vec<(u32, String)>> = latest_of_readmitted(&connection, false)
        .unwrap()
        .into_iter()
        .map(|outcome| {
            outcome
                .errors
                .into_iter()
                .map(|error| (error.attempt, error.code))
                .collect()
        })
        .collect();
    assert_eq!(
        (
            listed(&connection, false),
            errors,
            work_items::readmittable(&connection, false).unwrap(),
            counts.failed,
            counts.by_error_code,
        ),
        (
            vec!["1".to_owned(), "2".to_owned()],
            vec![
                vec![(1, "handler_lost".to_owned())],
                vec![(1, "handler_lost".to_owned())]
            ],
            2,
            2,
            vec![CountOf {
                name: "handler_lost".to_owned(),
                count: 2
            }],
        )
    );
}

/// A lease its engine lost, which a resume finds with retries left, goes
/// back to pending with its attempt's `handler_lost` error kept, which the
/// request's next outcome carries.
///
/// Covers: story:6-306
#[test]
fn a_lease_a_resume_retries_keeps_its_handler_lost_error_for_its_next_outcome() {
    let connection = crawl_of(1);
    work_items::lease(&connection, &pending(&connection)).unwrap();

    work_items::recover_leases(
        &connection,
        &lost(),
        1,
        &ended_at(),
        &mut Changed::default(),
    )
    .unwrap();
    end_each(&connection, |_| First::Fails);

    let errors: Vec<(u32, String)> = latest_of_readmitted(&connection, false)
        .unwrap()
        .into_iter()
        .flat_map(|outcome| outcome.errors)
        .map(|error| (error.attempt, error.code))
        .collect();
    assert_eq!(
        errors,
        vec![
            (1, "handler_lost".to_owned()),
            (2, "response_not_ok".to_owned())
        ]
    );
}

/// The model of the request counts: each work item pending or leased
/// counted pending, and each ended one by its latest outcome, decoded in
/// full from the outcomes table.
fn decoded(connection: &Connection) -> RequestCounts {
    let mut counts = RequestCounts {
        pending: work_items::left(connection).unwrap(),
        ..RequestCounts::default()
    };
    let by = |counts: &mut Vec<CountOf>, name: String| {
        match counts.iter_mut().find(|count| count.name == name) {
            Some(count) => count.count += 1,
            None => counts.push(CountOf { name, count: 1 }),
        }
        counts.sort_by(|left, right| left.name.cmp(&right.name));
    };
    let latest: Vec<String> = connection
        .prepare(
            "SELECT outcome FROM outcomes WHERE position IN
                 (SELECT MAX(position) FROM outcomes GROUP BY work_item)
               AND work_item IN (SELECT id FROM work_items
                 WHERE status NOT IN ('pending', 'leased'))",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for outcome in latest {
        let outcome: CrawlRequestOutcome = serde_json::from_str(&outcome).unwrap();
        match outcome.ending {
            Ending::Succeeded { .. } => counts.succeeded += 1,
            Ending::Failed { error } => {
                counts.failed += 1;
                by(&mut counts.by_error_code, error.code);
            }
            Ending::Skipped { reason, .. } => {
                counts.skipped += 1;
                by(
                    &mut counts.by_skip_reason,
                    serde_json::to_value(reason)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
            Ending::Blocked { cause } => {
                counts.blocked += 1;
                by(&mut counts.by_block_cause, cause.as_str().to_owned());
            }
        }
    }
    counts
}

/// A read of the request counts, which each progress read makes, reads
/// the counts kept as each outcome committed, never the outcomes: with
/// every outcome row gone, the counts stand.
///
/// Covers: story:3-149
#[test]
fn a_read_of_the_request_counts_decodes_no_outcome() {
    let connection = crawl_of(3);
    end_each(&connection, |index| {
        [First::Succeeds, First::Fails, First::Skipped][index]
    });
    let before = crate::report::ended(&connection).unwrap();

    connection.execute("DELETE FROM outcomes", []).unwrap();

    assert_eq!(crate::report::ended(&connection).unwrap(), before);
}

/// Which of the two writer jobs a re-admission racing a crawl's end runs
/// first.
#[derive(Clone, Copy, Debug)]
enum Order {
    /// The crawl's last look for pending work, then the re-admission.
    LastLookFirst,
    /// The re-admission, then the crawl's last look.
    ReadmissionFirst,
}

/// A re-admission and a crawl's last look before it ends, each a job of
/// the Run state's writer, in either order: the crawl either finds the
/// re-admitted request pending and goes on to deliver it, or the
/// re-admission finds the crawl closed to new work and re-admits nothing,
/// the request staying failed for a re-admission of the ended Run. It is
/// never re-admitted after the crawl's last look, left pending in a Run
/// that then finishes.
///
/// Covers: story:6-306
#[rstest::rstest]
fn a_re_admission_racing_the_crawls_end_is_delivered_or_refused(
    #[values(Order::LastLookFirst, Order::ReadmissionFirst)] order: Order,
) {
    let connection = crawl_of(1);
    end_each(&connection, |_| First::Fails);
    let intake = crate::intake::Intake::default();
    let readmit = |connection: &Connection| {
        intake.admitting(|| work_items::readmit(connection, false, &mut Changed::default()))
    };

    let (ended, readmitted) = match order {
        Order::LastLookFirst => {
            let ended = intake.close_if_drained(&connection).unwrap();
            (ended, readmit(&connection).unwrap())
        }
        Order::ReadmissionFirst => {
            let readmitted = readmit(&connection).unwrap();
            (intake.close_if_drained(&connection).unwrap(), readmitted)
        }
    };

    let left = (
        pending(&connection),
        work_items::readmittable(&connection, false).unwrap(),
    );
    match order {
        Order::LastLookFirst => assert_eq!((ended, readmitted, left), (true, None, (vec![], 1))),
        Order::ReadmissionFirst => assert_eq!(
            (ended, readmitted, left),
            (false, Some(vec!["1".to_owned()]), (vec!["1".to_owned()], 0))
        ),
    }
}

/// A crawl's finish does not close its intake over a work item still
/// leased, whose attempt ended with its outcome unwritten: the finish gives
/// it back as a failed attempt, retried while it has retries left, and it
/// is delivered before the crawl may end (issue 2320).
#[test]
fn a_work_item_still_leased_holds_the_intake_open_until_it_is_given_back() {
    let connection = crawl_of(1);
    let ids = pending(&connection);
    work_items::lease(&connection, &ids).unwrap();
    let intake = crate::intake::Intake::default();

    let closed_over_a_lease = intake.close_if_drained(&connection).unwrap();
    work_items::recover_leases(
        &connection,
        &lost(),
        2,
        &ended_at(),
        &mut Changed::default(),
    )
    .unwrap();
    let given_back = pending(&connection);
    let closed_over_a_retry = intake.close_if_drained(&connection).unwrap();

    assert_eq!(
        (closed_over_a_lease, given_back, closed_over_a_retry),
        (false, ids, false)
    );
}

/// A seed added once the crawl's last look closed it to new work is not
/// admitted, so the call answers it as for an ended crawl, never added and
/// then left pending.
///
/// Covers: story:3-128
#[test]
fn a_seed_added_after_the_crawls_last_look_is_not_admitted() {
    let connection = crawl_of(1);
    end_each(&connection, |_| First::Succeeds);
    let intake = crate::intake::Intake::default();
    let seed = goodfetch_protocol::foxhound::crawl::Seed::Request(
        goodfetch_protocol::foxhound::crawl::RequestSeed {
            url: "https://shop.test/late".to_owned(),
            label: "page".to_owned(),
            payload: None,
            priority: 0,
            forefront: false,
            method: goodfetch_protocol::foxhound::frontier::HttpMethod::get(),
            body: None,
            key: None,
        },
    );
    let every_host = goodfetch_engine::egress::Policy::new(&["*"], &[] as &[&str]).unwrap();
    let invariants = fixture::within(&connection, goodfetch_frontier::Scope::default()).unwrap();
    let checked = fixture::checked(&invariants, [seed]);

    let ended = intake.close_if_drained(&connection).unwrap();
    let added = intake
        .admitting(|| {
            work_items::admit_seeds(
                &connection,
                &mut work_items::Positions::of(&connection)?,
                &invariants,
                &every_host,
                &checked,
            )
        })
        .unwrap();

    assert_eq!(
        (ended, added.is_none(), pending(&connection)),
        (true, true, vec![])
    );
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
        let readmitted = work_items::readmit(&connection, blocked, &mut Changed::default()).unwrap();

        prop_assert_eq!((&before, &readmitted), (&expected, &expected));
    }

    /// The request counts the report and the progress read, kept as each
    /// outcome commits, equal each ended request's latest outcome decoded
    /// in full, for any mix of endings: a re-admitted request counts once,
    /// as pending, until it ends again, then by its new outcome.
    ///
    /// Covers: story:3-149
    #[test]
    fn the_request_counts_kept_equal_each_ended_requests_latest_outcome(
        endings in proptest::collection::vec(first(), 1..12),
        again in proptest::collection::vec(first(), 12),
        blocked in any::<bool>(),
    ) {
        let connection = crawl_of(endings.len());
        end_each(&connection, |index| endings[index]);
        let ended = crate::report::ended(&connection).unwrap().0;
        let ended_model = decoded(&connection);
        work_items::readmit(&connection, blocked, &mut Changed::default()).unwrap();
        let readmitted = crate::report::ended(&connection).unwrap().0;
        let readmitted_model = decoded(&connection);
        end_each(&connection, |index| again[index]);

        prop_assert_eq!(
            (ended, readmitted, crate::report::ended(&connection).unwrap().0),
            (ended_model, readmitted_model, decoded(&connection))
        );
    }

    /// Covers: story:3-149
    #[test]
    fn a_re_admitted_requests_next_outcome_supersedes_its_last(
        endings in proptest::collection::vec(first(), 1..12),
    ) {
        let connection = crawl_of(endings.len());
        end_each(&connection, |index| endings[index]);
        let readmitted = work_items::readmit(&connection, true, &mut Changed::default()).unwrap();
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
