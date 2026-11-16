//! The visits a batching writer has not committed, read beside the run
//! file by a reader (#2168): a reader's hop sees them, an admission on the
//! writer sees a chain's pending end, and each is forgotten once the file
//! holds it or its visit is dropped unrun.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_engine::egress::Policy;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::WriteWatch;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason, PathBudget};
use rusqlite::Connection;
use tempfile::TempDir;
use url::Url;

use crate::admit::{Candidate, admit, admit_in};
use crate::bounds::{self, Bounds};
use crate::identity::{
    self, Declared, Fixed, Hop, Key, Reached, Seen, admit_hop, admit_reached, fixed_only,
    hold_reached, redirect_hop, seen, visit_pending,
};
use crate::invariants::Invariants;
use crate::scope::{self, Scope};
use crate::staged::{Admitting, Staged};

/// A run file on disk in WAL mode, with the writer's connection and a
/// reader's, as a Run's writer and readers hold it.
struct RunFile {
    _folder: TempDir,
    writer: Connection,
    reader: Connection,
}

fn run_file() -> RunFile {
    let folder = TempDir::new().unwrap();
    let path = folder.path().join("run.sqlite");
    let writer = Connection::open(&path).unwrap();
    writer
        .query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
        .unwrap();
    for statement in identity::TABLES
        .statements
        .iter()
        .chain(scope::TABLES.statements)
        .chain(bounds::TABLES.statements)
        .chain(crate::rules::TABLES.statements)
    {
        writer.execute(statement, []).unwrap();
    }
    // The scope holds the shop, committed: a reader's hop there is in it.
    let widening = Invariants::new(Scope::default(), Bounds::default()).unwrap();
    seed(&writer, &widening, "http://shop.test/", "0");
    let reader = Connection::open(&path).unwrap();
    RunFile {
        _folder: folder,
        writer,
        reader,
    }
}

/// The Run's invariants, holding the visits its writer has not committed
/// against `commits`, the commits it acknowledged, at a budget of 0: once
/// the file holds a visit, memory keeps nothing of it.
fn holding(commits: &Arc<AtomicU64>) -> Invariants {
    within(0, commits)
}

/// The Run's invariants, holding the visits its writer has not committed
/// against `commits`, and keeping those it wrote within `budget` bytes.
fn within(budget: u64, commits: &Arc<AtomicU64>) -> Invariants {
    let invariants = Invariants::new(Scope::default(), Bounds::default()).unwrap();
    let commits = Arc::clone(commits);
    invariants.hold_unwritten(
        budget,
        move || commits.load(Ordering::SeqCst),
        &Metrics::none(),
    );
    invariants
}

/// The bytes memory holds, on a run file of its own, once it read the
/// file's visits when `filled`, and each of `seeds` was admitted in a
/// committed batch of its own, as request 1, 2 and on: the budget that
/// keeps them, and nothing more of what the file holds. Measured, since
/// the tables and queues holding them grow in steps.
fn holds(filled: bool, seeds: &[&str]) -> u64 {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let probe = within(1 << 20, &commits);
    if filled {
        probe.fill_memory(&file.reader).unwrap();
    }
    for (commit_number, url) in (1..).zip(seeds) {
        file.writer.execute_batch("BEGIN").unwrap();
        seed(&file.writer, &probe, url, &commit_number.to_string());
        commit(&file, &commits, commit_number);
    }
    probe.unwritten().unwrap().bytes()
}

/// A connection to a file that holds no table: a lookup on it that reads
/// the run file fails, so one that answers read nothing.
fn no_file() -> Connection {
    Connection::open_in_memory().unwrap()
}

