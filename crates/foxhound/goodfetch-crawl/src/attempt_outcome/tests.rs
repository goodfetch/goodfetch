//! An attempt's outcome on a real run file: its learning fails alone, a
//! failed attempt keeps what it learned, and one page's failing outcome
//! rolls back alone in its group. Its crash model is the lease's
//! (`lease::tests::model`), whose commits are outcomes.
#![expect(
    clippy::disallowed_methods,
    reason = "tests read the test Clock (ADR 0059)"
)]

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::foxhound::crawl::{PushedRecord, RequestSeed, Seed};
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::run::PointInTime;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::json;

use super::{Ended, Ending, Learning, Outcome};
use crate::lease::{Leases, WorkItemLease};
use crate::staging::Staging;
use crate::work_items::{self, Failure, Leased, Unsuccessful};

/// Two pages' work items leased in a run file under `env`, with their
/// leases, which stage in `staging`; `without_lessons`, the table of the
/// pages the Frontier learned from is gone, so no page can teach it.
struct Pages {
    state: RunState,
    owner: TaskOwner,
    staging: Staging,
    leased: Vec<(Leased, WorkItemLease)>,
}

/// Every table a crawl's outcome writes in.
fn tables() -> Vec<RunStateTables> {
    vec![
        work_items::TABLES,
        crate::labels::TABLES,
        goodfetch_dataset::TABLES,
        goodfetch_frontier::VISITED_TABLES,
        goodfetch_frontier::SCOPE_TABLES,
        goodfetch_frontier::BOUNDS_TABLES,
        goodfetch_frontier::payload::TABLES,
        goodfetch_frontier::sitemap::TABLES,
        goodfetch_frontier::trap_links::TABLES,
        goodfetch_frontier::RULES_TABLES,
    ]
}

fn seed(url: &str) -> Seed {
    Seed::Request(RequestSeed {
        url: url.to_owned(),
        label: "page".to_owned(),
        payload: None,
        priority: 0,
        forefront: false,
        method: HttpMethod::get(),
        body: None,
        key: None,
    })
}

fn ended_at() -> PointInTime {
    PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned())
}

async fn pages(env: &TestEnv, without_lessons: bool) -> Pages {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let state = RunState::open(
        env.folder().join("run.sqlite"),
        &RunStateOptions::default(),
        tables(),
        1,
        &owner,
    )
    .await
    .unwrap();
    let datasets = goodfetch_dataset::Datasets::declare(&[DatasetDeclaration {
        name: "pages".to_owned(),
        schema: None,
        key: Vec::new(),
    }])
    .unwrap();
    let declared = datasets.clone();
    let leased = state
        .write(move |connection| {
            declared.record(connection)?;
            work_items::admit_seeds(
                connection,
                &mut work_items::Positions::of(connection)?,
                &goodfetch_frontier::Scope::default(),
                &goodfetch_engine::egress::Policy::new(&["*"], &[] as &[&str]).unwrap(),
                &[seed("https://shop.test/1"), seed("https://shop.test/2")],
            )?;
            if without_lessons {
                connection.execute_batch("DROP TABLE rules_pages")?;
            }
            work_items::lease(connection, &["1".to_owned(), "2".to_owned()])
        })
        .await
        .unwrap();
    let staging = Staging::default();
    let granting = Leases::new(Clock::paused(env.start_time()), "outcome", 1)
        .checking_against(datasets)
        .staging_in(staging.clone());
    let leased = leased
        .into_iter()
        .map(|leased| {
            let lease = granting.grant(
                &leased.crawl_request.id,
                leased.lease_epoch,
                Duration::from_secs(70),
            );
            (leased, lease)
        })
        .collect();
    Pages {
        state,
        owner,
        staging,
        leased,
    }
}

/// What the page of `leased` teaches the Frontier.
fn learning(leased: &Leased, signature: u8) -> Learning {
    Learning {
        request: leased.crawl_request.clone(),
        url: goodfetch_frontier::Url::parse(&leased.crawl_request.url).unwrap(),
        signature: [signature; 32],
        canonical: None,
        ended_at: ended_at(),
    }
}

/// `leased`'s outcome under `lease`: one Record committed, with what its
/// page taught.
fn committing(
    state: &RunState,
    (leased, lease): &(Leased, WorkItemLease),
    signature: u8,
) -> Outcome {
    let records = vec![PushedRecord {
        dataset: "pages".to_owned(),
        data: json!({ "page": leased.crawl_request.id }),
    }];
    Outcome {
        scored: goodfetch_session_pool::Changes::default(),
        chosen: None,
        learning: Some(learning(leased, signature)),
        ending: Ending::Commit {
            commit: lease
                .committing(state, &Masks::new(), records, ended_at())
                .unwrap(),
            refused: Box::new(|refused| panic!("a Record was refused: {refused:?}")),
        },
    }
}

