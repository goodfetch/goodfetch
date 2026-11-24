use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Datasets, Declarations};

fn data(value: &Value) -> RecordData {
    RecordData::of(value).unwrap()
}

/// The Datasets of a crawl declaring `books`, under a schema requiring
/// `title`, and `free`, under none.
fn declared() -> Datasets {
    Datasets::declare(&[
        DatasetDeclaration {
            name: "books".to_owned(),
            schema: Some(json!({ "type": "object", "required": ["title"] })),
            key: Vec::new(),
        },
        DatasetDeclaration {
            name: "free".to_owned(),
            schema: None,
            key: Vec::new(),
        },
    ])
    .unwrap()
}

fn pushed(dataset: &str, value: &Value) -> PushedRecord {
    PushedRecord {
        dataset: dataset.to_owned(),
        data: data(value),
    }
}

/// The validator builds and the validations `work` counted.
fn counted(work: &CountedWork) -> (u64, u64) {
    (
        work.read(Row::RecordValidatorBuilds),
        work.read(Row::RecordValidations),
    )
}

/// F4's check validates once on both of its ways in, under a validator built
/// when the crawl starts, and a Dataset with no schema validates nothing;
/// building each schema's validator, once as the crawl starts, is no Record's
/// work (ADR 0138).
#[test]
fn a_records_check_validates_only_under_a_schema_and_builds_no_validator() {
    let datasets = declared();
    let work = CountedWork::default();
    assert_eq!(counted(&work), (0, 0));

    datasets
        .check(vec![pushed("books", &json!({ "title": "Dune" }))], &work)
        .unwrap();
    assert_eq!(counted(&work), (0, 1));

    datasets
        .check_pushed("books", vec![data(&json!({ "title": "Emma" }))], &work)
        .unwrap();
    assert_eq!(counted(&work), (0, 2));

    datasets
        .check_pushed("free", vec![data(&json!({ "title": "Kim" }))], &work)
        .unwrap();
    assert_eq!(counted(&work), (0, 2));
}

/// A refused push's rejection names the issues its one check found: each
/// Record of a push is validated once, refused or not (the audit's F4-1),
/// with no validator built.
#[test]
fn a_refused_push_validates_each_of_its_records_once() {
    let work = CountedWork::default();

    let refused = declared().check_pushed(
        "books",
        vec![
            data(&json!({ "title": "Emma", "isbn": "1" })),
            data(&json!({})),
            data(&json!({ "title": "Kim", "isbn": "3" })),
        ],
        &work,
    );

    assert!(refused.is_err());
    assert_eq!(counted(&work), (0, 3));
}

/// A run file no crawl wrote holds no Dataset's table, so it declares no
/// Dataset (as `dataset.list` answers), and a read of any Dataset in it
/// is refused with `dataset_unknown`, not as a Run whose state cannot be
/// read.
#[test]
fn a_run_file_no_crawl_wrote_declares_no_dataset() {
    let unwritten = Connection::open_in_memory().unwrap();

    let declared = Declarations::read(&unwritten).unwrap();

    assert_eq!(
        (
            declared.declarations().len(),
            declared.readable("books").unwrap_err().code()
        ),
        (0, "dataset_unknown")
    );
}
