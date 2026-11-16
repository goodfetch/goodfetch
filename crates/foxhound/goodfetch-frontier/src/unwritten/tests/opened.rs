//! The crawl's opening (#2169): once memory read the file's visits and
//! seeds on a reader, then, on the writer, the learned rules and
//! identities, the Bounds' counts and whether a trap mark is held, an
//! admission below the budget reads none of the Frontier's state from the
//! file, and counts no `F3.state_reads`.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use rusqlite::Connection;
use url::Url;

use super::{RAN, RunFile, answer, bounds, every_host, record, run_file, staged};
use crate::admit::{Candidate, Keyed, admit};
use crate::invariants::Invariants;
use crate::scope::{self, Narrowing, Scope};
use crate::staged::Staged;
use crate::trap_links::{self, Mark, seen_again, suspect, suspected};
use crate::unwritten::SharedBudget;
use crate::{Link, key_link};

/// The tables that hold the Frontier's state an admission reads.
const STATE: [&str; 7] = [
    "learned_rules",
    "learned_identities",
    "visited",
    "scope_seeds",
    "bounds_admitted",
    "bounds_path_counts",
    "trap_link_suspected",
];

/// A run file whose scope holds the shop, with the trap marks' table and
/// one learned rule, committed: `utm` has no effect on the shop.
fn state_file() -> RunFile {
    let file = run_file();
    for statement in trap_links::TABLES.statements {
        file.writer.execute(statement, []).unwrap();
    }
    file.writer
        .execute(
            "INSERT INTO learned_rules (kind, host, path, parameter, form, evidence)
             VALUES ('no_effect_parameter', 'shop.test', '', 'utm', '', '{}')",
            [],
        )
        .unwrap();
    file
}

/// The Run's invariants, within a budget that keeps every visit, with
/// `crawl.max_requests` and a path budget, counting its work on `metrics`;
/// when `open`, memory read the Frontier's state as the crawl opens.
fn opened(file: &RunFile, commits: &Arc<AtomicU64>, metrics: &Metrics, open: bool) -> Invariants {
    let scope = Scope::new(
        scope::Strategy::SameHostname,
        scope::Patterns::default(),
        scope::Patterns::default(),
    );
    let invariants = Invariants::new(scope, bounds(100)).unwrap();
    let commits = Arc::clone(commits);
    invariants.hold_unwritten(
        SharedBudget::new(1 << 20),
        move || commits.load(Ordering::SeqCst),
        metrics,
    );
    if open {
        invariants.fill_memory(&file.reader).unwrap();
        crate::keep_memory(&invariants, &file.writer).unwrap();
    }
    invariants
}

/// The shop's link `href`, keyed.
fn keyed(invariants: &Invariants, href: &str) -> Keyed {
    key_link(
        invariants,
        &every_host(),
        Link {
            href,
            base: &shop(),
            encoding: encoding_rs::UTF_8,
            keeps_fragment: false,
            narrowing: Narrowing::none(),
        },
    )
}

fn shop() -> Url {
    Url::parse("http://shop.test/").unwrap()
}

/// Admits each of `hrefs`, links of the shop's page, on the writer, its
/// `href` its request's ID, answering whether each was added or why not.
fn linked(file: &RunFile, invariants: &Invariants, hrefs: &[&str]) -> Vec<String> {
    let base = shop();
    hrefs
        .iter()
        .map(|href| {
            let enqueued = admit(
                &file.writer,
                invariants,
                &every_host(),
                Candidate::Href {
                    href,
                    base: &base,
                    encoding: encoding_rs::UTF_8,
                    keeps_fragment: false,
                    from: &base,
                    narrowing: Narrowing::none(),
                    depth: 1,
                },
                |_| Ok((*href).to_owned()),
            )
            .unwrap();
            answer(&enqueued)
        })
        .collect()
}

/// What `act` answers, and each statement it ran on `connection` that
/// reads the Frontier's state.
fn state_reads<T>(connection: &Connection, act: impl FnOnce() -> T) -> (T, Vec<String>) {
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
            statement.trim_start().starts_with("SELECT")
                && STATE.iter().any(|table| statement.contains(table))
        })
        .collect();
    (answered, reads)
}

/// Links that are added, merged by a learned rule, refused by a path
/// budget and seen again, then staged ones: what they answer, and their
/// admissions' reads of the Frontier's state.
fn admissions(
    file: &RunFile,
    invariants: &Invariants,
) -> ((Vec<String>, Vec<String>), Vec<String>) {
    state_reads(&file.writer, || {
        let linked = linked(file, invariants, &["/a", "/a?utm=1", "/b/1", "/b/2"]);
        let again = keyed(invariants, "/a");
        seen_again(&file.writer, invariants.unwritten(), &again, false).unwrap();
        let (_, staged) = staged(file, invariants, Staged::default(), &["c", "b/3"]);
        (linked, staged)
    })
}

fn answers(answers: &[&str]) -> Vec<String> {
    answers.iter().map(|answer| (*answer).to_owned()).collect()
}

/// Once the crawl opened, admissions that write and staged ones key a URL
/// under a learned rule, ask the visited set, the Scope's seeds and the
/// Bounds' counts, and probe the trap marks with no read of the file.
#[test]
fn once_the_crawl_opened_an_admission_reads_none_of_the_frontiers_state() {
    let file = state_file();
    let commits = Arc::new(AtomicU64::new(0));
    let metrics = Metrics::none();
    let invariants = opened(&file, &commits, &metrics, true);

    file.writer.execute_batch("BEGIN").unwrap();
    let (answered, reads) = admissions(&file, &invariants);

    assert_eq!(
        (
            answered,
            reads,
            metrics.counted_work().read(Row::StateReads)
        ),
        (
            (
                answers(&["added", "Duplicate", "added", "PathBudget"]),
                answers(&["added", "PathBudget"]),
            ),
            Vec::<String>::new(),
            0,
        )
    );
}

/// Before the opening, the same admissions read the Frontier's state from
/// the file, each read counted in `F3.state_reads`.
#[test]
fn before_the_opening_an_admission_reads_the_frontiers_state_counted() {
    let file = state_file();
    let commits = Arc::new(AtomicU64::new(0));
    let metrics = Metrics::none();
    let invariants = opened(&file, &commits, &metrics, false);

    file.writer.execute_batch("BEGIN").unwrap();
    let (_, reads) = admissions(&file, &invariants);

    let counted = metrics.counted_work().read(Row::StateReads);
    assert!(
        !reads.is_empty() && counted > 0,
        "{counted} counted for {reads:?}"
    );
}

/// A link the writer marks once the crawl opened has the next link seen
/// again clear its request's mark, keyed and looked up in memory, with no
/// read of the file.
#[test]
fn a_mark_after_the_opening_is_cleared_with_no_read() {
    let file = state_file();
    let commits = Arc::new(AtomicU64::new(0));
    let metrics = Metrics::none();
    let invariants = opened(&file, &commits, &metrics, true);

    file.writer.execute_batch("BEGIN").unwrap();
    linked(&file, &invariants, &["/a"]);
    suspect(&file.writer, invariants.unwritten(), "/a", Mark::Hidden).unwrap();
    let again = keyed(&invariants, "/a");
    let ((), reads) = state_reads(&file.writer, || {
        seen_again(&file.writer, invariants.unwritten(), &again, false).unwrap();
    });

    assert_eq!(
        (
            reads,
            suspected(&file.writer, "/a").unwrap(),
            metrics.counted_work().read(Row::StateReads)
        ),
        (Vec::<String>::new(), None, 0)
    );
}
