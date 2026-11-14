//! What the crawl's Run state counts of its requests by state from what
//! each job answers that it changed (ADR 0142; #2167), with no run file.
//! That the count equals the run file's after every job is the Run state's
//! model test's.

use goodfetch_protocol::foxhound::outcomes::OutcomeKind;
use goodfetch_protocol::foxhound::report::SitemapReport;

use super::Tally;
use crate::work_items::{Changed, CountedAs};

fn failed(code: &str) -> CountedAs {
    (OutcomeKind::Failed, code.to_owned())
}

fn succeeded() -> CountedAs {
    (OutcomeKind::Succeeded, String::new())
}

fn duplicate() -> CountedAs {
    (OutcomeKind::Skipped, "duplicate".to_owned())
}

/// What `tally` counts: the work items left, and the ended requests of
/// each kind by what they count under.
fn counted(tally: &Tally) -> (u64, Vec<(OutcomeKind, String, u64)>) {
    let kinds = [
        OutcomeKind::Succeeded,
        OutcomeKind::Failed,
        OutcomeKind::Skipped,
        OutcomeKind::Blocked,
    ];
    let ended = kinds
        .into_iter()
        .flat_map(|kind| {
            tally
                .ended(kind)
                .map(move |(name, count)| (kind, name.to_owned(), count))
        })
        .collect();
    (tally.left(), ended)
}

/// A work item added is left to deliver until its request ends, which
/// counts under its ending; a sitemap's fetch that ends counts under
/// nothing; a request made pending again leaves its ending's count and is
/// left to deliver again.
#[test]
fn a_tally_counts_each_job_s_changes_as_the_statements_that_made_them_did() {
    let mut tally = Tally::default();

    tally.change(Changed {
        added: 5,
        ..Changed::default()
    });
    tally.change(Changed {
        ended: vec![succeeded(), failed("timeout"), failed("timeout")],
        fetches_ended: 1,
        ..Changed::default()
    });
    let ended = counted(&tally);
    tally.change(Changed {
        reopened: vec![failed("timeout")],
        fetches_reopened: 1,
        ..Changed::default()
    });

    assert_eq!(
        (ended, counted(&tally)),
        (
            (
                1,
                vec![
                    (OutcomeKind::Succeeded, String::new(), 1),
                    (OutcomeKind::Failed, "timeout".to_owned(), 2),
                ]
            ),
            (
                3,
                vec![
                    (OutcomeKind::Succeeded, String::new(), 1),
                    (OutcomeKind::Failed, "timeout".to_owned(), 1),
                ]
            ),
        )
    );
}

/// A handler attempt's lesson is counted when its attempt ends, so the
/// request it merged may be given back by a later job first: the count
/// it then leaves shows as none, never below zero, and the merge counted
/// afterwards leaves the tally as if both had come in order.
#[test]
fn a_request_given_back_before_its_merge_is_counted_shows_no_count_until_then() {
    let mut tally = Tally::default();
    tally.change(Changed {
        added: 1,
        ..Changed::default()
    });

    tally.change(Changed {
        reopened: vec![duplicate()],
        ..Changed::default()
    });
    let before = counted(&tally);
    tally.change(Changed {
        ended: vec![duplicate()],
        ..Changed::default()
    });

    let mut in_order = Tally::default();
    in_order.change(Changed {
        added: 1,
        ..Changed::default()
    });
    assert_eq!(
        (before, &tally),
        ((2, Vec::new()), &in_order),
        "the merge and its giving back cancel out"
    );
}

/// The sitemap seeds' reports are those of the last job that read them: a
/// job that changed none leaves them as they were.
#[test]
fn a_tally_keeps_the_sitemap_reports_of_the_last_job_that_changed_them() {
    let report = SitemapReport {
        seed: "https://shop.test/sitemap.xml".to_owned(),
        read: 1,
        failed: 0,
        failures: Vec::new(),
        added: 3,
        not_added: 0,
        not_added_by_reason: Vec::new(),
    };
    let mut tally = Tally::default();

    tally.change(Changed {
        sitemaps: Some(vec![report.clone()]),
        ..Changed::default()
    });
    tally.change(Changed {
        added: 1,
        ..Changed::default()
    });

    assert_eq!(tally.sitemaps(), [report]);
}