/// Whether the visited set holds `url`'s identity, as an admission asks
/// it beside the visits memory holds, read on `connection`.
fn looked_up(
    connection: &Connection,
    identities_on: &Connection,
    invariants: &Invariants,
    url: &str,
) -> rusqlite::Result<Seen> {
    let get = HttpMethod::get();
    let identities = identity::identities(
        identities_on,
        &Key::of(&Url::parse(url).unwrap(), false),
        Declared {
            method: &get,
            body: None,
            key: None,
            keeps_fragment: false,
        },
    )?;
    let holding = invariants.unwritten().unwrap().lock();
    seen(connection, &identities, None, Some(&holding))
}

/// Whether the visited set holds `url`'s identity in a Run that does not
/// learn, beside the visits memory holds, read on `connection`: the
/// visited set alone, with no learned identity to read.
fn visited(connection: &Connection, invariants: &Invariants, url: &str) -> rusqlite::Result<Seen> {
    let get = HttpMethod::get();
    let fixed = Fixed::of(
        &Key::of(&Url::parse(url).unwrap(), false),
        Declared {
            method: &get,
            body: None,
            key: None,
            keeps_fragment: false,
        },
    );
    let holding = invariants.unwritten().unwrap().lock();
    seen(connection, &fixed_only(&fixed), None, Some(&holding))
}

/// Commits the batch open on `file`'s writer, as the writer's commit
/// `commit` acknowledged.
fn commit(file: &RunFile, commits: &AtomicU64, commit: u64) {
    file.writer.execute_batch("COMMIT").unwrap();
    commits.store(commit, Ordering::SeqCst);
}

fn every_host() -> Policy {
    Policy::new(&["*"], &[] as &[&str]).unwrap()
}

/// Admits the seed `url` on the writer as request `id`.
fn seed(connection: &Connection, invariants: &Invariants, url: &str, id: &str) -> Enqueued {
    let get = HttpMethod::get();
    admit(
        connection,
        invariants,
        &every_host(),
        Candidate::Seed {
            url,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        },
        |_| Ok(id.to_owned()),
    )
    .unwrap()
}

fn hop(connection: &Connection, invariants: &Invariants, crawl_request: &str, url: &str) -> Hop {
    let get = HttpMethod::get();
    admit_hop(
        connection,
        (invariants.scope(), invariants.unwritten()),
        &every_host(),
        crawl_request,
        (&Url::parse(url).unwrap(), &get, None),
    )
    .unwrap()
}

fn reached(
    connection: &Connection,
    invariants: &Invariants,
    crawl_request: &str,
    url: &str,
) -> Reached {
    let get = HttpMethod::get();
    hold_reached(
        invariants.unwritten().unwrap(),
        connection,
        crawl_request,
        (&Url::parse(url).unwrap(), &get, None),
    )
    .unwrap()
}

/// A seed the writer admitted in a batch not committed yet makes a
/// reader's hop to its key a duplicate, which the file alone does not;
/// once the batch commits, the file holds it and memory forgets it.
#[test]
fn a_visit_in_an_uncommitted_batch_makes_a_readers_hop_a_duplicate() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);
    let get = HttpMethod::get();

    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &invariants, "http://shop.test/a", "1");
    let from_the_file_alone = redirect_hop(
        &file.reader,
        &Scope::default(),
        &every_host(),
        "2",
        &Url::parse("http://shop.test/a").unwrap(),
        &get,
        None,
    )
    .unwrap();
    let beside_memory = hop(&file.reader, &invariants, "2", "http://shop.test/a");
    file.writer.execute_batch("COMMIT").unwrap();
    commits.store(1, Ordering::SeqCst);
    let committed = hop(&file.reader, &invariants, "2", "http://shop.test/a");
    let answered_by = invariants
        .unwritten()
        .unwrap()
        .lock()
        .visitor_or(
            &format!(
                "GET {}",
                identity::url_key(&Url::parse("http://shop.test/a").unwrap())
            ),
            || Ok(Some("the file".to_owned())),
        )
        .unwrap();

    assert_eq!(
        (from_the_file_alone, beside_memory, committed, answered_by),
        (
            Hop::Follow,
            Hop::Duplicate,
            Hop::Duplicate,
            Some("the file".to_owned())
        )
    );
}

