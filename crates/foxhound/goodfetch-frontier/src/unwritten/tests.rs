//! The visits a batching writer has not committed, read beside the run
//! file by a reader (#2168): a reader's hop sees them, an admission on the
//! writer sees a chain's pending end, and each is forgotten once the file
//! holds it or its visit is dropped unrun.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_engine::egress::Policy;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use rusqlite::Connection;
use tempfile::TempDir;
use url::Url;

use super::Reached;
use crate::admit::{Candidate, admit};
use crate::bounds::{self, Bounds};
use crate::identity::{self, Declared, Hop, admit_hop, admit_reached, redirect_hop};
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
/// against `commits`, the commits it acknowledged.
fn holding(commits: &Arc<AtomicU64>) -> Invariants {
    let invariants = Invariants::new(Scope::default(), Bounds::default()).unwrap();
    let commits = Arc::clone(commits);
    invariants.hold_unwritten(move || commits.load(Ordering::SeqCst));
    invariants
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
    invariants
        .unwritten()
        .unwrap()
        .hold_reached(
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
    let still_held = invariants
        .unwritten()
        .unwrap()
        .lock()
        .visitor(&format!(
            "GET {}",
            identity::url_key(&Url::parse("http://shop.test/a").unwrap())
        ))
        .map(str::to_owned);

    assert_eq!(
        (from_the_file_alone, beside_memory, committed, still_held),
        (Hop::Follow, Hop::Duplicate, Hop::Duplicate, None)
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
    super::visit_pending(&file.writer, &invariants, pending).unwrap();
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
