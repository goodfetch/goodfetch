//! The learned rules memory holds (#2169): read from the file once, by the
//! first admission that writes, then kept current as the writer learns or
//! stops one, so an admission keys a URL and asks whether a rule refuses
//! it with no read of the file.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use goodfetch_engine::run_state::WriteWatch as _;
use rusqlite::Connection;

use super::{RAN, RunFile, record, run_file, seeds, staged, within};
use crate::invariants::Invariants;
use crate::rules::pattern;
use crate::staged::Staged;

/// Commits, on `file`'s writer, the rule of `kind` on the shop about
/// `path` and `parameter`, as learning writes it; answers its ID.
fn learned(file: &RunFile, kind: &str, path: &str, parameter: &str) -> i64 {
    file.writer
        .query_row(
            "INSERT INTO learned_rules (kind, host, path, parameter, form, evidence)
             VALUES (?1, 'shop.test', ?2, ?3, '', '{}') RETURNING id",
            (kind, path, parameter),
            |row| row.get(0),
        )
        .unwrap()
}

/// The shop's rules: `utm` has no effect, the calendar's numbered pages
/// are a trap, and the list pages by a cursor rather than by `offset`.
fn shop_rules(file: &RunFile) {
    learned(file, "no_effect_parameter", "", "utm");
    learned(file, "trap_path", &pattern("/cal/7"), "");
    learned(file, "cursor_pagination", "/list", "offset");
}

/// What `act` answers, and how many statements it ran on `connection`
/// that read the learned rules.
fn rule_reads<T>(connection: &Connection, act: impl FnOnce() -> T) -> (T, usize) {
    RAN.with_borrow_mut(Vec::clear);
    connection.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
        Some(record),
    );
    let answered = act();
    connection.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    let reads = RAN
        .with_borrow_mut(std::mem::take)
        .into_iter()
        .filter(|statement| {
            statement.trim_start().starts_with("SELECT") && statement.contains("learned_rules")
        })
        .count();
    (answered, reads)
}

/// Admits each of `paths` on the shop on the writer, as [`seeds`] does,
/// answering whether each was added or why not, and how many statements
/// read the learned rules.
fn admitted(file: &RunFile, invariants: &Invariants, paths: &[&str]) -> (Vec<String>, usize) {
    rule_reads(&file.writer, || seeds(file, invariants, paths))
}

fn answers(answers: &[&str]) -> Vec<String> {
    answers.iter().map(|answer| (*answer).to_owned()).collect()
}

/// Within the budget, the first admission that writes reads the learned
/// rules once, and the next key a URL without a parameter with no effect,
/// refuse a trap's page and an offset its listing's cursor replaced, with
/// no read of the file.
#[test]
fn memory_holds_the_learned_rules_once_the_first_admission_that_writes_read_them() {
    let file = run_file();
    shop_rules(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    let first = admitted(&file, &invariants, &["p"]);
    let then = admitted(
        &file,
        &invariants,
        &["p?utm=1", "cal/7", "list?offset=20", "q"],
    );

    assert_eq!(
        (first, then),
        (
            (answers(&["added"]), 1),
            (answers(&["Duplicate", "TrapPath", "Duplicate", "added"]), 0),
        )
    );
}

/// A rule the writer learns once memory read the rules, and a contradiction
/// that stops one, hold from the next admission that writes, which reads
/// only the rules changed.
#[test]
fn a_rule_learned_or_stopped_on_the_writer_holds_from_the_next_admission() {
    let file = run_file();
    shop_rules(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    admitted(&file, &invariants, &["p"]);
    learned(&file, "no_effect_parameter", "", "ref");
    let after_learning = admitted(&file, &invariants, &["p?ref=1", "p?utm=1"]);
    file.writer
        .execute(
            "UPDATE learned_rules SET contradiction = 'a differing pair' WHERE parameter = 'utm'",
            [],
        )
        .unwrap();
    let after_stopping = admitted(&file, &invariants, &["p?utm=2", "p?ref=2"]);

    assert_eq!(
        (after_learning, after_stopping),
        (
            (answers(&["Duplicate", "Duplicate"]), 1),
            (answers(&["added", "Duplicate"]), 1),
        )
    );
}

/// A rule learned by a writer job whose work failed is rolled back with
/// it, and memory forgets it: the next job's admission reads the rules it
/// held again.
#[test]
fn a_rule_a_failed_job_learned_leaves_memory() {
    let file = run_file();
    shop_rules(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);
    let watch = invariants.unwritten().unwrap();

    file.writer.execute_batch("BEGIN").unwrap();
    watch.began();
    admitted(&file, &invariants, &["p"]);
    watch.began();
    file.writer.execute_batch("SAVEPOINT write").unwrap();
    learned(&file, "no_effect_parameter", "", "ref");
    let failed = admitted(&file, &invariants, &["p?ref=1"]);
    file.writer
        .execute_batch("ROLLBACK TO write; RELEASE write")
        .unwrap();
    watch.failed();
    watch.began();
    let again = admitted(&file, &invariants, &["p?ref=2", "p?utm=1"]);

    assert_eq!(
        (failed, again),
        (
            (answers(&["Duplicate"]), 1),
            (answers(&["added", "Duplicate"]), 1),
        )
    );
}

/// At a budget of 0, memory holds no rule: each admission reads them from
/// the file, as before.
#[test]
fn at_a_budget_of_0_the_rules_are_read_from_the_file() {
    let file = run_file();
    shop_rules(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(0, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    admitted(&file, &invariants, &["p"]);
    let (answered, reads) = admitted(&file, &invariants, &["p?utm=1", "cal/7"]);

    assert_eq!(answered, answers(&["Duplicate", "TrapPath"]));
    assert!(reads >= 2, "{reads} reads of the rules");
}

/// Once the writer changed a rule, and until an admission that writes
/// reads the rules again, an admission that does not write, a staged one,
/// reads them from the file rather than from memory, which is behind.
#[test]
fn an_admission_that_does_not_write_reads_the_file_while_memory_is_behind() {
    let file = run_file();
    shop_rules(&file);
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    admitted(&file, &invariants, &["p"]);
    learned(&file, "no_effect_parameter", "", "ref");
    let ((_, answered), reads) = rule_reads(&file.writer, || {
        staged(&file, &invariants, Staged::default(), &["p?ref=1"])
    });

    assert_eq!(answered, answers(&["Duplicate"]));
    assert!(reads >= 1, "{reads} reads of the rules");
}

/// The rules memory holds count in its bytes.
#[test]
fn the_rules_memory_holds_count_in_its_bytes() {
    let held = |rules: bool| {
        let file = run_file();
        if rules {
            shop_rules(&file);
        }
        let commits = Arc::new(AtomicU64::new(0));
        let invariants = within(1 << 20, &commits);
        file.writer.execute_batch("BEGIN").unwrap();
        admitted(&file, &invariants, &["p"]);
        invariants.unwritten().unwrap().bytes()
    };

    assert!(held(true) > held(false));
}