/// A chain's end a reader held as pending makes the writer's admission of
/// its key, and another chain's end at it, a duplicate; dropped unrun, it
/// holds nothing, and the key is admitted.
#[test]
fn a_pending_chain_end_holds_its_key_until_its_visit_is_dropped_unrun() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);

    let Reached::Follow(pending) = reached(&file.reader, &invariants, "1", "http://shop.test/c")
    else {
        panic!("a key no request holds is followed");
    };
    let other_chain = matches!(
        reached(&file.reader, &invariants, "2", "http://shop.test/c"),
        Reached::Duplicate
    );
    let admitted_while_pending = seed(&file.writer, &invariants, "http://shop.test/c", "3");
    drop(pending);
    let admitted_once_dropped = seed(&file.writer, &invariants, "http://shop.test/c", "4");

    assert!(other_chain);
    assert!(matches!(
        admitted_while_pending,
        Enqueued::NotAdded {
            reason: NotAddedReason::Duplicate,
            ..
        }
    ));
    assert!(matches!(admitted_once_dropped, Enqueued::Added { id, .. } if id == "4"));
}

/// A chain's end visited by its job joins the batch: until the batch
/// commits memory holds it for a reader, and once it commits the file
/// does; the writer's own record of a chain's end does the same.
#[test]
fn a_visited_chain_end_is_held_until_its_batch_commits() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);
    let get = HttpMethod::get();

    let Reached::Follow(pending) = reached(&file.reader, &invariants, "1", "http://shop.test/c")
    else {
        panic!("a key no request holds is followed");
    };
    file.writer.execute_batch("BEGIN").unwrap();
    visit_pending(&file.writer, invariants.unwritten(), pending).unwrap();
    let on_the_writer = admit_reached(
        &file.writer,
        invariants.unwritten(),
        "2",
        (&Url::parse("http://shop.test/d").unwrap(), &get, None),
    )
    .unwrap();
    let before_the_commit = (
        hop(&file.reader, &invariants, "5", "http://shop.test/c"),
        hop(&file.reader, &invariants, "5", "http://shop.test/d"),
    );
    file.writer.execute_batch("COMMIT").unwrap();
    commits.store(1, Ordering::SeqCst);
    let after_the_commit = (
        hop(&file.reader, &invariants, "5", "http://shop.test/c"),
        hop(&file.reader, &invariants, "1", "http://shop.test/c"),
    );

    assert_eq!(on_the_writer, Hop::Follow);
    assert_eq!(before_the_commit, (Hop::Duplicate, Hop::Duplicate));
    assert_eq!(after_the_commit, (Hop::Duplicate, Hop::Follow));
}

/// A seed widening the scope in a batch not committed yet is told, so a
/// reader's out-of-scope answer from before it can be asked again.
#[test]
fn a_seed_widening_the_scope_in_an_uncommitted_batch_is_told() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(3));
    let invariants = holding(&commits);
    let unwritten = invariants.unwritten().unwrap();
    let before = unwritten.widened_since(3);

    seed(&file.writer, &invariants, "http://shop.test/a", "1");

    assert_eq!(
        (
            before,
            unwritten.widened_since(3),
            unwritten.widened_since(4)
        ),
        (false, true, false)
    );
}

/// Seeds a written batch visited stay in memory once it commits while
/// memory is within the budget, and a lookup of one reads nothing.
#[test]
fn a_visit_a_committed_batch_wrote_is_looked_up_in_memory_within_the_budget() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &invariants, "http://shop.test/a", "1");
    commit(&file, &commits, 1);
    let found = looked_up(&no_file(), &file.writer, &invariants, "http://shop.test/a");

    assert!(matches!(found, Ok(Seen::Visited(visitor)) if visitor == "1"));
}

