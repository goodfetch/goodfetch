use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use serde_json::{Value, json};

use super::Datasets;

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

fn pushed(dataset: &str, data: Value) -> PushedRecord {
    PushedRecord {
        dataset: dataset.to_owned(),
        data,
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
        .check(vec![pushed("books", json!({ "title": "Dune" }))], &work)
        .unwrap();
    assert_eq!(counted(&work), (0, 1));

    datasets
        .check_pushed(pushed("books", json!({ "title": "Emma" })), &work)
        .unwrap();
    assert_eq!(counted(&work), (0, 2));

    datasets
        .check_pushed(pushed("free", json!({ "title": "Kim" })), &work)
        .unwrap();
    assert_eq!(counted(&work), (0, 2));
}

/// A refused push's rejection names the issues its one check found: the
/// Record is validated once, refused or not (the audit's F4-1).
#[test]
fn a_refused_push_validates_its_record_once() {
    let work = CountedWork::default();

    let refused = declared().check_pushed(pushed("books", json!({})), &work);

    assert!(refused.is_err());
    assert_eq!(counted(&work), (0, 1));
}
