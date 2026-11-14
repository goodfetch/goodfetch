//! The visits a batching writer has not committed, read beside the run
//! file by a reader (#2168): a reader's hop sees them, an admission on the
//! writer sees a chain's pending end, and each is forgotten once the file
//! holds it or its visit is dropped unrun.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_engine::egress::Policy;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::WriteWatch;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use rusqlite::Connection;
use tempfile::TempDir;
use url::Url;

use crate::admit::{Candidate, admit};
use crate::bounds::{self, Bounds};
use crate::identity::{
    self, Declared, Fixed, Hop, Key, Reached, Seen, admit_hop, admit_reached, fixed_only,
    hold_reached, redirect_hop, seen, visit_pending,
};
use crate::invariants::Invariants;
use crate::scope::{self, Scope};

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
        invariants,
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
    visit_pending(&file.writer, &invariants, pending).unwrap();
    let on_the_writer = admit_reached(
        &file.writer,
        &invariants,
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
    let probe = within(1 << 20, &commits);
    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &probe, "http://shop.test/p", "0");
    commit(&file, &commits, 1);
    let one_visit = probe.unwritten().unwrap().bytes();
    let invariants = within(2 * one_visit, &commits);

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
        commit(&file, &commits, 2 + u64::try_from(commit_number).unwrap());
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
    assert!(invariants.unwritten().unwrap().bytes() <= 2 * one_visit);
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

    invariants
        .unwritten()
        .unwrap()
        .read_visits(&file.reader)
        .unwrap();
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

    invariants
        .unwritten()
        .unwrap()
        .read_visits(&file.reader)
        .unwrap();
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
    let probe = within(1 << 20, &commits);
    probe
        .unwritten()
        .unwrap()
        .read_visits(&file.reader)
        .unwrap();
    let one_visit = probe.unwritten().unwrap().bytes();
    let invariants = within(2 * one_visit, &commits);
    invariants
        .unwritten()
        .unwrap()
        .read_visits(&file.reader)
        .unwrap();
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