/// Past the budget, the visits the oldest written batches made leave
/// memory first, and what stays is within the budget; one that left is
/// read from the run file.
#[test]
fn past_the_budget_the_oldest_written_visits_leave_memory_first() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let two_visits = holds(false, &["http://shop.test/a", "http://shop.test/b"]);
    let invariants = within(two_visits, &commits);

    for (commit_number, (url, id)) in [
        ("http://shop.test/a", "1"),
        ("http://shop.test/b", "2"),
        ("http://shop.test/c", "3"),
    ]
    .into_iter()
    .enumerate()
    {
        file.writer.execute_batch("BEGIN").unwrap();
        seed(&file.writer, &invariants, url, id);
        commit(&file, &commits, 1 + u64::try_from(commit_number).unwrap());
    }
    let in_memory = |url| match looked_up(&no_file(), &file.writer, &invariants, url) {
        Ok(Seen::Visited(visitor)) => Some(visitor),
        Ok(_) => panic!("a visit is held as it was admitted"),
        Err(_) => None,
    };
    let held = (
        in_memory("http://shop.test/a"),
        in_memory("http://shop.test/b"),
        in_memory("http://shop.test/c"),
    );
    let from_the_file = looked_up(
        &file.writer,
        &file.writer,
        &invariants,
        "http://shop.test/a",
    );

    assert_eq!(held, (None, Some("2".to_owned()), Some("3".to_owned())));
    assert!(invariants.unwritten().unwrap().bytes() <= two_visits);
    assert!(matches!(from_the_file, Ok(Seen::Visited(visitor)) if visitor == "1"));
}

/// At a budget of 0 memory keeps nothing a committed batch wrote: each
/// lookup reads the run file, the file-first Frontier (#2168).
#[test]
fn at_a_budget_of_0_memory_keeps_nothing_a_committed_batch_wrote() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);

    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &invariants, "http://shop.test/a", "1");
    commit(&file, &commits, 1);
    let found = looked_up(&no_file(), &file.writer, &invariants, "http://shop.test/a");

    assert!(found.is_err());
    assert_eq!(invariants.unwritten().unwrap().bytes(), 0);
}

/// A writer job whose work failed is rolled back to its savepoint while
/// its batch goes on: memory forgets what it visited, so its URL is
/// admitted again, and keeps what the job before it visited.
#[test]
fn a_failed_job_leaves_nothing_it_visited_in_memory() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);
    let watch = invariants.unwritten().unwrap();

    file.writer.execute_batch("BEGIN").unwrap();
    watch.began();
    seed(&file.writer, &invariants, "http://shop.test/b", "1");
    watch.began();
    file.writer.execute_batch("SAVEPOINT write").unwrap();
    seed(&file.writer, &invariants, "http://shop.test/a", "2");
    file.writer
        .execute_batch("ROLLBACK TO write; RELEASE write")
        .unwrap();
    watch.failed();
    commit(&file, &commits, 1);
    let kept = looked_up(&no_file(), &file.writer, &invariants, "http://shop.test/b");
    let again = seed(&file.writer, &invariants, "http://shop.test/a", "3");

    assert!(matches!(kept, Ok(Seen::Visited(visitor)) if visitor == "1"));
    assert!(matches!(again, Enqueued::Added { id, .. } if id == "3"));
}

/// Once memory read every visit the run file holds, a miss reads nothing:
/// a URL no request visited is new, and one the file held is a duplicate,
/// with no run file to read.
#[test]
fn a_miss_reads_nothing_once_memory_holds_every_visit_the_file_holds() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    invariants.fill_memory(&file.reader).unwrap();
    let new = visited(&no_file(), &invariants, "http://shop.test/new");
    let held = visited(&no_file(), &invariants, "http://shop.test/");

    assert!(matches!(new, Ok(Seen::New)));
    assert!(matches!(held, Ok(Seen::Visited(visitor)) if visitor == "0"));
}

