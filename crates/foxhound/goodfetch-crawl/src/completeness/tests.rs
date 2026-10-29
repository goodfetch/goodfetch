//! A crawl's Completeness reasons as a function of its run file (spec
//! #581's Testing Decisions: pure rules in process): the requests whose
//! latest outcome is a skip for the Model allowance make it `incomplete`,
//! counted, a request skipped for it and then handled counting none; no
//! fetch tier or AI verb raises the skip on Foxhound's executable yet
//! (#1102), so this is where the reason is pinned (ADR 0107).

use goodfetch_engine::runs::CompletenessReason;
use goodfetch_frontier::{BOUNDS_TABLES, RULES_TABLES};
use goodfetch_protocol::foxhound::outcomes::{
    CrawlRequestOutcome, DatasetRecords, Ending, OutcomeRequest, SkipReason,
};
use goodfetch_protocol::run::{AxisReason, Completeness, PointInTime};
use rusqlite::Connection;

use super::short_of_scope;
use crate::work_items::TABLES;

/// A run file in memory holding the crawl's tables and, in commit order,
/// each work item's outcome.
fn run_file(outcomes: &[(&str, Ending)]) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for tables in [TABLES, BOUNDS_TABLES, RULES_TABLES] {
        for statement in tables.statements {
            connection.execute_batch(statement).unwrap();
        }
    }
    for (position, (work_item, ending)) in (1_u64..).zip(outcomes) {
        let outcome = CrawlRequestOutcome {
            position,
            request: OutcomeRequest {
                id: (*work_item).to_owned(),
                url: format!("https://shop.test/{work_item}"),
                label: "page".to_owned(),
                depth: 0,
            },
            attempts: 1,
            errors: Vec::new(),
            ended_at: PointInTime::from_rfc3339("2026-10-24T06:00:00Z".to_owned()),
            ending: ending.clone(),
        };
        connection
            .execute(
                "INSERT INTO outcomes (position, work_item, kind, label, attempts, outcome)
                 VALUES (?1, ?2, 'any', 'page', 1, ?3)",
                (
                    i64::try_from(position).unwrap(),
                    work_item,
                    serde_json::to_string(&outcome).unwrap(),
                ),
            )
            .unwrap();
    }
    connection
}

fn skipped(reason: SkipReason) -> Ending {
    Ending::Skipped {
        reason,
        message: Some("The Model allowance was reached.".to_owned()),
    }
}

fn succeeded() -> Ending {
    Ending::Succeeded {
        records: vec![DatasetRecords {
            dataset: "books".to_owned(),
            committed: 1,
            dropped: 0,
            rejected: 0,
        }],
        nothing_committed: None,
        not_added: Vec::new(),
    }
}

/// Covers: story:6-309
#[test]
fn requests_the_model_allowance_skipped_leave_the_crawl_incomplete_counted() {
    let file = run_file(&[
        ("1", skipped(SkipReason::ModelAllowance)),
        ("2", skipped(SkipReason::ModelAllowance)),
        ("3", skipped(SkipReason::Handler)),
        ("4", succeeded()),
        ("5", skipped(SkipReason::ModelAllowance)),
        ("5", succeeded()),
    ]);

    assert_eq!(
        short_of_scope(&file).unwrap(),
        vec![CompletenessReason {
            completeness: Completeness::Incomplete,
            reason: AxisReason {
                code: "model_allowance".to_owned(),
                message: "The Model allowance skipped 2 requests the crawl would otherwise have handled; its `skipped` outcomes with `model_allowance` list them.".to_owned(),
                count: Some(2),
                call_id: None,
            },
        }]
    );
}

/// Covers: story:6-309
#[test]
fn a_crawl_nothing_cut_has_no_reason() {
    let file = run_file(&[("1", succeeded()), ("2", skipped(SkipReason::Handler))]);

    assert_eq!(short_of_scope(&file).unwrap(), Vec::new());
}
