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
#[expect(
    clippy::needless_pass_by_value,
    reason = "trace_v2 takes a function of a TraceEvent by value"
)]
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

/// Each Record `commit` writes counts its serialization, once, and neither
/// a copy of its data nor a lookup of a value constant for the Run: it
/// takes the hash of the schema it was checked against, the one its
/// Dataset's row holds.
#[test]
fn a_commit_counts_each_records_work_and_a_commit_of_none_counts_nothing() {
    use goodfetch_counted_work::{CountedWork, Row};
    use goodfetch_engine::mask::Masks;
    use goodfetch_protocol::foxhound::crawl::PushedRecord;
    use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
    use goodfetch_protocol::foxhound::provenance::RecordProvenance;
    use goodfetch_protocol::run::PointInTime;
    use serde_json::json;

    let env = TestEnv::new();
    let connection = run_file(&env.home().join("run"));
    let datasets = crate::Datasets::declare(&[DatasetDeclaration {
        name: "pages".to_owned(),
        schema: Some(json!({ "type": "object", "required": ["title"] })),
        key: Vec::new(),
    }])
    .unwrap();
    datasets.record(&connection).unwrap();
    let work = CountedWork::default();
    let records: Vec<PushedRecord> = (0..2)
        .map(|_| PushedRecord {
            dataset: "pages".to_owned(),
            data: json!({ "title": "Dune" }),
        })
        .collect();
    let checked = datasets.check(records, &work).unwrap();
    let (at, masks) = (
        PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned()),
        Masks::new(),
    );
    let provenance = RecordProvenance {
        request_id: "9".to_owned(),
        tier: None,
        encoding: None,
        origin: None,
        clean_up_rules: None,
        wall: None,
        payment: None,
        certificate_exception: None,
        robots_txt: None,
        value_transforms: Vec::new(),
        unread_fields: Vec::new(),
    };
    let committing = super::Committing {
        request_id: "9",
        url: "https://shop.test/",
        at: &at,
        provenance: &provenance,
        masks: &masks,
        work: &work,
    };
    let read = || {
        [
            Row::RecordDeepCopies,
            Row::RecordSerializations,
            Row::RecordConstantLookups,
        ]
        .map(|row| work.read(row))
    };

    super::commit(&connection, committing, &crate::Checked::default()).unwrap();
    assert_eq!(read(), [0, 0, 0]);

    super::commit(&connection, committing, &checked).unwrap();
    let committed: Vec<(String, Option<String>, Option<String>)> = connection
        .prepare(
            "SELECT data, schema_hash, (SELECT schema_hash FROM datasets WHERE name = 'pages')
             FROM records WHERE work_item = '9' ORDER BY number",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // The SHA-256 of `{"required":["title"],"type":"object"}`.
    let hash = Some("7db5dcaef9c7f2b340069704aea676e028b6c80dee2a0902cf163794b547f1d1".to_owned());
    assert_eq!(
        (read(), committed),
        (
            [0, 2, 0],
            vec![(r#"{"title":"Dune"}"#.to_owned(), hash.clone(), hash); 2]
        )
    );
}