/// A run file holding more visits than the budget keeps is read in part:
/// a miss then reads the file.
#[test]
fn a_miss_reads_the_file_when_its_visits_are_past_the_budget() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);

    invariants.fill_memory(&file.reader).unwrap();
    let without_the_file = visited(&no_file(), &invariants, "http://shop.test/new");
    let on_the_file = visited(&file.reader, &invariants, "http://shop.test/new");

    assert!(without_the_file.is_err());
    assert!(matches!(on_the_file, Ok(Seen::New)));
}

/// Once a visit the file holds leaves memory past the budget, memory no
/// longer holds every visit, and a miss reads the file again.
#[test]
fn a_miss_reads_the_file_again_once_a_visit_left_memory() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let two_visits = holds(true, &["http://shop.test/a"]);
    let invariants = within(two_visits, &commits);
    invariants.fill_memory(&file.reader).unwrap();
    let before = visited(&no_file(), &invariants, "http://shop.test/new");

    for (commit_number, (url, id)) in [("http://shop.test/a", "1"), ("http://shop.test/b", "2")]
        .into_iter()
        .enumerate()
    {
        file.writer.execute_batch("BEGIN").unwrap();
        seed(&file.writer, &invariants, url, id);
        commit(&file, &commits, 1 + u64::try_from(commit_number).unwrap());
    }
    let after = visited(&no_file(), &invariants, "http://shop.test/new");

    assert!(matches!(before, Ok(Seen::New)));
    assert!(after.is_err());
}

/// The Run's invariants under `bounds`, holding the visits its writer has
/// not committed against `commits`, within a budget that keeps them all.
fn bounded(bounds: Bounds, commits: &Arc<AtomicU64>) -> Invariants {
    let invariants = Invariants::new(Scope::default(), bounds).unwrap();
    let commits = Arc::clone(commits);
    invariants.hold_unwritten(
        1 << 20,
        move || commits.load(Ordering::SeqCst),
        &Metrics::none(),
    );
    invariants
}

/// `crawl.max_requests` at `max_requests`, and one path budget of one
/// request on `/b/*`.
fn bounds(max_requests: u64) -> Bounds {
    Bounds {
        max_requests: Some(max_requests),
        path_budgets: vec![PathBudget {
            pattern: "/b/*".to_owned(),
            max_requests: 1,
        }],
        ..Bounds::default()
    }
}

/// Admits each of `paths` on the shop as a seed on the writer, its path
/// its request's ID, answering whether each was added or why not.
fn seeds(file: &RunFile, invariants: &Invariants, paths: &[&str]) -> Vec<String> {
    paths
        .iter()
        .map(|path| answer(&seed(&file.writer, invariants, &shop(path), path)))
        .collect()
}

/// Admits each of `paths` on the shop as a seed on the writer, while an
/// attempt stages, against what its earlier calls did, `earlier`;
/// answers what this call did, and whether each was added or why not.
fn staged(
    file: &RunFile,
    invariants: &Invariants,
    earlier: Staged,
    paths: &[&str],
) -> (Staged, Vec<String>) {
    let get = HttpMethod::get();
    let mut admitting = Admitting::Staging {
        earlier: Arc::new(earlier),
        now: Staged::default(),
    };
    let answers = paths
        .iter()
        .map(|path| {
            let url = shop(path);
            let enqueued = admit_in(
                &file.writer,
                invariants,
                &every_host(),
                Candidate::Seed {
                    url: &url,
                    declared: Declared {
                        method: &get,
                        body: None,
                        key: None,
                        keeps_fragment: false,
                    },
                },
                &mut admitting,
                |_| Ok((*path).to_owned()),
            )
            .unwrap();
            answer(&enqueued)
        })
        .collect();
    let Admitting::Staging { now, .. } = admitting else {
        unreachable!("the admissions staged")
    };
    (now, answers)
}

fn shop(path: &str) -> String {
    format!("http://shop.test/{path}")
}

