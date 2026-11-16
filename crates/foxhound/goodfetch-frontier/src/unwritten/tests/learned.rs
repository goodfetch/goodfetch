//! The learned identities memory holds (#2169): read from the file once,
//! by the first admission that writes, then kept as the writer visits, and
//! read again for a rule's hosts once the writer learns or stops a rule
//! that merges, so an admission asks whether a visited request's learned
//! identity is its own with no read of the file.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use goodfetch_engine::run_state::WriteWatch as _;

use super::{RAN, RunFile, commit, record, run_file, seeds, within};
use crate::identity::{merge_learned, unmerge};
use crate::invariants::Invariants;
use crate::rules::Merging;

/// Commits, on `file`'s writer, the rule that `parameter` has no effect on
/// the shop, as learning writes it, and re-keys the shop's visited set as
/// learning then does; answers the rule.
fn learned(file: &RunFile, parameter: &str) -> Merging {
    let rule = file
        .writer
        .query_row(
            "INSERT INTO learned_rules (kind, host, path, parameter, form, evidence)
             VALUES ('no_effect_parameter', 'shop.test', '', ?1, '', '{}') RETURNING id",
            [parameter],
            |row| row.get(0),
        )
        .unwrap();
    let merging = Merging {
        rule,
        hosts: vec!["shop.test".to_owned()],
    };
    merge_learned(&file.writer, &merging).unwrap();
    merging
}

/// Stops `merging`'s rule by a contradiction, and re-keys the shop's
/// visited set as learning then does.
fn stopped(file: &RunFile, merging: &Merging) {
    file.writer
        .execute(
            "UPDATE learned_rules SET contradiction = 'a differing pair' WHERE id = ?1",
            [merging.rule],
        )
        .unwrap();
    unmerge(&file.writer, merging).unwrap();
}

/// Admits each of `paths` on the shop on the writer, as [`seeds`] does,
/// answering whether each was added or why not, and how many statements
/// it ran on the writer that read the learned identities.
fn admitted(file: &RunFile, invariants: &Invariants, paths: &[&str]) -> (Vec<String>, usize) {
    RAN.with_borrow_mut(Vec::clear);
    file.writer.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
        Some(record),
    );
    let answered = seeds(file, invariants, paths);
    file.writer
        .trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    let reads = RAN
        .with_borrow_mut(std::mem::take)
        .into_iter()
        .filter(|statement| {
            statement.trim_start().starts_with("SELECT") && statement.contains("learned_identities")
        })
        .count();
    (answered, reads)
}

fn answers(answers: &[&str]) -> Vec<String> {
    answers.iter().map(|answer| (*answer).to_owned()).collect()
}

/// The shop's rule that `utm` has no effect, learned before the crawl.
fn utm(file: &RunFile) {
    learned(file, "utm");
}

/// Within the budget, the first admission that writes reads the learned
/// identities once; from then on a visited request's learned identity
/// makes a URL a duplicate, in its batch and once the batch commits, and
/// a new one is added, with no read of the file.
#[test]
fn memory_holds_the_learned_identities_once_the_first_admission_that_writes_read_them() {
    let file = run_file();
    utm(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    let first = admitted(&file, &invariants, &["p?utm=1"]);
    let in_its_batch = admitted(&file, &invariants, &["p?utm=2", "p", "q"]);
    commit(&file, &commits, 1);
    file.writer.execute_batch("BEGIN").unwrap();
    let once_committed = admitted(&file, &invariants, &["p?utm=3", "r?utm=1"]);

    assert_eq!(
        (first, in_its_batch, once_committed),
        (
            (answers(&["added"]), 1),
            (answers(&["Duplicate", "Duplicate", "added"]), 0),
            (answers(&["Duplicate", "added"]), 0),
        )
    );
}

/// A rule the writer learns re-keys its host's visited set, and a
/// contradiction that stops it takes those identities back: the next
/// admission that writes reads the learned identities of the rule's hosts
/// again, and those after it read none.
#[test]
fn a_rule_learned_or_stopped_on_the_writer_reads_its_hosts_identities_again() {
    let file = run_file();
    utm(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    admitted(&file, &invariants, &["p?ref=1"]);
    let rule = learned(&file, "ref");
    let (after_learning, read_again) = admitted(&file, &invariants, &["p"]);
    let then = admitted(&file, &invariants, &["p?ref=2", "q?ref=1"]);
    stopped(&file, &rule);
    let (after_stopping, read_once_more) = admitted(&file, &invariants, &["p", "q?ref=2"]);
    let last = admitted(&file, &invariants, &["s"]);

    assert_eq!(
        (
            (after_learning, read_again > 0),
            then,
            (after_stopping, read_once_more > 0),
            last
        ),
        (
            (answers(&["Duplicate"]), true),
            (answers(&["Duplicate", "added"]), 0),
            (answers(&["added", "added"]), true),
            (answers(&["added"]), 0),
        )
    );
}

/// A rule learned by a writer job whose work failed is rolled back with
/// the identities it re-keyed, and memory forgets them: the next job's
/// admission reads its host's identities again.
#[test]
fn the_identities_a_failed_jobs_rule_merged_leave_memory() {
    let file = run_file();
    utm(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);
    let watch = invariants.unwritten().unwrap();

    file.writer.execute_batch("BEGIN").unwrap();
    watch.began();
    admitted(&file, &invariants, &["p?ref=1"]);
    watch.began();
    file.writer.execute_batch("SAVEPOINT write").unwrap();
    learned(&file, "ref");
    let (failed, _) = admitted(&file, &invariants, &["p"]);
    file.writer
        .execute_batch("ROLLBACK TO write; RELEASE write")
        .unwrap();
    watch.failed();
    watch.began();
    let (again, _) = admitted(&file, &invariants, &["p"]);
    let then = admitted(&file, &invariants, &["p?utm=1", "q"]);

    assert_eq!(
        (failed, again, then),
        (
            answers(&["Duplicate"]),
            answers(&["added"]),
            (answers(&["Duplicate", "added"]), 0),
        )
    );
}

/// At a budget of 0, memory holds no learned identity the file holds:
/// each admission's lookup reads the file, as before.
#[test]
fn at_a_budget_of_0_the_learned_identities_are_read_from_the_file() {
    let file = run_file();
    utm(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(0, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    admitted(&file, &invariants, &["p?utm=1"]);
    commit(&file, &commits, 1);
    file.writer.execute_batch("BEGIN").unwrap();
    let (answered, reads) = admitted(&file, &invariants, &["p?utm=2", "q"]);

    assert_eq!(
        (answered, reads > 0),
        (answers(&["Duplicate", "added"]), true)
    );
}
