use std::cell::RefCell;
use std::path::Path;

use goodfetch_test_support::TestEnv;
use rusqlite::trace::{TraceEvent, TraceEventCodes};
use rusqlite::{Connection, OpenFlags};

use super::{Query, read_through};

/// The `pages` Record numbered `number`, as the run file keeps it.
const INSERT: &str = "INSERT INTO records VALUES ('pages', ?1, ?1, 'https://shop.test/', '2026-01-05T09:00:00Z', '{}', NULL, '{}')";

thread_local! {
    /// The writer that commits `pages`' next Record once the traced reader
    /// of this thread starts its second statement over `records`, and how
    /// many it started.
    static WRITER: RefCell<Option<(Connection, u32)>> = const { RefCell::new(None) };
}

/// Commits the Record numbered 3 when the reader's second statement over
/// `records` begins, before that statement reads anything.
fn commit_before_the_second_read(event: TraceEvent<'_>) {
    let TraceEvent::Stmt(_, sql) = event else {
        return;
    };
    if sql.starts_with("--") || !sql.contains("FROM records") {
        return;
    }
    WRITER.with_borrow_mut(|writer| {
        let (connection, started) = writer.as_mut().expect("the test set a writer");
        *started += 1;
        if *started == 2 {
            connection.execute(INSERT, [3]).unwrap();
        }
    });
}

/// A run file in WAL mode at `file` holding `pages`' Records 1 and 2, and
/// its writer.
fn run_file(file: &Path) -> Connection {
    let writer = Connection::open(file).unwrap();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test's own new run file, which no other connection opens yet"
    )]
    writer
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .unwrap();
    for statement in crate::TABLES.statements {
        writer.execute(statement, []).unwrap();
    }
    for number in [1, 2] {
        writer.execute(INSERT, [number]).unwrap();
    }
    writer
}

/// A read through a number answers a page and whether that number has
/// committed from one state of the run file: a Record committed between
/// its statements is in neither or in both, so the read never says the
/// bound committed while its page lacks the bound's Record.
///
/// Covers: story:6-271
#[test]
fn a_read_through_a_number_answers_its_page_and_whether_it_committed_from_one_state() {
    let env = TestEnv::in_memory();
    let file = env.folder().join("run.sqlite");
    let writer = run_file(&file);
    let reader = Connection::open_with_flags(
        &file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    WRITER.with_borrow_mut(|slot| *slot = Some((writer, 0)));
    reader.trace_v2(
        TraceEventCodes::SQLITE_TRACE_STMT,
        Some(commit_before_the_second_read),
    );

    let (page, committed) = read_through(
        &reader,
        Query {
            dataset: "pages",
            after: 2,
            through: Some(3),
            wall: None,
        },
        10,
    )
    .unwrap();
    reader.trace_v2(TraceEventCodes::empty(), None);
    let numbers: Vec<u64> = page.iter().map(|record| record.number).collect();
    let started = WRITER.with_borrow_mut(|slot| slot.take().map(|(_, started)| started));

    assert_eq!(
        (started, (numbers.as_slice(), committed)),
        (Some(2), (&[][..], false)),
        "the Record committed between the read's statements is in its page \
         exactly when the read says it committed"
    );
}