/// `added`, or the reason it was not.
fn answer(enqueued: &Enqueued) -> String {
    match enqueued {
        Enqueued::Added { .. } => "added".to_owned(),
        Enqueued::NotAdded { reason, .. } => format!("{reason:?}"),
    }
}

thread_local! {
    static RAN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record(event: rusqlite::trace::TraceEvent<'_>) {
    if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event {
        RAN.with_borrow_mut(|ran| ran.push(statement.sql().into_owned()));
    }
}

/// What `act` answers, and each statement it ran on `connection` that
/// reads a count of the Bounds.
fn count_reads<T>(connection: &Connection, act: impl FnOnce() -> T) -> (T, Vec<String>) {
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
                && (statement.contains("bounds_admitted")
                    || statement.contains("bounds_path_counts"))
        })
        .collect();
    (answered, reads)
}

/// Memory counts the requests admitted against `crawl.max_requests` and
/// each path budget (#2169): the first admission reads the run file's
/// counts, its seed among them, and none reads them again, while the
/// bounds admit exactly as many as they allow.
#[test]
fn memory_counts_the_requests_admitted_once_it_read_the_files_counts() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = bounded(bounds(4), &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    let (first, read) = count_reads(&file.writer, || seeds(&file, &invariants, &["a"]));
    let (then, read_again) = count_reads(&file.writer, || {
        seeds(&file, &invariants, &["b/1", "b/2", "c", "d"])
    });

    assert_eq!(
        (first, read.len(), then, read_again),
        (
            vec!["added".to_owned()],
            2,
            vec![
                "added".to_owned(),
                "PathBudget".to_owned(),
                "added".to_owned(),
                "MaxRequests".to_owned(),
            ],
            Vec::<String>::new(),
        )
    );
}

/// A writer job whose work failed takes back what it counted, as it takes
/// back what it visited: the next job admits up to the bound again.
#[test]
fn a_failed_job_takes_back_what_it_counted() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = bounded(bounds(3), &commits);
    let watch = invariants.unwritten().unwrap();

    file.writer.execute_batch("BEGIN").unwrap();
    watch.began();
    let kept = seeds(&file, &invariants, &["a"]);
    watch.began();
    file.writer.execute_batch("SAVEPOINT write").unwrap();
    let failed = seeds(&file, &invariants, &["b/1"]);
    file.writer
        .execute_batch("ROLLBACK TO write; RELEASE write")
        .unwrap();
    watch.failed();
    watch.began();
    let again = seeds(&file, &invariants, &["b/2", "c"]);

    assert_eq!(
        (kept, failed, again),
        (
            vec!["added".to_owned()],
            vec!["added".to_owned()],
            vec!["added".to_owned(), "MaxRequests".to_owned()],
        )
    );
}

/// An attempt's staged admissions, rolled back, leave memory's counts as
/// they were, and count against the bounds with them: its earlier calls'
/// and this call's own.
#[test]
fn staged_admissions_count_beside_memory_and_leave_it_unchanged() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = bounded(bounds(5), &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    let written = seeds(&file, &invariants, &["a"]);
    file.writer.execute_batch("SAVEPOINT stage").unwrap();
    let (earlier, first_call) = staged(&file, &invariants, Staged::default(), &["b/1"]);
    file.writer
        .execute_batch("ROLLBACK TO stage; RELEASE stage")
        .unwrap();
    file.writer.execute_batch("SAVEPOINT stage").unwrap();
    let (_, second_call) = staged(&file, &invariants, earlier, &["b/2", "c", "d", "e"]);
    file.writer
        .execute_batch("ROLLBACK TO stage; RELEASE stage")
        .unwrap();
    let after = seeds(&file, &invariants, &["f", "g", "h", "i"]);

    assert_eq!(
        (written, first_call, second_call, after),
        (
            vec!["added".to_owned()],
            vec!["added".to_owned()],
            vec![
                "PathBudget".to_owned(),
                "added".to_owned(),
                "added".to_owned(),
                "MaxRequests".to_owned(),
            ],
            vec![
                "added".to_owned(),
                "added".to_owned(),
                "added".to_owned(),
                "MaxRequests".to_owned(),
            ],
        )
    );
}