/// Each work item's ID and status, the pages the Frontier learned, and
/// the Records kept, as the run file holds them.
fn stored(env: &TestEnv, with_rules: bool) -> (Vec<(String, String)>, Vec<String>, usize) {
    let connection = Connection::open(env.folder().join("run.sqlite")).unwrap();
    let statuses = connection
        .prepare("SELECT id, status FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let learned = if with_rules {
        connection
            .prepare("SELECT crawl_request FROM rules_pages ORDER BY crawl_request")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    } else {
        Vec::new()
    };
    let records: i64 = connection
        .query_row("SELECT count(*) FROM records", [], |row| row.get(0))
        .unwrap();
    (statuses, learned, usize::try_from(records).unwrap())
}

/// A page whose lesson the run file cannot take, here for want of the
/// table of the pages the Frontier learned from, still commits its
/// outcome: learning fails alone.
///
/// Covers: story:3-41
#[test]
fn an_outcome_whose_learning_fails_still_commits() {
    let env = TestEnv::new();
    goodfetch_test_support::runtime().block_on(async {
        let pages = pages(&env, true).await;
        let outcome = committing(&pages.state, &pages.leased[0], 1);

        let written = pages
            .state
            .write(move |connection| outcome.write(connection))
            .await;

        assert!(
            matches!(written, Ok((Ended::Committed(_), _))),
            "the outcome did not commit: {written:?}"
        );
        drop(pages.leased);
        drop(pages.state);
        pages.owner.wait().await;
    });
    assert_eq!(
        stored(&env, false),
        (
            vec![
                ("1".to_owned(), "succeeded".to_owned()),
                ("2".to_owned(), "leased".to_owned())
            ],
            Vec::new(),
            1
        )
    );
}

/// An attempt that failed after its page was fetched keeps what its page
/// taught the Frontier.
///
/// Covers: story:3-41
#[test]
fn a_failed_attempt_still_commits_what_it_learned() {
    let env = TestEnv::new();
    goodfetch_test_support::runtime().block_on(async {
        let pages = pages(&env, false).await;
        let (leased, _) = &pages.leased[0];
        let outcome = Outcome {
            scored: goodfetch_session_pool::Changes::default(),
            chosen: None,
            learning: Some(learning(leased, 1)),
            ending: Ending::End {
                work_item: leased.crawl_request.id.clone(),
                lease_epoch: leased.lease_epoch,
                unsuccessful: Unsuccessful::Failed(Failure::new("handler_failed", "it threw")),
                ended_at: ended_at(),
            },
        };

        let written = pages
            .state
            .write(move |connection| outcome.write(connection))
            .await
            .unwrap();

        assert!(
            matches!(written.0, Ended::Unsuccessful { ended: true, .. }),
            "{written:?}"
        );
        drop(pages.leased);
        drop(pages.state);
        pages.owner.wait().await;
    });
    assert_eq!(
        stored(&env, true),
        (
            vec![
                ("1".to_owned(), "failed".to_owned()),
                ("2".to_owned(), "leased".to_owned())
            ],
            vec!["1".to_owned()],
            0
        )
    );
}

/// Two pages' outcomes queued together, so the writer groups them: the
/// first's link cannot be admitted, and its outcome rolls back whole, its
/// lesson with it, while the second's commits.
///
/// Covers: story:3-41
#[test]
fn one_pages_failing_outcome_rolls_back_alone() {
    let env = TestEnv::new();
    goodfetch_test_support::runtime().block_on(async {
        let pages = pages(&env, false).await;
        let _ = pages
            .staging
            .defer("1", pages.leased[0].0.lease_epoch, |_, _| {
                Err(rusqlite::Error::InvalidQuery)
            })
            .unwrap();
        let first = committing(&pages.state, &pages.leased[0], 1);
        let second = committing(&pages.state, &pages.leased[1], 2);

        let first = pages
            .state
            .reserve()
            .await
            .unwrap()
            .write(move |connection| first.write(connection));
        let second = pages
            .state
            .reserve()
            .await
            .unwrap()
            .write(move |connection| second.write(connection));
        let (first, second) = tokio::join!(first, second);

        assert!(
            first.is_err() && matches!(second, Ok((Ended::Committed(_), _))),
            "{first:?}, {second:?}"
        );
        drop(pages.leased);
        drop(pages.state);
        pages.owner.wait().await;
    });
    assert_eq!(
        stored(&env, true),
        (
            vec![
                ("1".to_owned(), "leased".to_owned()),
                ("2".to_owned(), "succeeded".to_owned())
            ],
            vec!["2".to_owned()],
            1
        )
    );
}
