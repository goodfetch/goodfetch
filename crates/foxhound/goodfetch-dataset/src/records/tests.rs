#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use goodfetch_protocol::point_in_time;
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

/// The Provenance of the test attempt `9`'s Records.
fn provenance() -> RecordProvenance {
    RecordProvenance {
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
    }
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
/// Dataset's row holds. It writes a Record's fields in the order they were
/// pushed (issue 2394).
#[test]
fn a_commit_counts_each_records_work_and_a_commit_of_none_counts_nothing() {
    use goodfetch_counted_work::{CountedWork, Row};
    use goodfetch_engine::mask::Masks;
    use goodfetch_protocol::foxhound::crawl::PushedRecord;
    use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
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
            data: serde_json::from_str(r#"{ "title": "Dune", "price": 9 }"#).unwrap(),
        })
        .collect();
    let checked = datasets.check(records, &work).unwrap();
    let (at, masks) = (
        point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap(),
        Masks::new(),
    );
    let provenance = provenance();
    let committing = super::Committing {
        request_id: "9",
        url: "https://shop.test/",
        at: &at,
        provenance: &provenance,
        masks: &masks,
        work: &work,
        index: true,
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
            vec![
                (
                    r#"{"title":"Dune","price":9}"#.to_owned(),
                    hash.clone(),
                    hash
                );
                2
            ]
        )
    );
}

/// A commit with indexing on writes each Record's values into
/// `record_search`, where any of its words finds it, whatever the script,
/// case, normalization or the invisible marks around it; with indexing
/// off, it writes no index row.
///
/// Covers: story:12-208, setting:dataset.search.index
#[test]
fn a_commit_indexes_each_records_words_only_while_indexing_is_on() {
    use goodfetch_counted_work::CountedWork;
    use goodfetch_engine::mask::Masks;
    use goodfetch_protocol::foxhound::crawl::PushedRecord;
    use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};
    use goodfetch_test_support::unicode_record;
    use serde_json::{Map, Value};

    let env = TestEnv::new();
    let connection = run_file(&env.home().join("run"));
    let datasets = crate::Datasets::declare(&[DatasetDeclaration {
        name: "pages".to_owned(),
        schema: None,
        key: Vec::new(),
    }])
    .unwrap();
    datasets.record(&connection).unwrap();
    let work = CountedWork::default();
    let unicode = Value::Object(
        unicode_record()
            .fields
            .into_iter()
            .map(|field| (field.name, Value::String(field.text)))
            .collect::<Map<_, _>>(),
    );
    let (at, masks) = (
        point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap(),
        Masks::new(),
    );
    let provenance = provenance();
    for (value, index) in [
        (unicode, true),
        (serde_json::json!({ "title": "Dune", "pages": 412 }), false),
    ] {
        let checked = datasets
            .check(
                vec![PushedRecord {
                    dataset: "pages".to_owned(),
                    data: RecordData::of(&value).unwrap(),
                }],
                &work,
            )
            .unwrap();
        let committing = super::Committing {
            request_id: "9",
            url: "https://shop.test/",
            at: &at,
            provenance: &provenance,
            masks: &masks,
            work: &work,
            index,
        };
        super::commit(&connection, committing, &checked).unwrap();
    }
    let found = |words: &str| -> Vec<(String, i64)> {
        connection
            .prepare("SELECT dataset, number FROM record_search WHERE record_search MATCH ?1")
            .unwrap()
            .query_map([words], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let words = [
        "שלום",
        "world",
        "مرحبا",
        "עברית",
        "end",
        "café",
        "cafe\u{301}",
        "CAFE",
        "leads",
        "\"byte order\"",
        "three",
        "Dune",
        "412",
    ];

    assert_eq!(
        words.map(|words| (words, found(words))),
        words.map(|words| {
            let unicode = !matches!(words, "Dune" | "412");
            (
                words,
                if unicode {
                    // After the run file's Records 1 and 2.
                    vec![("pages".to_owned(), 3)]
                } else {
                    Vec::new()
                },
            )
        }),
        "each word of the indexed Record finds it, and none of the Record \
         committed with indexing off is indexed"
    );
}

#[test]
fn prune_acknowledged_removes_only_records_every_sink_acknowledged_past_retention() {
    use std::time::{Duration, UNIX_EPOCH};

    use goodfetch_counted_work::CountedWork;
    use goodfetch_engine::mask::Masks;
    use goodfetch_protocol::foxhound::crawl::PushedRecord;
    use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};

    let connection = Connection::open_in_memory().unwrap();
    for statement in crate::TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    let datasets = crate::Datasets::declare(&[DatasetDeclaration {
        name: "pages".to_owned(),
        schema: None,
        key: Vec::new(),
    }])
    .unwrap();
    datasets.record(&connection).unwrap();

    let t0 = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let t1 = t0 + Duration::from_secs(3_600);
    let t2 = t0 + Duration::from_secs(10_000);
    let work = CountedWork::default();
    let masks = Masks::new();
    let provenance = provenance();
    let push = |req: &str, at: std::time::SystemTime, title: &str| {
        let checked = datasets
            .check(
                vec![PushedRecord {
                    dataset: "pages".to_owned(),
                    data: RecordData::of(&serde_json::json!({ "title": title })).unwrap(),
                }],
                &work,
            )
            .unwrap();
        super::commit(
            &connection,
            super::Committing {
                request_id: req,
                url: "https://shop.test/",
                at: &at,
                provenance: &provenance,
                masks: &masks,
                work: &work,
                index: true,
            },
            &checked,
        )
        .unwrap();
    };
    push("r1", t0, "alpha");
    push("r2", t0, "beta");
    push("r3", t2, "gamma");

    let read_all = || {
        super::read(
            &connection,
            Query {
                dataset: "pages",
                after: 0,
                through: None,
                wall: None,
            },
            10,
        )
        .unwrap()
    };

    // With no Sink registered, nothing is ever pruned even past retention.
    assert_eq!(super::prune_acknowledged(&connection, t2).unwrap(), 0);
    assert_eq!(read_all().len(), 3);

    // Two Sinks registered; only one acknowledges through 2 -> min_ack is 0 -> nothing pruned.
    super::register_sink(&connection, "pages", "warehouse").unwrap();
    super::register_sink(&connection, "pages", "archive").unwrap();
    super::acknowledge_sink(&connection, "pages", "warehouse", 2).unwrap();
    assert_eq!(super::prune_acknowledged(&connection, t1).unwrap(), 0);

    // Second Sink acknowledges through 1 -> only Record 1 (at t0 <= t1) is pruned.
    super::acknowledge_sink(&connection, "pages", "archive", 1).unwrap();
    assert_eq!(super::prune_acknowledged(&connection, t1).unwrap(), 1);

    // Second Sink acknowledges through 3 -> Record 2 (t0 <= t1) pruned; Record 3 (t2 > t1) kept.
    super::acknowledge_sink(&connection, "pages", "archive", 3).unwrap();
    assert_eq!(super::prune_acknowledged(&connection, t1).unwrap(), 1);
    let remaining = read_all();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].number, 3);

    // Even past t2, warehouse has only acknowledged through 2, so Record 3 stays.
    assert_eq!(super::prune_acknowledged(&connection, t2).unwrap(), 0);

    // Once warehouse acknowledges 3 as well, Record 3 is pruned; a new commit takes number 4 (never 1).
    super::acknowledge_sink(&connection, "pages", "warehouse", 3).unwrap();
    assert_eq!(super::prune_acknowledged(&connection, t2).unwrap(), 1);
    push("r4", t2, "delta");
    let after = read_all();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].number, 4);
}