/// The Run's invariants under the scope `strategy`, holding the visits and
/// seeds its writer has not committed against `commits`, and keeping those
/// it wrote within `budget` bytes.
fn scoped(strategy: scope::Strategy, budget: u64, commits: &Arc<AtomicU64>) -> Invariants {
    let scope = Scope::new(
        strategy,
        scope::Patterns::default(),
        scope::Patterns::default(),
    );
    let invariants = Invariants::new(scope, Bounds::default()).unwrap();
    let commits = Arc::clone(commits);
    invariants.hold_unwritten(
        budget,
        move || commits.load(Ordering::SeqCst),
        &Metrics::none(),
    );
    invariants
}

/// Whether a seed reaches `url`, as a link's admission asks it, beside the
/// seeds memory holds, read on `connection`.
fn in_scope(connection: &Connection, invariants: &Invariants, url: &str) -> rusqlite::Result<bool> {
    let url = Url::parse(url).unwrap();
    let scope = invariants.scope();
    scope.reached(
        connection,
        &url,
        &scope.asks(&url),
        None,
        invariants.unwritten(),
    )
}

/// A seed a committed batch wrote stays in memory within the budget: a
/// link to its site is in the scope with no run file to read.
#[test]
fn a_seed_a_committed_batch_wrote_is_looked_up_in_memory_within_the_budget() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);

    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &invariants, "http://other.test/", "1");
    commit(&file, &commits, 1);
    let reached = in_scope(&no_file(), &invariants, "http://other.test/a");

    assert!(matches!(reached, Ok(true)));
}

/// Once memory read every seed the run file holds, written there by a Run
/// that held none, a link is measured against memory alone, under each strategy's measure: one a seed reaches
/// is in the scope, and one none reaches is out of it, with no run file to
/// read.
#[rstest::rstest]
#[case::same_hostname_without_www(
    scope::Strategy::SameHostname,
    "http://www.shop2.test/",
    "https://shop2.test:81/x",
    true
)]
#[case::same_hostname_not_a_subdomain(
    scope::Strategy::SameHostname,
    "http://shop2.test/",
    "http://sub.shop2.test/",
    false
)]
#[case::same_domain(
    scope::Strategy::SameDomain,
    "http://a.example.co.uk/",
    "https://b.example.co.uk/x",
    true
)]
#[case::same_domain_not_another(
    scope::Strategy::SameDomain,
    "http://a.example.co.uk/",
    "http://example.com/",
    false
)]
#[case::same_origin_https_of_http(
    scope::Strategy::SameOrigin,
    "http://o.test:8080/",
    "https://o.test:8080/x",
    true
)]
#[case::same_origin_default_ports(
    scope::Strategy::SameOrigin,
    "http://o.test/",
    "https://o.test/x",
    true
)]
#[case::same_origin_not_another_port(
    scope::Strategy::SameOrigin,
    "http://o.test:8080/",
    "https://o.test/x",
    false
)]
#[case::same_origin_not_http_of_https(
    scope::Strategy::SameOrigin,
    "https://o.test/",
    "http://o.test/x",
    false
)]
fn memory_measures_a_link_against_every_seed_the_file_holds(
    #[case] strategy: scope::Strategy,
    #[case] seeded: &str,
    #[case] link: &str,
    #[case] reached: bool,
) {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = scoped(strategy, 1 << 20, &commits);
    let file_only = Scope::new(
        strategy,
        scope::Patterns::default(),
        scope::Patterns::default(),
    );
    let file_only = Invariants::new(file_only, Bounds::default()).unwrap();
    seed(&file.writer, &file_only, seeded, "1");
    invariants.fill_memory(&file.reader).unwrap();

    let answer = in_scope(&no_file(), &invariants, link);

    assert_eq!(answer.ok(), Some(reached));
}

/// Past the budget, the seeds the oldest written batches widened the scope
/// with leave memory first, with their visits: a link to such a site is
/// read from the run file and is in the scope, and a site no seed reaches
/// is read from it too and is out of the scope.
#[test]
fn past_the_budget_a_seed_that_left_memory_is_read_from_the_file() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let two_seed_sites = holds(false, &["http://a.test/", "http://b.test/"]);
    let invariants = within(two_seed_sites, &commits);

    for (commit_number, (url, id)) in [
        ("http://a.test/", "1"),
        ("http://b.test/", "2"),
        ("http://c.test/", "3"),
    ]
    .into_iter()
    .enumerate()
    {
        file.writer.execute_batch("BEGIN").unwrap();
        seed(&file.writer, &invariants, url, id);
        commit(&file, &commits, 1 + u64::try_from(commit_number).unwrap());
    }
    let from_memory = (
        in_scope(&no_file(), &invariants, "http://a.test/x").ok(),
        in_scope(&no_file(), &invariants, "http://c.test/x").ok(),
    );
    let from_the_file = (
        in_scope(&file.writer, &invariants, "http://a.test/x").ok(),
        in_scope(&file.writer, &invariants, "http://elsewhere.test/").ok(),
    );

    assert_eq!(from_memory, (None, Some(true)));
    assert_eq!(from_the_file, (Some(true), Some(false)));
    assert!(invariants.unwritten().unwrap().bytes() <= two_seed_sites);
}

/// At a budget of 0, a seed is held while its batch is not committed, so
/// the scope is never behind the writer, and leaves memory once the file
/// holds it.
#[test]
fn at_a_budget_of_0_a_seed_is_held_until_its_batch_commits() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);

    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &invariants, "http://other.test/", "1");
    let before_the_commit = in_scope(&no_file(), &invariants, "http://other.test/a");
    commit(&file, &commits, 1);
    let after_the_commit = in_scope(&no_file(), &invariants, "http://other.test/a");

    assert!(matches!(before_the_commit, Ok(true)));
    assert!(after_the_commit.is_err());
    assert_eq!(invariants.unwritten().unwrap().bytes(), 0);
}

/// A writer job whose work failed leaves no seed in memory, and an
/// attempt's staged seed, rolled back, never joins it: with every seed
/// the file holds in memory, a link to either site is out of the scope.
#[test]
fn a_seed_the_file_never_holds_never_joins_memory() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = within(1 << 20, &commits);
    invariants.fill_memory(&file.reader).unwrap();
    let watch = invariants.unwritten().unwrap();
    let get = HttpMethod::get();

    file.writer.execute_batch("BEGIN").unwrap();
    watch.began();
    file.writer.execute_batch("SAVEPOINT write").unwrap();
    seed(&file.writer, &invariants, "http://failed.test/", "1");
    file.writer
        .execute_batch("ROLLBACK TO write; RELEASE write")
        .unwrap();
    watch.failed();
    watch.began();
    file.writer.execute_batch("SAVEPOINT stage").unwrap();
    let mut admitting = Admitting::Staging {
        earlier: Arc::new(Staged::default()),
        now: Staged::default(),
    };
    let staged = admit_in(
        &file.writer,
        &invariants,
        &every_host(),
        Candidate::Seed {
            url: "http://staged.test/",
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        },
        &mut admitting,
        |_| Ok("2".to_owned()),
    )
    .unwrap();
    file.writer
        .execute_batch("ROLLBACK TO stage; RELEASE stage")
        .unwrap();
    commit(&file, &commits, 1);

    assert_eq!(answer(&staged), "added");
    assert_eq!(
        (
            in_scope(&no_file(), &invariants, "http://failed.test/a").ok(),
            in_scope(&no_file(), &invariants, "http://staged.test/a").ok(),
        ),
        (Some(false), Some(false))
    );
}

mod learned;
mod rules;
